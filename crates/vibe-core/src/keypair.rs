// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{Error, Result};
use crate::hd::{self, DEFAULT_PATH};
use crate::mnemonic;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Keypair {
    seed: [u8; 32],
    public: [u8; 32],
}

impl Keypair {
    pub fn generate() -> Self {
        let signing = SigningKey::generate(&mut OsRng);
        Self::from_signing(signing)
    }

    pub fn from_seed32(seed: &[u8]) -> Result<Self> {
        if seed.len() != 32 {
            return Err(Error::msg("seed must be 32 bytes"));
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(seed);
        let signing = SigningKey::from_bytes(&bytes);
        bytes.zeroize();
        Ok(Self::from_signing(signing))
    }

    pub fn from_secret64(secret: &[u8]) -> Result<Self> {
        if secret.len() != 64 {
            return Err(Error::msg("secret key must be 64 bytes"));
        }
        Self::from_seed32(&secret[..32])
    }

    pub fn from_mnemonic(phrase: &str, passphrase: &str, path: &str) -> Result<Self> {
        let mut seed64 = mnemonic::to_seed(phrase, passphrase)?;
        let mut seed32 = hd::derive_seed32(&seed64, path)?;
        seed64.zeroize();
        let kp = Self::from_seed32(&seed32);
        seed32.zeroize();
        kp
    }

    pub fn from_mnemonic_default(phrase: &str) -> Result<Self> {
        Self::from_mnemonic(phrase, "", DEFAULT_PATH)
    }

    fn from_signing(signing: SigningKey) -> Self {
        let public = signing.verifying_key().to_bytes();
        let seed = signing.to_bytes();
        Self { seed, public }
    }

    pub fn address(&self) -> String {
        bs58::encode(self.public).into_string()
    }

    pub fn public_bytes(&self) -> [u8; 32] {
        self.public
    }

    pub fn secret64(&self) -> [u8; 64] {
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(&self.seed);
        out[32..].copy_from_slice(&self.public);
        out
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let signing = SigningKey::from_bytes(&self.seed);
        signing.sign(message).to_bytes()
    }

    pub fn verify(pubkey: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
        let Ok(vk) = VerifyingKey::from_bytes(pubkey) else {
            return false;
        };
        let sig = ed25519_dalek::Signature::from_bytes(signature);
        vk.verify_strict(message, &sig).is_ok()
    }
}

impl Clone for Keypair {
    fn clone(&self) -> Self {
        Self {
            seed: self.seed,
            public: self.public,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify() {
        let kp = Keypair::generate();
        let msg = b"old growth";
        let sig = kp.sign(msg);
        assert!(Keypair::verify(&kp.public_bytes(), msg, &sig));
        assert!(crate::format::looks_like_pubkey(&kp.address()));
    }

    #[test]
    fn mnemonic_is_deterministic() {
        let phrase = crate::mnemonic::generate(12).unwrap();
        let a = Keypair::from_mnemonic_default(&phrase).unwrap().address();
        let b = Keypair::from_mnemonic_default(&phrase).unwrap().address();
        assert_eq!(a, b);
    }
}
