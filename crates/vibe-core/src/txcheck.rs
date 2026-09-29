//! Vault-side transaction review: strict legacy/v0 message parsing, Address
//! Lookup Table (ALT) resolution and the static drainer rules (vault-spec.md §3.2, §4).
//!
//! Everything here runs inside vibe-vault on the exact bytes that will be signed.
//! Nothing the desk or a dApp says about the transaction is trusted.
//!
//! * Every account the transaction touches is listed, including the ones loaded
//!   from lookup tables (flagged with the table and index, writable or readonly).
//! * If a lookup table can't be fetched, isn't owned by the ALT program, is
//!   deactivated, or an index is out of range, the review BLOCKS (rule R15).
//! * Rules R1–R7, R10, R11, R13–R16, R18 are evaluated on the fully resolved
//!   account list. R8/R9 (simulation diff), R12 (mint extensions for dApp txs),
//!   R17 (USD threshold + delay) and R19 (Jupiter IDL) are not implemented yet
//!   and are reported as INFO so the gap is visible.

use std::collections::{BTreeMap, HashSet};

use crate::programs::ProgramIds;
use crate::tx::{self, AccountMeta, Instruction};

/// Size of the ALT metadata header (`LOOKUP_TABLE_META_SIZE`).
pub const ALT_META_SIZE: usize = 56;
/// Solana packet limit (bytes) for a serialized transaction.
pub const PACKET_DATA_SIZE: usize = 1232;
/// R14 default: warn when the priority fee is above 0.01 SOL.
pub const PRIORITY_FEE_WARN_LAMPORTS: u64 = 10_000_000;

pub const MEMO_V1: &str = "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo";
pub const JUPITER_V6: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
pub const JUPITER_V4: &str = "JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB";
pub const JUPITER_V3: &str = "JUP3c2Uh3WA4Ng34tw6kPd2G4C5BB21Xo36Je1s32Ph";

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledIx {
    pub program_index: u8,
    pub accounts: Vec<u8>,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupRef {
    pub table: String,
    pub writable: Vec<u8>,
    pub readonly: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedMessage {
    /// `None` = legacy, `Some(0)` = v0.
    pub version: Option<u8>,
    pub num_required_signatures: u8,
    pub num_readonly_signed: u8,
    pub num_readonly_unsigned: u8,
    pub static_keys: Vec<String>,
    pub recent_blockhash: String,
    pub instructions: Vec<CompiledIx>,
    pub lookups: Vec<LookupRef>,
}

impl ParsedMessage {
    pub fn loaded_count(&self) -> usize {
        self.lookups
            .iter()
            .map(|l| l.writable.len() + l.readonly.len())
            .sum()
    }
}

fn take<'a>(b: &'a [u8], off: &mut usize, n: usize, what: &str) -> Result<&'a [u8], String> {
    if *off + n > b.len() {
        return Err(format!("{what} truncated"));
    }
    let s = &b[*off..*off + n];
    *off += n;
    Ok(s)
}

fn cu16(b: &[u8], off: &mut usize) -> Result<usize, String> {
    tx::compact_u16_decode(b, off)
        .map(|v| v as usize)
        .map_err(|e| e.to_string())
}

fn key_at(b: &[u8], off: &mut usize, what: &str) -> Result<String, String> {
    let s = take(b, off, 32, what)?;
    let mut k = [0u8; 32];
    k.copy_from_slice(s);
    Ok(tx::encode_pubkey(&k))
}

/// Strict parse of a legacy or v0 message. Rejects unknown versions, trailing
/// bytes, duplicate static keys, bad headers and program ids outside the static keys.
pub fn parse_message(m: &[u8]) -> Result<ParsedMessage, String> {
    if m.is_empty() {
        return Err("empty message".into());
    }
    if m.len() > PACKET_DATA_SIZE {
        return Err("message larger than a Solana packet".into());
    }
    let mut off = 0usize;
    let version = if m[0] & 0x80 != 0 {
        let v = m[0] & 0x7f;
        if v != 0 {
            return Err(format!("unknown transaction version {v}"));
        }
        off = 1;
        Some(0u8)
    } else {
        None
    };
    let hdr = take(m, &mut off, 3, "header")?;
    let (n_req, n_ro_s, n_ro_u) = (hdr[0], hdr[1], hdr[2]);
    let n_keys = cu16(m, &mut off)?;
    let mut static_keys = Vec::with_capacity(n_keys);
    let mut seen = HashSet::new();
    for _ in 0..n_keys {
        let k = key_at(m, &mut off, "account keys")?;
        if !seen.insert(k.clone()) {
            return Err(format!("duplicate account key {k}"));
        }
        static_keys.push(k);
    }
    if n_req == 0 || n_req as usize > n_keys {
        return Err("bad header: signer count".into());
    }
    if n_ro_s >= n_req || n_ro_u as usize > n_keys - n_req as usize {
        return Err("bad header: readonly counts".into());
    }
    let recent_blockhash = key_at(m, &mut off, "blockhash")?;
    let n_ix = cu16(m, &mut off)?;
    let mut instructions = Vec::with_capacity(n_ix);
    for _ in 0..n_ix {
        let program_index = take(m, &mut off, 1, "instruction")?[0];
        if program_index as usize >= n_keys {
            // Programs can't be loaded from a lookup table.
            return Err("instruction program id is not a static account key".into());
        }
        let n_acc = cu16(m, &mut off)?;
        let accounts = take(m, &mut off, n_acc, "instruction accounts")?.to_vec();
        let n_data = cu16(m, &mut off)?;
        let data = take(m, &mut off, n_data, "instruction data")?.to_vec();
        instructions.push(CompiledIx {
            program_index,
            accounts,
            data,
        });
    }
    let mut lookups = Vec::new();
    if version.is_some() {
        let n_lu = cu16(m, &mut off)?;
        for _ in 0..n_lu {
            let table = key_at(m, &mut off, "lookup table key")?;
            let nw = cu16(m, &mut off)?;
            let writable = take(m, &mut off, nw, "lookup writable indexes")?.to_vec();
            let nr = cu16(m, &mut off)?;
            let readonly = take(m, &mut off, nr, "lookup readonly indexes")?.to_vec();
            if writable.is_empty() && readonly.is_empty() {
                return Err(format!("lookup table {table} has no indexes"));
            }
            lookups.push(LookupRef {
                table,
                writable,
                readonly,
            });
        }
    }
    if off != m.len() {
        return Err(format!("{} trailing bytes after the message", m.len() - off));
    }
    let parsed = ParsedMessage {
        version,
        num_required_signatures: n_req,
        num_readonly_signed: n_ro_s,
        num_readonly_unsigned: n_ro_u,
        static_keys,
        recent_blockhash,
        instructions,
        lookups,
    };
    let total = n_keys + parsed.loaded_count();
    if total > 256 {
        return Err("more than 256 accounts".into());
    }
    for ix in &parsed.instructions {
        if let Some(bad) = ix.accounts.iter().find(|&&i| i as usize >= total) {
            return Err(format!("instruction account index {bad} out of range"));
        }
    }
    Ok(parsed)
}

/// Split a wire transaction into (signature count, message bytes).
pub fn split_wire(txb: &[u8]) -> Result<(usize, &[u8]), String> {
    if txb.len() > PACKET_DATA_SIZE {
        return Err("transaction larger than a Solana packet".into());
    }
    let mut off = 0;
    let n = cu16(txb, &mut off)?;
    let start = off + n * 64;
    if n == 0 || start >= txb.len() {
        return Err("transaction truncated".into());
    }
    Ok((n, &txb[start..]))
}

// ---------------------------------------------------------------------------
// Address Lookup Tables
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AltTable {
    pub key: String,
    pub deactivation_slot: u64,
    pub last_extended_slot: u64,
    pub authority: Option<String>,
    pub addresses: Vec<String>,
}

impl AltTable {
    pub fn is_active(&self) -> bool {
        self.deactivation_slot == u64::MAX
    }
}

/// Parse an ALT account (owner must be the ALT program).
pub fn parse_alt_account(key: &str, owner: &str, data: &[u8]) -> Result<AltTable, String> {
    let short = crate::format::shorten_address(key);
    if owner != ProgramIds::ADDRESS_LOOKUP_TABLE {
        return Err(format!("lookup table {short} is not owned by the lookup-table program"));
    }
    if data.len() < ALT_META_SIZE {
        return Err(format!("lookup table {short} is too short"));
    }
    let typ = u32::from_le_bytes(data[0..4].try_into().unwrap());
    if typ != 1 {
        return Err(format!("lookup table {short} is not initialized"));
    }
    let deactivation_slot = u64::from_le_bytes(data[4..12].try_into().unwrap());
    let last_extended_slot = u64::from_le_bytes(data[12..20].try_into().unwrap());
    let authority = match data[21] {
        0 => None,
        1 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(&data[22..54]);
            Some(tx::encode_pubkey(&k))
        }
        _ => return Err(format!("lookup table {short} has a bad header")),
    };
    let body = &data[ALT_META_SIZE..];
    if body.len() % 32 != 0 {
        return Err(format!("lookup table {short} has a partial address"));
    }
    let addresses = body
        .chunks_exact(32)
        .map(|c| {
            let mut k = [0u8; 32];
            k.copy_from_slice(c);
            tx::encode_pubkey(&k)
        })
        .collect();
    Ok(AltTable {
        key: key.to_string(),
        deactivation_slot,
        last_extended_slot,
        authority,
        addresses,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountSource {
    Static,
    Lookup { table: String, index: u8 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedAccount {
    pub pubkey: String,
    pub signer: bool,
    pub writable: bool,
    pub source: AccountSource,
}

#[derive(Clone, Debug)]
pub struct ResolvedMessage {
    pub parsed: ParsedMessage,
    pub accounts: Vec<ResolvedAccount>,
    pub instructions: Vec<Instruction>,
}

impl ResolvedMessage {
    pub fn fee_payer(&self) -> &str {
        &self.accounts[0].pubkey
    }
    pub fn from_lookup_count(&self) -> usize {
        self.accounts
            .iter()
            .filter(|a| matches!(a.source, AccountSource::Lookup { .. }))
            .count()
    }
}

/// Account fetcher: `Ok(Some((owner, lamports, data)))`, `Ok(None)` when missing.
pub type Fetched = Result<Option<(String, u64, Vec<u8>)>, String>;

/// Resolve every lookup-table account. Any failure is fatal (R15).
pub fn resolve<F>(parsed: &ParsedMessage, mut fetch: F) -> Result<ResolvedMessage, String>
where
    F: FnMut(&str) -> Fetched,
{
    let n_keys = parsed.static_keys.len();
    let n_req = parsed.num_required_signatures as usize;
    let mut accounts = Vec::with_capacity(n_keys + parsed.loaded_count());
    for (i, k) in parsed.static_keys.iter().enumerate() {
        let signer = i < n_req;
        let writable = if signer {
            i < n_req - parsed.num_readonly_signed as usize
        } else {
            i < n_keys - parsed.num_readonly_unsigned as usize
        };
        accounts.push(ResolvedAccount {
            pubkey: k.clone(),
            signer,
            writable,
            source: AccountSource::Static,
        });
    }
    let mut tables: BTreeMap<String, AltTable> = BTreeMap::new();
    for lu in &parsed.lookups {
        if tables.contains_key(&lu.table) {
            continue;
        }
        let short = crate::format::shorten_address(&lu.table);
        let acct = fetch(&lu.table)
            .map_err(|e| format!("lookup table {short} could not be fetched: {e}"))?
            .ok_or_else(|| format!("lookup table {short} does not exist"))?;
        let table = parse_alt_account(&lu.table, &acct.0, &acct.2)?;
        if !table.is_active() {
            return Err(format!("lookup table {short} is deactivated"));
        }
        tables.insert(lu.table.clone(), table);
    }
    let loaded = |writable: bool, accounts: &mut Vec<ResolvedAccount>| -> Result<(), String> {
        for lu in &parsed.lookups {
            let table = &tables[&lu.table];
            let idxs = if writable { &lu.writable } else { &lu.readonly };
            for &ix in idxs {
                let addr = table.addresses.get(ix as usize).ok_or_else(|| {
                    format!(
                        "lookup table {} has no entry {ix} (it has {})",
                        crate::format::shorten_address(&lu.table),
                        table.addresses.len()
                    )
                })?;
                accounts.push(ResolvedAccount {
                    pubkey: addr.clone(),
                    signer: false,
                    writable,
                    source: AccountSource::Lookup {
                        table: lu.table.clone(),
                        index: ix,
                    },
                });
            }
        }
        Ok(())
    };
    loaded(true, &mut accounts)?;
    loaded(false, &mut accounts)?;
    let instructions = parsed
        .instructions
        .iter()
        .map(|ix| Instruction {
            program_id: accounts[ix.program_index as usize].pubkey.clone(),
            accounts: ix
                .accounts
                .iter()
                .map(|&i| {
                    let a = &accounts[i as usize];
                    AccountMeta {
                        pubkey: a.pubkey.clone(),
                        is_signer: a.signer,
                        is_writable: a.writable,
                    }
                })
                .collect(),
            data: ix.data.clone(),
        })
        .collect();
    Ok(ResolvedMessage {
        parsed: parsed.clone(),
        accounts,
        instructions,
    })
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warn,
    Block,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub level: Level,
    pub text: String,
}

fn f(rule: &'static str, level: Level, text: impl Into<String>) -> Finding {
    Finding {
        rule,
        level,
        text: text.into(),
    }
}

/// Inputs the rules need besides the bytes. Built by the vault, never by the desk.
pub struct RuleCtx<'a> {
    /// The signing wallet address.
    pub user: &'a str,
    /// Request came from a dApp / extension (not a native vault flow).
    pub from_dapp: bool,
    /// `max_sol_per_tx` from the vault policy (0 = off).
    pub max_sol_per_tx: f64,
    /// `block_infinite_approve` from the vault policy.
    pub block_infinite_approve: bool,
    /// Contact addresses (for the look-alike rule R16).
    pub contacts: &'a [String],
    /// R14 threshold.
    pub priority_fee_warn_lamports: u64,
    /// R14b hard cap (BLOCK above this). 0 = use `fees::MAX_PRIORITY_FEE_LAMPORTS`.
    pub priority_fee_max_lamports: u64,
}

pub fn known_program(id: &str) -> bool {
    matches!(
        id,
        ProgramIds::SYSTEM
            | ProgramIds::COMPUTE_BUDGET
            | ProgramIds::TOKEN
            | ProgramIds::TOKEN_2022
            | ProgramIds::ASSOCIATED_TOKEN
            | ProgramIds::MEMO
            | MEMO_V1
            | ProgramIds::STAKE
            | ProgramIds::ADDRESS_LOOKUP_TABLE
            | ProgramIds::BPF_UPGRADEABLE_LOADER
            | JUPITER_V6
            | JUPITER_V4
            | JUPITER_V3
    )
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}
fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    d.get(at..at + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}
fn key_in(d: &[u8], at: usize) -> Option<String> {
    d.get(at..at + 32).map(|s| {
        let mut k = [0u8; 32];
        k.copy_from_slice(s);
        tx::encode_pubkey(&k)
    })
}
fn acct(ix: &Instruction, i: usize) -> &str {
    ix.accounts.get(i).map(|a| a.pubkey.as_str()).unwrap_or("")
}
fn short(a: &str) -> String {
    crate::format::shorten_address(a)
}

/// Same first 4 and last 4 characters, different key (port of drain.py::lookalike_of).
pub fn lookalike_of<'a>(addr: &str, contacts: &'a [String]) -> Option<&'a str> {
    if addr.len() < 8 {
        return None;
    }
    contacts.iter().map(|c| c.as_str()).find(|c| {
        c.len() >= 8 && *c != addr && c[..4] == addr[..4] && c[c.len() - 4..] == addr[addr.len() - 4..]
    })
}

/// Static rules on a fully resolved message. `fetch` is used for R2a (token
/// balance of an approved account); a fetch failure there falls back to WARN.
pub fn evaluate<F>(msg: &ResolvedMessage, ctx: &RuleCtx, mut fetch: F) -> Vec<Finding>
where
    F: FnMut(&str) -> Fetched,
{
    let user = ctx.user;
    let mut out = Vec::new();
    if !msg
        .accounts
        .iter()
        .any(|a| a.signer && a.pubkey == user)
    {
        out.push(f("SIGNER", Level::Block, "This wallet is not a required signer on this transaction."));
    }
    if msg.fee_payer() != user {
        out.push(f("FEE", Level::Info, format!("Fee payer is {}, not this wallet.", short(msg.fee_payer()))));
    }
    let others = msg.accounts.iter().filter(|a| a.signer && a.pubkey != user).count();
    if others > 0 {
        out.push(f("SIGNERS", Level::Info, format!("Other signers: {others}.")));
    }
    let mut sol_out: u128 = 0;
    let mut cu_limit: Option<u32> = None;
    let mut cu_price: u64 = 0;
    let mut non_budget = 0u32;
    let mut unknown: Vec<String> = Vec::new();
    for (n, ix) in msg.instructions.iter().enumerate() {
        let d = &ix.data;
        let pid = ix.program_id.as_str();
        if pid != ProgramIds::COMPUTE_BUDGET {
            non_budget += 1;
        }
        match pid {
            ProgramIds::SYSTEM => {
                let op = u32_at(d, 0).unwrap_or(u32::MAX);
                match op {
                    0 if acct(ix, 0) == user => sol_out += u64_at(d, 4).unwrap_or(0) as u128,
                    2 if acct(ix, 0) == user => {
                        sol_out += u64_at(d, 4).unwrap_or(0) as u128;
                        if let Some(c) = lookalike_of(acct(ix, 1), ctx.contacts) {
                            out.push(f("R16", Level::Warn, format!(
                                "Recipient {} looks like your contact {} but is a different address.",
                                acct(ix, 1), c)));
                        }
                    }
                    1 | 8 if acct(ix, 0) == user => out.push(f("R4", Level::Block,
                        "Changes the owner or size of your wallet account (System Assign/Allocate). Drainers use this to take over a wallet.")),
                    9 | 10 if acct(ix, 0) == user || acct(ix, 1) == user => out.push(f("R4", Level::Block,
                        "Assign/Allocate with seed on your wallet account.")),
                    4 if n == 0 && ctx.from_dapp => out.push(f("R13", Level::Warn,
                        "Durable nonce: this signed transaction never expires and can be submitted later.")),
                    5 if acct(ix, 4) == user && acct(ix, 1) != user => out.push(f("R5", Level::Block,
                        format!("Withdraws your nonce account to {}.", acct(ix, 1)))),
                    7 if acct(ix, 1) == user => {
                        let new = key_in(d, 4).unwrap_or_default();
                        if new != user {
                            out.push(f("R5", Level::Block, format!("Hands your nonce account authority to {new}.")));
                        }
                    }
                    _ => {}
                }
            }
            ProgramIds::TOKEN | ProgramIds::TOKEN_2022 => {
                let op = d.first().copied().unwrap_or(255);
                match op {
                    // SetAuthority: [account, current authority]; data: type u8, COption<Pubkey>
                    6 if acct(ix, 1) == user => {
                        let kind = d.get(1).copied().unwrap_or(255);
                        let new = if d.get(2) == Some(&1) { key_in(d, 3) } else { None };
                        // 2 = AccountOwner, 3 = CloseAccount
                        if (kind == 2 || kind == 3) && new.as_deref() != Some(user) {
                            out.push(f("R1", Level::Block, format!(
                                "Hands {} of token account {} to {}.",
                                if kind == 2 { "ownership" } else { "close authority" },
                                short(acct(ix, 0)),
                                new.unwrap_or_else(|| "nobody".into()))));
                        }
                    }
                    4 | 13 => {
                        let (src, delegate, owner) = if op == 4 {
                            (acct(ix, 0), acct(ix, 1), acct(ix, 2))
                        } else {
                            (acct(ix, 0), acct(ix, 2), acct(ix, 3))
                        };
                        if owner == user {
                            let amount = u64_at(d, 1).unwrap_or(0);
                            let balance = match fetch(src) {
                                Ok(Some((_, _, data))) => u64_at(&data, 64),
                                _ => None,
                            };
                            let unlimited = amount == u64::MAX || balance.map(|b| amount >= b && b > 0).unwrap_or(false);
                            if unlimited && ctx.block_infinite_approve {
                                out.push(f("R2a", Level::Block, format!(
                                    "Approves {} to spend all tokens in {} (amount {}).",
                                    delegate, short(src),
                                    if amount == u64::MAX { "unlimited".to_string() } else { amount.to_string() })));
                            } else if delegate != user {
                                out.push(f("R2b", Level::Warn, format!(
                                    "Approves {} to spend up to {} raw units from {}.", delegate, amount, short(src))));
                            }
                        }
                    }
                    9 if acct(ix, 2) == user && acct(ix, 1) != user => out.push(f("R3", Level::Block,
                        format!("Closes token account {} and sends its SOL to {}.", short(acct(ix, 0)), acct(ix, 1)))),
                    3 | 12 => {
                        let dest = if op == 3 { acct(ix, 1) } else { acct(ix, 2) };
                        if let Some(c) = lookalike_of(dest, ctx.contacts) {
                            out.push(f("R16", Level::Warn, format!(
                                "Token destination {dest} looks like your contact {c} but is a different address.")));
                        }
                    }
                    _ => {}
                }
            }
            ProgramIds::STAKE => {
                let op = u32_at(d, 0).unwrap_or(u32::MAX);
                match op {
                    // Authorize: [stake, clock, authority, (custodian)], data new pubkey 4..36
                    1 if acct(ix, 2) == user => {
                        let new = key_in(d, 4).unwrap_or_default();
                        if new != user {
                            out.push(f("R5", Level::Block, format!("Hands stake authority to {new}.")));
                        }
                    }
                    // AuthorizeWithSeed: [stake, base, clock, ...], data new pubkey 4..36
                    8 if acct(ix, 1) == user => {
                        let new = key_in(d, 4).unwrap_or_default();
                        if new != user {
                            out.push(f("R5", Level::Block, format!("Hands stake authority (with seed) to {new}.")));
                        }
                    }
                    // AuthorizeChecked / AuthorizeCheckedWithSeed: new authority = account 3
                    10 | 11 if (acct(ix, 2) == user || acct(ix, 1) == user) && acct(ix, 3) != user => out.push(f("R5", Level::Block,
                        format!("Hands stake authority to {}.", acct(ix, 3)))),
                    // Withdraw: [stake, to, clock, history, withdraw authority]
                    4 if acct(ix, 4) == user && acct(ix, 1) != user => out.push(f("R5", Level::Block,
                        format!("Withdraws stake to {}.", acct(ix, 1)))),
                    // 1.9 F7: SetLockup [stake, authority] / SetLockupChecked [stake, authority, (new custodian)]
                    // signed by this wallet can freeze the stake until a date or epoch.
                    6 | 12 if acct(ix, 1) == user => out.push(f("R5b", Level::Warn,
                        format!("Sets a lockup on stake account {}: until it ends, only the lockup custodian can release it.",
                            short(acct(ix, 0))))),
                    _ => {}
                }
            }
            ProgramIds::BPF_UPGRADEABLE_LOADER => {
                let op = u32_at(d, 0).unwrap_or(u32::MAX);
                // SetAuthority (4): [account, current, new?]; SetAuthorityChecked (7): [account, current, new]
                if (op == 4 || op == 7) && acct(ix, 1) == user && acct(ix, 2) != user {
                    let new = acct(ix, 2);
                    out.push(f("R18", Level::Block, format!(
                        "Hands the upgrade authority of {} to {}.",
                        short(acct(ix, 0)),
                        if new.is_empty() { "nobody (makes it immutable)" } else { new })));
                }
            }
            ProgramIds::COMPUTE_BUDGET => match d.first() {
                Some(2) => cu_limit = u32_at(d, 1),
                Some(3) => cu_price = u64_at(d, 1).unwrap_or(0),
                _ => {}
            },
            other if !known_program(other) => {
                if !unknown.iter().any(|u| u == other) {
                    unknown.push(other.to_string());
                }
            }
            _ => {}
        }
    }
    for p in &unknown {
        let touches: Vec<String> = msg
            .instructions
            .iter()
            .filter(|ix| &ix.program_id == p)
            .flat_map(|ix| ix.accounts.iter())
            .filter(|a| a.is_writable)
            .map(|a| short(&a.pubkey))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        out.push(f("R6", Level::Warn, format!(
            "Unknown program {p}: VIBE can't tell what this does. Writable accounts: {}.",
            if touches.is_empty() { "none".into() } else { touches.join(", ") })));
    }
    // R7 (SOL part): outflow from the wallet above max_sol_per_tx.
    let sol = sol_out as f64 / crate::LAMPORTS_PER_SOL as f64;
    if ctx.max_sol_per_tx > 0.0 && sol > ctx.max_sol_per_tx + 1e-9 {
        out.push(f("R7", Level::Block, format!(
            "Vault rule: max {} SOL per tx (this sends {sol:.4} SOL).", ctx.max_sol_per_tx)));
    }
    // R14: priority fee = limit × price / 1e6.
    // Same math as `fees::priority_fee_lamports` (runtime clamps the limit to 1.4M, rounds up).
    let limit = cu_limit.unwrap_or_else(|| non_budget.saturating_mul(crate::fees::DEFAULT_UNITS_PER_IX));
    let prio = crate::fees::priority_fee_lamports(limit, cu_price);
    let max_fee = if ctx.priority_fee_max_lamports == 0 {
        crate::fees::MAX_PRIORITY_FEE_LAMPORTS
    } else {
        ctx.priority_fee_max_lamports.min(crate::fees::MAX_PRIORITY_FEE_LAMPORTS)
    };
    if prio > max_fee {
        out.push(f("R14b", Level::Block, format!(
            "Priority fee is {:.4} SOL ({} µ-lamports/CU). The vault maximum is {:.4} SOL.",
            prio as f64 / crate::LAMPORTS_PER_SOL as f64,
            cu_price,
            max_fee as f64 / crate::LAMPORTS_PER_SOL as f64)));
    } else if prio > ctx.priority_fee_warn_lamports {
        out.push(f("R14", Level::Warn, format!(
            "Priority fee is {:.4} SOL.", prio as f64 / crate::LAMPORTS_PER_SOL as f64)));
    }
    if ctx.from_dapp {
        out.push(f("PENDING", Level::Info,
            "Not checked yet by the vault: simulated balance changes (R8/R9), Token-2022 mint extensions (R12), first-payee USD delay (R17), Jupiter destination owner (R19)."));
    }
    out
}

// ---------------------------------------------------------------------------
// Review (parse → resolve → rules), what the Confirm screen shows
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Review {
    pub findings: Vec<Finding>,
    /// One line per account: index, address, flags, lookup source.
    pub account_lines: Vec<String>,
    /// One line per instruction (vault decode).
    pub instruction_lines: Vec<String>,
    pub version: String,
    pub lookup_accounts: usize,
}

impl Review {
    pub fn blocked(&self) -> bool {
        self.findings.iter().any(|x| x.level == Level::Block)
    }
    pub fn needs_ack(&self) -> bool {
        self.findings.iter().any(|x| x.level == Level::Warn)
    }
    pub fn first_block(&self) -> Option<&Finding> {
        self.findings.iter().find(|x| x.level == Level::Block)
    }
}

pub fn account_line(i: usize, a: &ResolvedAccount) -> String {
    let mut flags = Vec::new();
    if a.signer {
        flags.push("signer");
    }
    flags.push(if a.writable { "writable" } else { "readonly" });
    let src = match &a.source {
        AccountSource::Static => String::new(),
        AccountSource::Lookup { table, index } => {
            format!(" · via lookup table {} #{index}", short(table))
        }
    };
    format!("#{i} {} [{}]{src}", a.pubkey, flags.join(", "))
}

/// Review message bytes (not a wire transaction).
pub fn review_message<F>(message: &[u8], ctx: &RuleCtx, mut fetch: F) -> Review
where
    F: FnMut(&str) -> Fetched,
{
    let mut r = Review::default();
    let parsed = match parse_message(message) {
        Ok(p) => p,
        Err(e) => {
            r.findings.push(f("R15", Level::Block, format!("VIBE can't read this transaction: {e}.")));
            return r;
        }
    };
    r.version = match parsed.version {
        None => "legacy".into(),
        Some(v) => format!("v{v}"),
    };
    let resolved = match resolve(&parsed, &mut fetch) {
        Ok(m) => m,
        Err(e) => {
            r.findings.push(f("R15", Level::Block, format!(
                "VIBE can't show what this transaction touches: {e}. Signing is blocked.")));
            // Still list what is known, with the unresolved entries marked.
            for (i, k) in parsed.static_keys.iter().enumerate() {
                r.account_lines.push(format!("#{i} {k} [static]"));
            }
            for lu in &parsed.lookups {
                r.account_lines.push(format!(
                    "{} writable + {} readonly account(s) from lookup table {} — UNRESOLVED",
                    lu.writable.len(), lu.readonly.len(), lu.table));
            }
            return r;
        }
    };
    r.lookup_accounts = resolved.from_lookup_count();
    r.account_lines = resolved
        .accounts
        .iter()
        .enumerate()
        .map(|(i, a)| account_line(i, a))
        .collect();
    r.instruction_lines = crate::decode::summary_lines(
        &resolved
            .instructions
            .iter()
            .map(crate::decode::decode_instruction)
            .collect::<Vec<_>>(),
    );
    r.findings = evaluate(&resolved, ctx, &mut fetch);
    r
}

/// Review a serialized (wire) transaction.
pub fn review_wire<F>(txb: &[u8], ctx: &RuleCtx, fetch: F) -> Review
where
    F: FnMut(&str) -> Fetched,
{
    match split_wire(txb) {
        Ok((_, m)) => review_message(m, ctx, fetch),
        Err(e) => Review {
            findings: vec![f("R15", Level::Block, format!("VIBE can't read this transaction: {e}."))],
            ..Default::default()
        },
    }
}

/// R10: review a batch; one BLOCK blocks every transaction in it.
pub fn review_batch<F>(txs: &[Vec<u8>], ctx: &RuleCtx, mut fetch: F) -> Vec<Review>
where
    F: FnMut(&str) -> Fetched,
{
    let mut out: Vec<Review> = txs.iter().map(|t| review_wire(t, ctx, &mut fetch)).collect();
    if let Some(bad) = out.iter().position(|r| r.blocked()) {
        for (i, r) in out.iter_mut().enumerate() {
            if i != bad && !r.blocked() {
                r.findings.push(f("R10", Level::Block, format!(
                    "Transaction {} in this batch is blocked, so the whole batch is blocked.", bad + 1)));
            }
        }
    }
    out
}

/// R11: a `signMessage` payload that parses as a transaction (message or wire) is blocked.
pub fn message_payload_is_transaction(bytes: &[u8]) -> bool {
    parse_message(bytes).is_ok()
        || split_wire(bytes).map(|(_, m)| parse_message(m).is_ok()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keypair::Keypair;

    fn pk(s: &str) -> [u8; 32] {
        tx::decode_pubkey(s).unwrap()
    }
    fn rnd() -> [u8; 32] {
        Keypair::generate().public_bytes()
    }
    fn b58(k: &[u8; 32]) -> String {
        tx::encode_pubkey(k)
    }

    type Ix = (u8, Vec<u8>, Vec<u8>);
    type Lu = ([u8; 32], Vec<u8>, Vec<u8>);

    fn msg(version: Option<u8>, hdr: [u8; 3], keys: &[[u8; 32]], ixs: &[Ix], lus: &[Lu]) -> Vec<u8> {
        let mut m = Vec::new();
        if let Some(v) = version {
            m.push(0x80 | v);
        }
        m.extend_from_slice(&hdr);
        tx::compact_u16_encode(keys.len() as u16, &mut m);
        for k in keys {
            m.extend_from_slice(k);
        }
        m.extend_from_slice(&rnd()); // blockhash
        tx::compact_u16_encode(ixs.len() as u16, &mut m);
        for (p, a, d) in ixs {
            m.push(*p);
            tx::compact_u16_encode(a.len() as u16, &mut m);
            m.extend_from_slice(a);
            tx::compact_u16_encode(d.len() as u16, &mut m);
            m.extend_from_slice(d);
        }
        if version.is_some() {
            tx::compact_u16_encode(lus.len() as u16, &mut m);
            for (t, w, r) in lus {
                m.extend_from_slice(t);
                tx::compact_u16_encode(w.len() as u16, &mut m);
                m.extend_from_slice(w);
                tx::compact_u16_encode(r.len() as u16, &mut m);
                m.extend_from_slice(r);
            }
        }
        m
    }

    fn alt_data(addrs: &[[u8; 32]], deactivation: u64) -> Vec<u8> {
        let mut d = vec![0u8; ALT_META_SIZE];
        d[0..4].copy_from_slice(&1u32.to_le_bytes());
        d[4..12].copy_from_slice(&deactivation.to_le_bytes());
        d[12..20].copy_from_slice(&5u64.to_le_bytes());
        d[21] = 1;
        d[22..54].copy_from_slice(&rnd());
        for a in addrs {
            d.extend_from_slice(a);
        }
        d
    }

    fn ctx<'a>(user: &'a str, contacts: &'a [String]) -> RuleCtx<'a> {
        RuleCtx {
            user,
            from_dapp: true,
            max_sol_per_tx: 0.0,
            block_infinite_approve: true,
            contacts,
            priority_fee_warn_lamports: PRIORITY_FEE_WARN_LAMPORTS,
            priority_fee_max_lamports: 0,
        }
    }

    fn table_fetch(table: [u8; 32], data: Vec<u8>) -> impl FnMut(&str) -> Fetched {
        let key = b58(&table);
        move |k: &str| {
            if k == key {
                Ok(Some((ProgramIds::ADDRESS_LOOKUP_TABLE.to_string(), 1, data.clone())))
            } else {
                Ok(None)
            }
        }
    }

    fn rules(r: &Review) -> Vec<&'static str> {
        r.findings.iter().map(|x| x.rule).collect()
    }

    /// v0 SetAuthority(AccountOwner) where the victim token account AND the token
    /// program's target come from a lookup table. The old decoder dropped the
    /// table account, so the Confirm screen showed an incomplete list.
    fn drainer_v0() -> (Vec<u8>, [u8; 32], Vec<u8>, String, String) {
        let user = rnd();
        let token = pk(ProgramIds::TOKEN);
        let victim_ata = rnd();
        let attacker = rnd();
        let filler = rnd();
        let table = rnd();
        let mut data = vec![6u8, 2, 1];
        data.extend_from_slice(&attacker);
        // keys: 0 user (writable signer), 1 token program (readonly unsigned)
        // loaded: #2 = table[1] (writable) victim ATA
        let m = msg(
            Some(0),
            [1, 0, 1],
            &[user, token],
            &[(1, vec![2, 0], data)],
            &[(table, vec![1], vec![])],
        );
        (m, table, alt_data(&[filler, victim_ata], u64::MAX), b58(&user), b58(&victim_ata))
    }

    #[test]
    fn v0_alt_accounts_are_listed_and_flagged() {
        let (m, table, data, user, victim) = drainer_v0();
        let parsed = parse_message(&m).unwrap();
        let res = resolve(&parsed, table_fetch(table, data)).unwrap();
        assert_eq!(res.accounts.len(), 3);
        assert_eq!(res.accounts[2].pubkey, victim);
        assert!(res.accounts[2].writable);
        assert_eq!(
            res.accounts[2].source,
            AccountSource::Lookup { table: b58(&table), index: 1 }
        );
        assert_eq!(res.instructions[0].accounts[0].pubkey, victim);
        assert_eq!(res.instructions[0].accounts[1].pubkey, user);
    }

    #[test]
    fn rules_apply_to_lookup_accounts() {
        let (m, table, data, user, victim) = drainer_v0();
        let r = review_message(&m, &ctx(&user, &[]), table_fetch(table, data));
        assert!(r.blocked(), "{:?}", r.findings);
        assert!(rules(&r).contains(&"R1"));
        assert!(r.account_lines.iter().any(|l| l.contains(&victim) && l.contains("via lookup table") && l.contains("writable")));
    }

    #[test]
    fn unresolvable_table_blocks_rpc_failure() {
        let (m, _t, _d, user, _v) = drainer_v0();
        let r = review_message(&m, &ctx(&user, &[]), |_| Err("connection refused".into()));
        assert!(r.blocked());
        assert_eq!(r.first_block().unwrap().rule, "R15");
        assert!(r.first_block().unwrap().text.contains("could not be fetched"));
        assert!(r.account_lines.iter().any(|l| l.contains("UNRESOLVED")));
    }

    #[test]
    fn unresolvable_table_blocks_missing_account() {
        let (m, _t, _d, user, _v) = drainer_v0();
        let r = review_message(&m, &ctx(&user, &[]), |_| Ok(None));
        assert_eq!(r.first_block().unwrap().rule, "R15");
        assert!(r.first_block().unwrap().text.contains("does not exist"));
    }

    #[test]
    fn deactivated_table_blocks() {
        let (m, table, _d, user, _v) = drainer_v0();
        let data = alt_data(&[rnd(), rnd()], 12345);
        let r = review_message(&m, &ctx(&user, &[]), table_fetch(table, data));
        assert_eq!(r.first_block().unwrap().rule, "R15");
        assert!(r.first_block().unwrap().text.contains("deactivated"));
    }

    #[test]
    fn index_out_of_range_blocks() {
        let (m, table, _d, user, _v) = drainer_v0();
        let data = alt_data(&[rnd()], u64::MAX); // only entry 0, tx wants entry 1
        let r = review_message(&m, &ctx(&user, &[]), table_fetch(table, data));
        assert_eq!(r.first_block().unwrap().rule, "R15");
        assert!(r.first_block().unwrap().text.contains("no entry 1"));
    }

    #[test]
    fn wrong_owner_blocks() {
        let (m, _table, data, user, _v) = drainer_v0();
        let r = review_message(&m, &ctx(&user, &[]), move |_| {
            Ok(Some((ProgramIds::SYSTEM.to_string(), 1, data.clone())))
        });
        assert_eq!(r.first_block().unwrap().rule, "R15");
        assert!(r.first_block().unwrap().text.contains("not owned"));
    }

    #[test]
    fn offline_decode_keeps_lookup_placeholders() {
        let (m, _t, _d, _u, _v) = drainer_v0();
        let ixs = crate::decode::decode_compiled_message(&m).unwrap();
        assert_eq!(ixs[0].title, "Unresolved lookup table");
        assert_eq!(ixs[1].title, "Set authority");
        // The lookup-table account is still there as an explicit placeholder.
        assert!(ixs[1].detail.starts_with("<unr"), "{}", ixs[1].detail);
    }

    #[test]
    fn strict_parse_rejects_bad_messages() {
        let (m, _t, _d, _u, _v) = drainer_v0();
        let mut bad = m.clone();
        bad[0] = 0x81;
        assert!(parse_message(&bad).unwrap_err().contains("unknown transaction version"));
        let mut trailing = m.clone();
        trailing.push(0);
        assert!(parse_message(&trailing).unwrap_err().contains("trailing"));
        // Program id taken from a lookup table (index 2) is rejected.
        let user = rnd();
        let p = msg(Some(0), [1, 0, 0], &[user], &[(2, vec![0], vec![])], &[(rnd(), vec![0], vec![])]);
        assert!(parse_message(&p).is_err());
        // Account index past the resolved range.
        let q = msg(Some(0), [1, 0, 0], &[user, pk(ProgramIds::SYSTEM)], &[(1, vec![9], vec![])], &[(rnd(), vec![0], vec![])]);
        assert!(parse_message(&q).unwrap_err().contains("out of range"));
        let r = review_message(&bad, &ctx(&b58(&user), &[]), |_| Ok(None));
        assert_eq!(r.first_block().unwrap().rule, "R15");
    }

    #[test]
    fn close_account_to_lookup_destination_blocks() {
        let user = rnd();
        let token = pk(ProgramIds::TOKEN);
        let ata = rnd();
        let thief = rnd();
        let table = rnd();
        // accounts: [ata(static writable), thief (loaded writable #3), user]
        let m = msg(Some(0), [1, 0, 1], &[user, ata, token], &[(2, vec![1, 3, 0], vec![9])], &[(table, vec![0], vec![])]);
        let r = review_message(&m, &ctx(&b58(&user), &[]), table_fetch(table, alt_data(&[thief], u64::MAX)));
        assert!(rules(&r).contains(&"R3"), "{:?}", r.findings);
    }

    #[test]
    fn sol_cap_counts_lookup_destinations() {
        let user = rnd();
        let dest = rnd();
        let table = rnd();
        let mut d = 2u32.to_le_bytes().to_vec();
        d.extend_from_slice(&(5 * crate::LAMPORTS_PER_SOL).to_le_bytes());
        let m = msg(Some(0), [1, 0, 1], &[user, pk(ProgramIds::SYSTEM)], &[(1, vec![0, 2], d)], &[(table, vec![0], vec![])]);
        let u = b58(&user);
        let mut c = ctx(&u, &[]);
        c.max_sol_per_tx = 1.0;
        let r = review_message(&m, &c, table_fetch(table, alt_data(&[dest], u64::MAX)));
        assert!(rules(&r).contains(&"R7"));
        assert!(r.instruction_lines[0].contains("Transfer SOL"));
    }

    #[test]
    fn lookalike_contact_warns() {
        let user = rnd();
        let contact = b58(&rnd());
        // Forge an address with the same first/last 4 chars (string-level check).
        let fake = format!("{}{}{}", &contact[..4], "1".repeat(contact.len() - 8), &contact[contact.len() - 4..]);
        let contacts = vec![fake.clone()];
        let mut d = 2u32.to_le_bytes().to_vec();
        d.extend_from_slice(&1000u64.to_le_bytes());
        let m = msg(None, [1, 0, 1], &[user, pk(&contact), pk(ProgramIds::SYSTEM)], &[(2, vec![0, 1], d)], &[]);
        let r = review_message(&m, &ctx(&b58(&user), &contacts), |_| Ok(None));
        if fake != contact {
            assert!(rules(&r).contains(&"R16"), "{:?}", r.findings);
        }
    }

    #[test]
    fn approve_assign_nonce_stake_bpf_rules() {
        let user = rnd();
        let u = b58(&user);
        let other = rnd();
        let src = rnd();
        let token = pk(ProgramIds::TOKEN);
        let system = pk(ProgramIds::SYSTEM);
        let stake = pk(ProgramIds::STAKE);
        let bpf = pk(ProgramIds::BPF_UPGRADEABLE_LOADER);
        // R2a unlimited approve: [src, delegate, owner]
        let mut ap = vec![4u8];
        ap.extend_from_slice(&u64::MAX.to_le_bytes());
        let m = msg(None, [1, 0, 1], &[user, src, other, token], &[(3, vec![1, 2, 0], ap)], &[]);
        assert!(rules(&review_message(&m, &ctx(&u, &[]), |_| Ok(None))).contains(&"R2a"));
        // R2a: amount >= fetched balance
        let mut ap2 = vec![4u8];
        ap2.extend_from_slice(&500u64.to_le_bytes());
        let m = msg(None, [1, 0, 1], &[user, src, other, token], &[(3, vec![1, 2, 0], ap2.clone())], &[]);
        let mut acct_data = vec![0u8; 165];
        acct_data[64..72].copy_from_slice(&400u64.to_le_bytes());
        let r = review_message(&m, &ctx(&u, &[]), move |_| Ok(Some((ProgramIds::TOKEN.into(), 1, acct_data.clone()))));
        assert!(rules(&r).contains(&"R2a"));
        // R2b: limited approve to a foreign delegate
        let r = review_message(&m, &ctx(&u, &[]), |_| Ok(None));
        assert!(rules(&r).contains(&"R2b") && !r.blocked());
        // R4 Assign on the wallet
        let mut asg = 1u32.to_le_bytes().to_vec();
        asg.extend_from_slice(&other);
        let m = msg(None, [1, 0, 1], &[user, system], &[(1, vec![0], asg)], &[]);
        assert!(rules(&review_message(&m, &ctx(&u, &[]), |_| Ok(None))).contains(&"R4"));
        // R5 AuthorizeNonceAccount to foreign: [nonce, authority]
        let nonce = rnd();
        let mut an = 7u32.to_le_bytes().to_vec();
        an.extend_from_slice(&other);
        let m = msg(None, [1, 0, 1], &[user, nonce, system], &[(2, vec![1, 0], an)], &[]);
        assert!(rules(&review_message(&m, &ctx(&u, &[]), |_| Ok(None))).contains(&"R5"));
        // R5 stake Authorize: [stake, clock, authority]
        let sa = rnd();
        let mut au = 1u32.to_le_bytes().to_vec();
        au.extend_from_slice(&other);
        au.extend_from_slice(&0u32.to_le_bytes());
        let m = msg(None, [1, 0, 2], &[user, sa, pk(ProgramIds::SYSVAR_CLOCK), stake], &[(3, vec![1, 2, 0], au)], &[]);
        assert!(rules(&review_message(&m, &ctx(&u, &[]), |_| Ok(None))).contains(&"R5"));
        // R18 BPF SetAuthority: [programdata, current, new]
        let pd = rnd();
        let m = msg(None, [1, 0, 2], &[user, pd, other, bpf], &[(3, vec![1, 0, 2], 4u32.to_le_bytes().to_vec())], &[]);
        assert!(rules(&review_message(&m, &ctx(&u, &[]), |_| Ok(None))).contains(&"R18"));
        // R13 durable nonce first + R14 priority fee + R6 unknown program
        let unknown = rnd();
        let mut lim = vec![2u8];
        lim.extend_from_slice(&1_400_000u32.to_le_bytes());
        let mut price = vec![3u8];
        // 1.4M CU × 10M µL = 0.014 SOL: over the R14 warn line, under the R14b cap.
        price.extend_from_slice(&10_000_000u64.to_le_bytes());
        let cb = pk(ProgramIds::COMPUTE_BUDGET);
        let m = msg(
            None,
            [1, 0, 3],
            &[user, nonce, system, cb, unknown],
            &[(2, vec![1, 0], 4u32.to_le_bytes().to_vec()), (3, vec![], lim), (3, vec![], price), (4, vec![1], vec![1, 2])],
            &[],
        );
        let r = review_message(&m, &ctx(&u, &[]), |_| Ok(None));
        let got = rules(&r);
        assert!(got.contains(&"R13") && got.contains(&"R14") && got.contains(&"R6"), "{got:?}");
        assert!(!r.blocked() && r.needs_ack());
    }

    #[test]
    fn r14b_max_fee_blocks() {
        let user = rnd();
        let u = b58(&user);
        let cb = pk(ProgramIds::COMPUTE_BUDGET);
        let fee_msg = |price_micro: u64| {
            let mut lim = vec![2u8];
            lim.extend_from_slice(&1_400_000u32.to_le_bytes());
            let mut price = vec![3u8];
            price.extend_from_slice(&price_micro.to_le_bytes());
            msg(None, [1, 0, 2], &[user, cb, pk(ProgramIds::MEMO)], &[(1, vec![], lim), (1, vec![], price), (2, vec![], b"hi".to_vec())], &[])
        };
        // 1.4M CU × 40M µL = 0.056 SOL, over the 0.05 SOL cap → BLOCK R14b.
        let r = review_message(&fee_msg(40_000_000), &ctx(&u, &[]), |_| Ok(None));
        assert!(r.blocked(), "{:?}", rules(&r));
        assert_eq!(r.first_block().unwrap().rule, "R14b");
        // 1.4M CU × 10M µL = 0.014 SOL → warn only.
        let r = review_message(&fee_msg(crate::fees::MAX_MICRO_LAMPORTS), &ctx(&u, &[]), |_| Ok(None));
        assert!(!r.blocked() && rules(&r).contains(&"R14"));
        // A lower per-wallet max (policy) is honoured; a higher one is clamped to the vault max.
        let mut c = ctx(&u, &[]);
        c.priority_fee_max_lamports = 1_000;
        let r = review_message(&fee_msg(25_000), &c, |_| Ok(None));
        assert_eq!(r.first_block().map(|f| f.rule), Some("R14b"));
        c.priority_fee_max_lamports = u64::MAX;
        let r = review_message(&fee_msg(40_000_000), &c, |_| Ok(None));
        assert!(r.blocked());
    }

    /// 1.9 F7: the vault's own stake flows (Staking → Validators "Stake", Rewards
    /// "Unstake"/"Withdraw") build exactly these instructions. They must pass the
    /// rules cleanly and decode with readable names; withdrawing elsewhere still blocks.
    #[test]
    fn vault_stake_flows_pass_rules_and_decode() {
        use crate::tx::{create_account, stake_deactivate, stake_delegate, stake_initialize, stake_withdraw};
        let user = b58(&rnd());
        let stake = b58(&rnd());
        let vote = b58(&rnd());
        let bh = b58(&rnd());
        let review = |ixs: Vec<Instruction>| {
            let m = tx::compile_legacy_message(&user, &bh, &ixs).unwrap();
            let mut c = ctx(&user, &[]);
            c.from_dapp = false;
            review_message(&m, &c, |_| Ok(None))
        };
        let create = create_account(&user, &stake, 1_002_282_880, 200, ProgramIds::STAKE).unwrap();
        let init = stake_initialize(&stake, &user, &user).unwrap();
        let del = stake_delegate(&stake, &vote, &user);
        let r = review(vec![create, init, del]);
        assert!(!r.blocked() && !r.needs_ack(), "{:?}", r.findings);
        assert!(r.instruction_lines.iter().any(|l| l.contains("Initialize stake")));
        assert!(r.instruction_lines.iter().any(|l| l.contains("Delegate stake") && l.contains("validator")));
        let r = review(vec![stake_deactivate(&stake, &user)]);
        assert!(!r.blocked() && !r.needs_ack(), "{:?}", r.findings);
        assert!(r.instruction_lines[0].contains("Deactivate stake"));
        let r = review(vec![stake_withdraw(&stake, &user, &user, 5)]);
        assert!(!r.blocked() && !r.needs_ack(), "{:?}", r.findings);
        assert!(r.instruction_lines[0].contains("Withdraw stake") && r.instruction_lines[0].contains("5 lamports"));
        // Same withdraw, but to someone else: still R5 BLOCK.
        let thief = b58(&rnd());
        let r = review(vec![stake_withdraw(&stake, &thief, &user, 5)]);
        assert_eq!(r.first_block().map(|f| f.rule), Some("R5"));
    }

    #[test]
    fn stake_lockup_warns_and_all_stake_ops_have_names() {
        let user = rnd();
        let u = b58(&user);
        let sa = rnd();
        let stake = pk(ProgramIds::STAKE);
        // SetLockup: [stake, authority]; LockupArgs = 3 × Option (all Some here)
        let mut d = 6u32.to_le_bytes().to_vec();
        d.push(1);
        d.extend_from_slice(&i64::MAX.to_le_bytes());
        d.push(0);
        d.push(0);
        let m = msg(None, [1, 0, 1], &[user, sa, stake], &[(2, vec![1, 0], d)], &[]);
        let r = review_message(&m, &ctx(&u, &[]), |_| Ok(None));
        assert!(rules(&r).contains(&"R5b") && r.needs_ack() && !r.blocked(), "{:?}", r.findings);
        assert!(r.instruction_lines[0].contains("Set stake lockup"));
        for op in 0u32..=17 {
            let ix = Instruction {
                program_id: ProgramIds::STAKE.into(),
                accounts: vec![],
                data: op.to_le_bytes().to_vec(),
            };
            let t = crate::decode::decode_instruction(&ix).title;
            assert_ne!(t, "Stake", "stake op {op} has no name");
        }
    }

    #[test]
    fn not_a_signer_blocks() {
        let user = rnd();
        let someone = rnd();
        let m = msg(None, [1, 0, 1], &[someone, pk(ProgramIds::MEMO)], &[(1, vec![], b"hi".to_vec())], &[]);
        let r = review_message(&m, &ctx(&b58(&user), &[]), |_| Ok(None));
        assert_eq!(r.first_block().unwrap().rule, "SIGNER");
    }

    #[test]
    fn batch_one_block_blocks_all() {
        let user = rnd();
        let ok = msg(None, [1, 0, 1], &[user, pk(ProgramIds::MEMO)], &[(1, vec![], b"hi".to_vec())], &[]);
        let (bad, _t, _d, _u, _v) = drainer_v0();
        let wire = |m: &[u8]| {
            let mut w = vec![1u8];
            w.extend_from_slice(&[0u8; 64]);
            w.extend_from_slice(m);
            w
        };
        let u = b58(&user);
        let out = review_batch(&[wire(&ok), wire(&bad)], &ctx(&u, &[]), |_| Err("down".into()));
        assert!(out[0].blocked() && out[1].blocked());
        assert!(rules(&out[0]).contains(&"R10"));
    }

    #[test]
    fn sign_message_payload_that_is_a_tx_is_detected() {
        let (m, _t, _d, _u, _v) = drainer_v0();
        assert!(message_payload_is_transaction(&m));
        assert!(!message_payload_is_transaction(b"Sign in to example.com\nNonce: 123"));
    }

    #[test]
    fn mainnet_jupiter_v0_matches_rpc_loaded_addresses() {
        let fx: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/mainnet_jupiter_v0_alt.json"
        ))
        .unwrap();
        let wire = tx::decode_b64(fx["tx_b64"].as_str().unwrap()).unwrap();
        let alts = fx["alts"].as_object().unwrap().clone();
        let fetch = |k: &str| -> Fetched {
            match alts.get(k) {
                Some(a) => Ok(Some((
                    a["owner"].as_str().unwrap().to_string(),
                    1,
                    tx::decode_b64(a["data_b64"].as_str().unwrap()).unwrap(),
                ))),
                None => Ok(None),
            }
        };
        let (_, m) = split_wire(&wire).unwrap();
        let parsed = parse_message(m).unwrap();
        assert_eq!(parsed.version, Some(0));
        let res = resolve(&parsed, fetch).unwrap();
        let loaded: Vec<&ResolvedAccount> = res
            .accounts
            .iter()
            .filter(|a| matches!(a.source, AccountSource::Lookup { .. }))
            .collect();
        let want_w: Vec<&str> = fx["loadedAddresses"]["writable"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        let want_r: Vec<&str> = fx["loadedAddresses"]["readonly"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        let got_w: Vec<&str> = loaded.iter().filter(|a| a.writable).map(|a| a.pubkey.as_str()).collect();
        let got_r: Vec<&str> = loaded.iter().filter(|a| !a.writable).map(|a| a.pubkey.as_str()).collect();
        assert_eq!(got_w, want_w);
        assert_eq!(got_r, want_r);
        let user = res.accounts[0].pubkey.clone();
        let r = review_wire(&wire, &ctx(&user, &[]), fetch);
        assert!(!r.findings.iter().any(|x| x.rule == "R15"), "{:?}", r.findings);
        assert_eq!(r.lookup_accounts, want_w.len() + want_r.len());
        assert_eq!(r.account_lines.len(), parsed.static_keys.len() + want_w.len() + want_r.len());
        // Same bytes, table unreachable: blocked.
        let r = review_wire(&wire, &ctx(&user, &[]), |_| Err("timeout".into()));
        assert!(r.blocked());
    }
}
