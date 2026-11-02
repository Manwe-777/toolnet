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

/// Keeps the OS routes for peer prefixes (subnet routes) pointing at the TUN
#[derive(Default)]
pub struct Routes {
    installed: BTreeSet<Cidr>,
}

impl Routes {
    pub fn sync(&mut self, dev: &str, wanted: &[Cidr]) {
        let wanted: BTreeSet<Cidr> = wanted.iter().copied().collect();
        for old in self.installed.difference(&wanted) {
            match run("ip", &["route", "del", &old.to_string(), "dev", dev]) {
                Ok(()) => info!(route = %old, "route removed"),
                Err(e) => warn!("removing route {old}: {e:#}"),
            }
        }
        for new in wanted.difference(&self.installed) {
            if new.len == 0 {
                // A default route needs the exit node setup (policy routing so
                // our own peer traffic doesn't loop into the tunnel). Not yet.
                warn!("0.0.0.0/0 is routed to a peer, but using an exit node isn't supported yet");
                continue;
            }
            match run("ip", &["route", "replace", &new.to_string(), "dev", dev]) {
                Ok(()) => info!(route = %new, "route added"),
                Err(e) => warn!("adding route {new}: {e:#}"),
            }
        }
        self.installed = wanted.into_iter().filter(|c| c.len > 0).collect();
    }
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
