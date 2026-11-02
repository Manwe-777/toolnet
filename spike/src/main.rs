//! toolnet connectivity spike: a TUN device whose IP packets travel between
//! peers as QUIC datagrams over iroh (direct after hole punching, or relayed).
//!
//! No control plane yet: peers and their tailnet IPs come from the command
//! line. In toolnet they will come from tool-db (see ../docs/data-model.md).

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey, TransportAddr,
    endpoint::{Connection, presets},
};
use tokio::sync::RwLock;
use tracing::{info, warn};
use tun_rs::{AsyncDevice, DeviceBuilder};

const ALPN: &[u8] = b"toolnet/spike/0";
/// Below the smallest QUIC datagram size we expect over any path (relay included)
const TUN_MTU: u16 = 1150;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a secret key file (if missing) and print its endpoint id
    Keygen { key: PathBuf },
    /// Bring up the TUN device and connect to the peers
    Up {
        /// Secret key file, created by `keygen`
        #[arg(long)]
        key: PathBuf,
        /// Our tailnet IPv4 address
        #[arg(long)]
        ip: Ipv4Addr,
        /// Tailnet prefix length. Default range in docs: 10.213.0.0/24, outside
        /// Tailscale's 100.64.0.0/10 so both can run side by side
        #[arg(long, default_value_t = 24)]
        prefix: u8,
        /// Peers, as ENDPOINT_ID=TAILNET_IP (repeatable)
        #[arg(long = "peer", value_parser = parse_peer)]
        peers: Vec<(EndpointId, Ipv4Addr)>,
        /// Known direct address of a peer, as ENDPOINT_ID=IP:PORT (repeatable)
        #[arg(long = "peer-addr", value_parser = parse_peer_addr)]
        peer_addrs: Vec<(EndpointId, SocketAddr)>,
        /// UDP address to bind (default: random port)
        #[arg(long)]
        bind: Option<SocketAddr>,
        /// Relay to use instead of n0's public ones (repeatable). Peers are assumed
        /// to share it, so it is also used to dial them without an address lookup
        #[arg(long = "relay")]
        relays: Vec<RelayUrl>,
        /// No relays and no address lookup: direct addresses only (offline tests)
        #[arg(long)]
        offline: bool,
        /// TUN device name
        #[arg(long, default_value = "toolnet0")]
        dev: String,
    },
}

fn parse_peer(s: &str) -> Result<(EndpointId, Ipv4Addr)> {
    let (id, ip) = s.split_once('=').context("expected ENDPOINT_ID=IP")?;
    Ok((id.parse()?, ip.parse()?))
}

fn parse_peer_addr(s: &str) -> Result<(EndpointId, SocketAddr)> {
    let (id, addr) = s.split_once('=').context("expected ENDPOINT_ID=IP:PORT")?;
    Ok((id.parse()?, addr.parse()?))
}

fn load_or_create_key(path: &PathBuf) -> Result<SecretKey> {
    if let Ok(hex) = std::fs::read_to_string(path) {
        let bytes: [u8; 32] = data_encoding::HEXLOWER
            .decode(hex.trim().as_bytes())?
            .try_into()
            .map_err(|_| anyhow::anyhow!("key file must hold 32 hex bytes"))?;
        return Ok(SecretKey::from_bytes(&bytes));
    }
    let key = SecretKey::generate();
    std::fs::write(path, data_encoding::HEXLOWER.encode(&key.to_bytes()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

/// Destination IPv4 of an IP packet (IPv6 is not routed in the spike)
fn dst_ipv4(packet: &[u8]) -> Option<Ipv4Addr> {
    if packet.len() >= 20 && packet[0] >> 4 == 4 {
        Some(Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]))
    } else {
        None
    }
}

/// Live connections, by peer
type Conns = Arc<RwLock<HashMap<EndpointId, Connection>>>;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "toolnet_spike=info,iroh=warn".into()),
        )
        .init();

    match Cli::parse().cmd {
        Cmd::Keygen { key } => {
            let key = load_or_create_key(&key)?;
            println!("{}", key.public());
            Ok(())
        }
        Cmd::Up { key, ip, prefix, peers, peer_addrs, relays, bind, offline, dev } => {
            up(load_or_create_key(&key)?, ip, prefix, peers, peer_addrs, relays, bind, offline, dev).await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn up(
    key: SecretKey,
    ip: Ipv4Addr,
    prefix: u8,
    peers: Vec<(EndpointId, Ipv4Addr)>,
    peer_addrs: Vec<(EndpointId, SocketAddr)>,
    relays: Vec<RelayUrl>,
    bind: Option<SocketAddr>,
    offline: bool,
    dev: String,
) -> Result<()> {
    let me = key.public();
    let mut builder = if offline {
        Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
    } else {
        Endpoint::builder(presets::N0)
    };
    if !offline && !relays.is_empty() {
        builder = builder.relay_mode(RelayMode::custom(relays.clone()));
    }
    builder = builder.secret_key(key).alpns(vec![ALPN.to_vec()]);
    if let Some(bind) = bind {
        builder = builder.clear_ip_transports().bind_addr(bind)?;
    }
    let endpoint = builder.bind().await?;
    if !offline {
        endpoint.online().await;
    }
    let addr = endpoint.addr();
    info!(id = %me, "endpoint up");
    for a in addr.ip_addrs() {
        info!("  direct: {a}");
    }
    for r in addr.relay_urls() {
        info!("  relay:  {r}");
    }

    let tun = Arc::new(
        DeviceBuilder::new()
            .name(&dev)
            .ipv4(ip, prefix, None)
            .mtu(TUN_MTU)
            .build_async()
            .with_context(|| format!("creating TUN {dev} (needs CAP_NET_ADMIN)"))?,
    );
    info!("tun {dev} up: {ip}/{prefix}, mtu {TUN_MTU}");

    let routes: Arc<HashMap<Ipv4Addr, EndpointId>> =
        Arc::new(peers.iter().map(|(id, ip)| (*ip, *id)).collect());
    let allowed: Arc<HashMap<EndpointId, Ipv4Addr>> = Arc::new(peers.iter().copied().collect());
    let conns: Conns = Default::default();

    // Accept connections from known peers only
    {
        let (endpoint, conns, tun, allowed) =
            (endpoint.clone(), conns.clone(), tun.clone(), allowed.clone());
        tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let (conns, tun, allowed) = (conns.clone(), tun.clone(), allowed.clone());
                tokio::spawn(async move {
                    let conn = match async { anyhow::Ok(incoming.accept()?.await?) }.await {
                        Ok(conn) => conn,
                        Err(e) => return warn!("incoming failed: {e:#}"),
                    };
                    let peer = conn.remote_id();
                    if !allowed.contains_key(&peer) {
                        warn!(%peer, "rejecting unknown peer");
                        return conn.close(1u32.into(), b"unknown peer");
                    }
                    run_conn(peer, conn, conns, tun).await;
                });
            }
        });
    }

    // Dial the peers. To avoid two connections per pair, the lower id dials
    for (peer, peer_ip) in peers {
        if me > peer {
            info!(%peer, %peer_ip, "waiting for peer to dial us");
            continue;
        }
        let direct: Vec<SocketAddr> =
            peer_addrs.iter().filter(|(id, _)| *id == peer).map(|(_, a)| *a).collect();
        let (endpoint, conns, tun, relays) =
            (endpoint.clone(), conns.clone(), tun.clone(), relays.clone());
        tokio::spawn(async move {
            loop {
                let addr = EndpointAddr::from_parts(
                    peer,
                    direct
                        .iter()
                        .copied()
                        .map(TransportAddr::Ip)
                        .chain(relays.iter().cloned().map(TransportAddr::Relay)),
                );
                match endpoint.connect(addr, ALPN).await {
                    Ok(conn) => run_conn(peer, conn, conns.clone(), tun.clone()).await,
                    Err(e) => warn!(%peer, "connect failed: {e:#}"),
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    // Path report
    {
        let conns = conns.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                for (peer, conn) in conns.read().await.iter() {
                    for path in conn.paths().iter().filter(|p| p.is_selected()) {
                        let kind = if path.is_relay() { "RELAY" } else { "DIRECT" };
                        info!(
                            peer = %peer.fmt_short(),
                            "path {kind} {:?} rtt {:?} max_datagram {:?}",
                            path.remote_addr(),
                            path.rtt(),
                            conn.max_datagram_size()
                        );
                    }
                }
            }
        });
    }

    // TUN -> peers
    let mut buf = vec![0u8; 65536];
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            n = tun.recv(&mut buf) => {
                let packet = &buf[..n?];
                let Some(dst) = dst_ipv4(packet) else { continue };
                let Some(peer) = routes.get(&dst) else { continue };
                if let Some(conn) = conns.read().await.get(peer) {
                    if let Err(e) = conn.send_datagram(bytes::Bytes::copy_from_slice(packet)) {
                        warn!(%dst, "send failed: {e}");
                    }
                }
            }
        }
    }
    endpoint.close().await;
    Ok(())
}

/// Registers a connection and pipes its datagrams into the TUN until it closes
async fn run_conn(peer: EndpointId, conn: Connection, conns: Conns, tun: Arc<AsyncDevice>) {
    info!(peer = %peer.fmt_short(), "connected");
    conns.write().await.insert(peer, conn.clone());
    loop {
        match conn.read_datagram().await {
            Ok(packet) => {
                if let Err(e) = tun.send(&packet).await {
                    warn!("tun write failed: {e}");
                }
            }
            Err(e) => {
                warn!(peer = %peer.fmt_short(), "connection closed: {e}");
                break;
            }
        }
    }
    let mut conns = conns.write().await;
    if conns.get(&peer).is_some_and(|c| c.stable_id() == conn.stable_id()) {
        conns.remove(&peer);
    }
}
