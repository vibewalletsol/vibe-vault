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
        // 1.9 — agent task status + tools ported from Cinder (MIT, Gregory Nowakowski).
        read("wallet_task_status", "Status of a queued / approved agent task (pending_approval, approved, opened, done, rejected, expired)", json!({"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"]})),
        read("wallet_info", "Bound wallet address, cluster and active RPC host", json!({"type":"object","properties":{}})),
        read("wallet_get_transactions", "Recent signatures for the wallet", json!({"type":"object","properties":{"address":{"type":"string"},"limit":{"type":"integer"}}})),
        read("wallet_classify_transaction", "Classify a transaction by signature (send, swap, stake …)", json!({"type":"object","properties":{"signature":{"type":"string"}},"required":["signature"]})),
        read("wallet_get_rent_exempt", "Rent-exempt minimum for an account size", json!({"type":"object","properties":{"space":{"type":"integer"}},"required":["space"]})),
        read("wallet_get_validators", "Top validators by stake", json!({"type":"object","properties":{"limit":{"type":"integer"}}})),
        read("wallet_get_validator_info", "One validator by vote account", json!({"type":"object","properties":{"vote":{"type":"string"}},"required":["vote"]})),
        read("wallet_get_priority_fees", "Recent prioritization fees (µ-lamports/CU) and the vault's presets and cap", json!({"type":"object","properties":{}})),
        read("wallet_get_network_stats", "Epoch and slot", json!({"type":"object","properties":{}})),
        read("wallet_find_contact", "Search the address book by name or address", json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]})),
        read("wallet_simulate_transaction", "Simulate a base64 transaction (no signing, sigVerify off)", json!({"type":"object","properties":{"transaction_b64":{"type":"string"}},"required":["transaction_b64"]})),
        read("wallet_get_swap_quote", "Jupiter quote only (no transaction)", json!({"type":"object","properties":{"in_mint":{"type":"string"},"out_mint":{"type":"string"},"amount_raw":{"type":"integer"}},"required":["out_mint","amount_raw"]})),
        read("wallet_encode_base58", "Hex bytes → base58", json!({"type":"object","properties":{"hex":{"type":"string"}},"required":["hex"]})),
        read("wallet_decode_base58", "Base58 → hex bytes", json!({"type":"object","properties":{"base58":{"type":"string"}},"required":["base58"]})),
        write("wallet_send_token", "Send SPL / Token-2022 tokens — Confirm in vibe-vault", json!({"type":"object","properties":{"to":{"type":"string"},"mint":{"type":"string"},"amount":{"type":"string"},"decimals":{"type":"integer"}},"required":["to","mint","amount"]})),
        write("wallet_stake_deactivate", "Deactivate a stake account — Confirm in vibe-vault", json!({"type":"object","properties":{"stake_account":{"type":"string"}},"required":["stake_account"]})),
        write("wallet_stake_withdraw", "Withdraw a deactivated stake account to the wallet — Confirm in vibe-vault", json!({"type":"object","properties":{"stake_account":{"type":"string"}},"required":["stake_account"]})),
        write("wallet_nonce_advance", "Advance a durable nonce — Confirm in vibe-vault", json!({"type":"object","properties":{"nonce_account":{"type":"string"}},"required":["nonce_account"]})),
        write("wallet_nonce_withdraw", "Withdraw SOL from a nonce account to the wallet — Confirm in vibe-vault", json!({"type":"object","properties":{"nonce_account":{"type":"string"},"amount":{"type":"string"}},"required":["nonce_account"]})),
        write("wallet_nonce_close", "Close a nonce account back to the wallet — Confirm in vibe-vault", json!({"type":"object","properties":{"nonce_account":{"type":"string"}},"required":["nonce_account"]})),
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

/// 1.9 entry point. `agent_key` comes from the MCP client config (`VIBE_AGENT_KEY`).
///
/// * No agents registered and no key: 1.8.x read-only mode (reads work, writes refused).
/// * Agents registered: every tool except `wallet_ping` needs a valid, unrevoked key.
/// * Every call goes through `agents::decide`; writes become tasks and only ever open the
///   vibe-vault Confirm (which re-checks the task). The legacy `mcp-policy.json` write flag
///   no longer enables anything.
pub fn dispatch_as(agent_key: Option<&str>, name: &str, args: &Value, rpc_url: &str) -> Result<Value> {
    use crate::agents::{self, Decision, TaskStatus};
    let def = tools()
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| crate::error::Error::msg(format!("unknown tool {name}")))?;
    if name == "wallet_ping" {
        return run_tool(None, name, args, rpc_url);
    }
    let key = agent_key.map(str::trim).filter(|k| !k.is_empty());
    let agent = match key {
        Some(k) => Some(agents::authenticate(k).ok_or_else(|| {
            crate::error::Error::msg("Unknown or revoked agent key. Create one in VIBE Options → Agents.")
        })?),
        None => None,
    };
    let Some(agent) = agent else {
        if !agents::load_registry().agents.iter().any(|a| !a.revoked) && def.kind == ToolKind::Read && name != "wallet_task_status" {
            return run_tool(None, name, args, rpc_url);
        }
        return Err(crate::error::Error::msg(
            "This tool needs an agent key. Create an agent in VIBE Options → Agents and put its key in VIBE_AGENT_KEY.",
        ));
    };
    if name == "wallet_task_status" {
        return task_status(&agent, args, rpc_url);
    }
    let book = agents::load_tasks();
    match agents::decide(&agent, name, args, agents::spent_24h(&book, &agent.id, agents::now())) {
        Decision::Deny(r) => Err(crate::error::Error::msg(format!("Denied by vault agent policy: {r}"))),
        Decision::Allow if def.kind == ToolKind::Read => run_tool(Some(&agent), name, args, rpc_url),
        _ => {
            validate_write_args(name, args)?;
            let task = agents::submit(&agent, name, args)?;
            if task.status == TaskStatus::Allowed {
                open_task(&agent, &task)?;
                let t = agents::mark_opened(&task.id)?;
                return Ok(agents::task_json(&t));
            }
            Ok(agents::task_json(&task))
        }
    }
}

/// 1.8.x signature kept for callers/tests: no agent key.
pub fn dispatch(name: &str, args: &Value, rpc_url: &str) -> Result<Value> {
    dispatch_as(None, name, args, rpc_url)
}

fn task_status(agent: &crate::agents::Agent, args: &Value, rpc_url: &str) -> Result<Value> {
    use crate::agents::{self, TaskStatus};
    let id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    let task = agents::load_tasks()
        .tasks
        .into_iter()
        .find(|t| t.id == id && t.agent_id == agent.id)
        .ok_or_else(|| crate::error::Error::msg("no such task for this agent"))?;
    if task.status == TaskStatus::Approved {
        let kind = tools().into_iter().find(|t| t.name == task.tool).map(|t| t.kind);
        if kind == Some(ToolKind::Read) {
            let out = run_tool(Some(agent), &task.tool, &task.args, rpc_url)?;
            return Ok(agents::task_json(&agents::set_read_result(&task.id, out)?));
        }
        open_task(agent, &task)?;
        return Ok(agents::task_json(&agents::mark_opened(&task.id)?));
    }
    Ok(agents::task_json(&task))
}

fn arg_s<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(|v| v.as_str()).unwrap_or("").trim()
}

/// Reject malformed write requests before they reach the queue.
fn validate_write_args(name: &str, args: &Value) -> Result<()> {
    let need_pk = |k: &str| -> Result<()> {
        if format::looks_like_pubkey(arg_s(args, k)) {
            Ok(())
        } else {
            Err(crate::error::Error::msg(format!("{k} must be a Solana address")))
        }
    };
    match name {
        "wallet_send_sol" => need_pk("to"),
        "wallet_send_token" => need_pk("to").and(need_pk("mint")),
        "wallet_swap" => need_pk("out_mint"),
        "wallet_stake_create" => need_pk("vote"),
        "wallet_stake_deactivate" | "wallet_stake_withdraw" => need_pk("stake_account"),
        "wallet_nonce_advance" | "wallet_nonce_withdraw" | "wallet_nonce_close" => need_pk("nonce_account"),
        _ => Ok(()),
    }
}

/// Open the vibe-vault Confirm for an approved / in-cap task, bound to the agent's wallet.
fn open_task(agent: &crate::agents::Agent, task: &crate::agents::Task) -> Result<()> {
    let mut pairs: Vec<(String, String)> = crate::agents::env_expectations(&task.tool, &task.args)
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    if task.tool == "wallet_send_token" {
        if let Some(d) = task.args.get("decimals").and_then(|v| v.as_u64()) {
            pairs.push(("VIBE_SEND_DECIMALS".into(), d.to_string()));
        }
    }
    if task.tool == "wallet_token_create" {
        let d = task.args.get("decimals").and_then(|v| v.as_u64()).unwrap_or(6);
        pairs.push(("VIBE_MINT_DECIMALS".into(), d.to_string()));
    }
    pairs.push(("VIBE_ACTIVE_ID".into(), agent.wallet_id.clone()));
    pairs.push(("VIBE_AGENT_TASK".into(), task.id.clone()));
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    spawn_vault(&refs).map(|_| ())
}

fn run_tool(agent: Option<&crate::agents::Agent>, name: &str, args: &Value, rpc_url: &str) -> Result<Value> {
    let mut store = store::load_store(&store::wallets_path())?;
    if let Some(a) = agent {
        // Wallet binding: the agent only ever sees its own wallet.
        store.wallets.retain(|w| w.id == a.wallet_id);
        store.active_id = a.wallet_id.clone();
    }
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
        n if tools().iter().any(|t| t.name == n && t.kind == ToolKind::Write) => Err(crate::error::Error::msg(
            "write tools only run as agent tasks",
        )),
        "wallet_info" => Ok(json!({
            "address": active_address(&store),
            "rpc": crate::rpc::endpoint_host(rpc_url),
            "version": crate::APP_VERSION,
        })),
        "wallet_get_transactions" => {
            let addr = arg_addr(args, &store)?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20).clamp(1, 100) as usize;
            let rows = rpc.get_signatures(&addr, limit)?;
            Ok(json!(rows.iter().map(|r| json!({
                "signature": r.signature, "slot": r.slot, "err": r.err, "memo": r.memo, "block_time": r.block_time,
            })).collect::<Vec<_>>()))
        }
        "wallet_classify_transaction" => {
            let sig = arg_s(args, "signature");
            let raw = rpc.get_transaction(sig)?;
            let ixs = decode::decode_json_parsed(&raw);
            Ok(json!({"signature": sig, "kind": decode::classify(&ixs)}))
        }
        "wallet_get_rent_exempt" => {
            let space = args.get("space").and_then(|v| v.as_u64()).unwrap_or(0).min(10 * 1024 * 1024);
            let lamports = rpc.get_min_rent(space)?;
            Ok(json!({"space": space, "lamports": lamports, "sol": format::format_sol(lamports)}))
        }
        "wallet_get_validators" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20).clamp(1, 200) as usize;
            let mut v = rpc.get_vote_accounts()?;
            v.sort_by(|a, b| b.activated_stake.cmp(&a.activated_stake));
            Ok(json!(v.iter().take(limit).map(validator_json).collect::<Vec<_>>()))
        }
        "wallet_get_validator_info" => {
            let vote = arg_s(args, "vote");
            let v = rpc.get_vote_accounts()?;
            let hit = v.iter().find(|x| x.vote == vote).ok_or_else(|| crate::error::Error::msg("validator not found"))?;
            Ok(validator_json(hit))
        }
        "wallet_get_priority_fees" => {
            let mut fees = rpc.recent_priority_fees()?;
            fees.sort_unstable();
            let pick = |q: usize| fees.get((fees.len().saturating_sub(1)) * q / 100).copied().unwrap_or(0);
            Ok(json!({
                "median": pick(50), "p75": pick(75), "p90": pick(90), "samples": fees.len(),
                "presets": crate::fees::PRESETS.iter().map(|(id, _, m)| json!({"id": id, "micro_lamports": m})).collect::<Vec<_>>(),
                "vault_max_priority_fee_lamports": crate::fees::MAX_PRIORITY_FEE_LAMPORTS,
            }))
        }
        "wallet_get_network_stats" => {
            let (epoch, slot) = rpc.get_epoch_info()?;
            Ok(json!({"epoch": epoch, "slot": slot}))
        }
        "wallet_find_contact" => {
            let q = arg_s(args, "query").to_lowercase();
            let book = store::load_contacts(&store::contacts_path());
            Ok(json!(book.contacts.iter().filter(|c| !q.is_empty() && (c.name.to_lowercase().contains(&q) || c.address.to_lowercase().contains(&q)))
                .map(|c| json!({"name": c.name, "address": c.address, "trusted": c.trusted})).collect::<Vec<_>>()))
        }
        "wallet_simulate_transaction" => {
            let b64 = arg_s(args, "transaction_b64");
            tx::decode_b64(b64)?;
            let out = rpc.simulate(b64)?;
            Ok(json!({"ok": true, "logs": out}))
        }
        "wallet_get_swap_quote" => {
            let inm = { let m = arg_s(args, "in_mint"); if m.is_empty() { WSOL_MINT } else { m } };
            let outm = arg_s(args, "out_mint");
            let amt = args.get("amount_raw").and_then(|v| v.as_u64()).unwrap_or(0);
            if amt == 0 || !format::looks_like_pubkey(outm) {
                return Err(crate::error::Error::msg("need out_mint and amount_raw > 0"));
            }
            let o = crate::jupiter::Client::from_env_or("").order(inm, outm, amt, None, None)?;
            Ok(json!({"in_amount": o.in_amount, "out_amount": o.out_amount, "price_impact": o.price_impact,
                "router": o.router, "route": o.route_summary()}))
        }
        "wallet_encode_base58" => {
            let h = arg_s(args, "hex");
            let bytes = (0..h.len() / 2).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16))
                .collect::<std::result::Result<Vec<u8>, _>>()
                .map_err(|_| crate::error::Error::msg("hex must be pairs of hex digits"))?;
            if h.len() % 2 != 0 || bytes.len() > 4096 {
                return Err(crate::error::Error::msg("hex must be pairs of hex digits (max 4 KiB)"));
            }
            Ok(json!({"base58": bs58::encode(bytes).into_string()}))
        }
        "wallet_decode_base58" => {
            let raw = bs58::decode(arg_s(args, "base58")).into_vec().map_err(|_| crate::error::Error::msg("not base58"))?;
            Ok(json!({"hex": raw.iter().map(|b| format!("{b:02x}")).collect::<String>(), "len": raw.len()}))
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

fn validator_json(v: &crate::rpc::Validator) -> Value {
    json!({"vote": v.vote, "node": v.node, "activated_stake": v.activated_stake, "commission": v.commission, "delinquent": v.delinquent})
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
    // 1.9: the agent controls this process's environment. Drop every inherited VIBE_*
    // variable (RPC URL, memo, WalletConnect request, bot task …) so the Confirm shows
    // exactly the task and nothing the agent smuggled in.
    for (k, _) in std::env::vars_os() {
        if let Some(k) = k.to_str() {
            if k.starts_with("VIBE_") && k != "VIBE_DATA_DIR" && k != "VIBE_DEV" {
                cmd.env_remove(k);
            }
        }
    }
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
        // No agent key → no write, whatever mcp-policy.json says.
        let e = dispatch("wallet_send_sol", &json!({"to": "x", "amount": "1"}), "http://127.0.0.1:1").unwrap_err();
        assert!(e.to_string().contains("agent key"), "{e}");
        let e = dispatch_as(Some("vibe_ak_nope"), "wallet_get_balance", &json!({}), "http://127.0.0.1:1").unwrap_err();
        assert!(e.to_string().contains("Unknown or revoked"), "{e}");
    }

    #[test]
    fn every_tool_has_a_policy_kind() {
        for t in tools() {
            let k = crate::agents::tool_kind(t.name).unwrap_or_else(|| panic!("{} has no agent policy kind", t.name));
            match t.kind {
                ToolKind::Write => assert!(matches!(k, crate::agents::Kind::Spend | crate::agents::Kind::SelfWrite), "{}", t.name),
                ToolKind::Read => assert!(matches!(k, crate::agents::Kind::Read | crate::agents::Kind::Meta), "{}", t.name),
            }
            if t.kind == ToolKind::Write {
                assert!(!crate::agents::env_expectations(t.name, &json!({})).is_empty(), "{} has no Confirm check", t.name);
            }
        }
    }
}
