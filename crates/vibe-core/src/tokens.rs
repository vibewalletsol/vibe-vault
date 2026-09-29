// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
//! Known SPL mints + CoinGecko ids, aligned with Old Growth Portfolio.

use crate::WSOL_MINT;

#[derive(Debug, Clone, Copy)]
pub struct KnownToken {
    pub mint: &'static str,
    pub symbol: &'static str,
    pub name: &'static str,
    pub gecko_id: &'static str,
}

pub const KNOWN_TOKENS: &[KnownToken] = &[
    KnownToken {
        mint: WSOL_MINT,
        symbol: "SOL",
        name: "Solana",
        gecko_id: "solana",
    },
    KnownToken {
        mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        symbol: "USDC",
        name: "USD Coin",
        gecko_id: "usd-coin",
    },
    KnownToken {
        mint: "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        symbol: "USDT",
        name: "Tether",
        gecko_id: "tether",
    },
    KnownToken {
        mint: "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263",
        symbol: "BONK",
        name: "Bonk",
        gecko_id: "bonk",
    },
    KnownToken {
        mint: "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN",
        symbol: "JUP",
        name: "Jupiter",
        gecko_id: "jupiter-exchange-solana",
    },
    KnownToken {
        mint: "HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3",
        symbol: "PYTH",
        name: "Pyth Network",
        gecko_id: "pyth-network",
    },
    KnownToken {
        mint: "jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL",
        symbol: "JTO",
        name: "Jito",
        gecko_id: "jito-governance-token",
    },
    KnownToken {
        mint: "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm",
        symbol: "WIF",
        name: "dogwifhat",
        gecko_id: "dogwifcoin",
    },
    KnownToken {
        mint: "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn",
        symbol: "JitoSOL",
        name: "Jito Staked SOL",
        gecko_id: "jito-staked-sol",
    },
    KnownToken {
        mint: "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So",
        symbol: "mSOL",
        name: "Marinade Staked SOL",
        gecko_id: "msol",
    },
    KnownToken {
        mint: "rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBof",
        symbol: "RENDER",
        name: "Render",
        gecko_id: "render-token",
    },
    KnownToken {
        mint: "2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv",
        symbol: "PENGU",
        name: "Pudgy Penguins",
        gecko_id: "pudgy-penguins",
    },
    KnownToken {
        mint: "6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN",
        symbol: "TRUMP",
        name: "Official Trump",
        gecko_id: "official-trump",
    },
];

pub fn display_symbol(mint: &str) -> String {
    if mint == crate::WSOL_MINT {
        return "SOL".into();
    }
    resolve_known_token(mint)
        .map(|t| t.symbol.to_string())
        .unwrap_or_else(|| crate::format::shorten_address(mint))
}

pub fn gecko_id_for_mint(mint: &str) -> Option<&'static str> {
    if mint == crate::WSOL_MINT || mint.eq_ignore_ascii_case("native") {
        return Some("solana");
    }
    resolve_known_token(mint).map(|t| t.gecko_id)
}

pub fn resolve_known_token(mint: &str) -> Option<&'static KnownToken> {
    KNOWN_TOKENS.iter().find(|t| t.mint == mint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdc() {
        let t = resolve_known_token("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        assert_eq!(t.symbol, "USDC");
        assert_eq!(t.gecko_id, "usd-coin");
    }
}
