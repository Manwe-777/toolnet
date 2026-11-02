//! Data plane: an iroh endpoint plus (optionally) a TUN device. IPv4 packets
//! travel between peers as QUIC datagrams. The peer set is driven by the
//! control plane and can change at any time.

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey, TransportAddr,
    endpoint::{Connection, presets},
};
use tokio::{sync::RwLock, task::JoinHandle};
use tracing::{debug, info, warn};
use tun_rs::{AsyncDevice, DeviceBuilder};

pub const ALPN: &[u8] = b"toolnet/0";
/// Below the smallest QUIC datagram size we expect on any path (relay included)
const TUN_MTU: u16 = 1150;

/// An IPv4 prefix
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cidr {
    pub addr: Ipv4Addr,
    pub len: u8,
}

impl Cidr {
    pub fn host(ip: Ipv4Addr) -> Self {
        Self { addr: ip, len: 32 }
    }

    fn mask(&self) -> u32 {
        u32::MAX.checked_shl(32 - self.len as u32).unwrap_or(0)
    }

    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        u32::from(ip) & self.mask() == u32::from(self.addr) & self.mask()
    }
}

impl FromStr for Cidr {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let (ip, len) = s.split_once('/').unwrap_or((s, "32"));
        let len: u8 = len.parse()?;
        anyhow::ensure!(len <= 32, "bad prefix length in {s}");
        let addr: Ipv4Addr = ip.parse()?;
        // Normalize: 192.168.1.7/24 -> 192.168.1.0/24
        let net = Self { addr, len };
        Ok(Self { addr: Ipv4Addr::from(u32::from(addr) & net.mask()), len })
    }
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.len)
    }
}

/// A peer as the control plane describes it
#[derive(Debug, Clone, PartialEq)]
pub struct Peer {
    pub name: String,
    pub ip: Ipv4Addr,
    pub endpoint: EndpointId,
    pub addrs: Vec<SocketAddr>,
    pub relay: Option<RelayUrl>,
    /// Extra prefixes routed to this peer: approved by the owner AND
    /// advertised by the peer (subnet router, or 0.0.0.0/0 for an exit node)
    pub routes: Vec<Cidr>,
}

impl Peer {
    /// Whether the peer may send us packets from this source address
    fn may_send_from(&self, src: Ipv4Addr) -> bool {
        src == self.ip || self.routes.iter().any(|r| r.contains(src))
    }
}

impl Peer {
    fn endpoint_addr(&self) -> EndpointAddr {
        EndpointAddr::from_parts(
            self.endpoint,
            self.addrs
                .iter()
                .copied()
                .map(TransportAddr::Ip)
                .chain(self.relay.iter().cloned().map(TransportAddr::Relay)),
        )
    }
}

pub struct EngineConfig {
    pub secret: SecretKey,
    pub offline: bool,
    pub relays: Vec<RelayUrl>,
    pub bind: Option<SocketAddr>,
}

#[derive(Default)]
struct State {
    /// Desired peers by endpoint id
    peers: HashMap<EndpointId, Peer>,
    /// Live connections by endpoint id
    conns: HashMap<EndpointId, Connection>,
    /// Dialer tasks (we only dial peers with a higher id than ours)
    dialers: HashMap<EndpointId, (Peer, JoinHandle<()>)>,
    /// Prefix -> endpoint id, longest prefix first
    routes: Vec<(Cidr, EndpointId)>,
}

impl State {
    fn route(&self, dst: Ipv4Addr) -> Option<&Connection> {
        let (_, id) = self.routes.iter().find(|(c, _)| c.contains(dst))?;
        self.conns.get(id)
    }
}

#[derive(Clone)]
pub struct Engine {
    pub endpoint: Endpoint,
    state: Arc<RwLock<State>>,
    tun: Arc<RwLock<Option<Arc<AsyncDevice>>>>,
}

impl Engine {
    pub async fn start(cfg: EngineConfig) -> Result<Self> {
        // No address lookup service: peers' addresses come from tool-db.
        // (n0's pkarr/DNS lookup is blocked on some networks, see spike/README.md)
        let mut builder = Endpoint::builder(presets::Minimal);
        builder = if cfg.offline {
            builder.relay_mode(RelayMode::Disabled)
        } else if !cfg.relays.is_empty() {
            builder.relay_mode(RelayMode::custom(cfg.relays.clone()))
        } else {
            builder.relay_mode(RelayMode::Default)
        };
        builder = builder.secret_key(cfg.secret).alpns(vec![ALPN.to_vec()]);
        if let Some(bind) = cfg.bind {
            builder = builder.clear_ip_transports().bind_addr(bind)?;
        }
        let endpoint = builder.bind().await?;
        if !cfg.offline {
            // Don't block forever without internet; the relay connects later
            let _ = tokio::time::timeout(Duration::from_secs(10), endpoint.online()).await;
        }
        let engine = Self {
            endpoint,
            state: Default::default(),
            tun: Default::default(),
        };
        engine.spawn_accept();
        engine.spawn_path_report();
        Ok(engine)
    }

    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// Creates the TUN device and starts forwarding packets to peers
    pub async fn start_tun(&self, name: &str, ip: Ipv4Addr, prefix: u8) -> Result<()> {
        let dev = DeviceBuilder::new()
            .name(name)
            .ipv4(ip, prefix, None)
            .mtu(TUN_MTU)
            .build_async()
            .with_context(|| format!("creating TUN {name} (needs CAP_NET_ADMIN)"))?;
        let dev = Arc::new(dev);
        *self.tun.write().await = Some(dev.clone());
        info!("tun {name} up: {ip}/{prefix}, mtu {TUN_MTU}");
        let state = self.state.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            loop {
                let n = match dev.recv(&mut buf).await {
                    Ok(n) => n,
                    Err(e) => return warn!("tun read failed: {e}"),
                };
                let packet = &buf[..n];
                if packet.len() < 20 || packet[0] >> 4 != 4 {
                    continue; // IPv4 only for now
                }
                let dst = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
                let state = state.read().await;
                let Some(conn) = state.route(dst) else {
                    continue;
                };
                if let Err(e) = conn.send_datagram(bytes::Bytes::copy_from_slice(packet)) {
                    debug!(%dst, "send failed: {e}");
                }
            }
        });
        Ok(())
    }

    /// Applies the peer set from the control plane: starts dialing new peers,
    /// drops removed ones, updates routes and addresses
    pub async fn set_peers(&self, peers: Vec<Peer>) {
        let me = self.id();
        let mut state = self.state.write().await;
        let wanted: HashMap<EndpointId, Peer> = peers.into_iter().map(|p| (p.endpoint, p)).collect();

        // Removed peers: stop dialing, close connections
        let removed: Vec<EndpointId> =
            state.peers.keys().filter(|id| !wanted.contains_key(*id)).copied().collect();
        for id in removed {
            let name = state.peers.get(&id).map(|p| p.name.clone()).unwrap_or_default();
            info!(peer = %name, "peer removed");
            if let Some((_, task)) = state.dialers.remove(&id) {
                task.abort();
            }
            if let Some(conn) = state.conns.remove(&id) {
                conn.close(2u32.into(), b"removed");
            }
        }

        // New or changed peers: (re)start dialers for the ones we dial
        for (id, peer) in &wanted {
            if me > *id {
                continue; // the lower id dials
            }
            let unchanged = state.dialers.get(id).is_some_and(|(p, _)| p == peer);
            if unchanged {
                continue;
            }
            if let Some((_, task)) = state.dialers.remove(id) {
                task.abort();
            }
            let task = self.spawn_dialer(peer.clone());
            state.dialers.insert(*id, (peer.clone(), task));
        }

        let mut routes: Vec<(Cidr, EndpointId)> = wanted
            .values()
            .flat_map(|p| {
                std::iter::once((Cidr::host(p.ip), p.endpoint)).chain(p.routes.iter().map(|r| (*r, p.endpoint)))
            })
            .collect();
        routes.sort_by(|a, b| b.0.len.cmp(&a.0.len).then(a.0.cmp(&b.0)));
        state.routes = routes;
        state.peers = wanted;
    }

    fn spawn_dialer(&self, peer: Peer) -> JoinHandle<()> {
        let engine = self.clone();
        tokio::spawn(async move {
            info!(peer = %peer.name, "dialing");
            loop {
                match engine.endpoint.connect(peer.endpoint_addr(), ALPN).await {
                    Ok(conn) => engine.run_conn(conn).await,
                    Err(e) => debug!(peer = %peer.name, "connect failed: {e:#}"),
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        })
    }

    fn spawn_accept(&self) {
        let engine = self.clone();
        tokio::spawn(async move {
            while let Some(incoming) = engine.endpoint.accept().await {
                let engine = engine.clone();
                tokio::spawn(async move {
                    let conn = match async { anyhow::Ok(incoming.accept()?.await?) }.await {
                        Ok(conn) => conn,
                        Err(e) => return debug!("incoming failed: {e:#}"),
                    };
                    let known = engine.state.read().await.peers.contains_key(&conn.remote_id());
                    if !known {
                        warn!(peer = %conn.remote_id().fmt_short(), "rejecting unknown endpoint");
                        return conn.close(1u32.into(), b"not a member");
                    }
                    engine.run_conn(conn).await;
                });
            }
        });
    }

    /// Registers a connection and pipes its datagrams into the TUN until it
    /// closes. Without a TUN, pings the peer every second and answers pings.
    async fn run_conn(&self, conn: Connection) {
        let id = conn.remote_id();
        let name = self.peer_name(&id).await;
        info!(peer = %name, "connected");
        self.state.write().await.conns.insert(id, conn.clone());
        let start = Instant::now();
        let pinger = {
            let conn = conn.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tick.tick().await;
                    let mut msg = b"\0PNG".to_vec();
                    msg.extend_from_slice(&(start.elapsed().as_micros() as u64).to_be_bytes());
                    if conn.send_datagram(msg.into()).is_err() {
                        break;
                    }
                }
            })
        };
        let mut pongs = 0u64;
        loop {
            let packet = match conn.read_datagram().await {
                Ok(p) => p,
                Err(e) => {
                    info!(peer = %name, "connection closed: {e}");
                    break;
                }
            };
            // Control messages start with a zero byte, which no IP packet does
            if packet.first() == Some(&0) {
                if packet.starts_with(b"\0PNG") {
                    let mut pong = packet.to_vec();
                    pong[..4].copy_from_slice(b"\0PON");
                    let _ = conn.send_datagram(pong.into());
                } else if packet.starts_with(b"\0PON") && packet.len() == 12 {
                    pongs += 1;
                    let sent = u64::from_be_bytes(packet[4..12].try_into().unwrap());
                    let rtt = start.elapsed().as_micros() as u64 - sent;
                    if pongs == 1 || pongs % 30 == 0 {
                        info!(peer = %name, "ping {} {:.1} ms", path_kind(&conn), rtt as f64 / 1000.0);
                    }
                }
                continue;
            }
            // Anti-spoofing: a peer may only send from its own tailnet IP or
            // the prefixes it routes
            if packet.len() >= 20 && packet[0] >> 4 == 4 {
                let src = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
                let allowed = self.state.read().await.peers.get(&id).is_some_and(|p| p.may_send_from(src));
                if !allowed {
                    debug!(peer = %name, %src, "dropping packet with a source this peer may not use");
                    continue;
                }
            } else {
                continue;
            }
            let tun = self.tun.read().await.clone();
            if let Some(tun) = tun {
                if let Err(e) = tun.send(&packet).await {
                    debug!("tun write failed: {e}");
                }
            }
        }
        pinger.abort();
        let mut state = self.state.write().await;
        if state.conns.get(&id).is_some_and(|c| c.stable_id() == conn.stable_id()) {
            state.conns.remove(&id);
        }
    }

    async fn peer_name(&self, id: &EndpointId) -> String {
        self.state
            .read()
            .await
            .peers
            .get(id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| id.fmt_short().to_string())
    }

    /// Logs path changes (relay <-> direct) per peer
    fn spawn_path_report(&self) {
        let engine = self.clone();
        tokio::spawn(async move {
            let mut last: HashMap<EndpointId, String> = HashMap::new();
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let state = engine.state.read().await;
                for (id, conn) in &state.conns {
                    let now = path_desc(conn);
                    if last.get(id) != Some(&now) {
                        let name = state.peers.get(id).map(|p| p.name.as_str()).unwrap_or("?");
                        info!(peer = %name, "path {now}");
                        last.insert(*id, now);
                    }
                }
            }
        });
    }

    /// Prefixes routed to peers, beyond their own tailnet IPs (for OS routes)
    pub async fn routed_prefixes(&self) -> Vec<Cidr> {
        let state = self.state.read().await;
        let mut out: Vec<Cidr> = state.peers.values().flat_map(|p| p.routes.iter().copied()).collect();
        out.sort();
        out.dedup();
        out
    }

    /// Status lines for each desired peer
    pub async fn status(&self) -> Vec<String> {
        let state = self.state.read().await;
        let mut lines: Vec<String> = state
            .peers
            .values()
            .map(|p| {
                let conn = state.conns.get(&p.endpoint).map(path_desc);
                format!("{:<12} {:<15} {}", p.name, p.ip, conn.unwrap_or_else(|| "not connected".into()))
            })
            .collect();
        lines.sort();
        lines
    }
}

fn path_kind(conn: &Connection) -> &'static str {
    conn.paths()
        .iter()
        .find(|p| p.is_selected())
        .map(|p| if p.is_relay() { "RELAY" } else { "DIRECT" })
        .unwrap_or("?")
}

fn path_desc(conn: &Connection) -> String {
    conn.paths()
        .iter()
        .find(|p| p.is_selected())
        .map(|p| {
            let kind = if p.is_relay() { "RELAY" } else { "DIRECT" };
            format!("{kind} {:?} rtt {:?}", p.remote_addr(), p.rtt())
        })
        .unwrap_or_else(|| "no path".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr() {
        let c: Cidr = "192.168.77.9/24".parse().unwrap();
        assert_eq!(c.to_string(), "192.168.77.0/24");
        assert!(c.contains("192.168.77.200".parse().unwrap()));
        assert!(!c.contains("192.168.78.1".parse().unwrap()));
        let all: Cidr = "0.0.0.0/0".parse().unwrap();
        assert!(all.contains("8.8.8.8".parse().unwrap()));
        assert_eq!("10.0.0.1".parse::<Cidr>().unwrap().len, 32);
    }
}

pub fn parse_endpoint_id(s: &str) -> Result<EndpointId> {
    EndpointId::from_str(s).map_err(|e| anyhow::anyhow!("bad endpoint id {s}: {e}"))
}
