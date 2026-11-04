//! toolnet: a Tailscale-style VPN whose control plane is tool-db on Nostr.
//! See ../docs. Data plane: iroh (QUIC, hole punching, relays) + TUN.

use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use nostr_sdk::prelude::{Keys, PublicKey, SecretKey, ToBech32};
use tracing::{info, warn};

use toolnet::{
    crypto, dns, engine, ipc,
    model::{self, Member, NetConfig, Status, View},
    node::{MEMBERSHIP_GRACE, Node, NodeOpts, log_status, peers_of},
    os,
    state::{Config, State, hostname, join_payload, now, parse_pubkey, relays_or_default},
    tooldb::Db,
};

mod install;

#[derive(Parser)]
#[command(version, about = "Tailscale-style VPN with its control plane on tool-db / Nostr")]
struct Cli {
    /// State directory: keys and config
    #[arg(long, env = "TOOLNET_STATE", global = true)]
    state: Option<PathBuf>,
    /// The daemon's control socket
    #[arg(long, env = "TOOLNET_SOCKET", global = true, default_value = ipc::DEFAULT_SOCKET)]
    socket: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Set up this device: creates its keys and joins OWNER's network (pending approval)
    Init {
        /// The network owner's npub (or hex). Defaults to the owner key on this machine
        #[arg(long)]
        owner: Option<String>,
        #[arg(long, default_value = "home")]
        net: String,
        /// Nostr relays (repeatable). Default: a few public relays
        #[arg(long = "relay")]
        relays: Vec<String>,
    },
    /// Print this device's ids
    Id {
        /// Also print a QR code to scan with the owner app
        #[arg(long)]
        qr: bool,
    },
    /// Network owner commands
    Owner {
        #[command(subcommand)]
        cmd: OwnerCmd,
    },
    /// Show this device's status (from the running daemon, else from the relays)
    Status {
        /// The daemon's status as JSON
        #[arg(long)]
        json: bool,
    },
    /// Bring the tunnel up (the daemon must be running)
    Connect,
    /// Take the tunnel down, keeping the daemon running
    Disconnect,
    /// Send internet traffic through this member, or `none`
    ExitNode { name: String },
    /// Install (or update) toolnet as a systemd service on this machine; run as root
    Install {
        /// The network owner's npub. Required on a first install
        #[arg(long)]
        owner: Option<String>,
        /// The network's name (default: home, or what's already configured)
        #[arg(long)]
        net: Option<String>,
        /// Nostr relays (repeatable). Default: a few public relays
        #[arg(long = "relay")]
        relays: Vec<String>,
        /// Copy an existing identity (config.json, device.key, endpoint.key)
        /// from a state directory made with `toolnet init`
        #[arg(long)]
        state_from: Option<PathBuf>,
        /// Flags for `toolnet up`, e.g. "--advertise-exit-node" (kept when omitted)
        #[arg(long, allow_hyphen_values = true)]
        args: Option<String>,
        /// The user who may connect, disconnect and pick the exit node (default: the one running sudo)
        #[arg(long)]
        operator: Option<String>,
        /// RUST_LOG filter for the service, e.g. "info,toolnet=debug"
        #[arg(long)]
        log: Option<String>,
        /// Don't install the tray icon even if toolnet-tray is next to this binary
        #[arg(long)]
        no_tray: bool,
        /// Don't install missing iproute2 / nftables
        #[arg(long)]
        no_deps: bool,
        /// Set everything up but don't (re)start the service
        #[arg(long)]
        no_start: bool,
    },
    /// Update to the latest release (checksums verified, settings kept); run as root
    Update {
        /// Only say whether there's a newer version (no root needed)
        #[arg(long)]
        check: bool,
        /// Reinstall even if this version is the latest
        #[arg(long)]
        force: bool,
    },
    /// Remove the service and binaries (keeps the identity unless --purge); run as root
    Uninstall {
        /// Also delete this device's identity (/var/lib/toolnet)
        #[arg(long)]
        purge: bool,
    },
    /// Run: follow the network, publish our addresses, connect to peers
    Up {
        /// Don't create a TUN device; only connect and ping peers (no privileges needed)
        #[arg(long)]
        no_tun: bool,
        #[arg(long, default_value = "toolnet0")]
        dev: String,
        /// No iroh relays: direct addresses only (offline tests)
        #[arg(long)]
        offline: bool,
        /// UDP address to bind for peer traffic (default: random port)
        #[arg(long)]
        bind: Option<SocketAddr>,
        /// iroh relay URL to use instead of n0's public relays (repeatable)
        #[arg(long = "iroh-relay")]
        iroh_relays: Vec<iroh::RelayUrl>,
        /// Extra CA certificate(s) (PEM) to trust for the iroh relays: for a
        /// self-hosted relay with its own certificate
        #[arg(long)]
        relay_ca: Option<std::path::PathBuf>,
        /// Offer to route these prefixes for the tailnet (comma separated),
        /// e.g. a LAN behind this machine. The owner must approve them
        #[arg(long, value_delimiter = ',')]
        advertise_routes: Vec<engine::Cidr>,
        /// Offer to be an exit node (advertises 0.0.0.0/0; the owner must approve it)
        #[arg(long)]
        advertise_exit_node: bool,
        /// Send all internet traffic through this member (by name)
        #[arg(long)]
        exit_node: Option<String>,
        /// Don't serve MagicDNS (<member>.<domain> names)
        #[arg(long)]
        no_dns: bool,
        /// This user (name or uid) may connect, disconnect and pick the exit
        /// node through the control socket (the tray), besides root
        #[arg(long, env = "TOOLNET_OPERATOR")]
        operator: Option<String>,
    },
}

#[derive(Subcommand)]
enum OwnerCmd {
    /// Create the owner key (if needed) and a new network
    Create {
        #[arg(long, default_value = "home")]
        net: String,
        #[arg(long, default_value = "10.213.0.0/24")]
        ipv4: String,
        /// MagicDNS domain (default: <net>.toolnet)
        #[arg(long)]
        domain: Option<String>,
        #[arg(long = "relay")]
        relays: Vec<String>,
    },
    /// Approve a device into the network
    Approve {
        /// The device's npub (or hex), from `toolnet id` on that device
        device: String,
        #[arg(long)]
        name: String,
        /// Tailnet IP (default: the next free one)
        #[arg(long)]
        ip: Option<Ipv4Addr>,
    },
    /// Remove a device (by name or key) and rotate the group key
    Remove { device: String },
    /// Set the prefixes a device may route (none to clear), e.g. its LAN
    Routes {
        device: String,
        routes: Vec<engine::Cidr>,
    },
    /// List members and what they last published
    List,
}









#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "toolnet=info,iroh=warn,nostr_relay_pool=error,nostr_sdk=error".into()),
        )
        .init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Install { owner, net, relays, state_from, args, operator, log, no_tray, no_deps, no_start } => {
            return install::install(install::Opts { owner, net, relays, state_from, args, operator, log, no_tray, no_deps, no_start });
        }
        Cmd::Uninstall { purge } => return install::uninstall(purge),
        Cmd::Update { check, force } => return install::update(check, force).await,
        _ => {}
    }
    let state = State::new(cli.state)?;
    match cli.cmd {
        Cmd::Install { .. } | Cmd::Uninstall { .. } | Cmd::Update { .. } => unreachable!(),
        Cmd::Init { owner, net, relays } => init(&state, owner, net, relays),
        Cmd::Id { qr } => print_id(&state, qr),
        Cmd::Owner { cmd } => owner(&state, cmd).await,
        Cmd::Status { json } => match ipc::request(&cli.socket, &ipc::Request::Status).await {
            Ok(ipc::Response { status: Some(s), .. }) if json => {
                println!("{}", serde_json::to_string_pretty(&s)?);
                Ok(())
            }
            Ok(ipc::Response { status: Some(s), .. }) => {
                print_status(&s);
                Ok(())
            }
            Ok(r) => bail!("{}", r.error.unwrap_or_default()),
            Err(e) if json => Err(e),
            Err(e) => {
                println!("({e:#}; asking the relays instead)\n");
                status(&state).await
            }
        },
        Cmd::Connect => control(&cli.socket, ipc::Request::Connect).await,
        Cmd::Disconnect => control(&cli.socket, ipc::Request::Disconnect).await,
        Cmd::ExitNode { name } => {
            let name = (name != "none" && !name.is_empty()).then_some(name);
            control(&cli.socket, ipc::Request::ExitNode { name }).await
        }
        Cmd::Up { no_tun, dev, offline, bind, iroh_relays, relay_ca, mut advertise_routes, advertise_exit_node, exit_node, no_dns, operator } => {
            if advertise_exit_node {
                advertise_routes.push("0.0.0.0/0".parse()?);
            }
            let socket = cli.socket;
            up(&state, UpOpts { no_tun, dev, offline, bind, iroh_relays, relay_ca, advertise_routes, exit_node, no_dns, operator, socket }).await
        }
    }
}

fn print_id(state: &State, qr: bool) -> Result<()> {
    let keys = state.device_keys()?;
    let npub = keys.public_key().to_bech32()?;
    println!("device:   {npub}");
    println!("hex:      {}", keys.public_key().to_hex());
    println!("endpoint: {}", state.endpoint_key()?.public());
    if qr {
        let payload = join_payload(&npub, &hostname());
        let code = qrcode::QrCode::new(payload.as_bytes())?;
        let art = code
            .render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .quiet_zone(true)
            .build();
        println!("\nScan with the toolnet owner app to approve this device:\n{art}\n{payload}");
    }
    Ok(())
}

fn init(state: &State, owner: Option<String>, net: String, relays: Vec<String>) -> Result<()> {
    let existing = state.config().ok();
    let owner = match (owner, &existing, state.owner_keys().ok()) {
        (Some(o), _, _) => parse_pubkey(&o)?,
        (None, Some(c), _) => parse_pubkey(&c.owner)?,
        (None, None, Some(k)) => k.public_key(),
        (None, None, None) => bail!("--owner is required (the network owner's npub)"),
    };
    let relays = if relays.is_empty() {
        existing.map(|c| c.relays).unwrap_or_else(|| relays_or_default(vec![]))
    } else {
        relays
    };
    state.save_config(&Config { owner: owner.to_hex(), net, relays })?;
    if state.read_secret("device.key")?.is_none() {
        state.write_secret("device.key", &Keys::generate().secret_key().to_secret_hex())?;
    }
    let keys = state.device_keys()?;
    state.endpoint_key()?;
    println!("Device key: {}", keys.public_key().to_bech32()?);
    println!("Ask the owner to approve it:");
    println!("  toolnet owner approve {} --name <name>", keys.public_key().to_bech32()?);
    Ok(())
}

/// The owner's current view of its network (group key, config)
async fn owner_view(db: &Db, owner: &Keys, net: &str) -> Result<Option<View>> {
    let records = db.fetch([owner.public_key()]).await?;
    match model::view(&records, owner, &owner.public_key(), net) {
        (Status::NoNetwork, _) => Ok(None),
        (_, Some(v)) => Ok(Some(v)),
        (s, None) => bail!("can't open network {net}: {s:?} (is this the right owner key?)"),
    }
}

/// Publishes the group key for the owner and the given members, then the config
async fn publish_network(db: &Db, owner: &Keys, net: &str, epoch: u64, group: &SecretKey, config: &NetConfig) -> Result<()> {
    let mut recipients = vec![owner.public_key()];
    recipients.extend(config.members.keys().filter_map(|k| PublicKey::from_hex(k).ok()));
    for member in recipients {
        let wrapped = model::wrap_group_key(owner, &member, group, epoch)?;
        db.put(owner, &model::gk_key(net, &member), &wrapped).await?;
    }
    db.put(owner, &model::net_key(net), &model::seal(group, epoch, config)?).await?;
    Ok(())
}

async fn owner(state: &State, cmd: OwnerCmd) -> Result<()> {
    match cmd {
        OwnerCmd::Create { net, ipv4, domain, relays } => {
            if state.read_secret("owner.key")?.is_none() {
                state.write_secret("owner.key", &Keys::generate().secret_key().to_secret_hex())?;
            }
            let keys = state.owner_keys()?;
            let relays = relays_or_default(relays);
            if state.config().is_err() {
                state.save_config(&Config { owner: keys.public_key().to_hex(), net: net.clone(), relays: relays.clone() })?;
            }
            let db = Db::connect(&relays).await?;
            if owner_view(&db, &keys, &net).await?.is_some() {
                bail!("network {net} already exists for this owner");
            }
            let config = NetConfig { name: net.clone(), ipv4, members: BTreeMap::new(), updated_at: now(), domain };
            config.prefix()?;
            publish_network(&db, &keys, &net, 1, &crypto::new_group_key(), &config).await?;
            println!("Network {net} created. Owner: {}", keys.public_key().to_bech32()?);
            println!("On each device: toolnet init --owner {} --net {net}", keys.public_key().to_bech32()?);
            Ok(())
        }
        OwnerCmd::Approve { device, name, ip } => {
            let (keys, cfg) = (state.owner_keys()?, state.config()?);
            let device = parse_pubkey(&device)?;
            let db = Db::connect(&cfg.relays).await?;
            let mut view = owner_view(&db, &keys, &cfg.net).await?.context("no such network: `toolnet owner create`")?;
            if view.config.members.values().any(|m| m.name == name) {
                bail!("a member is already named {name}");
            }
            let ip = match ip {
                Some(ip) => ip,
                None => view.config.next_free_ip()?,
            };
            view.config.members.insert(device.to_hex(), Member { name: name.clone(), ip, added_at: now(), routes: vec![] });
            view.config.updated_at = now();
            // Same epoch: only the new member's group key is new
            let wrapped = model::wrap_group_key(&keys, &device, &view.group, view.epoch)?;
            db.put(&keys, &model::gk_key(&cfg.net, &device), &wrapped).await?;
            db.put(&keys, &model::net_key(&cfg.net), &model::seal(&view.group, view.epoch, &view.config)?).await?;
            println!("Approved {name} at {ip}");
            Ok(())
        }
        OwnerCmd::Remove { device } => {
            let (keys, cfg) = (state.owner_keys()?, state.config()?);
            let db = Db::connect(&cfg.relays).await?;
            let mut view = owner_view(&db, &keys, &cfg.net).await?.context("no such network")?;
            let key = view
                .config
                .members
                .iter()
                .find(|(k, m)| m.name == device || parse_pubkey(&device).is_ok_and(|p| p.to_hex() == **k))
                .map(|(k, _)| k.clone())
                .with_context(|| format!("no member {device}"))?;
            let member = view.config.members.remove(&key).unwrap();
            view.config.updated_at = now();
            // New epoch and group key, wrapped for the remaining members only:
            // the removed device can't read anything published from now on
            let epoch = view.epoch + 1;
            publish_network(&db, &keys, &cfg.net, epoch, &crypto::new_group_key(), &view.config).await?;
            println!("Removed {} ({}); group key rotated to epoch {epoch}", member.name, member.ip);
            Ok(())
        }
        OwnerCmd::Routes { device, routes } => {
            let (keys, cfg) = (state.owner_keys()?, state.config()?);
            let db = Db::connect(&cfg.relays).await?;
            let mut view = owner_view(&db, &keys, &cfg.net).await?.context("no such network")?;
            let member = view
                .config
                .members
                .iter_mut()
                .find(|(k, m)| m.name == device || parse_pubkey(&device).is_ok_and(|p| p.to_hex() == **k))
                .map(|(_, m)| m)
                .with_context(|| format!("no member {device}"))?;
            member.routes = routes.iter().map(|r| r.to_string()).collect();
            let summary = format!("{}: {:?}", member.name, member.routes);
            view.config.updated_at = now();
            db.put(&keys, &model::net_key(&cfg.net), &model::seal(&view.group, view.epoch, &view.config)?).await?;
            println!("Routes allowed for {summary}");
            Ok(())
        }
        OwnerCmd::List => {
            let (keys, cfg) = (state.owner_keys()?, state.config()?);
            let db = Db::connect(&cfg.relays).await?;
            let records = db
                .fetch(std::iter::once(keys.public_key()).chain({
                    let v = owner_view(&db, &keys, &cfg.net).await?;
                    v.map(|v| v.config.members.keys().filter_map(|k| PublicKey::from_hex(k).ok()).collect::<Vec<_>>())
                        .unwrap_or_default()
                }))
                .await?;
            let (_, view) = model::view(&records, &keys, &keys.public_key(), &cfg.net);
            print_view(view.as_ref());
            Ok(())
        }
    }
}

fn print_view(view: Option<&View>) {
    let Some(v) = view else {
        println!("(no network)");
        return;
    };
    println!("network {} ({}), epoch {}, {} members", v.config.name, v.config.ipv4, v.epoch, v.config.members.len());
    for (key, m) in &v.config.members {
        let node = PublicKey::from_hex(key).ok().and_then(|pk| v.nodes.get(&pk));
        match node {
            Some(n) => println!(
                "  {:<12} {:<15} {} addrs={:?} relay={}{}",
                m.name,
                m.ip,
                n.hostname,
                n.addrs,
                n.relay.as_deref().unwrap_or("-"),
                if m.routes.is_empty() && n.routes.is_empty() {
                    String::new()
                } else {
                    format!(" routes: allowed={:?} advertised={:?}", m.routes, n.routes)
                }
            ),
            None => println!("  {:<12} {:<15} (not online yet)", m.name, m.ip),
        }
    }
}

/// Sends a change to the daemon and prints the resulting status
async fn control(socket: &std::path::Path, req: ipc::Request) -> Result<()> {
    let r = ipc::request(socket, &req).await?;
    match (r.ok, r.status) {
        (true, Some(s)) => {
            print_status(&s);
            Ok(())
        }
        _ => bail!("{}", r.error.unwrap_or_else(|| "request failed".into())),
    }
}

fn print_status(s: &ipc::DaemonStatus) {
    let me = match (&s.name, s.ip) {
        (Some(n), Some(ip)) => format!("{n} {ip}"),
        _ => s.device.clone(),
    };
    let conn = if !s.connected { "disconnected" } else if s.state == "member" { "connected" } else { s.state.as_str() };
    println!("toolnet {} · network {} · {me} · {conn}", s.version, s.net);
    if let Some(exit) = &s.exit_node {
        println!("exit node: {exit}{}", if s.exit_active { "" } else { " (not available right now)" });
    }
    for p in &s.peers {
        let path = match (&p.path, p.rtt_ms) {
            (Some(path), Some(rtt)) => format!("{path} {rtt} ms"),
            _ if p.online => "online".into(),
            _ => "offline".into(),
        };
        let mut tags = vec![];
        if p.exit_node {
            tags.push("exit node".to_string());
        }
        tags.extend(p.routes.iter().cloned());
        let tags = if tags.is_empty() { String::new() } else { format!("  [{}]", tags.join(", ")) };
        println!("  {:<12} {:<15} {:<28} {path}{tags}", p.name, p.ip, p.dns);
    }
}

async fn status(state: &State) -> Result<()> {
    let (keys, cfg) = (state.device_keys()?, state.config()?);
    let owner = parse_pubkey(&cfg.owner)?;
    let db = Db::connect(&cfg.relays).await?;
    let records = db.fetch([owner]).await?;
    let members: Vec<PublicKey> = match model::view(&records, &keys, &owner, &cfg.net).1 {
        Some(v) => v.config.members.keys().filter_map(|k| PublicKey::from_hex(k).ok()).collect(),
        None => vec![],
    };
    let records = db.fetch(std::iter::once(owner).chain(members)).await?;
    let (status, view) = model::view(&records, &keys, &owner, &cfg.net);
    println!("this device: {} ({status:?})", keys.public_key().to_bech32()?);
    print_view(view.as_ref());
    Ok(())
}



struct UpOpts {
    no_tun: bool,
    dev: String,
    offline: bool,
    bind: Option<SocketAddr>,
    iroh_relays: Vec<iroh::RelayUrl>,
    relay_ca: Option<std::path::PathBuf>,
    advertise_routes: Vec<engine::Cidr>,
    exit_node: Option<String>,
    no_dns: bool,
    operator: Option<String>,
    socket: PathBuf,
}

/// IPv4 addresses of the hosts in these URLs (relays we must reach directly)
async fn resolve_hosts(urls: impl IntoIterator<Item = String>) -> std::collections::BTreeSet<Ipv4Addr> {
    let mut out = std::collections::BTreeSet::new();
    for url in urls {
        let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(&url);
        let hostport = rest.split('/').next().unwrap_or("");
        let host = hostport.rsplit_once(':').map(|(h, _)| h).unwrap_or(hostport).trim_end_matches('.');
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            out.insert(ip);
            continue;
        }
        if let Ok(addrs) = tokio::net::lookup_host((host, 443)).await {
            out.extend(addrs.filter_map(|a| match a.ip() {
                std::net::IpAddr::V4(v4) => Some(v4),
                _ => None,
            }));
        }
    }
    out
}

/// What we set up on the OS for the tunnel, so it can be taken down again
/// (disconnect, or exit) and brought back
#[derive(Default)]
struct Tunnel {
    ip: Option<Ipv4Addr>,
    routes: Option<os::Routes>,
    router: bool,
    exit: Option<os::ExitRouting>,
    dns: Option<tokio::task::JoinHandle<()>>,
    dns_registered: bool,
}

impl Tunnel {
    async fn down(&mut self, engine: &engine::Engine, dev: &str) {
        if let Some(r) = self.exit.take() {
            r.disable();
        }
        if let Some(r) = self.routes.take() {
            r.teardown();
        }
        if std::mem::take(&mut self.dns_registered) {
            dns::unregister_resolved(dev);
        }
        if let Some(t) = self.dns.take() {
            t.abort();
        }
        if std::mem::take(&mut self.router) {
            os::disable_router();
        }
        if self.ip.take().is_some() {
            engine.stop_tun().await;
            info!("tunnel down");
        }
    }
}

/// Members we may use as exit node: allowed by the owner and offered
fn exit_nodes(view: &View, me: &PublicKey) -> Vec<String> {
    view.config
        .members
        .values()
        .filter(|m| {
            peers_of(view, me, Some(&m.name)).iter().any(|p| p.name == m.name && p.routes.iter().any(|r| r.len == 0))
        })
        .map(|m| m.name.clone())
        .collect()
}

/// What the control socket reports (paths are added live from the engine)
fn daemon_status(
    cfg: &Config,
    keys: &Keys,
    status: &Status,
    view: Option<&View>,
    prefs: &ipc::Prefs,
    exit_active: bool,
) -> ipc::DaemonStatus {
    let me = keys.public_key();
    let mut s = ipc::DaemonStatus {
        version: env!("CARGO_PKG_VERSION").into(),
        state: match status {
            Status::Member => "member",
            Status::NotMember { .. } => "pending",
            Status::NoNetwork => "no network",
        }
        .into(),
        connected: prefs.connected,
        net: cfg.net.clone(),
        device: me.to_bech32().unwrap_or_default(),
        exit_node: prefs.exit_node.clone(),
        exit_active,
        ..Default::default()
    };
    let Some(view) = view else { return s };
    let domain = view.config.dns_domain();
    s.domain = domain.clone();
    if let Some(m) = view.config.members.get(&me.to_hex()) {
        s.name = Some(m.name.clone());
        s.ip = Some(m.ip);
    }
    s.exit_nodes = exit_nodes(view, &me);
    let routed = peers_of(view, &me, None);
    for (key, m) in &view.config.members {
        let Ok(pk) = PublicKey::from_hex(key) else { continue };
        if pk == me {
            continue;
        }
        let node = view.nodes.get(&pk);
        s.peers.push(ipc::PeerStatus {
            name: m.name.clone(),
            ip: m.ip,
            dns: format!("{}.{domain}", dns::label(&m.name)),
            online: node.is_some_and(|n| now().saturating_sub(n.updated_at) < 15 * 60),
            path: None,
            rtt_ms: None,
            exit_node: s.exit_nodes.contains(&m.name),
            routes: routed
                .iter()
                .find(|p| p.name == m.name)
                .map(|p| p.routes.iter().map(|r| r.to_string()).collect())
                .unwrap_or_default(),
        });
    }
    s.peers.sort_by_key(|p| p.ip);
    s
}

/// A user's uid, by name or number
fn uid_of(user: &str) -> Result<u32> {
    if let Ok(uid) = user.parse() {
        return Ok(uid);
    }
    let name = std::ffi::CString::new(user)?;
    // SAFETY: getpwnam returns a pointer into static storage or null
    let pw = unsafe { libc::getpwnam(name.as_ptr()) };
    if pw.is_null() {
        bail!("no such user: {user}");
    }
    Ok(unsafe { (*pw).pw_uid })
}

async fn up(state: &State, opts: UpOpts) -> Result<()> {
    os::inherit_capabilities();
    let UpOpts { no_tun, dev, offline, bind, iroh_relays, relay_ca, advertise_routes, exit_node, no_dns, operator, socket } = opts;
    let operator = operator.as_deref().map(uid_of).transpose()?;
    let node = Node::start(
        state,
        NodeOpts { offline, bind, iroh_relays, relay_ca, advertise_routes: advertise_routes.clone(), hostname: hostname() },
    )
    .await?;
    let Node { engine, keys, cfg, id, mut views, .. } = node;
    let view_rx = &mut views;

    // The user's choices (tray, `toolnet connect`, `toolnet exit-node`) win
    // over the flags once made
    let prefs_path = state.path("prefs.json");
    let mut prefs = ipc::Prefs::load(&prefs_path).unwrap_or(ipc::Prefs { connected: true, exit_node });
    let (status_tx, status_rx) = tokio::sync::watch::channel(ipc::DaemonStatus::default());
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<ipc::Command>(8);
    {
        let engine = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = ipc::serve(socket, operator, engine, status_rx, cmd_tx).await {
                warn!("no control socket (status and the tray won't work): {e:#}");
            }
        });
    }

    let mut tunnel = Tunnel::default();
    let names = dns::Names::default();
    let mut last_status = None;
    let mut report = tokio::time::interval(Duration::from_secs(60));
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    // Losing membership tears everything down, but only once it lasts: during
    // a key rotation the new network can arrive just before our new group key
    let mut teardown_at: Option<tokio::time::Instant> = None;
    loop {
        let grace = async {
            match teardown_at {
                Some(t) => tokio::time::sleep_until(t).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = sigterm.recv() => break,
            _ = report.tick() => {
                for line in engine.status().await {
                    info!("  {line}");
                }
                continue;
            }
            _ = grace => {
                teardown_at = None;
                warn!("not a member anymore: dropping all peers");
                engine.set_peers(vec![]).await;
                continue;
            }
            r = view_rx.changed() => r?,
            Some((req, reply)) = cmd_rx.recv() => {
                let view = view_rx.borrow().1.clone();
                let result = match req {
                    ipc::Request::Status => Ok(()),
                    ipc::Request::Connect => {
                        prefs.connected = true;
                        Ok(())
                    }
                    ipc::Request::Disconnect => {
                        prefs.connected = false;
                        Ok(())
                    }
                    ipc::Request::ExitNode { name: None } => {
                        prefs.exit_node = None;
                        Ok(())
                    }
                    ipc::Request::ExitNode { name: Some(name) } => {
                        let usable = view.as_ref().map(|v| exit_nodes(v, &keys.public_key())).unwrap_or_default();
                        if usable.contains(&name) {
                            prefs.exit_node = Some(name);
                            Ok(())
                        } else {
                            Err(format!("{name} isn't an exit node (it must offer it and the owner allow it); exit nodes: {usable:?}"))
                        }
                    }
                };
                if result.is_ok() {
                    info!(connected = prefs.connected, exit = ?prefs.exit_node, "settings changed");
                    prefs.save(&prefs_path);
                }
                let _ = reply.send(result);
            }
        }
        let (status, view) = view_rx.borrow_and_update().clone();
        if last_status.as_ref() != Some(&status) {
            log_status(&status, &cfg, &id.owner);
            last_status = Some(status.clone());
        }
        let publish = |exit_active: bool| {
            status_tx.send_replace(daemon_status(&cfg, &keys, &status, view.as_ref(), &prefs, exit_active));
        };
        let (Status::Member, Some(view)) = (&status, &view) else {
            if teardown_at.is_none() {
                teardown_at = Some(tokio::time::Instant::now() + MEMBERSHIP_GRACE);
            }
            publish(tunnel.exit.is_some());
            continue;
        };
        teardown_at = None;
        if !prefs.connected {
            if tunnel.ip.is_some() || !engine.status().await.is_empty() {
                info!("disconnected: dropping peers and the tunnel");
                engine.set_peers(vec![]).await;
                tunnel.down(&engine, &dev).await;
            }
            publish(false);
            continue;
        }
        let me = view.config.members.get(&keys.public_key().to_hex());
        if let (Some(me), false, None) = (me, no_tun, tunnel.ip) {
            let (_, prefix) = view.config.prefix()?;
            engine.start_tun(&dev, me.ip, prefix).await?;
            tunnel.ip = Some(me.ip);
            tunnel.routes = Some(os::Routes::new(&dev)?);
            if !no_dns {
                let (names, ip) = (names.clone(), me.ip);
                tunnel.dns = Some(tokio::spawn(async move {
                    if let Err(e) = dns::serve(ip, names).await {
                        warn!("MagicDNS: {e:#}");
                    }
                }));
                let (dev, ip, domain) = (dev.clone(), me.ip, view.config.dns_domain());
                tokio::task::spawn_blocking(move || dns::register_resolved(&dev, ip, &domain));
                tunnel.dns_registered = true;
            }
            if !advertise_routes.is_empty() {
                let tailnet: engine::Cidr = view.config.ipv4.parse()?;
                match os::enable_router(&dev, &tailnet) {
                    Ok(()) => tunnel.router = true,
                    Err(e) => warn!("can't route for the tailnet: {e:#}"),
                }
            }
        }
        if let (Some(me), Some(ip)) = (me, tunnel.ip) {
            if me.ip != ip {
                warn!(old = %ip, new = %me.ip, "our tailnet IP changed: restart toolnet to apply");
            }
        }
        names
            .set(&view.config.dns_domain(), view.config.members.values().map(|m| (m.name.clone(), m.ip)))
            .await;
        let exit_node = prefs.exit_node.as_deref();
        let peers = peers_of(view, &keys.public_key(), exit_node);
        info!(epoch = view.epoch, peers = ?peers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), "netmap");
        engine.set_peers(peers).await;
        if tunnel.ip.is_some() {
            if let Some(r) = tunnel.routes.as_mut() {
                r.sync(&engine.routed_prefixes().await);
            }
            let exit = exit_node.and_then(|name| {
                peers_of(view, &keys.public_key(), Some(name))
                    .into_iter()
                    .find(|p| p.name == name && p.routes.iter().any(|r| r.len == 0))
            });
            match (&exit, tunnel.exit.is_some()) {
                (Some(peer), false) => match os::ExitRouting::enable(&dev, &engine.udp_ports()) {
                    Ok(r) => {
                        info!(exit = %peer.name, "using exit node");
                        tunnel.exit = Some(r);
                    }
                    Err(e) => warn!("can't route through the exit node: {e:#}"),
                },
                (None, true) => {
                    match exit_node {
                        Some(name) => warn!(exit = %name, "exit node gone (not allowed, not advertised or offline): traffic goes direct"),
                        None => info!("not using an exit node anymore"),
                    }
                    tunnel.exit.take().unwrap().disable();
                }
                _ => {}
            }
            if let Some(routing) = tunnel.exit.as_mut() {
                // Everything we need to reach peers at all must bypass the tunnel
                let mut urls: Vec<String> = cfg.relays.clone();
                urls.extend(view.nodes.values().filter_map(|n| n.relay.clone()));
                urls.extend(engine.endpoint.addr().relay_urls().map(|r| r.to_string()));
                routing.set_bypass(resolve_hosts(urls).await);
            }
        } else if exit_node.is_some() && no_tun {
            warn!("an exit node needs the TUN device (not --no-tun)");
        }
        publish(tunnel.exit.is_some());
    }
    tunnel.down(&engine, &dev).await;
    engine.endpoint.close().await;
    Ok(())
}
