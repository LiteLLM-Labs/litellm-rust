//! AES-256-GCM encryption for user MCP credentials at rest.
//!
//! The key is derived from the gateway master key (`SHA-256(master_key)`), so
//! rotating the master key invalidates stored credentials by design. Stored
//! bytes are `12-byte nonce || ciphertext`.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use sha2::{Digest, Sha256};

use crate::errors::GatewayError;

const NONCE_LEN: usize = 12;

/// Derive a 32-byte AES key from the gateway master key.
pub fn derive_key(master_key: &str) -> [u8; 32] {
    let digest = Sha256::digest(master_key.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

/// Encrypt plaintext, returning `nonce || ciphertext`.
pub fn encrypt(key: &[u8; 32], plaintext: &str) -> Result<Vec<u8>, GatewayError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce_bytes = random_nonce();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|error| GatewayError::Crypto(format!("encrypt failed: {error}")))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt bytes produced by [`encrypt`].
pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<String, GatewayError> {
    if data.len() < NONCE_LEN {
        return Err(GatewayError::Crypto("ciphertext too short".to_owned()));
    }
    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|error| GatewayError::Crypto(format!("decrypt failed: {error}")))?;
    String::from_utf8(plaintext)
        .map_err(|error| GatewayError::Crypto(format!("decrypted bytes not utf-8: {error}")))
}

/// 12 random bytes sourced from a v4 UUID (getrandom-backed).
fn random_nonce() -> [u8; NONCE_LEN] {
    let uuid = uuid::Uuid::new_v4();
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&uuid.as_bytes()[..NONCE_LEN]);
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_recovers_plaintext() {
        let key = derive_key("sk-master");
        let blob = encrypt(&key, "ya29.secret-token").unwrap();
        assert_eq!(decrypt(&key, &blob).unwrap(), "ya29.secret-token");
    }

    #[test]
    fn nonce_differs_per_encryption() {
        let key = derive_key("sk-master");
        let a = encrypt(&key, "same").unwrap();
        let b = encrypt(&key, "same").unwrap();
        assert_ne!(a, b, "ciphertext must not be deterministic");
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let blob = encrypt(&derive_key("sk-master"), "secret").unwrap();
        assert!(decrypt(&derive_key("sk-other"), &blob).is_err());
    }
}
