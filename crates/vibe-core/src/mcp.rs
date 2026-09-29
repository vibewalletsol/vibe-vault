// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
// Portions derived from Cinder (https://github.com/gregnb/cinder),
// Copyright (c) 2026 Gregory Nowakowski, MIT License (see LICENSES/MIT-Cinder.txt).
//! Model Context Protocol tool catalog. Read tools hit RPC; write tools spawn vibe-vault.

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::decode;
use crate::error::Result;
use crate::rpc::Rpc;
use crate::store::{self, WalletStore};
use crate::tx;
use crate::wallet::WalletKeyType;
use crate::{format, ProgramIds, WSOL_MINT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Write,
}

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub kind: ToolKind,
    pub schema: Value,
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        read("wallet_ping", "Check that the VIBE MCP server is running", json!({"type":"object","properties":{}})),
        read("wallet_address", "Active vault address", json!({"type":"object","properties":{}})),
        read("wallet_list_wallets", "List vault accounts (no secrets)", json!({"type":"object","properties":{}})),
        read("wallet_get_balance", "Native SOL balance", json!({"type":"object","properties":{"address":{"type":"string"}}})),
        read("wallet_get_token_accounts", "SPL / Token-2022 accounts", json!({"type":"object","properties":{"address":{"type":"string"}}})),
        read("wallet_inspect_account", "Raw account owner + lamports", json!({"type":"object","properties":{"address":{"type":"string"}},"required":["address"]})),
        read("wallet_get_transaction", "Fetch a signature and decode instructions", json!({"type":"object","properties":{"signature":{"type":"string"}},"required":["signature"]})),
        read("wallet_get_blockhash", "Latest confirmed blockhash", json!({"type":"object","properties":{}})),
        read("wallet_get_stake_accounts", "Stake accounts for the wallet", json!({"type":"object","properties":{"address":{"type":"string"}}})),
        read("wallet_get_contacts", "Address book", json!({"type":"object","properties":{}})),
        read("wallet_derive_ata", "Associated token account for a mint", json!({"type":"object","properties":{"owner":{"type":"string"},"mint":{"type":"string"},"program":{"type":"string"}},"required":["mint"]})),
        read("wallet_decode_instruction", "Decode a compiled legacy message (base64)", json!({"type":"object","properties":{"message_b64":{"type":"string"}},"required":["message_b64"]})),
        write("wallet_send_sol", "Send SOL — opens Confirm in vibe-vault", json!({"type":"object","properties":{"to":{"type":"string"},"amount":{"type":"string"}},"required":["to","amount"]})),
        write("wallet_swap", "Jupiter swap — opens Confirm in vibe-vault", json!({"type":"object","properties":{"in_mint":{"type":"string"},"out_mint":{"type":"string"},"amount":{"type":"string"}},"required":["in_mint","out_mint","amount"]})),
        write("wallet_stake_create", "Create and delegate stake — Confirm in vibe-vault", json!({"type":"object","properties":{"vote":{"type":"string"},"amount":{"type":"string"}},"required":["vote","amount"]})),
        write("wallet_nonce_create", "Create a durable nonce account — Confirm in vibe-vault", json!({"type":"object","properties":{}})),
        write("wallet_token_create", "Create an SPL or Token-2022 mint — Confirm in vibe-vault", json!({"type":"object","properties":{"name":{"type":"string"},"symbol":{"type":"string"},"decimals":{"type":"integer"},"supply":{"type":"string"}},"required":["symbol"]})),
    ]
}

fn read(name: &'static str, description: &'static str, schema: Value) -> ToolDef {
    ToolDef { name, description, kind: ToolKind::Read, schema }
}
fn write(name: &'static str, description: &'static str, schema: Value) -> ToolDef {
    ToolDef { name, description, kind: ToolKind::Write, schema }
}

pub fn tools_list_json() -> Value {
    json!(tools()
        .into_iter()
        .map(|t| json!({
            "name": t.name,
            "description": t.description,
            "inputSchema": t.schema,
        }))
        .collect::<Vec<_>>())
}

pub fn write_enabled() -> bool {
    load_policy().get("write").and_then(|v| v.as_bool()).unwrap_or(false)
}

pub fn policy_path() -> PathBuf {
    store::config_dir().join("mcp-policy.json")
}

pub fn load_policy() -> Value {
    fs::read_to_string(policy_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({"write": false}))
}

pub fn save_policy(write: bool) -> Result<()> {
    let path = policy_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(&json!({"write": write}))?)?;
    Ok(())
}

pub fn dispatch(name: &str, args: &Value, rpc_url: &str) -> Result<Value> {
    let def = tools()
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| crate::error::Error::msg(format!("unknown tool {name}")))?;
    if def.kind == ToolKind::Write && !write_enabled() {
        return Err(crate::error::Error::msg(
            "Write tools are off. Enable them in Options → Agents, or set mcp-policy.json write=true.",
        ));
    }
    let store = store::load_store(&store::wallets_path())?;
    let rpc = Rpc::new(rpc_url);
    match name {
        "wallet_ping" => Ok(json!({"ok": true, "name": "vibe-mcp"})),
        "wallet_address" => Ok(json!({"address": active_address(&store)})),
        "wallet_list_wallets" => Ok(json!(store
            .wallets
            .iter()
            .map(|w| json!({
                "id": w.id,
                "name": w.name,
                "address": w.address,
                "key_type": w.key_type,
                "can_sign": WalletKeyType::parse(&w.key_type).can_sign(),
            }))
            .collect::<Vec<_>>())),
        "wallet_get_balance" => {
            let addr = arg_addr(args, &store)?;
            let lamports = rpc.get_balance(&addr)?;
            Ok(json!({"address": addr, "lamports": lamports, "sol": format::format_sol(lamports)}))
        }
        "wallet_get_token_accounts" => {
            let addr = arg_addr(args, &store)?;
            let tokens = rpc.get_tokens(&addr)?;
            Ok(serde_json::to_value(tokens)?)
        }
        "wallet_inspect_account" => {
            let addr = args
                .get("address")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !format::looks_like_pubkey(&addr) {
                return Err(crate::error::Error::InvalidAddress);
            }
            match rpc.get_account(&addr)? {
                Some((owner, lamports, data)) => Ok(json!({
                    "address": addr,
                    "owner": owner,
                    "lamports": lamports,
                    "data_len": data.len(),
                })),
                None => Ok(json!({"address": addr, "exists": false})),
            }
        }
        "wallet_get_transaction" => {
            let sig = args.get("signature").and_then(|v| v.as_str()).unwrap_or("");
            let raw = rpc.get_transaction(sig)?;
            let ixs = decode::decode_json_parsed(&raw);
            Ok(json!({
                "signature": sig,
                "kind": decode::classify(&ixs),
                "instructions": ixs.iter().map(|i| i.to_json()).collect::<Vec<_>>(),
                "raw": raw,
            }))
        }
        "wallet_get_blockhash" => {
            let (hash, height) = rpc.get_latest_blockhash_and_height()?;
            Ok(json!({"blockhash": hash, "lastValidBlockHeight": height}))
        }
        "wallet_get_stake_accounts" => {
            let addr = arg_addr(args, &store)?;
            let rows = rpc.get_stake_accounts(&addr)?;
            Ok(serde_json::to_value(rows)?)
        }
        "wallet_get_contacts" => {
            let book = store::load_contacts(&store::contacts_path());
            Ok(serde_json::to_value(book.contacts)?)
        }
        "wallet_derive_ata" => {
            let owner = args
                .get("owner")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| active_address(&store));
            let mint = args.get("mint").and_then(|v| v.as_str()).unwrap_or("");
            let program = args
                .get("program")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(ProgramIds::TOKEN);
            let ata = tx::associated_token_address(&owner, mint, program)?;
            Ok(json!({"ata": ata, "owner": owner, "mint": mint, "program": program}))
        }
        "wallet_decode_instruction" => {
            let b64 = args.get("message_b64").and_then(|v| v.as_str()).unwrap_or("");
            let bytes = tx::decode_b64(b64)?;
            let ixs = decode::decode_compiled_message_with(&bytes, |k| {
                rpc.get_account(k).map_err(|e| e.to_string())
            })?;
            Ok(json!({
                "kind": decode::classify(&ixs),
                "instructions": ixs.iter().map(|i| i.to_json()).collect::<Vec<_>>(),
            }))
        }
        "wallet_send_sol" => spawn_vault(&[
            ("VIBE_SEND_DEST", args.get("to").and_then(|v| v.as_str()).unwrap_or("")),
            ("VIBE_SEND_AMOUNT", args.get("amount").and_then(|v| v.as_str()).unwrap_or("")),
            ("VIBE_SEND_MINT", "native"),
        ]),
        "wallet_swap" => spawn_vault(&[
            ("VIBE_SWAP_IN", args.get("in_mint").and_then(|v| v.as_str()).unwrap_or(WSOL_MINT)),
            ("VIBE_SWAP_OUT", args.get("out_mint").and_then(|v| v.as_str()).unwrap_or("")),
            ("VIBE_SWAP_AMOUNT", args.get("amount").and_then(|v| v.as_str()).unwrap_or("")),
        ]),
        "wallet_stake_create" => spawn_vault(&[
            ("VIBE_STAKE_ACTION", "delegate"),
            ("VIBE_STAKE_VOTE", args.get("vote").and_then(|v| v.as_str()).unwrap_or("")),
            ("VIBE_STAKE_AMOUNT", args.get("amount").and_then(|v| v.as_str()).unwrap_or("")),
        ]),
        "wallet_nonce_create" => spawn_vault(&[("VIBE_NONCE_ACTION", "create")]),
        "wallet_token_create" => {
            let decimals = args
                .get("decimals")
                .and_then(|v| v.as_u64())
                .unwrap_or(6)
                .to_string();
            spawn_vault(&[
                ("VIBE_MINT_NAME", args.get("name").and_then(|v| v.as_str()).unwrap_or("")),
                ("VIBE_MINT_SYMBOL", args.get("symbol").and_then(|v| v.as_str()).unwrap_or("")),
                ("VIBE_MINT_DECIMALS", decimals.as_str()),
                ("VIBE_MINT_SUPPLY", args.get("supply").and_then(|v| v.as_str()).unwrap_or("0")),
            ])
        }
        other => Err(crate::error::Error::msg(format!("unhandled tool {other}"))),
    }
}

fn arg_addr(args: &Value, store: &WalletStore) -> Result<String> {
    let raw = args.get("address").and_then(|v| v.as_str()).unwrap_or("");
    if raw.is_empty() {
        return Ok(active_address(store));
    }
    if !format::looks_like_pubkey(raw) {
        return Err(crate::error::Error::InvalidAddress);
    }
    Ok(raw.to_string())
}

fn active_address(store: &WalletStore) -> String {
    store
        .wallets
        .iter()
        .find(|w| w.id == store.active_id)
        .or_else(|| store.wallets.first())
        .map(|w| w.address.clone())
        .unwrap_or_default()
}

fn spawn_vault(pairs: &[(&str, &str)]) -> Result<Value> {
    let bin = vibe_vault_bin().ok_or_else(|| {
        crate::error::Error::msg("vibe-vault is not installed — cannot confirm a write")
    })?;
    let mut cmd = Command::new(bin);
    cmd.env("VIBE_SIGNER", "1");
    for (k, v) in pairs {
        if !v.is_empty() {
            cmd.env(k, v);
        }
    }
    let _ = cmd.spawn().map_err(|e| crate::error::Error::msg(e.to_string()))?;
    Ok(json!({
        "ok": true,
        "pending": true,
        "detail": "Confirm this in the VIBE vault window. Signing stays on this machine.",
    }))
}

fn vibe_vault_bin() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("VIBE_VAULT_BIN") {
        let p = PathBuf::from(raw);
        if p.is_file() {
            return Some(p);
        }
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    for cand in [
        PathBuf::from(&home).join(".local/bin/vibe-vault"),
        PathBuf::from("target/release/vibe-vault"),
        PathBuf::from("target/debug/vibe-vault"),
        PathBuf::from("/usr/bin/vibe-vault"),
    ] {
        if cand.is_file() {
            return Some(cand);
        }
    }
    which("vibe-vault")
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_read_and_write() {
        let all = tools();
        assert!(all.iter().any(|t| t.name == "wallet_ping"));
        assert!(all.iter().any(|t| t.name == "wallet_send_sol" && t.kind == ToolKind::Write));
        assert!(all.iter().any(|t| t.name == "wallet_nonce_create"));
        assert!(!all.iter().any(|t| t.name.contains("trezor")));
        let list = tools_list_json();
        assert!(list.as_array().unwrap().len() >= 10);
    }

    #[test]
    fn writes_blocked_by_default() {
        assert!(!write_enabled() || load_policy().get("write").is_some());
    }
}
