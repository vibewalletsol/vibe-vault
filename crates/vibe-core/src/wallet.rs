// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
//! Wallet kind tags for mnemonic, private-key, hardware, and watch accounts.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletKeyType {
    Unknown,
    Mnemonic,
    PrivateKey,
    Ledger,
    Trezor,
    Lattice,
    Watch,
}

impl WalletKeyType {
    pub fn as_storage(&self) -> &'static str {
        match self {
            WalletKeyType::Mnemonic => "mnemonic",
            WalletKeyType::PrivateKey => "private_key",
            WalletKeyType::Ledger => "ledger",
            WalletKeyType::Trezor => "trezor",
            WalletKeyType::Lattice => "lattice",
            WalletKeyType::Watch => "watch",
            WalletKeyType::Unknown => "",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "mnemonic" => WalletKeyType::Mnemonic,
            "private_key" => WalletKeyType::PrivateKey,
            "ledger" => WalletKeyType::Ledger,
            "trezor" => WalletKeyType::Trezor,
            "lattice" => WalletKeyType::Lattice,
            "watch" => WalletKeyType::Watch,
            _ => WalletKeyType::Unknown,
        }
    }

    pub fn is_hardware(&self) -> bool {
        matches!(
            self,
            WalletKeyType::Ledger | WalletKeyType::Trezor | WalletKeyType::Lattice
        )
    }

    pub fn is_watch(&self) -> bool {
        matches!(self, WalletKeyType::Watch)
    }

    pub fn can_sign(&self) -> bool {
        matches!(
            self,
            WalletKeyType::Mnemonic
                | WalletKeyType::PrivateKey
                | WalletKeyType::Ledger
                | WalletKeyType::Trezor
        )
    }

    pub fn can_export_secret(&self) -> bool {
        matches!(self, WalletKeyType::Mnemonic | WalletKeyType::PrivateKey)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwarePlugin {
    None,
    Ledger,
    Trezor,
    Lattice,
}

impl HardwarePlugin {
    pub fn as_storage(&self) -> &'static str {
        match self {
            HardwarePlugin::Ledger => "ledger",
            HardwarePlugin::Trezor => "trezor",
            HardwarePlugin::Lattice => "lattice",
            HardwarePlugin::None => "",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "ledger" => HardwarePlugin::Ledger,
            "trezor" => HardwarePlugin::Trezor,
            "lattice" => HardwarePlugin::Lattice,
            _ => HardwarePlugin::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trezor_tag() {
        assert_eq!(WalletKeyType::parse("trezor"), WalletKeyType::Trezor);
        assert!(WalletKeyType::Trezor.is_hardware());
        assert!(WalletKeyType::Trezor.can_sign());
        assert!(!WalletKeyType::Trezor.can_export_secret());
        assert!(!WalletKeyType::Watch.can_export_secret());
        assert!(WalletKeyType::Ledger.is_hardware());
        assert!(WalletKeyType::Watch.is_watch());
        assert_eq!(HardwarePlugin::parse("trezor").as_storage(), "trezor");
    }
}
