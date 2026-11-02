//! MagicDNS: a tiny authoritative DNS server for `<member>.<domain>`, served on
//! our tailnet IP (port 53). With systemd-resolved it is registered as split
//! DNS for the tailnet domain only, so other queries never come here.

use std::{collections::HashMap, net::Ipv4Addr, sync::Arc};

use anyhow::{Context, Result};
use tokio::{net::UdpSocket, sync::RwLock};
use tracing::{debug, info, warn};

/// Name (lowercase, no trailing dot) -> address
#[derive(Clone, Default)]
pub struct Names(Arc<RwLock<(String, HashMap<String, Ipv4Addr>)>>);

/// A member name as a DNS label: lowercase letters, digits and dashes
pub fn label(name: &str) -> String {
    let l: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    l.trim_matches('-').to_string()
}

impl Names {
    pub async fn set(&self, domain: &str, members: impl IntoIterator<Item = (String, Ipv4Addr)>) {
        let domain = domain.trim_end_matches('.').to_lowercase();
        let map = members.into_iter().map(|(n, ip)| (format!("{}.{domain}", label(&n)), ip)).collect();
        *self.0.write().await = (domain, map);
    }

    async fn answer(&self, query: &[u8]) -> Option<Vec<u8>> {
        let (name, qtype, end) = parse_question(query)?;
        let (domain, map) = &*self.0.read().await;
        let in_domain = name == *domain || name.ends_with(&format!(".{domain}"));
        let (rcode, ip) = match map.get(&name) {
            Some(ip) if qtype == 1 || qtype == 255 => (0, Some(*ip)),
            Some(_) => (0, None),                // known name, no record of that type
            None if in_domain => (3, None),      // NXDOMAIN
            None => (5, None),                   // REFUSED: not our zone
        };
        debug!(%name, qtype, rcode, "dns query");
        Some(build_response(query, end, rcode, ip))
    }
}

/// (name, qtype, end of the question section)
fn parse_question(q: &[u8]) -> Option<(String, u16, usize)> {
    if q.len() < 12 || q[2] & 0x80 != 0 || u16::from_be_bytes([q[4], q[5]]) != 1 {
        return None; // not a query with exactly one question
    }
    let mut pos = 12;
    let mut labels = vec![];
    loop {
        let len = *q.get(pos)? as usize;
        pos += 1;
        if len == 0 {
            break;
        }
        if len > 63 {
            return None; // compression isn't used in questions
        }
        labels.push(String::from_utf8_lossy(q.get(pos..pos + len)?).to_lowercase());
        pos += len;
    }
    let qtype = u16::from_be_bytes([*q.get(pos)?, *q.get(pos + 1)?]);
    Some((labels.join("."), qtype, pos + 4))
}

fn build_response(q: &[u8], question_end: usize, rcode: u8, ip: Option<Ipv4Addr>) -> Vec<u8> {
    let mut r = Vec::with_capacity(question_end + 16);
    r.extend_from_slice(&q[0..2]); // id
    r.push(0x84 | (q[2] & 0x01)); // QR, AA, keep RD
    r.push(rcode & 0x0f); // RA=0, rcode
    r.extend_from_slice(&[0, 1, 0, ip.is_some() as u8, 0, 0, 0, 0]);
    r.extend_from_slice(&q[12..question_end]);
    if let Some(ip) = ip {
        r.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]); // name ptr, A, IN, TTL 60
        r.extend_from_slice(&ip.octets());
    }
    r
}

pub async fn serve(ip: Ipv4Addr, names: Names) -> Result<()> {
    let sock = UdpSocket::bind((ip, 53))
        .await
        .with_context(|| format!("binding DNS on {ip}:53 (needs CAP_NET_BIND_SERVICE)"))?;
    info!("MagicDNS on {ip}:53");
    let mut buf = [0u8; 1500];
    loop {
        let (n, from) = sock.recv_from(&mut buf).await?;
        if let Some(resp) = names.answer(&buf[..n]).await {
            let _ = sock.send_to(&resp, from).await;
        }
    }
}

/// Registers the resolver with systemd-resolved as split DNS for the domain
pub fn register_resolved(dev: &str, ip: Ipv4Addr, domain: &str) {
    let run = |args: &[&str]| crate::os::command("resolvectl").args(args).output();
    let ok = |o: std::io::Result<std::process::Output>| o.map(|o| o.status.success()).unwrap_or(false);
    if ok(run(&["dns", dev, &ip.to_string()])) && ok(run(&["domain", dev, &format!("~{domain}")])) {
        info!("systemd-resolved: *.{domain} -> {ip}");
    } else {
        warn!("couldn't register with systemd-resolved (not running, or no permission): names resolve with `dig @{ip} <name>.{domain}`");
    }
}

pub fn unregister_resolved(dev: &str) {
    let _ = crate::os::command("resolvectl").args(["revert", dev]).output();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(name: &str, qtype: u16) -> Vec<u8> {
        let mut q = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        for l in name.split('.') {
            q.push(l.len() as u8);
            q.extend_from_slice(l.as_bytes());
        }
        q.push(0);
        q.extend_from_slice(&qtype.to_be_bytes());
        q.extend_from_slice(&[0, 1]);
        q
    }

    #[tokio::test]
    async fn answers() {
        let names = Names::default();
        names.set("home.toolnet", [("My Server".to_string(), Ipv4Addr::new(10, 213, 0, 2))]).await;
        let r = names.answer(&query("my-server.home.toolnet", 1)).await.unwrap();
        assert_eq!(&r[0..2], &[0x12, 0x34]);
        assert_eq!(r[3] & 0x0f, 0);
        assert_eq!(&r[r.len() - 4..], &[10, 213, 0, 2]);
        let r = names.answer(&query("nope.home.toolnet", 1)).await.unwrap();
        assert_eq!(r[3] & 0x0f, 3);
        let r = names.answer(&query("example.com", 1)).await.unwrap();
        assert_eq!(r[3] & 0x0f, 5);
        let r = names.answer(&query("My-Server.HOME.toolnet", 28)).await.unwrap();
        assert_eq!((r[3] & 0x0f, r[7]), (0, 0)); // AAAA: no data, no error
    }
}
