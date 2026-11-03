//! A device's files: keys and config in a state directory

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use nostr_sdk::prelude::{FromBech32, Keys, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};

pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
    "wss://nostr.mom",
];

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub owner: String,
    pub net: String,
    pub relays: Vec<String>,
}

/// What a device's QR code carries: `toolnet:join?device=<npub>&name=<name>`
pub fn join_payload(npub: &str, name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "-._".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32 & 0xff) })
        .collect();
    format!("toolnet:join?device={npub}&name={name}")
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

pub fn parse_pubkey(s: &str) -> Result<PublicKey> {
    PublicKey::from_bech32(s)
        .or_else(|_| PublicKey::from_hex(s))
        .with_context(|| format!("not an npub or hex public key: {s}"))
}

pub struct State {
    pub dir: PathBuf,
}

impl State {
    pub fn new(dir: Option<PathBuf>) -> Result<Self> {
        let dir = match dir {
            Some(d) => d,
            None => dirs::data_local_dir().context("no data dir")?.join("toolnet"),
        };
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn read_secret(&self, name: &str) -> Result<Option<String>> {
        match std::fs::read_to_string(self.path(name)) {
            Ok(s) => Ok(Some(s.trim().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn write_secret(&self, name: &str, hex: &str) -> Result<()> {
        write_private(&self.path(name), hex)
    }

    pub fn device_keys(&self) -> Result<Keys> {
        let hex = self.read_secret("device.key")?.context("no device key: run `toolnet init` first")?;
        Ok(Keys::new(SecretKey::from_hex(&hex)?))
    }

    pub fn owner_keys(&self) -> Result<Keys> {
        let hex = self.read_secret("owner.key")?.context("no owner key on this machine: `toolnet owner create`")?;
        Ok(Keys::new(SecretKey::from_hex(&hex)?))
    }

    pub fn endpoint_key(&self) -> Result<iroh::SecretKey> {
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

    pub fn config(&self) -> Result<Config> {
        let s = std::fs::read_to_string(self.path("config.json")).context("no config: run `toolnet init` first")?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn save_config(&self, c: &Config) -> Result<()> {
        std::fs::write(self.path("config.json"), serde_json::to_string_pretty(c)?)?;
        Ok(())
    }
}

pub fn write_private(path: &Path, contents: &str) -> Result<()> {
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

pub fn relays_or_default(relays: Vec<String>) -> Vec<String> {
    if relays.is_empty() {
        DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
    } else {
        relays
    }
}

/// This machine's name for node info (Linux: /etc/hostname)
pub fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}
