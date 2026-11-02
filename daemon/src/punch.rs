//! Hole punching assist: short-lived (low TTL) UDP packets from iroh's own
//! socket toward a peer's public addresses.
//!
//! Why: when our first packets reach the peer's NAT before the peer has sent
//! anything to us, that NAT (or the router behind it) can answer with an ICMP
//! error, and our own NAT drops the mapping we just created on seeing it. Then
//! the peer's packets never get in, even though both NATs map consistently.
//! A packet with a TTL of a few hops opens our NAT for the peer's address and
//! dies in the carrier's network before it reaches the peer: no ICMP error,
//! the mapping stays, and iroh's own punching from the peer side gets through.
//! Measured on an LTE hotspot -> home (both behind carrier NAT), raw UDP:
//! both at TTL 64 never connects; LTE at TTL 5 and home at 64 connects at
//! once; home at TTL 5 or 3 and LTE at 64 never does. The same trick HyperDHT
//! uses (its client punches with low TTL, its server sends first).

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::fd::RawFd;

/// Hops a punch packet lives: enough to leave our own NATs (hotspot, carrier),
/// short of the peer's
pub const PUNCH_TTL: i32 = 5;



/// iroh's IPv4 UDP sockets in this process, found by their local port
pub fn iroh_sockets(ports: &[u16]) -> Vec<RawFd> {
    let Ok(dir) = std::fs::read_dir("/proc/self/fd") else { return vec![] };
    dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<RawFd>().ok())
        .filter(|&fd| match local_v4(fd) {
            Some((port, true)) => ports.contains(&port),
            _ => false,
        })
        .collect()
}

/// (port, is UDP) for an IPv4 socket fd
fn local_v4(fd: RawFd) -> Option<(u16, bool)> {
    // SAFETY: plain getsockname/getsockopt on an fd we own; buffers are sized
    unsafe {
        let mut sa: libc::sockaddr_in = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        if libc::getsockname(fd, &mut sa as *mut _ as *mut libc::sockaddr, &mut len) != 0
            || sa.sin_family != libc::AF_INET as libc::sa_family_t
        {
            return None;
        }
        let mut ty: libc::c_int = 0;
        let mut tl = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        if libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_TYPE, &mut ty as *mut _ as *mut libc::c_void, &mut tl) != 0 {
            return None;
        }
        Some((u16::from_be(sa.sin_port), ty == libc::SOCK_DGRAM))
    }
}

/// Sends one tiny packet with the given TTL (per packet, through a control
/// message: the socket's own TTL is untouched)
pub fn send_with_ttl(fd: RawFd, dst: SocketAddrV4, ttl: i32) -> std::io::Result<()> {
    // SAFETY: sendmsg with stack buffers that outlive the call
    unsafe {
        let mut sa: libc::sockaddr_in = std::mem::zeroed();
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_port = dst.port().to_be();
        sa.sin_addr.s_addr = u32::from(*dst.ip()).to_be();
        let mut payload = [0u8; 1];
        let mut iov = libc::iovec { iov_base: payload.as_mut_ptr() as *mut libc::c_void, iov_len: 1 };
        let space = libc::CMSG_SPACE(std::mem::size_of::<libc::c_int>() as u32) as usize;
        let mut cbuf = vec![0u8; space];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_name = &mut sa as *mut _ as *mut libc::c_void;
        msg.msg_namelen = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = space as _;
        let c = libc::CMSG_FIRSTHDR(&msg);
        (*c).cmsg_level = libc::IPPROTO_IP;
        (*c).cmsg_type = libc::IP_TTL;
        (*c).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(c) as *mut libc::c_int, ttl);
        if libc::sendmsg(fd, &msg, libc::MSG_DONTWAIT) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Public IPv4 addresses worth punching toward (not LAN, CGNAT or loopback)
pub fn punch_targets(addrs: &[SocketAddr]) -> Vec<SocketAddrV4> {
    addrs
        .iter()
        .filter_map(|a| match a {
            SocketAddr::V4(v4) if is_public(*v4.ip()) => Some(*v4),
            _ => None,
        })
        .collect()
}

fn is_public(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast()
        || ip.is_multicast() || (o[0] == 100 && (o[1] & 0xc0) == 64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pun_roundtrip_and_ready() {
        let t: Vec<SocketAddrV4> = vec!["45.1.2.3:9".parse().unwrap(), "45.1.2.4:10".parse().unwrap()];
        let m = encode_pun(Duration::from_millis(2500), &t);
        assert_eq!(decode_pun(&m), Some((Duration::from_millis(2500), t.clone())));
        assert_eq!(decode_pun(b"\0PUN\0\0"), None);
        let now = Instant::now();
        let mut p = PeerPunch { since: Some(now - Duration::from_secs(3)), ..Default::default() };
        p.peer_open = Duration::from_secs(3);
        p.peer_heard = Some(now);
        p.peer_targets = vec![t[0]];
        assert!(p.ready(&[t[0]], now));
        assert!(!p.ready(&[t[1]], now), "the peer must aim at our current address");
        p.peer_open = Duration::from_secs(1);
        assert!(!p.ready(&[t[0]], now), "the peer must have opened its NAT long enough");
    }

    #[test]
    fn low_ttl_packet_from_an_existing_socket() {
        let a = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        b.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let port = a.local_addr().unwrap().port();
        let fds = iroh_sockets(&[port]);
        assert_eq!(fds.len(), 1, "finds the socket by port");
        let SocketAddr::V4(dst) = b.local_addr().unwrap() else { unreachable!() };
        send_with_ttl(fds[0], dst, PUNCH_TTL).unwrap();
        let mut buf = [0u8; 8];
        let (n, from) = b.recv_from(&mut buf).unwrap();
        assert_eq!((n, from.port()), (1, port), "sent from that socket");
        assert_eq!(punch_targets(&["192.168.1.2:1".parse().unwrap(), "45.1.2.3:9".parse().unwrap()]).len(), 1);
    }
}

// ---------------------------------------------------------------------------
// Coordinated punching. Measured (LTE <-> home, and reproduced with plain
// Linux NAT in tests/netns-nat.sh): if a full-size packet reaches the other
// side's NAT before that side has sent anything toward us, the NAT treats the
// flow as aimed at itself, and the other device's own packets then get a
// different public port, so everything aimed at the port it advertised
// misses. iroh's probes are full size and go out as soon as a round starts.
// So: hold iroh's full-size packets to a relayed peer's public address (a
// firewall rule on this device, which our 1-byte punch packets pass), open our
// NATs with low-TTL packets, tell the peer over the relay connection, and
// release when both sides have been at it for a moment.
// ---------------------------------------------------------------------------

use std::time::{Duration, Instant};

/// Both sides must have opened their NATs this long before iroh may probe
pub const OPEN_BEFORE_RELEASE: Duration = Duration::from_secs(2);
/// No direct path this long after a release: start over
pub const RELEASE_RETRY: Duration = Duration::from_secs(20);

/// What we know about punching toward one peer
#[derive(Default, Debug)]
pub struct PeerPunch {
    /// The peer's public addresses we punch toward, and since when
    pub targets: Vec<SocketAddrV4>,
    pub since: Option<Instant>,
    /// From the peer's last PUN message: how long it has punched, toward which
    /// addresses (its view of ours), and when we heard it
    pub peer_open: Duration,
    pub peer_targets: Vec<SocketAddrV4>,
    pub peer_heard: Option<Instant>,
    pub released: Option<Instant>,
    pub last_pun: Option<Instant>,
}

impl PeerPunch {
    /// Starts over when the peer's public addresses change: hold them, and
    /// open our NATs toward them from now on
    pub fn track(&mut self, hold: &mut Option<Hold>, targets: &[SocketAddrV4], now: Instant) -> bool {
        if self.targets == targets {
            return false;
        }
        self.restart(hold, targets, now);
        true
    }

    pub fn restart(&mut self, hold: &mut Option<Hold>, targets: &[SocketAddrV4], now: Instant) {
        if let Some(h) = hold.as_mut() {
            for t in &self.targets {
                h.release(*t);
            }
            for t in targets {
                h.hold(*t);
            }
        }
        self.targets = targets.to_vec();
        self.since = Some(now);
        self.released = None;
    }

    /// Stops punching (direct path, or nothing to punch toward)
    pub fn stop(&mut self, hold: &mut Option<Hold>) {
        if let Some(h) = hold.as_mut() {
            for t in &self.targets {
                h.release(*t);
            }
        }
        *self = Default::default();
    }

    /// Whether iroh may now probe this peer's public addresses
    pub fn ready(&self, ours: &[SocketAddrV4], now: Instant) -> bool {
        let opened = self.since.is_some_and(|s| now - s >= OPEN_BEFORE_RELEASE);
        let heard = self.peer_heard.is_some_and(|h| now - h < Duration::from_secs(3));
        let aimed_at_us = self.peer_targets.iter().any(|t| ours.contains(t));
        opened && heard && self.peer_open >= OPEN_BEFORE_RELEASE && aimed_at_us
    }
}

/// `\0PUN`, u32 ms we've been opening our NAT, u8 count, then the addresses
/// we punch toward (ip, port)
pub fn encode_pun(open: Duration, targets: &[SocketAddrV4]) -> Vec<u8> {
    let mut m = b"\0PUN".to_vec();
    m.extend_from_slice(&(open.as_millis().min(u32::MAX as u128) as u32).to_be_bytes());
    let targets = &targets[..targets.len().min(8)];
    m.push(targets.len() as u8);
    for t in targets {
        m.extend_from_slice(&t.ip().octets());
        m.extend_from_slice(&t.port().to_be_bytes());
    }
    m
}

pub fn decode_pun(m: &[u8]) -> Option<(Duration, Vec<SocketAddrV4>)> {
    let rest = m.strip_prefix(b"\0PUN")?;
    let open = Duration::from_millis(u32::from_be_bytes(rest.get(0..4)?.try_into().ok()?) as u64);
    let n = *rest.get(4)? as usize;
    let addrs = rest.get(5..5 + n * 6)?;
    let targets = addrs
        .chunks(6)
        .map(|c| SocketAddrV4::new(Ipv4Addr::new(c[0], c[1], c[2], c[3]), u16::from_be_bytes([c[4], c[5]])))
        .collect();
    Some((open, targets))
}

/// The firewall hold: drops iroh's full-size UDP packets to held addresses
/// while the flow has had no reply (conntrack state new). Our 1-byte punch
/// packets pass, and so does everything once the peer's packets arrive.
pub struct Hold {
    held: std::collections::BTreeSet<SocketAddrV4>,
}

const HOLD_TABLE: &str = "toolnet_punch";

impl Hold {
    pub fn new(iroh_ports: &[u16]) -> anyhow::Result<Self> {
        let ports = iroh_ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        let _ = crate::os::nft(&format!("delete table inet {HOLD_TABLE}"));
        crate::os::nft(&format!(
            "table inet {HOLD_TABLE} {{
                set ports {{ type inet_service; elements = {{ {ports} }} }}
                set hold {{ type ipv4_addr . inet_service; }}
                chain out {{
                    type filter hook output priority 0; policy accept;
                    udp sport @ports ip daddr . udp dport @hold ct state new udp length > 32 drop
                }}
            }}"
        ))?;
        tracing::info!(ports = %ports, "hole punching: holding iroh's first probes until both NATs are open");
        Ok(Self { held: Default::default() })
    }

    pub fn hold(&mut self, a: SocketAddrV4) {
        if self.held.insert(a) {
            let _ = crate::os::nft(&format!("add element inet {HOLD_TABLE} hold {{ {} . {} }}", a.ip(), a.port()));
        }
    }

    pub fn release(&mut self, a: SocketAddrV4) {
        if self.held.remove(&a) {
            let _ = crate::os::nft(&format!("delete element inet {HOLD_TABLE} hold {{ {} . {} }}", a.ip(), a.port()));
        }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let _ = crate::os::nft(&format!("delete table inet {HOLD_TABLE}"));
    }
}
