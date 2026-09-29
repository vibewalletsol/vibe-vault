// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
//! Solana wallet core for VIBE Wallet (no GTK).

pub mod cluster;
pub mod coingecko;
pub mod decode;
pub mod error;
pub mod format;
pub mod hd;
pub mod jupiter;
pub mod keypair;
pub mod ledger;
pub mod mcp;
pub mod trezor;
pub mod mnemonic;
pub mod policy;
pub mod programs;
pub mod rpc;
pub mod signer;
pub mod store;
pub mod tokens;
pub mod tx;
pub mod txcheck;
pub mod vault;
pub mod wallet;

pub use cluster::Cluster;
pub use error::{Error, Result};
pub use format::{
    format_lamports, format_sol, format_ui_amount, format_usd, parse_sol, parse_ui_amount,
    shorten_address,
};
pub use keypair::Keypair;
pub use programs::ProgramIds;
pub use rpc::{Rpc, Snapshot};
pub use signer::Signer;
pub use store::WalletStore;
pub use wallet::{HardwarePlugin, WalletKeyType};

pub const APP_ID: &str = "com.vibewallet.Vault";
pub const APP_NAME: &str = "VIBE Wallet";
pub const APP_BINARY: &str = "vibe-vault";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GETTEXT_DOMAIN: &str = "vibe-vault";
pub const DEVELOPER: &str = "Old Growth Crypto";
pub const HOMEPAGE: &str = "https://oldgrowthcrypto.com";

pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
