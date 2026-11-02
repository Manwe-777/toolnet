//! NIP-44 v2, compatible with tool-db's `nip44Encrypt`, `wrapGroupKey` and
//! `encryptForGroup` (packages/tool-db/lib/data/groups.ts):
//! - pairwise: conversation key of (our secret, their public key);
//! - group: conversation key of (group secret, group public key), so anyone
//!   holding the group key can encrypt and decrypt.

use anyhow::{Context, Result, anyhow};
use data_encoding::BASE64;
use nostr::nips::nip44::v2::{ConversationKey, decrypt_to_bytes, encrypt_to_bytes_with_nonce};
use nostr::prelude::{Keys, PublicKey, SecretKey};

pub fn pair_key(secret: &SecretKey, public: &PublicKey) -> Result<ConversationKey> {
    ConversationKey::derive(secret, public).map_err(|e| anyhow!("conversation key: {e}"))
}

pub fn group_key(group: &SecretKey) -> Result<ConversationKey> {
    pair_key(group, &Keys::new(group.clone()).public_key())
}

pub fn encrypt(key: &ConversationKey, plaintext: &str) -> Result<String> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|e| anyhow!("random: {e}"))?;
    let payload = encrypt_to_bytes_with_nonce(key, plaintext.as_bytes(), nonce)
        .map_err(|e| anyhow!("nip44 encrypt: {e}"))?;
    Ok(BASE64.encode(&payload))
}

pub fn decrypt(key: &ConversationKey, payload: &str) -> Result<String> {
    let bytes = BASE64.decode(payload.as_bytes()).context("nip44 payload is not base64")?;
    let plain = decrypt_to_bytes(key, &bytes).map_err(|e| anyhow!("nip44 decrypt: {e}"))?;
    String::from_utf8(plain).context("nip44 plaintext is not utf-8")
}

/// A new random group key
pub fn new_group_key() -> SecretKey {
    Keys::generate().secret_key().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_roundtrip() {
        let g = new_group_key();
        let k = group_key(&g).unwrap();
        let ct = encrypt(&k, "hello toolnet").unwrap();
        assert_eq!(decrypt(&group_key(&g).unwrap(), &ct).unwrap(), "hello toolnet");
    }

    #[test]
    fn pair_is_symmetric() {
        let (a, b) = (Keys::generate(), Keys::generate());
        let ct = encrypt(&pair_key(a.secret_key(), &b.public_key()).unwrap(), "x").unwrap();
        let pt = decrypt(&pair_key(b.secret_key(), &a.public_key()).unwrap(), &ct).unwrap();
        assert_eq!(pt, "x");
    }
}
