//! `toolnet install` / `toolnet uninstall`: set this machine up as a systemd
//! service. The curl installer only downloads the binaries and runs this, so
//! re-running it is also how to update, or change the flags.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

use toolnet::{
    find_command,
    state::{State, parse_pubkey},
};

const BIN: &str = "/usr/local/bin/toolnet";
const TRAY_BIN: &str = "/usr/local/bin/toolnet-tray";
const STATE_DIR: &str = "/var/lib/toolnet";
const UNIT: &str = "/etc/systemd/system/toolnet.service";
const DROPINS: &str = "/etc/systemd/system/toolnet.service.d";
const TRAY_UNIT: &str = "/etc/systemd/user/toolnet-tray.service";
const UNIT_TEXT: &str = include_str!("../../packaging/toolnet.service");
const TRAY_UNIT_TEXT: &str = include_str!("../../packaging/toolnet-tray.service");

pub struct Opts {
    pub owner: Option<String>,
    pub net: Option<String>,
    pub relays: Vec<String>,
    pub state_from: Option<PathBuf>,
    pub args: Option<String>,
    pub operator: Option<String>,
    pub log: Option<String>,
    pub no_tray: bool,
    pub no_deps: bool,
    pub no_start: bool,
}

pub fn install(o: Opts) -> Result<()> {
    preflight()?;
    if !o.no_deps {
        dependencies()?;
    }

    // Binaries: ourselves, and the tray if it sits next to us
    let me = std::env::current_exe()?.canonicalize()?;
    copy_binary(&me, Path::new(BIN))?;
    let tray = me.with_file_name("toolnet-tray");
    let with_tray = !o.no_tray && (tray.is_file() || Path::new(TRAY_BIN).is_file());
    if with_tray && tray.is_file() {
        copy_binary(&tray, Path::new(TRAY_BIN))?;
    }

    // Identity: keep what's there, copy it from elsewhere, or make a new one
    let state = State::new(Some(STATE_DIR.into()))?;
    fs::set_permissions(STATE_DIR, fs::Permissions::from_mode(0o700))?;
    let has_identity = state.path("device.key").exists();
    let mut fresh = false;
    if let Some(from) = &o.state_from {
        if has_identity {
            bail!("{STATE_DIR} already has a device identity: not overwriting it with {}", from.display());
        }
        for f in ["config.json", "device.key", "endpoint.key"] {
            let text = fs::read_to_string(from.join(f)).with_context(|| format!("reading {}", from.join(f).display()))?;
            write(&state.path(f).to_string_lossy(), &text, 0o600)?;
        }
        println!("device identity copied from {}", from.display());
    } else if !has_identity || o.owner.is_some() || o.net.is_some() || !o.relays.is_empty() {
        if !has_identity && o.owner.is_none() {
            bail!("--owner is required on a first install (the network owner's npub)");
        }
        if let Some(owner) = &o.owner {
            parse_pubkey(owner)?;
        }
        let changed = match state.config() {
            Ok(c) => o.owner.as_deref().is_some_and(|w| parse_pubkey(w).ok().map(|k| k.to_hex()) != Some(c.owner.clone()))
                || o.net.as_deref().is_some_and(|n| n != c.net),
            Err(_) => true,
        };
        let net = o.net.clone().or_else(|| state.config().ok().map(|c| c.net)).unwrap_or_else(|| "home".into());
        crate::init(&state, o.owner.clone(), net, o.relays.clone())?;
        fresh = !has_identity || changed;
    }

    // Service
    write(UNIT, UNIT_TEXT, 0o644)?;
    fs::create_dir_all(DROPINS)?;
    let args_conf = format!("{DROPINS}/args.conf");
    if o.args.is_some() || !Path::new(&args_conf).exists() {
        write(&args_conf, &dropin("TOOLNET_ARGS", o.args.as_deref().unwrap_or("")), 0o644)?;
    }
    let operator = o.operator.clone().or_else(|| std::env::var("SUDO_USER").ok()).filter(|u| u != "root" && !u.is_empty());
    if let Some(user) = &operator {
        crate::uid_of(user)?;
        write(&format!("{DROPINS}/operator.conf"), &dropin("TOOLNET_OPERATOR", user), 0o644)?;
    }
    if let Some(log) = &o.log {
        write(&format!("{DROPINS}/log.conf"), &dropin("RUST_LOG", log), 0o644)?;
    }
    if with_tray {
        write(TRAY_UNIT, TRAY_UNIT_TEXT, 0o644)?;
    }
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--quiet", "toolnet"])?;
    if with_tray {
        // For every user, started with their graphical session
        systemctl(&["--global", "enable", "--quiet", "toolnet-tray"])?;
    }
    if !o.no_start {
        systemctl(&["restart", "toolnet"])?;
        if with_tray {
            if let Some(user) = &operator {
                if user_systemctl(user, &["is-active", "--quiet", "graphical-session.target"]) {
                    user_systemctl(user, &["restart", "toolnet-tray"]);
                }
            }
        }
    }

    println!();
    println!("toolnet is installed: {BIN}, state in {STATE_DIR}");
    println!("  service:  systemctl status toolnet   (logs: journalctl -u toolnet -f)");
    if let Some(user) = &operator {
        println!("  operator: {user} (may connect, disconnect and pick the exit node)");
    }
    if with_tray {
        println!("  tray:     toolnet-tray, starts with the desktop session");
    }
    if fresh {
        println!();
        crate::print_id(&state, true)?;
        println!("\nOnce approved, the device connects on its own: check with `toolnet status`.");
    } else {
        println!("  status:   toolnet status");
    }
    Ok(())
}

pub fn uninstall(purge: bool) -> Result<()> {
    preflight()?;
    if Path::new(TRAY_UNIT).exists() {
        if let Ok(user) = std::env::var("SUDO_USER") {
            let _ = user_systemctl(&user, &["stop", "toolnet-tray"]);
        }
        let _ = systemctl(&["--global", "disable", "--quiet", "toolnet-tray"]);
    }
    let _ = systemctl(&["disable", "--now", "--quiet", "toolnet"]);
    for f in [UNIT, TRAY_UNIT, BIN, TRAY_BIN] {
        remove(Path::new(f))?;
    }
    if Path::new(DROPINS).exists() {
        fs::remove_dir_all(DROPINS)?;
    }
    systemctl(&["daemon-reload"])?;
    if purge {
        if Path::new(STATE_DIR).exists() {
            fs::remove_dir_all(STATE_DIR)?;
        }
        println!("toolnet removed, with its identity ({STATE_DIR})");
    } else {
        println!("toolnet removed. This device's identity is kept in {STATE_DIR}: reinstalling");
        println!("brings it back as the same member (`toolnet uninstall --purge` deletes it)");
    }
    Ok(())
}

fn preflight() -> Result<()> {
    // SAFETY: geteuid has no preconditions
    if unsafe { libc::geteuid() } != 0 {
        bail!("run as root: sudo toolnet install ...");
    }
    if !Path::new("/run/systemd/system").exists() {
        bail!(
            "this system doesn't run systemd. Run toolnet by hand instead:\n  \
             TOOLNET_STATE={STATE_DIR} toolnet init --owner <npub> --net <net>\n  \
             TOOLNET_STATE={STATE_DIR} toolnet up"
        );
    }
    Ok(())
}

/// iproute2 and nftables are required; installed with the distro's package
/// manager if missing. systemd-resolved (MagicDNS names) is only reported
fn dependencies() -> Result<()> {
    let missing: Vec<&str> = ["ip", "nft"].into_iter().filter(|c| find_command(c).is_none()).collect();
    if !missing.is_empty() {
        let (pm, args, pkgs): (&str, &[&str], [&str; 2]) = if find_command("apt-get").is_some() {
            ("apt-get", &["install", "-y", "-q"], ["iproute2", "nftables"])
        } else if find_command("dnf").is_some() {
            ("dnf", &["install", "-y", "-q"], ["iproute", "nftables"])
        } else if find_command("pacman").is_some() {
            ("pacman", &["-S", "--noconfirm", "--needed"], ["iproute2", "nftables"])
        } else if find_command("zypper").is_some() {
            ("zypper", &["--non-interactive", "install"], ["iproute2", "nftables"])
        } else if find_command("apk").is_some() {
            ("apk", &["add"], ["iproute2", "nftables"])
        } else {
            bail!("missing {}: install iproute2 and nftables, then run this again", missing.join(", "));
        };
        println!("installing {} ({pm})...", pkgs.join(" "));
        if pm == "apt-get" {
            let _ = Command::new(pm).args(["update", "-q"]).status();
        }
        let ok = Command::new(pm).args(args).args(pkgs).status().with_context(|| format!("running {pm}"))?.success();
        if !ok || ["ip", "nft"].iter().any(|c| find_command(c).is_none()) {
            bail!("couldn't install iproute2 / nftables with {pm}: install them, then run this again");
        }
    }
    if find_command("resolvectl").is_none() || !Path::new("/run/systemd/resolve").exists() {
        println!("note: systemd-resolved isn't running, so <member>.<domain> names won't resolve");
        println!("      on this machine (tailnet IPs work; other devices are unaffected)");
    }
    Ok(())
}

/// Replaces `to` atomically, so a running copy keeps working until restarted
fn copy_binary(from: &Path, to: &Path) -> Result<()> {
    if to.exists() && from == to.canonicalize()? {
        return Ok(());
    }
    let tmp = to.with_extension("new");
    fs::copy(from, &tmp).with_context(|| format!("copying {} to {}", from.display(), tmp.display()))?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
    fs::rename(&tmp, to)?;
    Ok(())
}

fn write(path: &str, text: &str, mode: u32) -> Result<()> {
    fs::write(path, text).with_context(|| format!("writing {path}"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn remove(p: &Path) -> Result<()> {
    match fs::remove_file(p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).with_context(|| format!("removing {}", p.display())),
        _ => Ok(()),
    }
}

/// A systemd drop-in setting one environment variable
fn dropin(var: &str, value: &str) -> String {
    let value = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("[Service]\nEnvironment=\"{var}={value}\"\n")
}

fn systemctl(args: &[&str]) -> Result<()> {
    let ok = Command::new("systemctl").args(args).status().context("running systemctl")?.success();
    if !ok {
        bail!("systemctl {} failed", args.join(" "));
    }
    Ok(())
}

/// systemctl --user for a logged-in user (false if it fails or they have no session)
fn user_systemctl(user: &str, args: &[&str]) -> bool {
    let Ok(uid) = crate::uid_of(user) else { return false };
    let runtime = format!("/run/user/{uid}");
    if !Path::new(&runtime).exists() {
        return false;
    }
    Command::new("runuser")
        .args(["-u", user, "--", "env", &format!("XDG_RUNTIME_DIR={runtime}"), "systemctl", "--user"])
        .args(args)
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
