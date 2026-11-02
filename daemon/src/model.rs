//! toolnet's data in tool-db (docs/data-model.md). O = owner, D = device,
//! N = network id. All keys are user-namespaced, so only their writer can
//! change them:
//!
//!   :O.net-N       {"v":1,"epoch":E,"data":<group(E) ciphertext of NetConfig>}
//!   :O.gk-N-D      {"v":1,"epoch":E,"key":<NIP-44 O->D ciphertext of group key E>}
//!   :D.node-O-N    {"v":1,"epoch":E,"data":<group(E) ciphertext of NodeInfo>}
//!
//! The owner wraps the group key for itself too (`:O.gk-N-O`), so the owner's
//! secret key alone is enough to recover and administer the network.

use std::{
    collections::{BTreeMap, HashMap},
    net::Ipv4Addr,
};

use anyhow::{Context, Result, anyhow, bail};
use nostr::prelude::{Keys, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{crypto, tooldb::Records};

pub fn net_key(net: &str) -> String {
    format!("net-{net}")
}
pub fn gk_key(net: &str, device: &PublicKey) -> String {
    format!("gk-{net}-{}", device.to_hex())
}
pub fn node_key(owner: &PublicKey, net: &str) -> String {
    format!("node-{}-{net}", owner.to_hex())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NetConfig {
    pub name: String,
    /// Tailnet IPv4 range, e.g. "10.213.0.0/24". Not 100.64.0.0/10: toolnet
    /// must coexist with Tailscale
    pub ipv4: String,
    /// Device public key (hex) -> member
    pub members: BTreeMap<String, Member>,
    pub updated_at: u64,
    /// MagicDNS domain: members resolve as `<name>.<domain>`.
    /// Default `<name>.toolnet`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub name: String,
    pub ip: Ipv4Addr,
    pub added_at: u64,
    /// Prefixes the owner allows this member to route (subnets, 0.0.0.0/0
    /// for an exit node). Only used if the member also advertises them
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    /// iroh endpoint id (ed25519 public key, hex)
    pub endpoint_id: String,
    /// Direct addresses worth trying (LAN, public)
    pub addrs: Vec<String>,
    /// Home relay URL
    pub relay: Option<String>,
    pub hostname: String,
    pub version: String,
    pub updated_at: u64,
    /// Prefixes this device offers to route
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<String>,
}

impl NetConfig {
    pub fn dns_domain(&self) -> String {
        self.domain.clone().unwrap_or_else(|| format!("{}.toolnet", crate::dns::label(&self.name)))
    }

    pub fn prefix(&self) -> Result<(Ipv4Addr, u8)> {
        let (ip, len) = self.ipv4.split_once('/').context("ipv4 must be a CIDR")?;
        Ok((ip.parse()?, len.parse()?))
    }

    /// The lowest free host address in the range
    pub fn next_free_ip(&self) -> Result<Ipv4Addr> {
        let (base, len) = self.prefix()?;
        let base = u32::from(base) & (u32::MAX.checked_shl(32 - len as u32).unwrap_or(0));
        let size = 1u32 << (32 - len as u32);
        let used: Vec<Ipv4Addr> = self.members.values().map(|m| m.ip).collect();
        (1..size - 1)
            .map(|i| Ipv4Addr::from(base + i))
            .find(|ip| !used.contains(ip))
            .ok_or_else(|| anyhow!("no free addresses in {}", self.ipv4))
    }
}

/// Encrypts a value for the group, in the envelope stored in tool-db
pub fn seal(group: &SecretKey, epoch: u64, value: &impl Serialize) -> Result<Value> {
    let data = crypto::encrypt(&crypto::group_key(group)?, &serde_json::to_string(value)?)?;
    Ok(json!({ "v": 1, "epoch": epoch, "data": data }))
}

fn open<T: for<'de> Deserialize<'de>>(group: &SecretKey, envelope: &Value) -> Result<(u64, T)> {
    let epoch = envelope["epoch"].as_u64().context("missing epoch")?;
    let data = envelope["data"].as_str().context("missing data")?;
    let plain = crypto::decrypt(&crypto::group_key(group)?, data)?;
    Ok((epoch, serde_json::from_str(&plain)?))
}

/// Wraps the group key for a member, signed and encrypted by the owner
pub fn wrap_group_key(owner: &Keys, member: &PublicKey, group: &SecretKey, epoch: u64) -> Result<Value> {
    let key = crypto::encrypt(&crypto::pair_key(owner.secret_key(), member)?, &group.to_secret_hex())?;
    Ok(json!({ "v": 1, "epoch": epoch, "key": key }))
}

fn unwrap_group_key(me: &Keys, owner: &PublicKey, envelope: &Value) -> Result<(u64, SecretKey)> {
    let epoch = envelope["epoch"].as_u64().context("missing epoch")?;
    let wrapped = envelope["key"].as_str().context("missing key")?;
    let hex = crypto::decrypt(&crypto::pair_key(me.secret_key(), owner)?, wrapped)?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid group key");
    }
    Ok((epoch, SecretKey::from_hex(&hex)?))
}

/// What a device (or the owner) can see of the network
#[derive(Debug, Clone)]
pub struct View {
    pub epoch: u64,
    pub group: SecretKey,
    pub config: NetConfig,
    /// Node info of current members, by device public key. Only info sealed
    /// with the current epoch counts
    pub nodes: HashMap<PublicKey, NodeInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// The owner has published nothing for this network (yet)
    NoNetwork,
    /// No group key for us at the network's epoch: not approved, or removed
    NotMember { epoch: u64 },
    Member,
}

/// Computes the view from the records, as `me` (a device, or the owner).
/// Every check here is identity based: the net must be signed by the owner,
/// our group key wrapped by the owner, node infos signed by listed members.
pub fn view(records: &Records, me: &Keys, owner: &PublicKey, net: &str) -> (Status, Option<View>) {
    let Some(net_rec) = records.get(owner, &net_key(net)) else {
        return (Status::NoNetwork, None);
    };
    let Some(epoch) = net_rec.value["epoch"].as_u64() else {
        return (Status::NoNetwork, None);
    };
    let group = records
        .get(owner, &gk_key(net, &me.public_key()))
        .and_then(|r| unwrap_group_key(me, owner, &r.value).ok())
        .filter(|(e, _)| *e == epoch)
        .map(|(_, g)| g);
    let Some(group) = group else {
        return (Status::NotMember { epoch }, None);
    };
    let Ok((_, config)) = open::<NetConfig>(&group, &net_rec.value) else {
        return (Status::NotMember { epoch }, None);
    };
    let mut nodes = HashMap::new();
    for device in config.members.keys() {
        let Ok(pk) = PublicKey::from_hex(device) else { continue };
        if let Some(rec) = records.get(&pk, &node_key(owner, net)) {
            if let Ok((e, info)) = open::<NodeInfo>(&group, &rec.value) {
                if e == epoch {
                    nodes.insert(pk, info);
                }
            }
        }
    }
    let status = if me.public_key() == *owner || config.members.contains_key(&me.public_key().to_hex()) {
        Status::Member
    } else {
        Status::NotMember { epoch }
    };
    (status, Some(View { epoch, group, config, nodes }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tooldb::{Record, Records};
    use nostr::prelude::{EventId, Timestamp};

    fn rec(author: &PublicKey, key: &str, value: Value, t: u64) -> Record {
        Record {
            final_key: crate::tooldb::final_key(author, key),
            author: *author,
            created_at: Timestamp::from(t),
            id: EventId::from_byte_array([0u8; 32]),
            value,
        }
    }

    #[test]
    fn approve_view_and_remove() {
        let (owner, laptop, server) = (Keys::generate(), Keys::generate(), Keys::generate());
        let o = owner.public_key();
        let g1 = crypto::new_group_key();
        let mut config = NetConfig {
            name: "home".into(),
            ipv4: "10.213.0.0/24".into(),
            members: BTreeMap::new(),
            updated_at: 0,
            domain: None,
        };
        for (k, name) in [(&laptop, "laptop"), (&server, "server")] {
            let ip = config.next_free_ip().unwrap();
            config.members.insert(k.public_key().to_hex(), Member { name: name.into(), ip, added_at: 0, routes: vec![] });
        }
        assert_eq!(config.members[&server.public_key().to_hex()].ip, Ipv4Addr::new(10, 213, 0, 2));

        let mut records = Records::default();
        records.insert(rec(&o, &net_key("home"), seal(&g1, 1, &config).unwrap(), 1));
        for k in [&laptop, &server] {
            let w = wrap_group_key(&owner, &k.public_key(), &g1, 1).unwrap();
            records.insert(rec(&o, &gk_key("home", &k.public_key()), w, 1));
        }
        let info = NodeInfo {
            endpoint_id: "e".into(),
            addrs: vec![],
            relay: None,
            hostname: "s".into(),
            version: "0".into(),
            updated_at: 0,
            routes: vec![],
        };
        records.insert(rec(&server.public_key(), &node_key(&o, "home"), seal(&g1, 1, &info).unwrap(), 1));

        let (status, v) = view(&records, &laptop, &o, "home");
        assert_eq!(status, Status::Member);
        assert_eq!(v.unwrap().nodes[&server.public_key()], info);

        // Remove the server: new epoch and group key, rewrapped for the laptop only
        let g2 = crypto::new_group_key();
        config.members.remove(&server.public_key().to_hex());
        records.insert(rec(&o, &net_key("home"), seal(&g2, 2, &config).unwrap(), 2));
        let w = wrap_group_key(&owner, &laptop.public_key(), &g2, 2).unwrap();
        records.insert(rec(&o, &gk_key("home", &laptop.public_key()), w, 2));

        assert_eq!(view(&records, &server, &o, "home").0, Status::NotMember { epoch: 2 });
        let (status, v) = view(&records, &laptop, &o, "home");
        assert_eq!(status, Status::Member);
        // The server's node info is from epoch 1 and it's no longer a member
        assert!(v.unwrap().nodes.is_empty());
    }
}
