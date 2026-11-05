//! The owner app, served by the daemon itself (embedded by build.rs), so the
//! owner key lives in the storage of a private origin, http://127.0.0.1:PORT,
//! instead of the GitHub Pages one that other sites share. The members of the
//! network can open it at this node's tailnet address too (http, but inside
//! the encrypted tunnel); the LAN and the internet are refused.
//!
//! Static files only, no state, no API: a few GETs per page load.

use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use anyhow::{Context, Result};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};
use toolnet::ipc::DaemonStatus;
use tracing::{debug, info, warn};

include!(concat!(env!("OUT_DIR"), "/owner_app.rs"));

pub const DEFAULT_PORT: u16 = 7477;

/// Whether this build has the app (app/dist existed when it was built)
pub fn available() -> bool {
    !FILES.is_empty()
}

pub async fn bind(port: u16) -> Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await.with_context(|| format!("port {port}"))
}

pub async fn serve(listener: TcpListener, status: watch::Receiver<DaemonStatus>) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    info!("owner app: http://127.0.0.1:{port}/ (members: this node's tailnet address)");
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                warn!("owner app: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if !allowed(peer.ip(), &status.borrow()) {
            debug!("owner app: refused {peer}");
            continue;
        }
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(10), handle(stream)).await;
        });
    }
}

/// This machine, and the members of our network while the tunnel is up. A
/// spoofed member address can't finish the handshake: our answer goes into
/// the tunnel, to the real member.
fn allowed(peer: IpAddr, s: &DaemonStatus) -> bool {
    let peer = match peer {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(peer, IpAddr::V4),
        v4 => v4,
    };
    if peer.is_loopback() {
        return true;
    }
    let IpAddr::V4(v4) = peer else { return false };
    s.connected && s.state == "member" && (s.ip == Some(v4) || s.peers.iter().any(|p| p.ip == v4))
}

async fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    // The request head; the body (there is none for GET) is ignored
    let mut buf = vec![0; 8192];
    let mut n = 0;
    while !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
        if n == buf.len() {
            return Ok(());
        }
        match stream.read(&mut buf[n..]).await? {
            0 => return Ok(()),
            r => n += r,
        }
    }
    let head = String::from_utf8_lossy(&buf[..n]);
    let mut words = head.split_ascii_whitespace();
    let (method, target) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    let (code, ctype, body): (&str, &str, &[u8]) = if method != "GET" && method != "HEAD" {
        ("405 Method Not Allowed", "text/plain", b"GET only\n")
    } else {
        match file(target) {
            Some((path, body)) => ("200 OK", content_type(path), body),
            None => ("404 Not Found", "text/plain", b"not found\n"),
        }
    };
    let head = format!(
        "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\
         X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: frame-ancestors 'none'\r\n\
         Referrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    if method != "HEAD" {
        stream.write_all(body).await?;
    }
    stream.shutdown().await
}

/// The embedded file for a request target ("/" is index.html)
fn file(target: &str) -> Option<(&'static str, &'static [u8])> {
    let path = target.split(['?', '#']).next()?.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    FILES.iter().find(|(p, _)| *p == path).copied()
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toolnet::ipc::PeerStatus;

    #[test]
    fn who_may_load_it() {
        let mut s = DaemonStatus { connected: true, state: "member".into(), ip: Some([10, 213, 0, 2].into()), ..Default::default() };
        s.peers.push(PeerStatus {
            name: "phone".into(),
            ip: [10, 213, 0, 3].into(),
            dns: String::new(),
            online: true,
            path: None,
            rtt_ms: None,
            exit_node: false,
            routes: vec![],
        });
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(allowed(ip("127.0.0.1"), &s));
        assert!(allowed(ip("::1"), &s));
        assert!(allowed(ip("10.213.0.3"), &s));
        assert!(allowed(ip("::ffff:10.213.0.3"), &s));
        assert!(!allowed(ip("10.213.0.9"), &s), "not a member");
        assert!(!allowed(ip("192.168.1.20"), &s), "the LAN");
        s.connected = false;
        assert!(!allowed(ip("10.213.0.3"), &s), "tunnel down");
        assert!(allowed(ip("127.0.0.1"), &s));
    }

    #[test]
    fn paths() {
        if !available() {
            return;
        }
        assert_eq!(file("/").unwrap().0, "index.html");
        assert_eq!(file("/?x=1").unwrap().0, "index.html");
        assert!(file("/../etc/passwd").is_none());
        assert!(file("/nope.js").is_none());
    }
}
