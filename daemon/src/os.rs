//! Host network configuration (Linux, through `ip`, `nft` and /proc).
//! Needs CAP_NET_ADMIN, like the TUN device itself.

use std::{collections::BTreeSet, process::Command};

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

use crate::engine::Cidr;

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(cmd).args(args).output().with_context(|| format!("running {cmd}"))?;
    if !out.status.success() {
        bail!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// toolnet's routing table: subnet routes from peers, plus the default route
/// while an exit node is in use. It is consulted after the main table (minus
/// the main default route), so a network the machine is directly on, like the
/// home LAN when you're home, keeps being reached directly.
const TABLE: &str = "5213";
const PREF_UDP: u32 = 5200;
const PREF_BYPASS_FIRST: u32 = 5201;
const PREF_SUPPRESS: u32 = 5290;
const PREF_TUNNEL: u32 = 5291;

fn flush_policy() {
    for pref in [PREF_UDP, PREF_BYPASS_FIRST, PREF_SUPPRESS, PREF_TUNNEL] {
        while run("ip", &["rule", "del", "pref", &pref.to_string()]).is_ok() {}
    }
    let _ = run("ip", &["route", "flush", "table", TABLE]);
}

/// Keeps the routes for peer prefixes (subnet routes) pointing at the TUN, in
/// toolnet's table
pub struct Routes {
    dev: String,
    installed: BTreeSet<Cidr>,
}

impl Routes {
    pub fn new(dev: &str) -> Result<Self> {
        flush_policy();
        run("ip", &["rule", "add", "pref", &PREF_SUPPRESS.to_string(), "lookup", "main", "suppress_prefixlength", "0"])?;
        run("ip", &["rule", "add", "pref", &PREF_TUNNEL.to_string(), "lookup", TABLE])?;
        Ok(Self { dev: dev.to_string(), installed: BTreeSet::new() })
    }

    pub fn sync(&mut self, wanted: &[Cidr]) {
        let dev = self.dev.as_str();
        let wanted: BTreeSet<Cidr> = wanted.iter().copied().filter(|c| c.len > 0).collect();
        for old in self.installed.difference(&wanted) {
            match run("ip", &["route", "del", &old.to_string(), "dev", dev, "table", TABLE]) {
                Ok(()) => info!(route = %old, "route removed"),
                Err(e) => warn!("removing route {old}: {e:#}"),
            }
        }
        for new in wanted.difference(&self.installed) {
            match run("ip", &["route", "replace", &new.to_string(), "dev", dev, "table", TABLE]) {
                Ok(()) => info!(route = %new, "route added"),
                Err(e) => warn!("adding route {new}: {e:#}"),
            }
            if local_route(new) {
                info!(route = %new, "also a local network here: reached directly while on it");
            }
        }
        self.installed = wanted;
    }

    /// Removes every rule and route toolnet added
    pub fn teardown(self) {
        flush_policy();
        info!("routing table {TABLE} and rules removed");
    }
}

/// Whether the main table has a route for exactly this prefix (it wins)
fn local_route(c: &Cidr) -> bool {
    Command::new("ip")
        .args(["route", "show", "table", "main", "exact", &c.to_string()])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

const NFT_TABLE: &str = "toolnet";

/// Makes this machine a router for the tailnet: IP forwarding, and source NAT
/// (masquerade) for tailnet traffic leaving through any other interface, so
/// hosts on the routed networks need no route back to the tailnet
pub fn enable_router(dev: &str, tailnet: &Cidr) -> Result<()> {
    if std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward").map(|s| s.trim() == "1").unwrap_or(false) {
        info!("IP forwarding already on");
    } else {
        std::fs::write("/proc/sys/net/ipv4/ip_forward", "1")
            .context("enabling IP forwarding (run as root, or: sysctl -w net.ipv4.ip_forward=1)")?;
        info!("IP forwarding enabled");
    }
    let _ = run("nft", &["delete", "table", "ip", NFT_TABLE]);
    let rules = format!(
        "table ip {NFT_TABLE} {{
            chain forward {{
                type filter hook forward priority -1; policy accept;
                iifname \"{dev}\" accept
                oifname \"{dev}\" ct state established,related accept
            }}
            chain postrouting {{
                type nat hook postrouting priority 100; policy accept;
                ip saddr {tailnet} oifname != \"{dev}\" masquerade
            }}
        }}"
    );
    let mut child = Command::new("nft")
        .args(["-f", "-"])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("running nft")?;
    use std::io::Write;
    child.stdin.take().unwrap().write_all(rules.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("nft: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    info!(%tailnet, "routing for the tailnet: forwarding + masquerade (nft table {NFT_TABLE})");
    Ok(())
}

pub fn disable_router() {
    let _ = run("nft", &["delete", "table", "ip", NFT_TABLE]);
}

/// Using an exit node: a default route in toolnet's table, except for our own
/// peer traffic (iroh's UDP port) and the relays and Nostr relays we need to
/// reach the exit node at all. Anything the main table routes more
/// specifically than its default route (LAN, the tailnet) stays direct.
pub struct ExitRouting {
    dev: String,
    bypass: BTreeSet<std::net::Ipv4Addr>,
}

impl ExitRouting {
    pub fn enable(dev: &str, iroh_ports: &[u16]) -> Result<Self> {
        for port in iroh_ports {
            let (pref, port) = (PREF_UDP.to_string(), port.to_string());
            run("ip", &["rule", "add", "pref", &pref, "ipproto", "udp", "sport", &port, "lookup", "main"])?;
        }
        run("ip", &["route", "replace", "default", "dev", dev, "table", TABLE])?;
        // Replies from the internet arrive on the TUN with foreign source
        // addresses: strict reverse path filtering would drop them
        let _ = std::fs::write(format!("/proc/sys/net/ipv4/conf/{dev}/rp_filter"), "2");
        info!(?iroh_ports, "exit node routing on: default route through {dev}");
        Ok(Self { dev: dev.to_string(), bypass: BTreeSet::new() })
    }

    /// Destinations that must not go through the tunnel (relays, Nostr relays)
    pub fn set_bypass(&mut self, ips: BTreeSet<std::net::Ipv4Addr>) {
        if ips == self.bypass {
            return;
        }
        while run("ip", &["rule", "del", "pref", &PREF_BYPASS_FIRST.to_string()]).is_ok() {}
        for ip in &ips {
            if let Err(e) = run("ip", &["rule", "add", "pref", &PREF_BYPASS_FIRST.to_string(), "to", &ip.to_string(), "lookup", "main"]) {
                warn!("bypass rule for {ip}: {e:#}");
            }
        }
        info!(count = ips.len(), "exit node bypass for relays updated");
        self.bypass = ips;
    }

    pub fn disable(self) {
        let _ = run("ip", &["route", "del", "default", "dev", &self.dev, "table", TABLE]);
        for pref in [PREF_UDP, PREF_BYPASS_FIRST] {
            while run("ip", &["rule", "del", "pref", &pref.to_string()]).is_ok() {}
        }
        info!("exit node routing off ({})", self.dev);
    }
}
