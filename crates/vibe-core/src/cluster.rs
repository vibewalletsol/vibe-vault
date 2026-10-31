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

    /// Genesis hash of a public cluster (None for Custom: learned from the primary RPC).
    pub fn genesis_hash(&self) -> Option<&'static str> {
        match self {
            Cluster::Mainnet => Some(crate::rpc::GENESIS_MAINNET),
            Cluster::Devnet => Some(crate::rpc::GENESIS_DEVNET),
            Cluster::Testnet => Some(crate::rpc::GENESIS_TESTNET),
            Cluster::Custom(_) => None,
        }
    }

    pub fn explorer_tx_url(&self, signature: &str) -> String {
        // Mainnet (and custom mainnet RPCs) open Vibe Explorer. It only indexes mainnet,
        // so devnet and testnet keep Solscan with the cluster param.
        match self {
            Cluster::Devnet => format!("https://solscan.io/tx/{signature}?cluster=devnet"),
            Cluster::Testnet => format!("https://solscan.io/tx/{signature}?cluster=testnet"),
            Cluster::Mainnet | Cluster::Custom(_) => {
                format!("https://vibewallet.net/tx.html?sig={signature}")
            }
        }
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
