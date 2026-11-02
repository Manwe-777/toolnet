//! Control plane: follows the network in tool-db live and publishes our own
//! node info. Feeds the engine with the current peer set.

use std::{collections::BTreeSet, net::SocketAddr, time::Duration};

use anyhow::Result;
use nostr_sdk::prelude::*;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::{
    model::{self, NodeInfo, Status, View},
    tooldb::{Db, Records, verify_value},
};

/// Who we are and which network we follow
#[derive(Clone)]
pub struct Identity {
    pub device: Keys,
    pub owner: PublicKey,
    pub net: String,
}

/// Follows the network: keeps a subscription on the owner and every member,
/// recomputes the view on each change, and publishes it on `tx`
pub async fn follow(db: &Db, id: &Identity, tx: watch::Sender<(Status, Option<View>)>) -> Result<()> {
    let mut records = Records::default();
    let mut authors: BTreeSet<PublicKey> = BTreeSet::from([id.owner]);
    // A fresh subscription id each time the author set changes: nostr-sdk
    // ignores a subscribe with an id that already exists
    let mut generation = 0u32;
    let mut sub_id = SubscriptionId::new(format!("toolnet-{generation}"));
    db.client.subscribe(Db::filter(authors.clone())).with_id(sub_id.clone()).await?;
    let mut notifications = db.client.notifications();

    loop {
        let Some(n) = notifications.next().await else {
            anyhow::bail!("relay notifications ended");
        };
        let ClientNotification::Event { event, .. } = n else { continue };
        let Some(record) = verify_value(&event) else { continue };
        db.observe(&record).await;
        if !records.insert(record) {
            continue;
        }
        let (status, view) = model::view(&records, &id.device, &id.owner, &id.net);

        // Follow every member's node info: resubscribe when the member set
        // changes. Without a view (e.g. mid key rotation) keep what we follow
        let wanted: BTreeSet<PublicKey> = match &view {
            Some(v) => std::iter::once(id.owner)
                .chain(v.config.members.keys().filter_map(|k| PublicKey::from_hex(k).ok()))
                .collect(),
            None => authors.clone(),
        };
        if wanted != authors {
            debug!(count = wanted.len(), "following authors");
            authors = wanted;
            generation += 1;
            let old = std::mem::replace(&mut sub_id, SubscriptionId::new(format!("toolnet-{generation}")));
            db.client.subscribe(Db::filter(authors.clone())).with_id(sub_id.clone()).await?;
            let _ = db.client.unsubscribe(&old).await;
        }

        tx.send_if_modified(|current| {
            let changed = current.0 != status
                || match (&current.1, &view) {
                    (Some(a), Some(b)) => {
                        a.epoch != b.epoch || a.config != b.config || a.nodes != b.nodes
                    }
                    (None, None) => false,
                    _ => true,
                };
            if changed {
                *current = (status.clone(), view.clone());
            }
            changed
        });
    }
}

/// Publishes our node info whenever it changes (at most every 5 s)
pub async fn publish_node(
    db: &Db,
    id: &Identity,
    mut views: watch::Receiver<(Status, Option<View>)>,
    mut infos: watch::Receiver<NodeInfo>,
) -> Result<()> {
    let mut last: Option<(u64, NodeInfo)> = None;
    loop {
        let (status, view) = views.borrow_and_update().clone();
        let info = infos.borrow_and_update().clone();
        if let (Status::Member, Some(view)) = (status, view) {
            let same = last.as_ref().is_some_and(|(e, i)| {
                *e == view.epoch && i.addrs == info.addrs && i.relay == info.relay && i.endpoint_id == info.endpoint_id
            });
            if !same {
                let value = model::seal(&view.group, view.epoch, &info)?;
                match db.put(&id.device, &model::node_key(&id.owner, &id.net), &value).await {
                    Ok(_) => {
                        info!(addrs = ?info.addrs, relay = ?info.relay, epoch = view.epoch, "published node info");
                        last = Some((view.epoch, info));
                    }
                    Err(e) => warn!("publishing node info failed: {e:#}"),
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        tokio::select! {
            r = views.changed() => r?,
            r = infos.changed() => r?,
        }
    }
}

/// Addresses worth advertising to peers: drops loopback, link-local, Docker
/// and other virtual bridges, Tailscale and our own TUN, which peers can't
/// reach (and which push useful ones out of iroh's 32 address limit)
pub fn useful_addrs(addrs: impl IntoIterator<Item = SocketAddr>) -> Vec<SocketAddr> {
    let virtual_ips: Vec<std::net::IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| {
            let n = i.name.as_str();
            ["docker", "br-", "veth", "virbr", "tailscale", "toolnet", "zt", "wg", "lxc", "cni", "flannel"]
                .iter()
                .any(|p| n.starts_with(p))
        })
        .map(|i| i.ip())
        .collect();
    let mut out: Vec<SocketAddr> = addrs
        .into_iter()
        .filter(|a| {
            let ip = a.ip();
            !ip.is_loopback()
                && !virtual_ips.contains(&ip)
                && match ip {
                    std::net::IpAddr::V4(v4) => !v4.is_link_local() && !(v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64),
                    std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
                }
        })
        .collect();
    out.sort();
    out.dedup();
    out
}
