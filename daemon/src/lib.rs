//! toolnet: a Tailscale-style VPN whose control plane is tool-db on Nostr.
//! See ../docs. Data plane: iroh (QUIC, hole punching, relays) + TUN.
//!
//! The library is everything a device needs (control plane, engine, hole
//! punching); `main.rs` is the Linux CLI and daemon around it, and the
//! Android app (`android/`) is another shell around the same core.

pub mod control;
pub mod crypto;
pub mod dns;
pub mod engine;
#[cfg(unix)]
pub mod ipc;
pub mod model;
pub mod node;
#[cfg(target_os = "linux")]
pub mod os;
pub mod punch;
pub mod state;
pub mod tooldb;

use std::process::Command;

use anyhow::{Context, Result, bail};

pub use state::now;

/// A system command by name, also looked up in the sbin directories that
/// Debian and others leave off a user's PATH (nft lives there)
pub fn command(cmd: &str) -> Command {
    let path = std::env::var("PATH").unwrap_or_default();
    let found = path
        .split(':')
        .chain(["/usr/sbin", "/sbin", "/usr/bin", "/bin"])
        .map(|d| std::path::Path::new(d).join(cmd))
        .find(|p| p.is_file());
    Command::new(found.as_deref().unwrap_or(std::path::Path::new(cmd)))
}

/// Runs an nft script (from stdin)
pub fn nft(script: &str) -> Result<()> {
    let mut child = command("nft")
        .args(["-f", "-"])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("running nft")?;
    use std::io::Write;
    child.stdin.take().unwrap().write_all(script.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("nft: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

pub(crate) fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = command(cmd).args(args).output().with_context(|| format!("running {cmd}"))?;
    if !out.status.success() {
        bail!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
