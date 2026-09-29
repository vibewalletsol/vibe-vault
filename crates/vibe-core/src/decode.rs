// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
// Portions derived from Cinder (https://github.com/gregnb/cinder),
// Copyright (c) 2026 Gregory Nowakowski, MIT License (see LICENSES/MIT-Cinder.txt).
//! Human-readable Solana instruction decode for explorer, MCP, and review UI.

use serde_json::{json, Value};

use crate::error::Result;
use crate::programs::ProgramIds;
use crate::tx::{self, Instruction};

#[derive(Clone, Debug)]
pub struct DecodedIx {
    pub program: String,
    pub program_id: String,
    pub title: String,
    pub detail: String,
}

impl DecodedIx {
    pub fn to_json(&self) -> Value {
        json!({
            "program": self.program,
            "programId": self.program_id,
            "title": self.title,
            "detail": self.detail,
        })
    }
}

pub fn program_name(program_id: &str) -> &'static str {
    match program_id {
        ProgramIds::SYSTEM => "System",
        ProgramIds::COMPUTE_BUDGET => "Compute Budget",
        ProgramIds::TOKEN => "Token",
        ProgramIds::TOKEN_2022 => "Token-2022",
        ProgramIds::ASSOCIATED_TOKEN => "Associated Token",
        ProgramIds::MEMO => "Memo",
        ProgramIds::STAKE => "Stake",
        ProgramIds::VOTE => "Vote",
        "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4" => "Jupiter",
        _ => "Program",
    }
}

pub fn decode_instruction(ix: &Instruction) -> DecodedIx {
    let program = program_name(&ix.program_id).to_string();
    let (title, detail) = match ix.program_id.as_str() {
        ProgramIds::SYSTEM => decode_system(&ix.data, &ix.accounts),
        ProgramIds::TOKEN | ProgramIds::TOKEN_2022 => decode_token(&ix.data, &ix.accounts),
        ProgramIds::ASSOCIATED_TOKEN => (
            "Create token account".into(),
            format!("owner {}", short(&first_account(&ix.accounts, 2))),
        ),
        ProgramIds::COMPUTE_BUDGET => decode_budget(&ix.data),
        ProgramIds::MEMO => {
            let text = String::from_utf8_lossy(&ix.data).trim().to_string();
            ("Memo".into(), text)
        }
        ProgramIds::STAKE => decode_stake(&ix.data, &ix.accounts),
        _ => (
            program.clone(),
            format!("{} accounts, {} bytes", ix.accounts.len(), ix.data.len()),
        ),
    };
    DecodedIx {
        program,
        program_id: ix.program_id.clone(),
        title,
        detail,
    }
}

/// Decode without network access. v0 lookup-table accounts are shown as
/// "unresolved" placeholders; use `decode_compiled_message_with` to resolve them.
pub fn decode_compiled_message(message: &[u8]) -> Result<Vec<DecodedIx>> {
    decode_compiled_message_with(message, |_| Err("lookup tables are not fetched offline".into()))
}

pub fn decode_json_parsed(tx: &Value) -> Vec<DecodedIx> {
    let mut out = Vec::new();
    let message = tx
        .pointer("/transaction/message")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(arr) = message.get("instructions").and_then(|v| v.as_array()) {
        for ix in arr {
            out.push(decode_json_ix(ix));
        }
    }
    if let Some(groups) = tx
        .pointer("/meta/innerInstructions")
        .and_then(|v| v.as_array())
    {
        for group in groups {
            if let Some(arr) = group.get("instructions").and_then(|v| v.as_array()) {
                for ix in arr {
                    let mut d = decode_json_ix(ix);
                    d.title = format!("inner · {}", d.title);
                    out.push(d);
                }
            }
        }
    }
    out
}

pub fn classify(ixs: &[DecodedIx]) -> &'static str {
    let titles: Vec<&str> = ixs.iter().map(|i| i.title.as_str()).collect();
    if titles.iter().any(|t| t.contains("Swap") || t.contains("Jupiter")) {
        return "swap";
    }
    if titles.iter().any(|t| t.contains("Delegate") || t.contains("Stake")) {
        return "stake";
    }
    if titles.iter().any(|t| t.contains("Burn")) {
        return "burn";
    }
    if titles.iter().any(|t| t.contains("Initialize mint") || t.contains("Mint to")) {
        return "mint";
    }
    if titles.iter().any(|t| t.contains("Nonce")) {
        return "nonce";
    }
    if titles.iter().any(|t| t.contains("Transfer")) {
        return "send";
    }
    "other"
}

pub fn summary_lines(ixs: &[DecodedIx]) -> Vec<String> {
    ixs.iter()
        .map(|ix| {
            if ix.detail.is_empty() {
                format!("{} · {}", ix.program, ix.title)
            } else {
                format!("{} · {} — {}", ix.program, ix.title, ix.detail)
            }
        })
        .collect()
}

fn decode_json_ix(ix: &Value) -> DecodedIx {
    let program_id = ix
        .get("programId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(parsed) = ix.get("parsed") {
        if parsed.is_object() {
            let typ = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("instruction");
            let info = parsed.get("info").cloned().unwrap_or(Value::Null);
            let detail = json_info_line(&info);
            return DecodedIx {
                program: program_name(&program_id).to_string(),
                program_id,
                title: human_parsed_type(typ).into(),
                detail,
            };
        }
    }
    DecodedIx {
        program: program_name(&program_id).to_string(),
        program_id,
        title: "Instruction".into(),
        detail: String::new(),
    }
}

fn human_parsed_type(typ: &str) -> &'static str {
    match typ {
        "transfer" | "transferChecked" => "Transfer",
        "transferCheckedWithFee" => "Transfer with fee",
        "createAccount" => "Create account",
        "initializeNonceAccount" => "Initialize nonce",
        "advanceNonceAccount" => "Advance nonce",
        "withdrawNonceAccount" => "Withdraw nonce",
        "initializeMint" | "initializeMint2" => "Initialize mint",
        "mintTo" | "mintToChecked" => "Mint to",
        "burn" | "burnChecked" => "Burn",
        "closeAccount" => "Close account",
        "approve" | "approveChecked" => "Approve",
        "revoke" => "Revoke",
        "setAuthority" => "Set authority",
        "freezeAccount" => "Freeze account",
        "thawAccount" => "Thaw account",
        "delegate" => "Delegate stake",
        "deactivate" => "Deactivate stake",
        "withdraw" => "Withdraw",
        "createIdempotent" | "create" => "Create token account",
        _ => "Instruction",
    }
}

fn json_info_line(info: &Value) -> String {
    let mut bits = Vec::new();
    for key in [
        "lamports",
        "amount",
        "uiAmount",
        "destination",
        "source",
        "authority",
        "mint",
        "newAccount",
        "nonceAccount",
        "stakeAccount",
    ] {
        if let Some(v) = info.get(key) {
            if let Some(s) = v.as_str() {
                bits.push(format!("{key} {}", short(s)));
            } else if let Some(n) = v.as_u64() {
                bits.push(format!("{key} {n}"));
            } else if let Some(n) = v.as_f64() {
                bits.push(format!("{key} {n}"));
            }
        }
    }
    bits.join(" · ")
}

fn decode_system(data: &[u8], accounts: &[tx::AccountMeta]) -> (String, String) {
    let ix = if data.len() >= 4 {
        u32::from_le_bytes(data[0..4].try_into().unwrap_or([0; 4]))
    } else {
        255
    };
    match ix {
        0 => ("Create account".into(), short(&first_account(accounts, 1))),
        2 => {
            let lamports = if data.len() >= 12 {
                u64::from_le_bytes(data[4..12].try_into().unwrap_or([0; 8]))
            } else {
                0
            };
            (
                "Transfer SOL".into(),
                format!(
                    "{} → {}  ({} lamports)",
                    short(&first_account(accounts, 0)),
                    short(&first_account(accounts, 1)),
                    lamports
                ),
            )
        }
        4 => ("Advance nonce".into(), short(&first_account(accounts, 0))),
        5 => ("Withdraw nonce".into(), short(&first_account(accounts, 0))),
        6 => ("Initialize nonce".into(), short(&first_account(accounts, 0))),
        7 => ("Authorize nonce".into(), short(&first_account(accounts, 0))),
        _ => ("System".into(), format!("ix {ix}")),
    }
}

fn decode_token(data: &[u8], accounts: &[tx::AccountMeta]) -> (String, String) {
    let op = data.first().copied().unwrap_or(255);
    let title = match op {
        3 => "Transfer",
        4 => "Approve",
        5 => "Revoke",
        6 => "Set authority",
        7 => "Mint to",
        8 => "Burn",
        9 => "Close account",
        10 => "Freeze account",
        11 => "Thaw account",
        12 => "Transfer checked",
        13 => "Approve",
        14 => "Mint to checked",
        15 => "Burn checked",
        20 => "Initialize mint",
        0x1A => "Transfer fee",
        _ => "Token",
    };
    (title.into(), short(&first_account(accounts, 0)))
}

fn decode_budget(data: &[u8]) -> (String, String) {
    match data.first().copied().unwrap_or(255) {
        2 => {
            let units = if data.len() >= 5 {
                u32::from_le_bytes(data[1..5].try_into().unwrap_or([0; 4]))
            } else {
                0
            };
            ("Compute unit limit".into(), format!("{units} CU"))
        }
        3 => {
            let micro = if data.len() >= 9 {
                u64::from_le_bytes(data[1..9].try_into().unwrap_or([0; 8]))
            } else {
                0
            };
            ("Compute unit price".into(), format!("{micro} µ-lamports/CU"))
        }
        _ => ("Compute budget".into(), String::new()),
    }
}

fn decode_stake(data: &[u8], accounts: &[tx::AccountMeta]) -> (String, String) {
    let ix = if data.len() >= 4 {
        u32::from_le_bytes(data[0..4].try_into().unwrap_or([0; 4]))
    } else {
        255
    };
    let title = match ix {
        0 => "Initialize stake",
        2 => "Delegate stake",
        4 => "Withdraw stake",
        5 => "Deactivate stake",
        _ => "Stake",
    };
    (title.into(), short(&first_account(accounts, 0)))
}

fn first_account(accounts: &[tx::AccountMeta], idx: usize) -> String {
    accounts
        .get(idx)
        .map(|a| a.pubkey.clone())
        .unwrap_or_default()
}

fn short(addr: &str) -> String {
    crate::format::shorten_address(addr)
}

/// Parse a legacy or v0 message into instructions. Accounts loaded from address
/// lookup tables are resolved through `fetch`; if a table can't be resolved the
/// accounts are shown as explicit "unresolved" placeholders (never dropped) and
/// the error is returned alongside.
fn parse_ixs_with<F>(message: &[u8], fetch: F) -> Result<(Vec<Instruction>, Option<String>)>
where
    F: FnMut(&str) -> crate::txcheck::Fetched,
{
    let parsed = crate::txcheck::parse_message(message).map_err(crate::error::Error::msg)?;
    match crate::txcheck::resolve(&parsed, fetch) {
        Ok(m) => Ok((m.instructions, None)),
        Err(e) => {
            let mut names: Vec<(String, bool)> = Vec::new();
            let n_keys = parsed.static_keys.len();
            let n_req = parsed.num_required_signatures as usize;
            for (i, k) in parsed.static_keys.iter().enumerate() {
                let w = if i < n_req {
                    i < n_req - parsed.num_readonly_signed as usize
                } else {
                    i < n_keys - parsed.num_readonly_unsigned as usize
                };
                names.push((k.clone(), w));
            }
            for writable in [true, false] {
                for lu in &parsed.lookups {
                    let idxs = if writable { &lu.writable } else { &lu.readonly };
                    for ix in idxs {
                        names.push((
                            format!("<unresolved: lookup table {} entry {ix}>", short(&lu.table)),
                            writable,
                        ));
                    }
                }
            }
            let ixs = parsed
                .instructions
                .iter()
                .map(|ix| Instruction {
                    program_id: names[ix.program_index as usize].0.clone(),
                    accounts: ix
                        .accounts
                        .iter()
                        .map(|&i| {
                            let (k, w) = &names[i as usize];
                            tx::AccountMeta {
                                pubkey: k.clone(),
                                is_signer: (i as usize) < n_req,
                                is_writable: *w,
                            }
                        })
                        .collect(),
                    data: ix.data.clone(),
                })
                .collect();
            Ok((ixs, Some(e)))
        }
    }
}

/// Decode with lookup-table resolution (`fetch` = the vault's RPC `getAccountInfo`).
/// An unresolved table is reported as the first entry (title "Unresolved lookup table").
pub fn decode_compiled_message_with<F>(message: &[u8], fetch: F) -> Result<Vec<DecodedIx>>
where
    F: FnMut(&str) -> crate::txcheck::Fetched,
{
    let (ixs, err) = parse_ixs_with(message, fetch)?;
    let mut out = Vec::new();
    if let Some(e) = err {
        out.push(DecodedIx {
            program: "VIBE".into(),
            program_id: String::new(),
            title: "Unresolved lookup table".into(),
            detail: e,
        });
    }
    out.extend(ixs.iter().map(decode_instruction));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keypair::Keypair;
    use crate::tx::{self, system_transfer};

    #[test]
    fn decodes_system_transfer() {
        let from = Keypair::generate();
        let to = Keypair::generate();
        let ix = system_transfer(&from.address(), &to.address(), 1_000_000);
        let d = decode_instruction(&ix);
        assert_eq!(d.program, "System");
        assert!(d.title.contains("Transfer"));
        let bh = Keypair::generate().address();
        let msg = tx::compile_legacy_message(&from.address(), &bh, &[ix]).unwrap();
        let ixs = decode_compiled_message(&msg).unwrap();
        assert_eq!(ixs.len(), 1);
        assert_eq!(classify(&ixs), "send");
    }

    #[test]
    fn json_parsed_transfer() {
        let tx = json!({
            "transaction": {"message": {"instructions": [{
                "programId": ProgramIds::SYSTEM,
                "parsed": {"type": "transfer", "info": {"lamports": 10, "destination": "Dest111111111111111111111111111111111111111"}}
            }]}}
        });
        let ixs = decode_json_parsed(&tx);
        assert_eq!(ixs.len(), 1);
        assert_eq!(ixs[0].title, "Transfer");
    }
}
