//! AES-256-GCM encryption of a [`SyntheticTwin`](super::manager::SyntheticTwin)
//! at rest, keyed by resolved material rather than a compiled-in constant
//! (T-008/D8).
//!
//! **What this protects, honestly stated:** the twin payload is
//! *synthetic* PII — a fake name, email, password, card number and SSN,
//! generated fresh by [`super::manager::SyntheticTwin::generate`] for the
//! dry run to hand out when the agent asks for "the user's" data. None of
//! it identifies a real person or grants access to anything. Encrypting the
//! on-disk cache file is defense-in-depth hygiene (don't leave *any*
//! structured-looking secret-shaped file world-readable on principle, and
//! don't get in the habit of writing plaintext-secret-shaped files even
//! when today's payload is fake) — it is not standing in for a security
//! claim about protecting real user data. If a future change ever routes
//! real user data through this path, that is a different, much bigger
//! decision than "reuse this encryption function."

use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit};
use rand::Rng;
use sha2::{Digest, Sha256};

use ferrite_model::secret::Token;

use super::manager::SyntheticTwin;

/// A 32-byte AES-256 key, derived from resolved secret material.
///
/// Deliberately opaque — the only way to get one is [`derive_key`], so a
/// caller cannot accidentally hand [`encrypt_twin`]/[`decrypt_twin`] the raw
/// [`Token`] bytes (wrong length in general) or a stray literal.
#[derive(Clone, Copy)]
pub struct TwinKey([u8; 32]);

impl std::fmt::Debug for TwinKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TwinKey(<redacted>)")
    }
}

/// Derives a 32-byte AES-256 key from resolved secret material via SHA-256.
///
/// A plain hash, not a password-based KDF (no salt, no iteration count) —
/// deliberately: this key protects a cache file of *synthetic* data (see
/// module docs), so the cost/complexity of a proper KDF buys nothing here.
/// Do not copy this function for a key that protects real secrets.
#[must_use]
pub fn derive_key(secret: &Token) -> TwinKey {
    let digest = Sha256::digest(secret.expose().as_bytes());
    TwinKey(digest.into())
}

#[cfg(any(test, feature = "test-util"))]
impl TwinKey {
    /// Builds a key directly from 32 bytes, bypassing resolution — for
    /// tests that want a fixed, reproducible key without going through
    /// [`derive_key`]/a [`Token`].
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// Encrypts a [`SyntheticTwin`] with AES-256-GCM.
/// Returns nonce (12 bytes) prepended to ciphertext.
pub fn encrypt_twin(twin: &SyntheticTwin, key: &TwinKey) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(twin).map_err(|e| e.to_string())?;
    let cipher = Aes256Gcm::new(GenericArray::from_slice(&key.0));
    let nonce_bytes: [u8; 12] = rand::thread_rng().gen();
    let nonce = GenericArray::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, json.as_ref())
        .map_err(|e| format!("encrypt: {}", e))?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypts bytes produced by [`encrypt_twin`] back into a [`SyntheticTwin`].
pub fn decrypt_twin(data: &[u8], key: &TwinKey) -> Result<SyntheticTwin, String> {
    if data.len() < 12 {
        return Err("data too short".to_string());
    }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let cipher = Aes256Gcm::new(GenericArray::from_slice(&key.0));
    let nonce = GenericArray::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("decrypt: {}", e))?;
    serde_json::from_slice(&plaintext).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> TwinKey {
        TwinKey::from_bytes(*b"ferrite-test-only-key-32-bytes!!")
    }

    #[test]
    fn derive_key_is_deterministic_for_the_same_secret() {
        let secret = Token::new("some-resolved-secret");
        let k1 = derive_key(&secret);
        let k2 = derive_key(&secret);
        assert_eq!(k1.0, k2.0);
    }

    #[test]
    fn derive_key_differs_for_different_secrets() {
        let k1 = derive_key(&Token::new("secret-one"));
        let k2 = derive_key(&Token::new("secret-two"));
        assert_ne!(k1.0, k2.0);
    }

    #[test]
    fn twin_key_debug_never_prints_bytes() {
        let key = test_key();
        assert_eq!(format!("{key:?}"), "TwinKey(<redacted>)");
    }

    #[test]
    fn encrypt_decrypt_roundtrip_with_a_resolved_key() {
        let key = derive_key(&Token::new("test-only-resolved-secret"));
        let twin = SyntheticTwin::generate();
        let enc = encrypt_twin(&twin, &key).expect("encrypt failed");
        let dec = decrypt_twin(&enc, &key).expect("decrypt failed");
        assert_eq!(dec.email, twin.email);
        assert_eq!(dec.name, twin.name);
    }

    #[test]
    fn decrypting_with_the_wrong_key_fails() {
        let twin = SyntheticTwin::generate();
        let enc = encrypt_twin(&twin, &derive_key(&Token::new("key-a"))).unwrap();
        let wrong = derive_key(&Token::new("key-b"));
        assert!(decrypt_twin(&enc, &wrong).is_err());
    }
}
