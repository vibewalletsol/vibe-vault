//! AI agent access control (1.9), enforced by the vault binaries.
//!
//! * Per-agent API keys: `vibe_ak_<base58 32 random bytes>`. Only a salted SHA-256 is
//!   stored; the key is shown once at creation.
//! * Wallet binding: an agent sees and acts for exactly one vault wallet.
//! * Per-tool level: `blocked`, `approval` or `allowed`.
//! * Per-tx and per-day caps (SOL). Value-moving calls with no cap set, above a cap, or
//!   whose SOL value the vault can't determine always need approval: there is no
//!   unlimited auto-approve.
//! * Every write becomes a task. `approval` tasks wait in the desk queue
//!   (Options → Agents). An approved or in-cap task opens the normal vibe-vault Confirm,
//!   which re-checks the task (agent not revoked, same wallet, same arguments, caps) and
//!   then runs the usual txcheck / R14b / policy rules on the exact bytes. The human still
//!   enters the vault password: 1.9 has no unattended signing.
//!
//! Files (0600, in the vault config dir): `agents.json`, `agent-tasks.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::store;

pub const KEY_PREFIX: &str = "vibe_ak_";
/// Pending approvals expire after an hour.
pub const PENDING_TTL_SECS: u64 = 3600;
/// Approved / in-cap tasks must be opened within 15 minutes.
pub const OPEN_TTL_SECS: u64 = 900;
const DAY_SECS: u64 = 86_400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Blocked,
    Approval,
    Allowed,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        match s.trim().to_ascii_lowercase().as_str() {
            "blocked" | "block" | "off" => Some(Level::Blocked),
            "approval" | "approve" | "ask" => Some(Level::Approval),
            "allowed" | "allow" | "on" => Some(Level::Allowed),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Blocked => "blocked",
            Level::Approval => "approval",
            Level::Allowed => "allowed",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub key_salt: String,
    pub key_hash: String,
    /// First characters of the key, for recognising it in the UI (not enough to use it).
    pub key_hint: String,
    pub wallet_id: String,
    pub wallet_address: String,
    #[serde(default)]
    pub levels: BTreeMap<String, Level>,
    #[serde(default)]
    pub per_tx_cap_sol: f64,
    #[serde(default)]
    pub per_day_cap_sol: f64,
    pub created: u64,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub last_used: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub agents: Vec<Agent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Waiting in the desk queue.
    PendingApproval,
    /// Approved by the human in the desk; the vault Confirm opens on the next status poll.
    Approved,
    /// In level + caps; the vault Confirm opens right away.
    Allowed,
    /// vibe-vault Confirm is open.
    Opened,
    Done,
    Failed,
    Rejected,
    Expired,
}

impl TaskStatus {
    pub fn counts_toward_day_cap(&self) -> bool {
        matches!(self, TaskStatus::PendingApproval | TaskStatus::Approved | TaskStatus::Allowed | TaskStatus::Opened | TaskStatus::Done)
    }
    pub fn is_final(&self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Rejected | TaskStatus::Expired)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub wallet_id: String,
    pub tool: String,
    pub args: Value,
    /// SOL leaving the wallet, when the vault can tell. None = unknown.
    pub sol_out: Option<f64>,
    pub status: TaskStatus,
    #[serde(default)]
    pub reason: String,
    pub created: u64,
    pub updated: u64,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub error: String,
    /// Result of an approved read.
    #[serde(default)]
    pub result: Value,
    /// A human approved it on the desk (caps are not re-applied at Confirm).
    #[serde(default)]
    pub approved_by_human: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskBook {
    #[serde(default)]
    pub tasks: Vec<Task>,
}

// ---------------------------------------------------------------------------
// Tool metadata
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Reads chain / vault data.
    Read,
    /// Moves value out of the wallet (caps apply).
    Spend,
    /// Signs, but funds only return to the wallet (stake/nonce withdraw to self, advance).
    SelfWrite,
    /// Always allowed, never queued (ping, task status).
    Meta,
}

pub fn tool_kind(tool: &str) -> Option<Kind> {
    Some(match tool {
        "wallet_ping" | "wallet_task_status" => Kind::Meta,
        "wallet_address" | "wallet_info" | "wallet_list_wallets" | "wallet_get_balance"
        | "wallet_get_token_accounts" | "wallet_inspect_account" | "wallet_get_transaction"
        | "wallet_get_transactions" | "wallet_classify_transaction" | "wallet_get_blockhash"
        | "wallet_get_stake_accounts" | "wallet_get_contacts" | "wallet_find_contact"
        | "wallet_derive_ata" | "wallet_decode_instruction" | "wallet_get_rent_exempt"
        | "wallet_get_validators" | "wallet_get_validator_info" | "wallet_get_priority_fees"
        | "wallet_get_network_stats" | "wallet_simulate_transaction" | "wallet_get_swap_quote"
        | "wallet_encode_base58" | "wallet_decode_base58" => Kind::Read,
        "wallet_send_sol" | "wallet_send_token" | "wallet_swap" | "wallet_stake_create"
        | "wallet_nonce_create" | "wallet_token_create" => Kind::Spend,
        "wallet_stake_deactivate" | "wallet_stake_withdraw" | "wallet_nonce_advance"
        | "wallet_nonce_withdraw" | "wallet_nonce_close" => Kind::SelfWrite,
        _ => return None,
    })
}

pub fn default_level(tool: &str) -> Level {
    match tool_kind(tool) {
        Some(Kind::Read) | Some(Kind::Meta) => Level::Allowed,
        Some(_) => Level::Approval,
        None => Level::Blocked,
    }
}

fn is_sol_mint(m: &str) -> bool {
    let m = m.trim();
    m.is_empty() || m.eq_ignore_ascii_case("sol") || m.eq_ignore_ascii_case("native") || m == crate::WSOL_MINT
}

fn arg<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(|v| v.as_str()).unwrap_or("").trim()
}

fn parse_sol_amount(s: &str) -> Option<f64> {
    let v: f64 = s.replace(',', "").trim().parse().ok()?;
    (v.is_finite() && v > 0.0).then_some(v)
}

/// SOL leaving the wallet for a Spend tool, if the vault can tell from the arguments.
pub fn sol_out(tool: &str, args: &Value) -> Option<f64> {
    match tool {
        "wallet_send_sol" | "wallet_stake_create" => parse_sol_amount(arg(args, "amount")),
        "wallet_swap" if is_sol_mint(arg(args, "in_mint")) => parse_sol_amount(arg(args, "amount")),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hash_key(salt_hex: &str, key: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"vibe-agent-key-v1");
    h.update(salt_hex.as_bytes());
    h.update(key.trim().as_bytes());
    hex(&h.finalize())
}

fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_id(prefix: &str, n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut b);
    format!("{prefix}{}", bs58::encode(b).into_string())
}

// ---------------------------------------------------------------------------
// Storage (0600 JSON, with a simple cross-process lock)
// ---------------------------------------------------------------------------

pub fn agents_path() -> PathBuf {
    store::config_dir().join("agents.json")
}

pub fn tasks_path() -> PathBuf {
    store::config_dir().join("agent-tasks.json")
}

struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn lock() -> Result<Lock> {
    let path = store::config_dir().join("agents.lock");
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    for _ in 0..200 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(Lock(path)),
            Err(_) => {
                // Stale lock (crashed process) after 10 s.
                if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.modified().ok().and_then(|m| m.elapsed().ok()).map(|e| e.as_secs() > 10).unwrap_or(false) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
    Err(Error::msg("agent store is busy"))
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &PathBuf) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn load_registry() -> Registry {
    read_json(&agents_path())
}

fn save_registry(r: &Registry) -> Result<()> {
    store::write_private(&agents_path(), &serde_json::to_string_pretty(r)?)
}

pub fn load_tasks() -> TaskBook {
    let mut book: TaskBook = read_json(&tasks_path());
    expire(&mut book, now());
    book
}

fn save_tasks(b: &TaskBook) -> Result<()> {
    store::write_private(&tasks_path(), &serde_json::to_string_pretty(b)?)
}

fn expire(book: &mut TaskBook, t: u64) {
    for task in &mut book.tasks {
        let age = t.saturating_sub(task.updated);
        let stale = match task.status {
            TaskStatus::PendingApproval => age > PENDING_TTL_SECS,
            TaskStatus::Approved | TaskStatus::Allowed => age > OPEN_TTL_SECS,
            TaskStatus::Opened => age > PENDING_TTL_SECS,
            _ => false,
        };
        if stale {
            task.status = TaskStatus::Expired;
            task.updated = t;
        }
    }
    // Keep a week of history.
    book.tasks.retain(|x| !x.status.is_final() || t.saturating_sub(x.updated) < 7 * DAY_SECS);
}

// ---------------------------------------------------------------------------
// Admin (desk Options → Agents, via `vibe-mcp admin …`)
// ---------------------------------------------------------------------------

pub struct Created {
    pub agent: Agent,
    /// Shown once. Never stored.
    pub key: String,
}

pub fn create_agent(name: &str, wallet_id: &str, per_tx_cap_sol: f64, per_day_cap_sol: f64) -> Result<Created> {
    let st = store::load_store(&store::wallets_path())?;
    let w = st
        .wallets
        .iter()
        .find(|w| w.id == wallet_id || w.address == wallet_id)
        .ok_or_else(|| Error::msg("pick a wallet from this vault"))?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err(Error::msg("agent name must be 1–40 characters"));
    }
    let _g = lock()?;
    let mut reg = load_registry();
    let key = random_id(KEY_PREFIX, 32);
    let salt = hex(&{
        let mut s = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut s);
        s
    });
    let agent = Agent {
        id: random_id("ag_", 9),
        name: name.to_string(),
        key_hash: hash_key(&salt, &key),
        key_salt: salt,
        key_hint: key.chars().take(KEY_PREFIX.len() + 4).collect::<String>() + "…",
        wallet_id: w.id.clone(),
        wallet_address: w.address.clone(),
        levels: BTreeMap::new(),
        per_tx_cap_sol: clean_cap(per_tx_cap_sol),
        per_day_cap_sol: clean_cap(per_day_cap_sol),
        created: now(),
        revoked: false,
        last_used: 0,
    };
    reg.version = 1;
    reg.agents.push(agent.clone());
    save_registry(&reg)?;
    Ok(Created { agent, key })
}

fn clean_cap(v: f64) -> f64 {
    if v.is_finite() && v > 0.0 {
        v.min(1_000_000.0)
    } else {
        0.0
    }
}

pub fn revoke_agent(id: &str) -> Result<()> {
    let _g = lock()?;
    let mut reg = load_registry();
    let a = reg.agents.iter_mut().find(|a| a.id == id).ok_or_else(|| Error::msg("no such agent"))?;
    a.revoked = true;
    a.key_hash = String::new(); // the key can never match again
    save_registry(&reg)?;
    drop(_g);
    // Cancel anything still in flight for that agent.
    let _g = lock()?;
    let mut book = load_tasks();
    let t = now();
    for task in book.tasks.iter_mut().filter(|x| x.agent_id == id && !x.status.is_final()) {
        task.status = TaskStatus::Rejected;
        task.reason = "agent revoked".into();
        task.updated = t;
    }
    save_tasks(&book)
}

pub fn set_level(id: &str, tool: &str, level: Level) -> Result<()> {
    if tool_kind(tool).is_none() {
        return Err(Error::msg(format!("unknown tool {tool}")));
    }
    if tool_kind(tool) == Some(Kind::Meta) {
        return Err(Error::msg("wallet_ping and wallet_task_status are always on"));
    }
    let _g = lock()?;
    let mut reg = load_registry();
    let a = reg.agents.iter_mut().find(|a| a.id == id && !a.revoked).ok_or_else(|| Error::msg("no such agent"))?;
    a.levels.insert(tool.to_string(), level);
    save_registry(&reg)
}

pub fn set_caps(id: &str, per_tx_cap_sol: f64, per_day_cap_sol: f64) -> Result<()> {
    let _g = lock()?;
    let mut reg = load_registry();
    let a = reg.agents.iter_mut().find(|a| a.id == id && !a.revoked).ok_or_else(|| Error::msg("no such agent"))?;
    a.per_tx_cap_sol = clean_cap(per_tx_cap_sol);
    a.per_day_cap_sol = clean_cap(per_day_cap_sol);
    save_registry(&reg)
}

pub fn level_of(a: &Agent, tool: &str) -> Level {
    if tool_kind(tool) == Some(Kind::Meta) {
        return Level::Allowed;
    }
    a.levels.get(tool).copied().unwrap_or_else(|| default_level(tool))
}

/// Public view (no hash / salt).
pub fn agent_json(a: &Agent) -> Value {
    let spent = spent_24h(&load_tasks(), &a.id, now()) + 0.0; // no "-0.0"
    json!({
        "id": a.id, "name": a.name, "key_hint": a.key_hint,
        "wallet_id": a.wallet_id, "wallet_address": a.wallet_address,
        "per_tx_cap_sol": a.per_tx_cap_sol, "per_day_cap_sol": a.per_day_cap_sol,
        "created": a.created, "revoked": a.revoked, "last_used": a.last_used,
        "levels": crate::mcp::tools().iter().filter(|t| tool_kind(t.name) != Some(Kind::Meta))
            .map(|t| (t.name.to_string(), json!(level_of(a, t.name).as_str())))
            .collect::<serde_json::Map<String, Value>>(),
        "spent_24h_sol": spent,
    })
}

// ---------------------------------------------------------------------------
// Authentication + decision
// ---------------------------------------------------------------------------

pub fn authenticate(key: &str) -> Option<Agent> {
    let key = key.trim();
    if !key.starts_with(KEY_PREFIX) {
        return None;
    }
    let reg = load_registry();
    let found = reg
        .agents
        .into_iter()
        .find(|a| !a.revoked && !a.key_hash.is_empty() && ct_eq(&hash_key(&a.key_salt, key), &a.key_hash))?;
    if let Ok(_g) = lock() {
        let mut reg = load_registry();
        if let Some(a) = reg.agents.iter_mut().find(|a| a.id == found.id) {
            a.last_used = now();
            let _ = save_registry(&reg);
        }
    }
    Some(found)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Deny(String),
    NeedsApproval(String),
    Allow,
}

pub fn spent_24h(book: &TaskBook, agent_id: &str, t: u64) -> f64 {
    book.tasks
        .iter()
        .filter(|x| x.agent_id == agent_id && x.status.counts_toward_day_cap() && t.saturating_sub(x.created) < DAY_SECS)
        .filter_map(|x| x.sol_out)
        .sum()
}

/// Pure policy decision (no I/O). `spent` = SOL already committed in the last 24 h.
pub fn decide(a: &Agent, tool: &str, args: &Value, spent: f64) -> Decision {
    if a.revoked {
        return Decision::Deny("agent revoked".into());
    }
    let Some(kind) = tool_kind(tool) else {
        return Decision::Deny(format!("unknown tool {tool}"));
    };
    let level = level_of(a, tool);
    if level == Level::Blocked {
        return Decision::Deny(format!("{tool} is blocked for this agent"));
    }
    if level == Level::Approval {
        return Decision::NeedsApproval(format!("{tool} needs approval"));
    }
    if kind != Kind::Spend {
        return Decision::Allow;
    }
    if a.per_tx_cap_sol <= 0.0 || a.per_day_cap_sol <= 0.0 {
        return Decision::NeedsApproval("no per-tx / per-day cap set, so every spend needs approval".into());
    }
    let Some(sol) = sol_out(tool, args) else {
        return Decision::NeedsApproval("the vault can't value this in SOL, so it needs approval".into());
    };
    if sol > a.per_tx_cap_sol + 1e-9 {
        return Decision::NeedsApproval(format!("{sol} SOL is above the per-tx cap of {} SOL", a.per_tx_cap_sol));
    }
    if spent + sol > a.per_day_cap_sol + 1e-9 {
        return Decision::NeedsApproval(format!(
            "{sol} SOL would take today's total to {:.4} SOL, above the per-day cap of {} SOL",
            spent + sol,
            a.per_day_cap_sol
        ));
    }
    Decision::Allow
}

/// Record a task for a write (or an approval-level read). Returns the stored task.
pub fn submit(a: &Agent, tool: &str, args: &Value) -> Result<Task> {
    let _g = lock()?;
    let mut book = load_tasks();
    let t = now();
    let decision = decide(a, tool, args, spent_24h(&book, &a.id, t));
    let (status, reason) = match decision {
        Decision::Deny(r) => return Err(Error::msg(format!("Denied by vault agent policy: {r}"))),
        Decision::NeedsApproval(r) => (TaskStatus::PendingApproval, r),
        Decision::Allow => (TaskStatus::Allowed, "within this agent's level and caps".to_string()),
    };
    // Rate limit: 20 open tasks per agent.
    if book.tasks.iter().filter(|x| x.agent_id == a.id && !x.status.is_final()).count() >= 20 {
        return Err(Error::msg("too many open tasks for this agent; approve or reject some in Options → Agents"));
    }
    let task = Task {
        id: random_id("tk_", 9),
        agent_id: a.id.clone(),
        agent_name: a.name.clone(),
        wallet_id: a.wallet_id.clone(),
        tool: tool.to_string(),
        args: args.clone(),
        sol_out: sol_out(tool, args),
        status,
        reason,
        created: t,
        updated: t,
        signature: String::new(),
        error: String::new(),
        result: Value::Null,
        approved_by_human: false,
    };
    book.tasks.push(task.clone());
    save_tasks(&book)?;
    Ok(task)
}

fn update<F: FnOnce(&mut Task) -> Result<()>>(id: &str, f: F) -> Result<Task> {
    let _g = lock()?;
    let mut book = load_tasks();
    let task = book.tasks.iter_mut().find(|x| x.id == id).ok_or_else(|| Error::msg("no such task"))?;
    f(task)?;
    task.updated = now();
    let out = task.clone();
    save_tasks(&book)?;
    Ok(out)
}

/// Desk approval. Re-checks that the agent still exists and is bound to the same wallet.
pub fn approve(id: &str) -> Result<Task> {
    let reg = load_registry();
    update(id, |t| {
        if t.status != TaskStatus::PendingApproval {
            return Err(Error::msg(format!("task is {:?}, not pending", t.status)));
        }
        let a = reg.agents.iter().find(|a| a.id == t.agent_id && !a.revoked).ok_or_else(|| Error::msg("agent revoked"))?;
        if a.wallet_id != t.wallet_id || level_of(a, &t.tool) == Level::Blocked {
            return Err(Error::msg("agent policy changed since this was queued; reject it"));
        }
        t.status = TaskStatus::Approved;
        t.approved_by_human = true;
        t.reason = "approved on the desk".into();
        Ok(())
    })
}

pub fn reject(id: &str) -> Result<Task> {
    update(id, |t| {
        if t.status.is_final() {
            return Err(Error::msg("task already finished"));
        }
        t.status = TaskStatus::Rejected;
        t.reason = "rejected on the desk".into();
        Ok(())
    })
}

pub fn mark_opened(id: &str) -> Result<Task> {
    update(id, |t| {
        if !matches!(t.status, TaskStatus::Approved | TaskStatus::Allowed) {
            return Err(Error::msg("task is not ready to open"));
        }
        t.status = TaskStatus::Opened;
        Ok(())
    })
}

pub fn set_read_result(id: &str, result: Value) -> Result<Task> {
    update(id, |t| {
        t.status = TaskStatus::Done;
        t.result = result;
        Ok(())
    })
}

pub fn record_result(id: &str, res: std::result::Result<String, String>) -> Result<Task> {
    update(id, |t| {
        match res {
            Ok(sig) => {
                t.status = TaskStatus::Done;
                t.signature = sig;
            }
            // The Confirm stays open for a retry; the task stays Opened (expires in 1 h).
            Err(e) => {
                t.error = e.chars().take(300).collect();
            }
        }
        Ok(())
    })
}

pub fn task_json(t: &Task) -> Value {
    json!({
        "task_id": t.id, "tool": t.tool, "status": t.status, "reason": t.reason,
        "sol_out": t.sol_out, "created": t.created, "updated": t.updated,
        "signature": t.signature, "error": t.error, "result": t.result,
        "agent": t.agent_name, "args": t.args,
    })
}

// ---------------------------------------------------------------------------
// Vault Confirm check (vibe-vault, at sign time)
// ---------------------------------------------------------------------------

/// Env var names the vault compares with the stored task arguments.
pub fn env_expectations(tool: &str, args: &Value) -> Vec<(&'static str, String)> {
    let a = |k: &str| arg(args, k).to_string();
    match tool {
        "wallet_send_sol" => vec![("VIBE_SEND_DEST", a("to")), ("VIBE_SEND_AMOUNT", a("amount")), ("VIBE_SEND_MINT", "native".into())],
        "wallet_send_token" => vec![("VIBE_SEND_DEST", a("to")), ("VIBE_SEND_AMOUNT", a("amount")), ("VIBE_SEND_MINT", a("mint"))],
        "wallet_swap" => vec![("VIBE_SWAP_IN", if a("in_mint").is_empty() { crate::WSOL_MINT.into() } else { a("in_mint") }), ("VIBE_SWAP_OUT", a("out_mint")), ("VIBE_SWAP_AMOUNT", a("amount"))],
        "wallet_stake_create" => vec![("VIBE_STAKE_ACTION", "delegate".into()), ("VIBE_STAKE_VOTE", a("vote")), ("VIBE_STAKE_AMOUNT", a("amount"))],
        "wallet_stake_deactivate" => vec![("VIBE_STAKE_ACTION", "deactivate".into()), ("VIBE_UNSTAKE_ACCOUNT", a("stake_account"))],
        "wallet_stake_withdraw" => vec![("VIBE_STAKE_ACTION", "withdraw".into()), ("VIBE_UNSTAKE_ACCOUNT", a("stake_account"))],
        "wallet_nonce_create" => vec![("VIBE_NONCE_ACTION", "create".into())],
        "wallet_nonce_advance" => vec![("VIBE_NONCE_ACTION", "advance".into()), ("VIBE_NONCE_ACCOUNT", a("nonce_account"))],
        "wallet_nonce_withdraw" => vec![("VIBE_NONCE_ACTION", "withdraw".into()), ("VIBE_NONCE_ACCOUNT", a("nonce_account")), ("VIBE_NONCE_AMOUNT", a("amount"))],
        "wallet_nonce_close" => vec![("VIBE_NONCE_ACTION", "close".into()), ("VIBE_NONCE_ACCOUNT", a("nonce_account"))],
        "wallet_token_create" => vec![("VIBE_MINT_SYMBOL", a("symbol")), ("VIBE_MINT_NAME", a("name")), ("VIBE_MINT_SUPPLY", a("supply"))],
        _ => vec![],
    }
}

/// Which vault Confirm flow a tool must open ("send", "swap", "stake", "nonce", "mint").
pub fn confirm_kind(tool: &str) -> &'static str {
    match tool {
        "wallet_send_sol" | "wallet_send_token" => "send",
        "wallet_swap" => "swap",
        "wallet_stake_create" | "wallet_stake_deactivate" | "wallet_stake_withdraw" => "stake",
        "wallet_nonce_create" | "wallet_nonce_advance" | "wallet_nonce_withdraw" | "wallet_nonce_close" => "nonce",
        "wallet_token_create" => "mint",
        _ => "",
    }
}

/// Called by vibe-vault when `VIBE_AGENT_TASK` is set. The Confirm is refused unless the
/// task is open, its agent still exists and is bound to the wallet being signed with,
/// the Confirm's parameters are exactly the task's, and the per-tx / per-day caps still
/// hold for in-cap (non-approved) tasks.
pub fn verify_for_confirm<F: Fn(&str) -> String>(task_id: &str, signing_wallet_id: &str, confirm: &str, env: F) -> std::result::Result<Task, String> {
    let book = load_tasks();
    let task = book.tasks.iter().find(|t| t.id == task_id).cloned().ok_or("agent task not found")?;
    if confirm_kind(&task.tool) != confirm {
        return Err(format!("this Confirm ({confirm}) is not the agent task's flow ({})", task.tool));
    }
    if !matches!(task.status, TaskStatus::Approved | TaskStatus::Allowed | TaskStatus::Opened) {
        return Err(format!("agent task is {:?}", task.status));
    }
    let reg = load_registry();
    let agent = reg.agents.iter().find(|a| a.id == task.agent_id).ok_or("agent no longer exists")?;
    if agent.revoked {
        return Err("agent was revoked".into());
    }
    if agent.wallet_id != task.wallet_id || signing_wallet_id != agent.wallet_id {
        return Err("agent is not bound to this wallet".into());
    }
    if level_of(agent, &task.tool) == Level::Blocked {
        return Err(format!("{} is now blocked for this agent", task.tool));
    }
    for (k, want) in env_expectations(&task.tool, &task.args) {
        let got = env(k);
        if got.trim() != want.trim() {
            return Err(format!("Confirm parameters differ from the agent task ({k})"));
        }
    }
    if !task.approved_by_human {
        // In-cap task: re-run the decision now (caps may have been lowered), excluding itself.
        let others = TaskBook { tasks: book.tasks.iter().filter(|t| t.id != task.id).cloned().collect() };
        match decide(agent, &task.tool, &task.args, spent_24h(&others, &agent.id, now())) {
            Decision::Allow => {}
            Decision::NeedsApproval(r) | Decision::Deny(r) => return Err(format!("agent policy: {r}")),
        }
    }
    Ok(task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(tx: f64, day: f64) -> Agent {
        Agent {
            id: "ag_1".into(), name: "bot".into(), key_salt: "00".into(), key_hash: hash_key("00", "vibe_ak_x"),
            key_hint: String::new(), wallet_id: "w1".into(), wallet_address: "A".into(), levels: BTreeMap::new(),
            per_tx_cap_sol: tx, per_day_cap_sol: day, created: 0, revoked: false, last_used: 0,
        }
    }

    #[test]
    fn defaults_reads_allowed_writes_approval() {
        let a = agent(1.0, 5.0);
        assert_eq!(decide(&a, "wallet_get_balance", &json!({}), 0.0), Decision::Allow);
        assert!(matches!(decide(&a, "wallet_send_sol", &json!({"to":"x","amount":"0.1"}), 0.0), Decision::NeedsApproval(_)));
        assert!(matches!(decide(&a, "wallet_nuke", &json!({}), 0.0), Decision::Deny(_)));
        assert_eq!(level_of(&a, "wallet_task_status"), Level::Allowed);
    }

    #[test]
    fn no_unlimited_auto_approve() {
        let mut a = agent(0.0, 0.0);
        a.levels.insert("wallet_send_sol".into(), Level::Allowed);
        // No caps → approval, even at "allowed".
        assert!(matches!(decide(&a, "wallet_send_sol", &json!({"amount":"0.01"}), 0.0), Decision::NeedsApproval(_)));
        a.per_tx_cap_sol = 0.5;
        assert!(matches!(decide(&a, "wallet_send_sol", &json!({"amount":"0.01"}), 0.0), Decision::NeedsApproval(_)), "day cap missing");
        a.per_day_cap_sol = 1.0;
        assert_eq!(decide(&a, "wallet_send_sol", &json!({"amount":"0.5"}), 0.0), Decision::Allow);
        assert!(matches!(decide(&a, "wallet_send_sol", &json!({"amount":"0.51"}), 0.0), Decision::NeedsApproval(_)));
        assert!(matches!(decide(&a, "wallet_send_sol", &json!({"amount":"0.5"}), 0.6), Decision::NeedsApproval(_)));
        // Unknown value (token send, non-SOL swap) → approval.
        a.levels.insert("wallet_send_token".into(), Level::Allowed);
        a.levels.insert("wallet_swap".into(), Level::Allowed);
        assert!(matches!(decide(&a, "wallet_send_token", &json!({"amount":"1"}), 0.0), Decision::NeedsApproval(_)));
        assert!(matches!(decide(&a, "wallet_swap", &json!({"in_mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","amount":"1"}), 0.0), Decision::NeedsApproval(_)));
        assert_eq!(decide(&a, "wallet_swap", &json!({"in_mint":"","amount":"0.1"}), 0.0), Decision::Allow);
        // Junk amounts never auto-approve.
        for bad in ["-1", "NaN", "inf", "", "0"] {
            assert!(matches!(decide(&a, "wallet_send_sol", &json!({"amount": bad}), 0.0), Decision::NeedsApproval(_)), "{bad}");
        }
    }

    #[test]
    fn blocked_and_revoked_deny() {
        let mut a = agent(1.0, 1.0);
        a.levels.insert("wallet_get_balance".into(), Level::Blocked);
        assert!(matches!(decide(&a, "wallet_get_balance", &json!({}), 0.0), Decision::Deny(_)));
        a.revoked = true;
        assert!(matches!(decide(&a, "wallet_ping", &json!({}), 0.0), Decision::Deny(_)));
    }

    #[test]
    fn keys_hash_and_compare() {
        let a = agent(0.0, 0.0);
        assert!(ct_eq(&hash_key(&a.key_salt, "vibe_ak_x"), &a.key_hash));
        assert!(!ct_eq(&hash_key(&a.key_salt, "vibe_ak_y"), &a.key_hash));
        assert_ne!(hash_key("01", "vibe_ak_x"), a.key_hash);
    }

    #[test]
    fn day_cap_counts_open_and_done_only() {
        let mk = |st: TaskStatus, sol: f64, age: u64| Task {
            id: "t".into(), agent_id: "ag_1".into(), agent_name: String::new(), wallet_id: "w1".into(), tool: "wallet_send_sol".into(),
            args: json!({}), sol_out: Some(sol), status: st, reason: String::new(), created: 100_000 - age, updated: 0,
            signature: String::new(), error: String::new(), result: Value::Null, approved_by_human: false,
        };
        let book = TaskBook { tasks: vec![
            mk(TaskStatus::Done, 0.3, 10), mk(TaskStatus::PendingApproval, 0.2, 10), mk(TaskStatus::Rejected, 5.0, 10),
            mk(TaskStatus::Failed, 5.0, 10), mk(TaskStatus::Done, 9.0, DAY_SECS + 1),
        ]};
        assert!((spent_24h(&book, "ag_1", 100_000) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn confirm_env_must_match_task() {
        let e = env_expectations("wallet_send_sol", &json!({"to":"Dest","amount":"0.1"}));
        assert_eq!(e[0], ("VIBE_SEND_DEST", "Dest".to_string()));
        assert_eq!(e[2], ("VIBE_SEND_MINT", "native".to_string()));
    }
}
