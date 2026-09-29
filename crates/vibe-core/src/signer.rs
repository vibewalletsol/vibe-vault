// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
//! Software vs hardware vs watch-only signing.
//! Secrets never leave software wallets; Ledger and Trezor sign on-device.

use crate::error::{Error, Result};
use crate::keypair::Keypair;
use crate::wallet::WalletKeyType;

pub trait Signer {
    fn address(&self) -> &str;
    fn public_key(&self) -> [u8; 32];
    fn sign(&self, message: &[u8]) -> Result<[u8; 64]>;
    fn can_export_secret(&self) -> bool;
    fn is_connected(&self) -> bool;
    fn kind(&self) -> WalletKeyType;
}

pub struct SoftwareSigner {
    address: String,
    keypair: Keypair,
}

impl SoftwareSigner {
    pub fn new(keypair: Keypair) -> Self {
        Self {
            address: keypair.address(),
            keypair,
        }
    }

    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }
}

impl Signer for SoftwareSigner {
    fn address(&self) -> &str {
        &self.address
    }
    fn public_key(&self) -> [u8; 32] {
        self.keypair.public_bytes()
    }
    fn sign(&self, message: &[u8]) -> Result<[u8; 64]> {
        Ok(self.keypair.sign(message))
    }
    fn can_export_secret(&self) -> bool {
        true
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn kind(&self) -> WalletKeyType {
        WalletKeyType::Mnemonic
    }
}

pub struct WatchSigner {
    address: String,
    public: [u8; 32],
}

impl WatchSigner {
    pub fn new(address: impl Into<String>) -> Result<Self> {
        let address = address.into();
        let public = crate::tx::decode_pubkey(&address)?;
        Ok(Self { address, public })
    }
}

impl Signer for WatchSigner {
    fn address(&self) -> &str {
        &self.address
    }
    fn public_key(&self) -> [u8; 32] {
        self.public
    }
    fn sign(&self, _message: &[u8]) -> Result<[u8; 64]> {
        Err(Error::msg("Watch-only account — it cannot sign"))
    }
    fn can_export_secret(&self) -> bool {
        false
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn kind(&self) -> WalletKeyType {
        WalletKeyType::Watch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_signs_watch_does_not() {
        let kp = Keypair::generate();
        let sw = SoftwareSigner::new(kp.clone());
        let msg = b"vibe";
        let sig = sw.sign(msg).unwrap();
        assert!(Keypair::verify(&sw.public_key(), msg, &sig));
        let watch = WatchSigner::new(kp.address()).unwrap();
        assert!(watch.sign(msg).is_err());
        assert!(!watch.can_export_secret());
    }
}
