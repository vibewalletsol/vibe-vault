// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use crate::error::{Error, Result};
use crate::LAMPORTS_PER_SOL;

/// Format lamports as a SOL string with trimmed trailing zeros.
pub fn format_sol(lamports: u64) -> String {
    let sol = lamports as f64 / LAMPORTS_PER_SOL as f64;
    let s = format!("{sol:.9}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".into()
    } else {
        s.to_string()
    }
}

pub fn format_lamports(lamports: u64) -> String {
    format!("{} SOL", format_sol(lamports))
}

/// Base58 Solana addresses are 32–44 chars. Show `aaaa…zzzz`.
pub fn shorten_address(address: &str) -> String {
    if address.len() <= 12 {
        address.to_string()
    } else {
        format!("{}…{}", &address[..4], &address[address.len() - 4..])
    }
}

const BASE58_ALPHABET: &[u8] =
    b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

pub fn looks_like_pubkey(address: &str) -> bool {
    let len = address.len();
    (32..=44).contains(&len) && address.bytes().all(|b| BASE58_ALPHABET.contains(&b))
}

pub fn looks_like_signature(sig: &str) -> bool {
    let len = sig.len();
    (80..=90).contains(&len) && sig.bytes().all(|b| BASE58_ALPHABET.contains(&b))
}

/// Parse a SOL decimal string into lamports without using binary floats.
pub fn parse_sol(s: &str) -> Result<u64> {
    parse_ui_amount(s, 9)
}

/// Parse a UI amount into raw token units for `decimals`.
pub fn parse_ui_amount(s: &str, decimals: u8) -> Result<u64> {
    let s = s.trim().replace(',', "");
    if s.is_empty() {
        return Err(Error::msg("enter an amount"));
    }
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s.as_str(), ""),
    };
    if whole.chars().any(|c| !c.is_ascii_digit()) || frac.chars().any(|c| !c.is_ascii_digit()) {
        return Err(Error::msg("amount must be a number"));
    }
    if frac.len() > decimals as usize {
        return Err(Error::msg(format!("max {decimals} decimal places")));
    }
    let whole_n: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| Error::msg("amount too large"))?
    };
    let mut frac_pad = frac.to_string();
    while frac_pad.len() < decimals as usize {
        frac_pad.push('0');
    }
    let frac_n: u64 = if frac_pad.is_empty() {
        0
    } else {
        frac_pad.parse().map_err(|_| Error::msg("amount too large"))?
    };
    let scale = 10u64.checked_pow(decimals as u32).ok_or_else(|| Error::msg("amount too large"))?;
    whole_n
        .checked_mul(scale)
        .and_then(|w| w.checked_add(frac_n))
        .ok_or_else(|| Error::msg("amount too large"))
}

pub fn format_ui_amount(raw: u64, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let scale = 10u64.pow(decimals as u32);
    let whole = raw / scale;
    let frac = raw % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let s = format!("{whole}.{frac:0width$}", width = decimals as usize);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub fn parse_raw_u64(s: &str) -> u64 {
    s.trim().parse().unwrap_or(0)
}

pub fn format_usd(v: f64) -> String {
    if v <= 0.0 {
        "$0.00".into()
    } else if v < 0.01 {
        format!("${v:.6}")
    } else {
        format!("${v:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sol_format() {
        assert_eq!(format_sol(0), "0");
        assert_eq!(format_sol(1_000_000_000), "1");
        assert_eq!(format_sol(1_500_000_000), "1.5");
        assert_eq!(format_sol(1), "0.000000001");
    }

    #[test]
    fn short_addr() {
        let a = "So11111111111111111111111111111111111111112";
        assert_eq!(shorten_address(a), "So11…1112");
        assert!(looks_like_pubkey(a));
        assert!(!looks_like_pubkey("not an address"));
    }

    #[test]
    fn parse_amounts() {
        assert_eq!(parse_sol("1").unwrap(), 1_000_000_000);
        assert_eq!(parse_sol("1.5").unwrap(), 1_500_000_000);
        assert_eq!(parse_sol("0.000000001").unwrap(), 1);
        assert!(parse_sol("nope").is_err());
        assert_eq!(parse_ui_amount("1.5", 6).unwrap(), 1_500_000);
        assert_eq!(format_ui_amount(1_500_000, 6), "1.5");
    }
}
