// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
//! Argon2id + XSalsa20-Poly1305 vault blobs.
//! INTERACTIVE ops=2, mem=64 MiB, parallel=1, 16-byte salt, 24-byte nonce.

use argon2::{Algorithm, Argon2, Params, Version};
use crypto_secretbox::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Key, Nonce, XSalsa20Poly1305,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::{Error, Result};

const SALT_LEN: usize = 16;
const M_COST_KIB: u32 = 65_536; // 64 MiB
const T_COST: u32 = 2;
const P_COST: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EncryptedBlob {
    pub salt: String,
    pub nonce: String,
    pub ciphertext: String,
}

fn kdf(password: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let params = Params::new(M_COST_KIB, T_COST, P_COST, Some(32))
        .map_err(|e| Error::msg(format!("argon2 params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| Error::msg(format!("argon2: {e}")))?;
    Ok(key)
}

fn b64enc(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
}

fn b64dec(s: &str) -> Result<Vec<u8>> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s.trim())
        .map_err(|_| Error::msg("invalid base64"))
}

pub fn encrypt(plaintext: &[u8], password: &str) -> Result<EncryptedBlob> {
    if password.len() < 8 {
        return Err(Error::msg("password must be at least 8 characters"));
    }
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let mut key_bytes = kdf(password, &salt)?;
    let cipher = XSalsa20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = XSalsa20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| Error::msg("encrypt failed"))?;
    key_bytes.zeroize();
    Ok(EncryptedBlob {
        salt: b64enc(&salt),
        nonce: b64enc(nonce.as_slice()),
        ciphertext: b64enc(&ciphertext),
    })
}

pub fn decrypt(blob: &EncryptedBlob, password: &str) -> Result<Vec<u8>> {
    let salt = b64dec(&blob.salt)?;
    let nonce_bytes = b64dec(&blob.nonce)?;
    let ciphertext = b64dec(&blob.ciphertext)?;
    if salt.len() != SALT_LEN || nonce_bytes.len() != 24 {
        return Err(Error::msg("corrupt vault blob"));
    }
    let mut key_bytes = kdf(password, &salt)?;
    let cipher = XSalsa20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| Error::WrongPassword)?;
    key_bytes.zeroize();
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let blob = encrypt(b"hello-old-growth", "hunter2-not-really").unwrap();
        let out = decrypt(&blob, "hunter2-not-really").unwrap();
        assert_eq!(out, b"hello-old-growth");
        assert!(matches!(decrypt(&blob, "wrong-password"), Err(Error::WrongPassword)));
    }
}
