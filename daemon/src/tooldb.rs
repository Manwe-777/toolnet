//! Minimal native ToolDB (Nostr-first) client: user-namespaced values only.
//!
//! Wire format (tool-db/NOSTR_FIRST.md, packages/tool-db/lib/data/*.ts):
//! - a value is a kind 30078 event, tags `["d", "<topic>:<finalKey>"]` and
//!   `["t", "<topic>"]`, content `JSON.stringify(value)`;
//! - a user-namespaced final key is `:<hex pubkey>.<key>`, and only that
//!   pubkey may write it;
//! - newest `created_at` wins, ties go to the lowest event id;
//! - events more than 30 s in the future are rejected.

use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result, bail};
use nostr_sdk::prelude::*;
use serde_json::Value;
use tokio::sync::Mutex;

pub const TOPIC: &str = "toolnet";
pub const VALUE_KIND: u16 = 30078;
const MAX_FUTURE_SECS: u64 = 30;

/// The final key of a user-namespaced key
pub fn final_key(author: &PublicKey, key: &str) -> String {
    format!(":{}.{key}", author.to_hex())
}

/// A verified value
#[derive(Debug, Clone)]
pub struct Record {
    pub final_key: String,
    #[allow(dead_code)]
    pub author: PublicKey,
    pub created_at: Timestamp,
    pub id: EventId,
    pub value: Value,
}

impl Record {
    pub fn is_newer_than(&self, other: &Record) -> bool {
        self.created_at > other.created_at
            || (self.created_at == other.created_at && self.id < other.id)
    }
}

/// Verifies a value event the way ToolDB clients do. Returns None for
/// anything that isn't a valid, user-namespaced value of our topic.
pub fn verify_value(event: &Event) -> Option<Record> {
    if event.kind.as_u16() != VALUE_KIND || event.verify().is_err() {
        return None;
    }
    if event.created_at.as_secs() > Timestamp::now().as_secs() + MAX_FUTURE_SECS {
        return None;
    }
    let d = event.tags.identifier()?;
    let final_key = d.strip_prefix(TOPIC)?.strip_prefix(':')?;
    // User namespace: only the owner of the namespace may write it
    let owner = final_key.strip_prefix(':')?.split('.').next()?;
    if owner != event.pubkey.to_hex() {
        return None;
    }
    let value = serde_json::from_str(&event.content).ok()?;
    Some(Record {
        final_key: final_key.to_string(),
        author: event.pubkey,
        created_at: event.created_at,
        id: event.id,
        value,
    })
}

/// Latest verified value per final key
#[derive(Debug, Default, Clone)]
pub struct Records(pub HashMap<String, Record>);

impl Records {
    /// Keeps the record if it is newer than the one we have. Returns whether it changed
    pub fn insert(&mut self, record: Record) -> bool {
        match self.0.get(&record.final_key) {
            Some(existing) if !record.is_newer_than(existing) => false,
            _ => {
                self.0.insert(record.final_key.clone(), record);
                true
            }
        }
    }

    pub fn get(&self, author: &PublicKey, key: &str) -> Option<&Record> {
        self.0.get(&final_key(author, key))
    }
}

pub struct Db {
    pub client: Client,
    /// Last created_at we used per final key, so updates within the same
    /// second still win (like ToolDB's putData)
    last_put: Mutex<HashMap<String, u64>>,
}

impl Db {
    pub async fn connect(relays: &[String]) -> Result<Self> {
        if relays.is_empty() {
            bail!("no Nostr relays configured");
        }
        let client = Client::default();
        for relay in relays {
            client
                .add_relay(relay.as_str())
                .await
                .with_context(|| format!("adding relay {relay}"))?;
        }
        client.connect().and_wait(Duration::from_secs(5)).await;
        Ok(Self { client, last_put: Default::default() })
    }

    /// Filter for every value of our topic written by these authors
    pub fn filter(authors: impl IntoIterator<Item = PublicKey>) -> Filter {
        Filter::new()
            .kind(Kind::Custom(VALUE_KIND))
            .authors(authors)
            .hashtag(TOPIC)
    }

    /// Writes a user-namespaced value, signed by `keys`
    pub async fn put(&self, keys: &Keys, key: &str, value: &Value) -> Result<Event> {
        if key.contains('.') {
            bail!("ToolDB keys cannot contain dots: {key}");
        }
        let fk = final_key(&keys.public_key(), key);
        let mut created_at = Timestamp::now().as_secs();
        {
            let mut last = self.last_put.lock().await;
            if let Some(prev) = last.get(&fk) {
                created_at = created_at.max(prev + 1);
            }
            last.insert(fk.clone(), created_at);
        }
        let event = EventBuilder::new(Kind::Custom(VALUE_KIND), serde_json::to_string(value)?)
            .tags([
                Tag::identifier(format!("{TOPIC}:{fk}")),
                Tag::hashtag(TOPIC),
            ])
            .custom_created_at(Timestamp::from(created_at))
            .finalize(keys)?;
        let out = self.client.send_event(&event).await.context("publishing")?;
        if out.success.is_empty() {
            bail!("no relay accepted {fk}: {:?}", out.failed);
        }
        Ok(event)
    }

    /// Remembers a value we saw, so our next write to its key is newer even
    /// within the same second (newest created_at wins, ties are random)
    pub async fn observe(&self, record: &Record) {
        let mut last = self.last_put.lock().await;
        let entry = last.entry(record.final_key.clone()).or_insert(0);
        *entry = (*entry).max(record.created_at.as_secs());
    }

    /// Fetches the latest values of these authors
    pub async fn fetch(&self, authors: impl IntoIterator<Item = PublicKey>) -> Result<Records> {
        let events = self
            .client
            .fetch_events(Self::filter(authors))
            .timeout(Duration::from_secs(8))
            .await?;
        let mut records = Records::default();
        for event in events.iter() {
            if let Some(record) = verify_value(event) {
                self.observe(&record).await;
                records.insert(record);
            }
        }
        Ok(records)
    }
}
