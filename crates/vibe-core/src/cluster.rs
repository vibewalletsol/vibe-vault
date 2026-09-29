// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
//! RPC cluster selection with a multi-endpoint pool.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cluster {
    Mainnet,
    Devnet,
    Testnet,
    Custom(String),
}

impl Cluster {
    pub fn as_label(&self) -> &str {
        match self {
            Cluster::Mainnet => "Mainnet",
            Cluster::Devnet => "Devnet",
            Cluster::Testnet => "Testnet",
            Cluster::Custom(_) => "Custom",
        }
    }

    pub fn rpc_url(&self) -> &str {
        match self {
            Cluster::Mainnet => "https://api.mainnet-beta.solana.com",
            Cluster::Devnet => "https://api.devnet.solana.com",
            Cluster::Testnet => "https://api.testnet.solana.com",
            Cluster::Custom(url) => url.as_str(),
        }
    }

    pub fn explorer_tx_url(&self, signature: &str) -> String {
        let cluster = match self {
            Cluster::Mainnet => "",
            Cluster::Devnet => "?cluster=devnet",
            Cluster::Testnet => "?cluster=testnet",
            Cluster::Custom(_) => "?cluster=custom",
        };
        format!("https://solscan.io/tx/{signature}{cluster}")
    }

    pub fn from_label(label: &str) -> Self {
        match label {
            "Devnet" => Cluster::Devnet,
            "Testnet" => Cluster::Testnet,
            "Mainnet" => Cluster::Mainnet,
            other => Cluster::Custom(other.to_string()),
        }
    }
}

impl Default for Cluster {
    fn default() -> Self {
        Cluster::Mainnet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_rpc() {
        assert!(Cluster::Mainnet.rpc_url().contains("mainnet"));
    }
}
