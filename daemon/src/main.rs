//! toolnet: a Tailscale-style VPN whose control plane is tool-db on Nostr.
//! See ../docs. Data plane: iroh (QUIC, hole punching, relays) + TUN.

mod control;
mod crypto;
mod engine;
mod model;
mod tooldb;

use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use nostr_sdk::prelude::{FromBech32, Keys, PublicKey, SecretKey, ToBech32};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::{
    control::Identity,
    engine::{Engine, EngineConfig, Peer},
    model::{Member, NetConfig, NodeInfo, Status, View},
    tooldb::Db,
};

/// How long a lost membership must last before we drop our peers
const MEMBERSHIP_GRACE: Duration = Duration::from_secs(15);

const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
    "wss://nostr.mom",
];

#[derive(Parser)]
#[command(version, about = "Tailscale-style VPN with its control plane on tool-db / Nostr")]
struct Cli {
    /// State directory: keys and config
    #[arg(long, env = "TOOLNET_STATE", global = true)]
    state: Option<PathBuf>,
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
    Id,
    /// Network owner commands
    Owner {
        #[command(subcommand)]
        cmd: OwnerCmd,
    },
    /// Show the network as this device sees it
    Status,
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
    /// List members and what they last published
    List,
}

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    owner: String,
    net: String,
    relays: Vec<String>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn parse_pubkey(s: &str) -> Result<PublicKey> {
    PublicKey::from_bech32(s)
        .or_else(|_| PublicKey::from_hex(s))
        .with_context(|| format!("not an npub or hex public key: {s}"))
}

struct State {
    dir: PathBuf,
}

impl State {
    fn new(dir: Option<PathBuf>) -> Result<Self> {
        let dir = match dir {
            Some(d) => d,
            None => dirs::data_local_dir().context("no data dir")?.join("toolnet"),
        };
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn read_secret(&self, name: &str) -> Result<Option<String>> {
        match std::fs::read_to_string(self.path(name)) {
            Ok(s) => Ok(Some(s.trim().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write_secret(&self, name: &str, hex: &str) -> Result<()> {
        write_private(&self.path(name), hex)
    }

    fn device_keys(&self) -> Result<Keys> {
        let hex = self.read_secret("device.key")?.context("no device key: run `toolnet init` first")?;
        Ok(Keys::new(SecretKey::from_hex(&hex)?))
    }

    fn owner_keys(&self) -> Result<Keys> {
        let hex = self.read_secret("owner.key")?.context("no owner key on this machine: `toolnet owner create`")?;
        Ok(Keys::new(SecretKey::from_hex(&hex)?))
    }

    fn endpoint_key(&self) -> Result<iroh::SecretKey> {
        if let Some(hex) = self.read_secret("endpoint.key")? {
            let bytes: [u8; 32] = data_encoding::HEXLOWER
                .decode(hex.as_bytes())?
                .try_into()
                .map_err(|_| anyhow::anyhow!("endpoint.key must be 32 bytes"))?;
            return Ok(iroh::SecretKey::from_bytes(&bytes));
        }
        let key = iroh::SecretKey::generate();
        self.write_secret("endpoint.key", &data_encoding::HEXLOWER.encode(&key.to_bytes()))?;
        Ok(key)
    }

    fn config(&self) -> Result<Config> {
        let s = std::fs::read_to_string(self.path("config.json")).context("no config: run `toolnet init` first")?;
        Ok(serde_json::from_str(&s)?)
    }

    fn save_config(&self, c: &Config) -> Result<()> {
        std::fs::write(self.path("config.json"), serde_json::to_string_pretty(c)?)?;
        Ok(())
    }
}

fn write_private(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(contents.as_bytes())?;
    Ok(())
}

fn relays_or_default(relays: Vec<String>) -> Vec<String> {
    if relays.is_empty() {
        DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
    } else {
        relays
    }
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
    let state = State::new(cli.state)?;
    match cli.cmd {
        Cmd::Init { owner, net, relays } => init(&state, owner, net, relays),
        Cmd::Id => {
            let keys = state.device_keys()?;
            println!("device:   {}", keys.public_key().to_bech32()?);
            println!("endpoint: {}", state.endpoint_key()?.public());
            Ok(())
        }
        Cmd::Owner { cmd } => owner(&state, cmd).await,
        Cmd::Status => status(&state).await,
        Cmd::Up { no_tun, dev, offline, bind, iroh_relays } => {
            up(&state, no_tun, dev, offline, bind, iroh_relays).await
        }
    }
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
        OwnerCmd::Create { net, ipv4, relays } => {
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
            let config = NetConfig { name: net.clone(), ipv4, members: BTreeMap::new(), updated_at: now() };
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
            view.config.members.insert(device.to_hex(), Member { name: name.clone(), ip, added_at: now() });
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
                "  {:<12} {:<15} {} addrs={:?} relay={}",
                m.name,
                m.ip,
                n.hostname,
                n.addrs,
                n.relay.as_deref().unwrap_or("-")
            ),
            None => println!("  {:<12} {:<15} (not online yet)", m.name, m.ip),
        }
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

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// Peers for the engine: members other than us, with valid node info
fn peers_of(view: &View, me: &PublicKey) -> Vec<Peer> {
    let mut peers = vec![];
    for (key, member) in &view.config.members {
        let Ok(pk) = PublicKey::from_hex(key) else { continue };
        if pk == *me {
            continue;
        }
        let Some(node) = view.nodes.get(&pk) else { continue };
        let Ok(endpoint) = engine::parse_endpoint_id(&node.endpoint_id) else { continue };
        peers.push(Peer {
            name: member.name.clone(),
            ip: member.ip,
            endpoint,
            addrs: node.addrs.iter().filter_map(|a| a.parse().ok()).collect(),
            relay: node.relay.as_deref().and_then(|r| r.parse().ok()),
        });
    }
    peers
}

async fn up(
    state: &State,
    no_tun: bool,
    dev: String,
    offline: bool,
    bind: Option<SocketAddr>,
    iroh_relays: Vec<iroh::RelayUrl>,
) -> Result<()> {
    let (keys, cfg) = (state.device_keys()?, state.config()?);
    let id = Identity { device: keys.clone(), owner: parse_pubkey(&cfg.owner)?, net: cfg.net.clone() };
    let engine = Engine::start(EngineConfig {
        secret: state.endpoint_key()?,
        offline,
        relays: iroh_relays,
        bind,
    })
    .await?;
    info!(device = %keys.public_key().to_bech32()?, endpoint = %engine.id(), "toolnet up");
    let db = Db::connect(&cfg.relays).await?;

    // Our node info, kept current from the endpoint's addresses
    let endpoint_id = engine.id().to_string();
    let make_info = move |addr: &iroh::EndpointAddr| NodeInfo {
        endpoint_id: endpoint_id.clone(),
        addrs: control::useful_addrs(addr.ip_addrs().copied()).iter().map(|a| a.to_string()).collect(),
        relay: addr.relay_urls().next().map(|r| r.to_string()),
        hostname: hostname(),
        version: env!("CARGO_PKG_VERSION").into(),
        updated_at: now(),
    };
    let (info_tx, info_rx) = watch::channel(make_info(&engine.endpoint.addr()));
    {
        let engine = engine.clone();
        tokio::spawn(async move {
            use n0_watcher::Watcher;
            let mut w = engine.endpoint.watch_addr();
            while let Ok(addr) = w.updated().await {
                let info = make_info(&addr);
                info_tx.send_if_modified(|cur| {
                    let changed = cur.addrs != info.addrs || cur.relay != info.relay;
                    if changed {
                        *cur = info;
                    }
                    changed
                });
            }
        });
    }

    let (view_tx, mut view_rx) = watch::channel((Status::NoNetwork, None::<View>));
    let db = std::sync::Arc::new(db);
    {
        let (db, id, view_tx) = (db.clone(), id.clone(), view_tx);
        tokio::spawn(async move {
            if let Err(e) = control::follow(&db, &id, view_tx).await {
                warn!("control plane stopped: {e:#}");
            }
        });
    }
    {
        let (db, id, views) = (db.clone(), id.clone(), view_rx.clone());
        tokio::spawn(async move {
            if let Err(e) = control::publish_node(&db, &id, views, info_rx).await {
                warn!("node info publisher stopped: {e:#}");
            }
        });
    }

    let mut tun_ip: Option<Ipv4Addr> = None;
    let mut last_status = None;
    let mut report = tokio::time::interval(Duration::from_secs(60));
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
        }
        let (status, view) = view_rx.borrow_and_update().clone();
        if last_status.as_ref() != Some(&status) {
            match &status {
                Status::NoNetwork => info!("waiting for network {} of owner {}", cfg.net, id.owner.to_bech32()?),
                Status::NotMember { epoch } => warn!(epoch, "not a member: waiting for approval (or removed)"),
                Status::Member => info!("member of network {}", cfg.net),
            }
            last_status = Some(status.clone());
        }
        let (Status::Member, Some(view)) = (&status, &view) else {
            if teardown_at.is_none() {
                teardown_at = Some(tokio::time::Instant::now() + MEMBERSHIP_GRACE);
            }
            continue;
        };
        teardown_at = None;
        let me = view.config.members.get(&keys.public_key().to_hex());
        if let (Some(me), false, None) = (me, no_tun, tun_ip) {
            let (_, prefix) = view.config.prefix()?;
            engine.start_tun(&dev, me.ip, prefix).await?;
            tun_ip = Some(me.ip);
        }
        if let (Some(me), Some(ip)) = (me, tun_ip) {
            if me.ip != ip {
                warn!(old = %ip, new = %me.ip, "our tailnet IP changed: restart toolnet to apply");
            }
        }
        let peers = peers_of(view, &keys.public_key());
        info!(epoch = view.epoch, peers = ?peers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), "netmap");
        engine.set_peers(peers).await;
    }
    engine.endpoint.close().await;
    Ok(())
}
