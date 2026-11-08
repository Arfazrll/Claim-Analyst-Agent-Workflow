//! Encryption at rest (AES-256-GCM), hashing and random tokens.
//!
//! Claim payloads and analyses contain health data, so they are stored encrypted in
//! the database and in IPFS. Only hashes of the plaintext go on-chain: a public
//! ledger is immutable, so personal health data must never be written to it.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};

const VERSION: &str = "v1";

#[derive(Clone)]
pub struct Cipher {
    key: Key<Aes256Gcm>,
}

impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cipher(..)")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("ciphertext is malformed")]
    Malformed,
    #[error("decryption failed (wrong key or tampered data)")]
    Decrypt,
}

impl Cipher {
    pub fn new(key: [u8; 32]) -> Self {
        Cipher {
            key: Key::<Aes256Gcm>::from(key),
        }
    }

    /// `v1:<base64(nonce || ciphertext+tag)>`. `aad` binds the ciphertext to its record.
    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> String {
        let cipher = Aes256Gcm::new(&self.key);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = cipher
            .encrypt(
                &nonce,
                aes_gcm::aead::Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .expect("AES-GCM encryption cannot fail for in-memory buffers");
        let mut out = nonce.to_vec();
        out.extend(ct);
        format!("{VERSION}:{}", STANDARD.encode(out))
    }

    pub fn decrypt(&self, envelope: &str, aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let body = envelope.strip_prefix("v1:").ok_or(CryptoError::Malformed)?;
        let raw = STANDARD.decode(body).map_err(|_| CryptoError::Malformed)?;
        if raw.len() < 12 + 16 {
            return Err(CryptoError::Malformed);
        }
        let (nonce, ct) = raw.split_at(12);
        Aes256Gcm::new(&self.key)
            .decrypt(
                Nonce::from_slice(nonce),
                aes_gcm::aead::Payload { msg: ct, aad },
            )
            .map_err(|_| CryptoError::Decrypt)
    }

    pub fn encrypt_json(&self, v: &Value, aad: &str) -> String {
        self.encrypt(canonical_json(v).as_bytes(), aad.as_bytes())
    }

    pub fn decrypt_json(&self, envelope: &str, aad: &str) -> Result<Value, CryptoError> {
        let bytes = self.decrypt(envelope, aad.as_bytes())?;
        serde_json::from_slice(&bytes).map_err(|_| CryptoError::Malformed)
    }
}

/// Deterministic JSON: object keys sorted recursively, no whitespace. Explicit so the
/// hash never depends on whether a dependency enables serde_json's `preserve_order`.
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn sha256_json(v: &Value) -> String {
    sha256_hex(canonical_json(v).as_bytes())
}

/// URL-safe random token with `bytes` bytes of entropy.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn constant_time_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper_detection() {
        let c = Cipher::new([7u8; 32]);
        let env = c.encrypt(b"patient data", b"claim-1");
        assert_eq!(c.decrypt(&env, b"claim-1").unwrap(), b"patient data");
        assert!(
            c.decrypt(&env, b"claim-2").is_err(),
            "AAD must bind ciphertext to its record"
        );
        let mut tampered = env.clone();
        tampered.pop();
        tampered.push('A');
        assert!(c.decrypt(&tampered, b"claim-1").is_err());
        assert!(Cipher::new([8u8; 32]).decrypt(&env, b"claim-1").is_err());
    }

    #[test]
    fn canonical_hash_is_key_order_independent() {
        let a: Value = serde_json::from_str(r#"{"b":1,"a":2}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":2,"b":1}"#).unwrap();
        assert_eq!(sha256_json(&a), sha256_json(&b));
    }
}
