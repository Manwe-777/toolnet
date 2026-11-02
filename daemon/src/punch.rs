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

/// Seconds before the low / full TTL roles swap between the two sides.
/// Measured LTE hotspot -> home: home must send at full TTL, LTE at low TTL;
/// the other way around never connects
pub const PUNCH_PHASE_SECS: u64 = 15;

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
