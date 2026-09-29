//! Priority fee presets and the vault's hard fee cap (1.9).
//!
//! Priority fee (lamports) = compute-unit limit × compute-unit price (micro-lamports) / 1e6.
//! The desk may *suggest* a price; the vault decides. Anything over the hard cap is refused
//! at sign time on the exact message bytes (see `txcheck` rule R14b and `check_message`).

use crate::LAMPORTS_PER_SOL;

/// Default compute-unit price when nothing is chosen (same as 1.8.x).
pub const DEFAULT_MICRO_LAMPORTS: u64 = 25_000;
/// Highest compute-unit price the Confirm *custom* field accepts (10 lamports per CU).
/// This is input validation only: the vault's signing rule (R14b) caps the *total*
/// priority fee, because dApp / Jupiter transactions legitimately use high per-CU prices
/// with small CU limits.
pub const MAX_MICRO_LAMPORTS: u64 = 10_000_000;
/// Hard cap on the total priority fee of one transaction: 0.05 SOL. Vault rule R14b (BLOCK).
pub const MAX_PRIORITY_FEE_LAMPORTS: u64 = 50_000_000;
/// Warn above this total priority fee: 0.01 SOL (same as rule R14).
pub const WARN_PRIORITY_FEE_LAMPORTS: u64 = 10_000_000;
/// Solana's default compute-unit limit per non-budget instruction when none is set.
pub const DEFAULT_UNITS_PER_IX: u32 = 200_000;
pub const MAX_UNITS: u32 = 1_400_000;

/// Named presets, in micro-lamports per compute unit.
pub const PRESETS: &[(&str, &str, u64)] = &[
    ("low", "Low", 1_000),
    ("normal", "Normal", 25_000),
    ("high", "High", 250_000),
    ("turbo", "Turbo", 2_000_000),
];

pub fn preset_micro(id: &str) -> Option<u64> {
    let id = id.trim().to_ascii_lowercase();
    PRESETS.iter().find(|(k, _, _)| *k == id).map(|(_, _, v)| *v)
}

/// Total priority fee in lamports, rounded up like the runtime does. The runtime clamps
/// the requested CU limit to `MAX_UNITS`, so we do too.
pub fn priority_fee_lamports(units: u32, micro_lamports: u64) -> u64 {
    let prod = units.min(MAX_UNITS) as u128 * micro_lamports as u128;
    let fee = (prod + 999_999) / 1_000_000;
    fee.min(u64::MAX as u128) as u64
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeeVerdict {
    Ok,
    /// Above the warning threshold but under the cap.
    Warn(String),
    /// Refused by the vault.
    Block(String),
}

impl FeeVerdict {
    pub fn is_block(&self) -> bool {
        matches!(self, FeeVerdict::Block(_))
    }
}

fn sol(l: u64) -> String {
    format!("{:.4}", l as f64 / LAMPORTS_PER_SOL as f64)
}

/// Check a (units, price) pair against the vault cap (total lamports).
pub fn check_fee(units: u32, micro_lamports: u64) -> FeeVerdict {
    let total = priority_fee_lamports(units, micro_lamports);
    if total > MAX_PRIORITY_FEE_LAMPORTS {
        return FeeVerdict::Block(format!(
            "Priority fee {} SOL is above the vault maximum of {} SOL.",
            sol(total),
            sol(MAX_PRIORITY_FEE_LAMPORTS)
        ));
    }
    if total > WARN_PRIORITY_FEE_LAMPORTS {
        return FeeVerdict::Warn(format!("High priority fee: {} SOL.", sol(total)));
    }
    FeeVerdict::Ok
}

/// Validate a price typed into the Confirm custom field (units = the CU limit the vault
/// will set). Returns the verdict for the resulting fee, or an input error.
pub fn check_custom(units: u32, micro_lamports: u64) -> Result<FeeVerdict, String> {
    if micro_lamports > MAX_MICRO_LAMPORTS {
        return Err(format!("Custom price is capped at {MAX_MICRO_LAMPORTS} µ-lamports/CU."));
    }
    Ok(check_fee(units, micro_lamports))
}

/// One line for the Confirm details: "Priority fee · 25,000 µL/CU × 80,000 CU ≈ 0.000002 SOL".
pub fn describe(units: u32, micro_lamports: u64) -> String {
    let fee = priority_fee_lamports(units, micro_lamports);
    format!(
        "Priority fee · {} µ-lamports/CU × {} CU ≈ {:.6} SOL",
        group(micro_lamports),
        group(units.min(MAX_UNITS) as u64),
        fee as f64 / LAMPORTS_PER_SOL as f64
    )
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Parse a user-typed micro-lamport value ("25,000", " 1000 "). Rejects junk and zero-length.
pub fn parse_micro(text: &str) -> Option<u64> {
    let t: String = text.trim().chars().filter(|c| *c != ',' && *c != '_' && *c != ' ').collect();
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

/// Compute-budget values found in a message: (limit, price). Limit falls back to the
/// runtime default (200k per non-budget instruction, max 1.4M).
pub fn budget_from_message(message: &[u8]) -> Option<(u32, u64)> {
    let parsed = crate::txcheck::parse_message(message).ok()?;
    let budget = crate::ProgramIds::COMPUTE_BUDGET;
    let mut limit: Option<u32> = None;
    let mut price: u64 = 0;
    let mut non_budget = 0u32;
    for ix in &parsed.instructions {
        let pid = parsed
            .static_keys
            .get(ix.program_index as usize)
            .map(|s| s.as_str())
            .unwrap_or("");
        if pid != budget {
            non_budget += 1;
            continue;
        }
        let d = &ix.data;
        match d.first() {
            Some(2) if d.len() >= 5 => {
                limit = Some(u32::from_le_bytes([d[1], d[2], d[3], d[4]]));
            }
            Some(3) if d.len() >= 9 => {
                let mut b = [0u8; 8];
                b.copy_from_slice(&d[1..9]);
                price = u64::from_le_bytes(b);
            }
            _ => {}
        }
    }
    let units = limit.unwrap_or_else(|| (non_budget.saturating_mul(DEFAULT_UNITS_PER_IX)).min(MAX_UNITS));
    Some((units, price))
}

/// Vault sign-time fee gate on the exact message bytes. Non-transaction payloads
/// (signMessage text) pass: they carry no fee.
pub fn check_message(message: &[u8]) -> Result<(), String> {
    match budget_from_message(message) {
        Some((units, price)) => match check_fee(units, price) {
            FeeVerdict::Block(e) => Err(format!("Blocked by the vault [R14b]: {e}")),
            _ => Ok(()),
        },
        None => Ok(()),
    }
}

/// Message bytes of a base64 wire transaction (signatures stripped).
pub fn wire_message(tx: &[u8]) -> Option<&[u8]> {
    let mut off = 0usize;
    let mut n: usize = 0;
    let mut shift = 0;
    loop {
        let b = *tx.get(off)? as usize;
        off += 1;
        n |= (b & 0x7f) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 14 {
            return None;
        }
    }
    tx.get(off + n * 64..)
}

/// Priority fee (lamports) of a wire transaction, from its exact bytes.
pub fn wire_priority_lamports(tx: &[u8]) -> Option<u64> {
    let (units, price) = budget_from_message(wire_message(tx)?)?;
    Some(priority_fee_lamports(units, price))
}

/// Swap gate: when the user picked a fee in Confirm, the transaction Jupiter returns may not
/// carry a priority fee above it (10% + 5,000 lamports slack for rounding). Also applies R14b.
pub fn check_wire_against(tx_b64: &str, picked_lamports: u64) -> Result<(), String> {
    let bytes = crate::tx::decode_b64(tx_b64).map_err(|e| e.to_string())?;
    let msg = wire_message(&bytes).ok_or("transaction truncated")?;
    check_message(msg)?;
    let actual = wire_priority_lamports(&bytes).unwrap_or(0);
    let ceiling = picked_lamports.saturating_add(picked_lamports / 10).saturating_add(5_000);
    if actual > ceiling {
        return Err(format!(
            "Blocked by the vault [R14b]: the swap carries a {:.6} SOL priority fee, above the {:.6} SOL you picked.",
            actual as f64 / LAMPORTS_PER_SOL as f64,
            picked_lamports as f64 / LAMPORTS_PER_SOL as f64
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx;

    #[test]
    fn fee_math_rounds_up() {
        assert_eq!(priority_fee_lamports(200_000, 25_000), 5_000);
        assert_eq!(priority_fee_lamports(1, 1), 1);
        assert_eq!(priority_fee_lamports(0, 1_000_000), 0);
        assert_eq!(priority_fee_lamports(1_400_000, MAX_MICRO_LAMPORTS), 14_000_000);
        // CU limit is clamped to the runtime max.
        assert_eq!(priority_fee_lamports(u32::MAX, 1_000_000), 1_400_000);
    }

    #[test]
    fn describe_and_custom() {
        assert_eq!(describe(80_000, 25_000), "Priority fee · 25,000 µ-lamports/CU × 80,000 CU ≈ 0.000002 SOL");
        assert_eq!(group(1_400_000), "1,400,000");
        assert!(check_custom(80_000, MAX_MICRO_LAMPORTS + 1).is_err());
        assert_eq!(check_custom(80_000, 1_000), Ok(FeeVerdict::Ok));
    }

    #[test]
    fn presets_are_under_the_cap() {
        for (id, _, micro) in PRESETS {
            assert!(!check_fee(MAX_UNITS, *micro).is_block(), "{id}");
            assert_eq!(preset_micro(id), Some(*micro));
        }
        assert_eq!(preset_micro("NORMAL"), Some(25_000));
        assert_eq!(preset_micro("bogus"), None);
    }

    #[test]
    fn cap_blocks_total_not_price() {
        assert_eq!(check_fee(80_000, 25_000), FeeVerdict::Ok);
        // A Jupiter-style high price on a small CU limit is fine: 100k CU × 50M µL = 0.005 SOL.
        assert!(!check_fee(100_000, 50_000_000).is_block());
        // 1.4M CU at 8M µL = 11.2M lamports → warn (over 0.01 SOL), under 0.05
        assert!(matches!(check_fee(1_400_000, 8_000_000), FeeVerdict::Warn(_)));
        // 1.4M CU at 40M µL = 0.056 SOL → block.
        assert!(check_fee(1_400_000, 40_000_000).is_block());
        assert!(check_fee(u32::MAX, 40_000_000).is_block());
    }

    #[test]
    fn parse_micro_accepts_grouping_only() {
        assert_eq!(parse_micro("25,000"), Some(25_000));
        assert_eq!(parse_micro(" 1000 "), Some(1000));
        assert_eq!(parse_micro("1e6"), None);
        assert_eq!(parse_micro("-5"), None);
        assert_eq!(parse_micro(""), None);
    }

    fn msg_with(units: Option<u32>, price: u64) -> Vec<u8> {
        let kp = crate::keypair::Keypair::generate();
        let from = kp.address();
        let to = crate::keypair::Keypair::generate().address();
        let mut ixs = Vec::new();
        if let Some(u) = units {
            ixs.push(tx::set_compute_unit_limit(u));
        }
        ixs.push(tx::set_compute_unit_price(price));
        ixs.push(tx::system_transfer(&from, &to, 1));
        let hash = crate::keypair::Keypair::generate().address();
        tx::compile_legacy_message(&from, &hash, &ixs).unwrap()
    }

    #[test]
    fn wire_gate_checks_picked_ceiling() {
        let m = msg_with(Some(400_000), 25_000); // 10,000 lamports
        let mut wire = vec![1u8];
        wire.extend(std::iter::repeat(0u8).take(64));
        wire.extend_from_slice(&m);
        assert_eq!(wire_priority_lamports(&wire), Some(10_000));
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &wire);
        assert!(check_wire_against(&b64, 10_000).is_ok());
        assert!(check_wire_against(&b64, 5_000).is_ok()); // 5,000 + 10% + 5,000 slack
        let e = check_wire_against(&b64, 1_000).unwrap_err();
        assert!(e.contains("R14b"), "{e}");
    }

    #[test]
    fn message_gate_reads_exact_bytes() {
        assert!(check_message(&msg_with(Some(80_000), 25_000)).is_ok());
        let e = check_message(&msg_with(Some(1_400_000), 40_000_000)).unwrap_err();
        assert!(e.contains("R14b"), "{e}");
        // No explicit limit: default 200k per non-budget ix (1 ix) → 200k × 10M µL = 2M lamports → ok
        assert!(check_message(&msg_with(None, MAX_MICRO_LAMPORTS)).is_ok());
        // No explicit limit, 1 ix → 200k CU × 300M µL = 0.06 SOL → block.
        assert!(check_message(&msg_with(None, 300_000_000)).is_err());
        // Text payloads pass (signMessage).
        assert!(check_message(b"hello").is_ok());
    }
}
