//! A running device: engine, Nostr connection, following the network and
//! publishing our node info. The Linux daemon and the Android app both start
//! one and then do their own OS integration (TUN, routes, DNS) in a loop over
//! `views`.

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use nostr_sdk::prelude::{Keys, PublicKey, ToBech32};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::{
    control::{self, Identity},
    engine::{self, Engine, EngineConfig, Peer},
    model::{NodeInfo, Status, View},
    state::{Config, State, now, parse_pubkey},
    tooldb::Db,
};

/// How long a lost membership must last before we drop our peers
pub const MEMBERSHIP_GRACE: Duration = Duration::from_secs(15);

/// Peers for the engine: members other than us, with valid node info. A
/// default route is only kept for the exit node we chose
pub fn peers_of(view: &View, me: &PublicKey, exit_node: Option<&str>) -> Vec<Peer> {
    let mut peers = vec![];
    for (key, member) in &view.config.members {
        let Ok(pk) = PublicKey::from_hex(key) else { continue };
        if pk == *me {
            continue;
        }
        let Some(node) = view.nodes.get(&pk) else { continue };
        let Ok(endpoint) = engine::parse_endpoint_id(&node.endpoint_id) else { continue };
        // Routed only if the owner allows it AND the device offers it
        let allowed: Vec<engine::Cidr> = member.routes.iter().filter_map(|r| r.parse().ok()).collect();
        let routes = node
            .routes
            .iter()
            .filter_map(|r| r.parse::<engine::Cidr>().ok())
            .filter(|r| allowed.contains(r))
            .filter(|r| r.len > 0 || exit_node == Some(member.name.as_str()))
            .collect();
        peers.push(Peer {
            name: member.name.clone(),
            ip: member.ip,
            endpoint,
            addrs: node.addrs.iter().filter_map(|a| a.parse().ok()).collect(),
            relay: node.relay.as_deref().and_then(|r| r.parse().ok()),
            routes,
        });
    }
    peers
}

pub struct NodeOpts {
    /// No iroh relays, direct addresses only (offline tests)
    pub offline: bool,
    pub bind: Option<SocketAddr>,
    pub iroh_relays: Vec<iroh::RelayUrl>,
    pub relay_ca: Option<PathBuf>,
    /// Prefixes we offer to route (0.0.0.0/0: exit node)
    pub advertise_routes: Vec<engine::Cidr>,
    /// Our name in node info
    pub hostname: String,
}

pub struct Node {
    pub engine: Engine,
    pub db: Arc<Db>,
    pub id: Identity,
    pub keys: Keys,
    pub cfg: Config,
    /// Our membership and the network as we see it, kept current
    pub views: watch::Receiver<(Status, Option<View>)>,
}

impl Node {
    pub async fn start(state: &State, opts: NodeOpts) -> Result<Self> {
        let (keys, cfg) = (state.device_keys()?, state.config()?);
        let id = Identity { device: keys.clone(), owner: parse_pubkey(&cfg.owner)?, net: cfg.net.clone() };
        let engine = Engine::start(EngineConfig {
            secret: state.endpoint_key()?,
            offline: opts.offline,
            relays: opts.iroh_relays,
            bind: opts.bind,
            relay_ca: opts.relay_ca,
        })
        .await?;
        info!(device = %keys.public_key().to_bech32()?, endpoint = %engine.id(), "toolnet up");
        if !opts.offline {
            engine.spawn_punch_assist();
        }
        let db = Arc::new(Db::connect(&cfg.relays).await?);

        // Our node info, kept current from the endpoint's addresses
        let endpoint_id = engine.id().to_string();
        let advertised: Vec<String> = opts.advertise_routes.iter().map(|r| r.to_string()).collect();
        let hostname = opts.hostname;
        let make_info = move |addr: &iroh::EndpointAddr| NodeInfo {
            routes: advertised.clone(),
            endpoint_id: endpoint_id.clone(),
            addrs: control::useful_addrs(addr.ip_addrs().copied()).iter().map(|a| a.to_string()).collect(),
            relay: addr.relay_urls().next().map(|r| r.to_string()),
            hostname: hostname.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            updated_at: now(),
        };
        let (info_tx, info_rx) = watch::channel(make_info(&engine.endpoint.addr()));
        {
            let (engine, db) = (engine.clone(), db.clone());
            tokio::spawn(async move {
                use n0_watcher::Watcher;
                let mut w = engine.endpoint.watch_addr();
                let local = |a: &iroh::EndpointAddr| -> std::collections::BTreeSet<std::net::IpAddr> {
                    // Only real interfaces: our own TUN (and Docker, Tailscale...)
                    // coming and going is not a network switch
                    let public = control::useful_addrs(a.ip_addrs().copied()).iter().map(|s| s.ip()).collect::<std::collections::BTreeSet<_>>();
                    if_addrs::get_if_addrs().unwrap_or_default().into_iter().map(|i| i.ip()).filter(|ip| public.contains(ip)).collect()
                };
                let mut last_local = local(&engine.endpoint.addr());
                while let Ok(addr) = w.updated().await {
                    // Our own interface addresses changed: a network switch. The
                    // Nostr websockets on the old network are dead: reconnect now
                    let now_local = local(&addr);
                    if now_local != last_local {
                        info!(from = ?last_local, to = ?now_local, "network changed");
                        last_local = now_local;
                        let db = db.clone();
                        tokio::spawn(async move { db.reconnect().await });
                    }
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

        let (view_tx, views) = watch::channel((Status::NoNetwork, None::<View>));
        {
            let (db, id, view_tx) = (db.clone(), id.clone(), view_tx);
            tokio::spawn(async move {
                if let Err(e) = control::follow(&db, &id, view_tx).await {
                    warn!("control plane stopped: {e:#}");
                }
            });
        }
        {
            let (db, id, views) = (db.clone(), id.clone(), views.clone());
            tokio::spawn(async move {
                if let Err(e) = control::publish_node(&db, &id, views, info_rx).await {
                    warn!("node info publisher stopped: {e:#}");
                }
            });
        }
        Ok(Self { engine, db, id, keys, cfg, views })
    }

    pub fn me(&self) -> PublicKey {
        self.keys.public_key()
    }
}

/// Logs membership changes once each
pub fn log_status(status: &Status, cfg: &Config, owner: &PublicKey) {
    match status {
        Status::NoNetwork => info!("waiting for network {} of owner {}", cfg.net, owner.to_bech32().unwrap_or_default()),
        Status::NotMember { epoch } => warn!(epoch, "not a member: waiting for approval (or removed)"),
        Status::Member => info!("member of network {}", cfg.net),
    }
}

#[allow(unused)]
const _: Duration = MEMBERSHIP_GRACE;
