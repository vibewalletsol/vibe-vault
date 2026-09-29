//! Anchor IDL instruction decode for the vault Confirm (1.9 F8). **Display only.**
//!
//! Safety contract:
//! * Nothing here is used by `txcheck` rules, the fee gate, the agent re-check or the
//!   pre-sign exact-bytes re-check. The only output is extra text lines on the Confirm
//!   screen. The bytes that get signed are never touched.
//! * Every instruction/argument/account name comes from the program's IDL and every string
//!   value from the transaction: both are untrusted. Names are sanitised and length-capped,
//!   strings are quoted with control and bidi characters replaced, and every line is
//!   labelled as coming from the program IDL.
//! * Any failure (no IDL, fetch error, bad JSON, unknown discriminator, unsupported type)
//!   yields nothing, and the Confirm keeps the existing raw decode.
//!
//! Where IDLs come from, in order: memory → disk cache (`~/.cache/vibe-wallet/idl`, 0600,
//! 7 days; "no IDL" remembered for 1 day) → the on-chain IDL account fetched through the
//! vault's own RPC (owner and discriminator checked, zlib-inflated with a 4 MiB cap) →
//! a small bundled set (`crates/vibe-core/idl/`).
//!
//! Port notes: the IDL address derivation (`create_with_seed(find_program_address([],
//! program), "anchor:idl", program)`), the IDL account layout and the bundled IDL set follow
//! Cinder's IdlRegistry.cpp / AnchorIdl.h (https://github.com/gregnb/cinder, MIT License,
//! Copyright (c) 2026 Gregory Nowakowski). Argument (Borsh) decoding, legacy snake_case
//! discriminators, the size limits and the cache are new.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::programs::ProgramIds;
use crate::tx::{self, Instruction};
use crate::txcheck::Fetched;

pub const IDL_SEED: &str = "anchor:idl";
/// Largest compressed IDL payload we read from an account.
pub const MAX_IDL_COMPRESSED: usize = 2 << 20;
/// Largest inflated IDL JSON (zip-bomb guard).
pub const MAX_IDL_JSON: usize = 4 << 20;
const MAX_DEPTH: usize = 12;
const MAX_ARG_CHARS: usize = 160;
const MAX_ITEMS: usize = 32;
const MAX_NAME: usize = 48;
const MAX_PROGRAMS_PER_TX: usize = 8;
const CACHE_TTL_OK: u64 = 7 * 86_400;
const CACHE_TTL_NONE: u64 = 86_400;
/// Decode work per instruction (values visited). Stops zero-size / nested-array blow-ups.
const MAX_STEPS: usize = 20_000;
/// Network reads (IDL accounts + lookup tables) per Confirm, and wall-clock budget.
pub const MAX_FETCHES_PER_CONFIRM: usize = 12;
pub const FETCH_TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(8);

/// Bundled IDLs: (program id, file). From Cinder `resources/idl/` (MIT), minified.
pub const BUNDLED: &[(&str, &str)] = &[
    ("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4", include_str!("../idl/jupiter_v6.json")),
    ("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK", include_str!("../idl/raydium_clmm.json")),
    ("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C", include_str!("../idl/raydium_cp_swap.json")),
    ("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo", include_str!("../idl/meteora_dlmm.json")),
    ("dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH", include_str!("../idl/drift_v2.json")),
    ("MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD", include_str!("../idl/marinade.json")),
    ("TSWAPaqyCSx2KABk68Shruf4rp7CxcNi8hAsbdwmHbN", include_str!("../idl/tensor_tswap.json")),
    ("M2mx93ekt1fmXSVkTrUL9xVFHkmME8HTUi5Cyc5aF7K", include_str!("../idl/magic_eden_m2.json")),
    ("SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf", include_str!("../idl/squads_v4.json")),
    ("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P", include_str!("../idl/pump_fun.json")),
];

// ---------------------------------------------------------------------------
// IDL model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct IdlAccountItem {
    pub name: String,
    pub writable: bool,
    pub signer: bool,
}

#[derive(Clone, Debug)]
pub struct IdlInstruction {
    pub name: String,
    pub disc: Vec<u8>,
    pub accounts: Vec<IdlAccountItem>,
    /// (name, type) — `None` when the IDL has no `args` (e.g. the stripped bundled set).
    pub args: Option<Vec<(String, Value)>>,
}

#[derive(Clone, Debug)]
pub struct Idl {
    pub address: String,
    pub name: String,
    pub instructions: Vec<IdlInstruction>,
    pub types: HashMap<String, Value>,
}

impl Idl {
    /// "jupiter_v6" → "Jupiter V6" (Cinder `displayName`).
    pub fn display_name(&self) -> String {
        let mut out = String::new();
        let mut up = true;
        for ch in self.name.chars() {
            if ch == '_' || ch == '-' || ch == ' ' {
                out.push(' ');
                up = true;
            } else if up && ch.is_alphabetic() {
                out.extend(ch.to_uppercase());
                up = false;
            } else {
                out.push(ch);
                up = false;
            }
        }
        let s = sanitize_name(out.trim());
        if s == "?" { "Anchor program".into() } else { s }
    }

    pub fn find(&self, data: &[u8]) -> Option<&IdlInstruction> {
        self.instructions
            .iter()
            .filter(|ix| !ix.disc.is_empty() && data.len() >= ix.disc.len() && data[..ix.disc.len()] == ix.disc[..])
            .max_by_key(|ix| ix.disc.len())
    }
}

/// heck-style snake_case (what Anchor < 0.30 hashes for the sighash).
pub fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                Some(p) if p.is_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_uppercase() => next.map(|n| n.is_lowercase()).unwrap_or(false),
                _ => false,
            };
            if boundary && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

pub fn sighash(namespace: &str, name: &str) -> [u8; 8] {
    let h: [u8; 32] = Sha256::digest(format!("{namespace}:{name}").as_bytes()).into();
    let mut d = [0u8; 8];
    d.copy_from_slice(&h[..8]);
    d
}

fn flatten_accounts(items: &[Value], prefix: &str, out: &mut Vec<IdlAccountItem>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for it in items {
        let name = it.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let full = if prefix.is_empty() { name.to_string() } else { format!("{prefix}.{name}") };
        if let Some(sub) = it.get("accounts").and_then(|v| v.as_array()) {
            flatten_accounts(sub, &full, out, depth + 1);
            continue;
        }
        let flag = |a: &str, b: &str| {
            it.get(a).or_else(|| it.get(b)).and_then(|v| v.as_bool()).unwrap_or(false)
        };
        out.push(IdlAccountItem {
            name: sanitize_name(&full),
            writable: flag("writable", "isMut"),
            signer: flag("signer", "isSigner"),
        });
    }
}

/// Parse a legacy (< 0.30) or new (0.30+) Anchor IDL JSON.
pub fn parse_idl(json: &Value, program_id: &str) -> Result<Idl, String> {
    let ixs = json
        .get("instructions")
        .and_then(|v| v.as_array())
        .ok_or("IDL has no instructions")?;
    let name = json
        .pointer("/metadata/name")
        .or_else(|| json.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let address = json
        .get("address")
        .or_else(|| json.pointer("/metadata/address"))
        .and_then(|v| v.as_str())
        .unwrap_or(program_id)
        .to_string();
    let mut instructions = Vec::with_capacity(ixs.len());
    for ix in ixs {
        let Some(n) = ix.get("name").and_then(|v| v.as_str()) else { continue };
        let disc = match ix.get("discriminator").and_then(|v| v.as_array()) {
            Some(arr) => {
                let d: Option<Vec<u8>> = arr.iter().map(|b| b.as_u64().and_then(|x| u8::try_from(x).ok())).collect();
                match d {
                    Some(d) if !d.is_empty() => d,
                    _ => continue,
                }
            }
            None => sighash("global", &snake_case(n)).to_vec(),
        };
        let mut accounts = Vec::new();
        if let Some(a) = ix.get("accounts").and_then(|v| v.as_array()) {
            flatten_accounts(a, "", &mut accounts, 0);
        }
        let args = ix.get("args").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .map(|a| {
                    (
                        sanitize_name(a.get("name").and_then(|v| v.as_str()).unwrap_or("?")),
                        a.get("type").cloned().unwrap_or(Value::Null),
                    )
                })
                .collect()
        });
        instructions.push(IdlInstruction { name: sanitize_name(n), disc, accounts, args });
    }
    if instructions.is_empty() {
        return Err("IDL has no usable instructions".into());
    }
    let mut types = HashMap::new();
    for key in ["types", "accounts"] {
        if let Some(arr) = json.get(key).and_then(|v| v.as_array()) {
            for t in arr {
                if let (Some(n), Some(ty)) = (t.get("name").and_then(|v| v.as_str()), t.get("type")) {
                    types.entry(n.to_string()).or_insert_with(|| ty.clone());
                }
            }
        }
    }
    Ok(Idl { address, name, instructions, types })
}

// ---------------------------------------------------------------------------
// On-chain IDL account
// ---------------------------------------------------------------------------

/// `create_with_seed(find_program_address([], program), "anchor:idl", program)`.
pub fn idl_address(program_id: &str) -> Result<String, String> {
    let prog = tx::decode_pubkey(program_id).map_err(|e| e.to_string())?;
    let (base, _) = tx::find_program_address(&[], &prog).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    h.update(base);
    h.update(IDL_SEED.as_bytes());
    h.update(prog);
    let out: [u8; 32] = h.finalize().into();
    Ok(tx::encode_pubkey(&out))
}

/// IDL account: `[8 disc = sha256("internal:IdlAccount")[..8]][32 authority][u32 len][zlib JSON]`.
pub fn parse_idl_account(data: &[u8]) -> Result<Value, String> {
    if data.len() < 44 {
        return Err("IDL account too short".into());
    }
    if data[..8] != sighash("internal", "IdlAccount") {
        return Err("not an Anchor IDL account".into());
    }
    let len = u32::from_le_bytes(data[40..44].try_into().unwrap()) as usize;
    if len == 0 || len > MAX_IDL_COMPRESSED || 44 + len > data.len() {
        return Err("bad IDL length".into());
    }
    let json = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&data[44..44 + len], MAX_IDL_JSON)
        .map_err(|_| "IDL does not inflate (corrupt or larger than 4 MiB)".to_string())?;
    serde_json::from_slice(&json).map_err(|e| format!("IDL is not JSON: {e}"))
}

pub fn fetch_onchain<F>(program_id: &str, fetch: &mut F) -> Result<Option<Value>, String>
where
    F: FnMut(&str) -> Fetched,
{
    let addr = idl_address(program_id)?;
    match fetch(&addr)? {
        None => Ok(None),
        Some((owner, _, data)) => {
            if owner != program_id {
                return Err("IDL account is not owned by the program".into());
            }
            parse_idl_account(&data).map(Some)
        }
    }
}

// ---------------------------------------------------------------------------
// Borsh argument decode (display strings)
// ---------------------------------------------------------------------------

pub fn sanitize_name(s: &str) -> String {
    let out: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ' ' | '-' | '.'))
        .take(MAX_NAME)
        .collect();
    if out.trim().is_empty() { "?".into() } else { out }
}

fn is_sneaky(c: char) -> bool {
    c.is_control()
        || matches!(c as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2069 | 0xFEFF | 0x061C | 0x00AD)
}

/// Quote an untrusted string: control/bidi/zero-width characters → U+FFFD, max 64 chars.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    let mut n = 0;
    for c in s.chars() {
        if n >= 64 {
            out.push('…');
            break;
        }
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if is_sneaky(c) => out.push('\u{FFFD}'),
            c => out.push(c),
        }
        n += 1;
    }
    out.push('"');
    out
}

struct Reader<'a> {
    d: &'a [u8],
    off: usize,
    steps: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.left() {
            return Err("argument data ends early".into());
        }
        let s = &self.d[self.off..self.off + n];
        self.off += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn left(&self) -> usize {
        self.d.len().saturating_sub(self.off)
    }
    fn step(&mut self) -> Result<(), String> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err("IDL types too complex to display".into());
        }
        Ok(())
    }
}

fn hex(b: &[u8]) -> String {
    let mut s = String::from("0x");
    for x in b.iter().take(32) {
        s.push_str(&format!("{x:02x}"));
    }
    if b.len() > 32 {
        s.push_str(&format!("…({} bytes)", b.len()));
    }
    s
}

fn is_u8(t: &Value) -> bool {
    t.as_str() == Some("u8")
}

fn prim(name: &str, r: &mut Reader) -> Result<String, String> {
    macro_rules! int {
        ($t:ty, $n:expr) => {{
            let b = r.take($n)?;
            <$t>::from_le_bytes(b.try_into().unwrap()).to_string()
        }};
    }
    Ok(match name {
        "u8" => int!(u8, 1),
        "i8" => int!(i8, 1),
        "u16" => int!(u16, 2),
        "i16" => int!(i16, 2),
        "u32" => int!(u32, 4),
        "i32" => int!(i32, 4),
        "u64" => int!(u64, 8),
        "i64" => int!(i64, 8),
        "u128" => int!(u128, 16),
        "i128" => int!(i128, 16),
        "f32" => int!(f32, 4),
        "f64" => int!(f64, 8),
        "bool" => match r.take(1)?[0] {
            0 => "false".into(),
            1 => "true".into(),
            _ => return Err("bad bool".into()),
        },
        "string" => {
            let n = r.u32()? as usize;
            let b = r.take(n)?;
            quote(&String::from_utf8_lossy(b))
        }
        "bytes" => {
            let n = r.u32()? as usize;
            hex(r.take(n)?)
        }
        "pubkey" | "publicKey" => {
            let mut k = [0u8; 32];
            k.copy_from_slice(r.take(32)?);
            tx::encode_pubkey(&k)
        }
        other => return Err(format!("unsupported type {}", sanitize_name(other))),
    })
}

fn value(idl: &Idl, ty: &Value, r: &mut Reader, depth: usize) -> Result<String, String> {
    if depth > MAX_DEPTH {
        return Err("type nesting too deep".into());
    }
    r.step()?;
    if let Some(s) = ty.as_str() {
        return prim(s, r);
    }
    let obj = ty.as_object().ok_or("bad type")?;
    if let Some(inner) = obj.get("option") {
        return match r.take(1)?[0] {
            0 => Ok("none".into()),
            1 => value(idl, inner, r, depth + 1),
            _ => Err("bad option tag".into()),
        };
    }
    if let Some(inner) = obj.get("coption") {
        return match r.u32()? {
            0 => Ok("none".into()),
            1 => value(idl, inner, r, depth + 1),
            _ => Err("bad coption tag".into()),
        };
    }
    if let Some(inner) = obj.get("vec") {
        let n = r.u32()? as usize;
        if n > r.left() {
            return Err("vec longer than the data".into());
        }
        if is_u8(inner) {
            return Ok(hex(r.take(n)?));
        }
        return items(idl, inner, n, r, depth);
    }
    if let Some(arr) = obj.get("array").and_then(|v| v.as_array()) {
        let inner = arr.first().ok_or("bad array")?;
        let n = arr.get(1).and_then(|v| v.as_u64()).ok_or("array length is generic")? as usize;
        if n > r.left().max(1) * 8 {
            return Err("array longer than the data".into());
        }
        if is_u8(inner) {
            return Ok(hex(r.take(n)?));
        }
        return items(idl, inner, n, r, depth);
    }
    if let Some(def) = obj.get("defined") {
        let (name, generic) = match def {
            Value::String(s) => (s.as_str(), false),
            Value::Object(o) => (
                o.get("name").and_then(|v| v.as_str()).ok_or("bad defined")?,
                o.get("generics").and_then(|v| v.as_array()).map(|g| !g.is_empty()).unwrap_or(false),
            ),
            _ => return Err("bad defined".into()),
        };
        if generic {
            return Err("generic types are not decoded".into());
        }
        let td = idl.types.get(name).ok_or_else(|| format!("type {} not in IDL", sanitize_name(name)))?;
        return typedef(idl, td, r, depth + 1);
    }
    Err("unsupported type".into())
}

fn items(idl: &Idl, inner: &Value, n: usize, r: &mut Reader, depth: usize) -> Result<String, String> {
    let mut parts = Vec::new();
    for i in 0..n {
        let v = value(idl, inner, r, depth + 1)?;
        if i < MAX_ITEMS {
            parts.push(v);
        }
    }
    if n > MAX_ITEMS {
        parts.push(format!("…{} more", n - MAX_ITEMS));
    }
    Ok(format!("[{}]", parts.join(", ")))
}

fn fields(idl: &Idl, fs: &[Value], r: &mut Reader, depth: usize) -> Result<String, String> {
    let named = fs.first().map(|f| f.get("name").is_some() && f.get("type").is_some()).unwrap_or(false);
    let mut parts = Vec::new();
    for f in fs {
        if named {
            let n = sanitize_name(f.get("name").and_then(|v| v.as_str()).unwrap_or("?"));
            let v = value(idl, f.get("type").ok_or("bad field")?, r, depth + 1)?;
            parts.push(format!("{n}: {v}"));
        } else {
            parts.push(value(idl, f, r, depth + 1)?);
        }
    }
    Ok(if named { format!("{{{}}}", parts.join(", ")) } else { format!("({})", parts.join(", ")) })
}

fn typedef(idl: &Idl, td: &Value, r: &mut Reader, depth: usize) -> Result<String, String> {
    if depth > MAX_DEPTH {
        return Err("type nesting too deep".into());
    }
    match td.get("kind").and_then(|v| v.as_str()) {
        Some("struct") => {
            let fs = td.get("fields").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            fields(idl, &fs, r, depth)
        }
        Some("enum") => {
            let vs = td.get("variants").and_then(|v| v.as_array()).ok_or("bad enum")?;
            let tag = r.take(1)?[0] as usize;
            let var = vs.get(tag).ok_or("enum variant out of range")?;
            let name = sanitize_name(var.get("name").and_then(|v| v.as_str()).unwrap_or("?"));
            match var.get("fields").and_then(|v| v.as_array()) {
                Some(fs) if !fs.is_empty() => Ok(format!("{name}{}", fields(idl, fs, r, depth)?)),
                _ => Ok(name),
            }
        }
        Some("type") | Some("alias") => {
            let t = td.get("alias").or_else(|| td.get("value")).ok_or("bad alias")?;
            value(idl, t, r, depth + 1)
        }
        _ => Err("unsupported type definition".into()),
    }
}

fn cap(s: String) -> String {
    if s.chars().count() <= MAX_ARG_CHARS {
        return s;
    }
    let mut t: String = s.chars().take(MAX_ARG_CHARS).collect();
    t.push('…');
    t
}

// ---------------------------------------------------------------------------
// Instruction decode
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub program: String,
    pub program_id: String,
    pub instruction: String,
    pub args: Vec<(String, String)>,
    /// (IDL account name, address)
    pub accounts: Vec<(String, String)>,
    pub notes: Vec<String>,
    pub source: &'static str,
}

impl Decoded {
    pub fn lines(&self, n: usize) -> Vec<String> {
        let mut out = vec![format!(
            "#{n} {} · {} — decoded from the {} IDL of program {} (display only; not checked by the vault)",
            self.program,
            self.instruction,
            self.source,
            crate::format::shorten_address(&self.program_id)
        )];
        if !self.args.is_empty() {
            out.push(format!(
                "   args: {}",
                self.args.iter().map(|(k, v)| format!("{k} = {v}")).collect::<Vec<_>>().join(", ")
            ));
        }
        if !self.accounts.is_empty() {
            let named: Vec<String> = self
                .accounts
                .iter()
                .filter(|(k, _)| !k.starts_with("remaining["))
                .map(|(k, v)| format!("{k} = {}", crate::format::shorten_address(v)))
                .collect();
            let extra = self.accounts.len() - named.len();
            let mut s = named.join(", ");
            if extra > 0 {
                if !s.is_empty() {
                    s.push_str(", ");
                }
                s.push_str(&format!("+{extra} more (full list in the vault decode)"));
            }
            out.push(format!("   accounts: {s}"));
        }
        for note in &self.notes {
            out.push(format!("   note: {note}"));
        }
        out
    }
}

pub fn decode_with(idl: &Idl, ix: &Instruction, source: &'static str) -> Option<Decoded> {
    let def = idl.find(&ix.data)?;
    let mut notes = Vec::new();
    let mut args = Vec::new();
    match &def.args {
        None => notes.push("this IDL lists no argument types".into()),
        Some(spec) => {
            let mut r = Reader { d: &ix.data, off: def.disc.len(), steps: 0 };
            for (name, ty) in spec {
                match value(idl, ty, &mut r, 0) {
                    Ok(v) => args.push((name.clone(), cap(v))),
                    Err(e) => {
                        notes.push(format!("arguments from “{name}” on could not be decoded ({e})"));
                        r.off = r.d.len();
                        break;
                    }
                }
            }
            if r.left() > 0 {
                notes.push(format!("{} trailing byte(s) not described by the IDL", r.left()));
            }
        }
    }
    let mut accounts = Vec::new();
    for (i, meta) in ix.accounts.iter().enumerate() {
        // Anchor passes the program id for an omitted optional account.
        if meta.pubkey == ix.program_id && def.accounts.get(i).map(|a| a.name != "program").unwrap_or(false) {
            continue;
        }
        let name = def.accounts.get(i).map(|a| a.name.clone()).unwrap_or_else(|| format!("remaining[{}]", i - def.accounts.len()));
        accounts.push((name, meta.pubkey.clone()));
    }
    Some(Decoded {
        program: idl.display_name(),
        program_id: ix.program_id.clone(),
        instruction: def.name.clone(),
        args,
        accounts,
        notes,
        source,
    })
}

/// Programs VIBE already decodes natively (never looked up as Anchor).
pub fn is_native(program_id: &str) -> bool {
    matches!(
        program_id,
        ProgramIds::SYSTEM
            | ProgramIds::COMPUTE_BUDGET
            | ProgramIds::TOKEN
            | ProgramIds::TOKEN_2022
            | ProgramIds::ASSOCIATED_TOKEN
            | ProgramIds::MEMO
            | crate::txcheck::MEMO_V1
            | ProgramIds::STAKE
            | ProgramIds::VOTE
            | ProgramIds::ADDRESS_LOOKUP_TABLE
            | ProgramIds::BPF_LOADER
            | ProgramIds::BPF_UPGRADEABLE_LOADER
    )
}

// ---------------------------------------------------------------------------
// Registry + cache
// ---------------------------------------------------------------------------

type Entry = Option<(Idl, &'static str)>;

pub struct Registry {
    mem: HashMap<String, Entry>,
    tmp: Entry,
    cache_dir: Option<PathBuf>,
    pub allow_fetch: bool,
}

pub fn default_cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("vibe-wallet").join("idl"))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Registry {
    pub fn new(cache_dir: Option<PathBuf>) -> Self {
        Registry { mem: HashMap::new(), tmp: None, cache_dir, allow_fetch: true }
    }

    fn cache_path(&self, program: &str) -> Option<PathBuf> {
        // Program ids are base58 (validated by the caller), safe as file names.
        self.cache_dir.as_ref().map(|d| d.join(format!("{program}.json")))
    }

    fn read_cache(&self, program: &str) -> Option<Option<Value>> {
        let p = self.cache_path(program)?;
        if std::fs::metadata(&p).ok()?.len() > (MAX_IDL_JSON as u64) * 2 {
            return None;
        }
        let v: Value = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
        let at = v.get("fetched").and_then(|x| x.as_u64())?;
        let idl = v.get("idl").cloned().filter(|x| !x.is_null());
        let ttl = if idl.is_some() { CACHE_TTL_OK } else { CACHE_TTL_NONE };
        if now().saturating_sub(at) > ttl {
            return None;
        }
        Some(idl)
    }

    fn write_cache(&self, program: &str, idl: Option<&Value>) {
        let Some(p) = self.cache_path(program) else { return };
        if let Some(dir) = p.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                return;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        let body = serde_json::json!({"v": 1, "program": program, "fetched": now(), "idl": idl});
        let tmp = p.with_extension("json.tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        if let Ok(mut f) = opts.open(&tmp) {
            use std::io::Write;
            if f.write_all(body.to_string().as_bytes()).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    fn bundled(program: &str) -> Entry {
        let (_, text) = BUNDLED.iter().find(|(id, _)| *id == program)?;
        let v: Value = serde_json::from_str(text).ok()?;
        parse_idl(&v, program).ok().map(|i| (i, "bundled"))
    }

    /// Look up (and remember) the IDL for `program`. `fetch` is the vault RPC's
    /// `getAccountInfo`; it is only called when there is no fresh cache entry.
    pub fn get<F>(&mut self, program: &str, fetch: &mut F) -> Option<&(Idl, &'static str)>
    where
        F: FnMut(&str) -> Fetched,
    {
        if !self.mem.contains_key(program) {
            let (entry, transient) = self.load(program, fetch);
            if transient {
                // RPC failed or the budget ran out: show what we have (bundled) this time
                // but try the chain again on the next Confirm.
                self.tmp = entry;
                return self.tmp.as_ref();
            }
            self.mem.insert(program.to_string(), entry);
        }
        self.mem.get(program).and_then(|e| e.as_ref())
    }

    fn load<F>(&mut self, program: &str, fetch: &mut F) -> (Entry, bool)
    where
        F: FnMut(&str) -> Fetched,
    {
        if tx::decode_pubkey(program).is_err() {
            return (None, false);
        }
        let mut transient = false;
        let from_cache = self.read_cache(program);
        let onchain: Option<Value> = match from_cache {
            Some(cached) => cached,
            None if self.allow_fetch => match fetch_onchain(program, fetch) {
                Ok(v) => {
                    self.write_cache(program, v.as_ref());
                    v
                }
                // RPC error / budget: don't cache, fall back to the bundled set.
                Err(_) => {
                    transient = true;
                    None
                }
            },
            None => None,
        };
        if let Some(v) = onchain {
            if let Ok(idl) = parse_idl(&v, program) {
                return (Some((idl, "on-chain")), transient);
            }
        }
        (Self::bundled(program), transient)
    }
}

/// Caps the network reads one Confirm may make for the IDL display (count + wall clock).
/// Once spent, every further read fails, which the decode treats as "no IDL".
pub struct FetchBudget {
    left: usize,
    deadline: std::time::Instant,
}

impl FetchBudget {
    pub fn new(max_fetches: usize, time: std::time::Duration) -> Self {
        FetchBudget { left: max_fetches, deadline: std::time::Instant::now() + time }
    }

    pub fn confirm_default() -> Self {
        Self::new(MAX_FETCHES_PER_CONFIRM, FETCH_TIME_BUDGET)
    }

    pub fn fetch<F>(&mut self, key: &str, f: &mut F) -> Fetched
    where
        F: FnMut(&str) -> Fetched,
    {
        if self.left == 0 || std::time::Instant::now() >= self.deadline {
            return Err("IDL fetch budget used up".into());
        }
        self.left -= 1;
        f(key)
    }
}

pub fn global() -> &'static Mutex<Registry> {
    static REG: OnceLock<Mutex<Registry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(Registry::new(default_cache_dir())))
}

/// Lines for every non-native instruction that an IDL can decode.
pub fn describe_instructions<F>(ixs: &[Instruction], reg: &mut Registry, mut fetch: F) -> Vec<String>
where
    F: FnMut(&str) -> Fetched,
{
    let mut out = Vec::new();
    let mut programs: Vec<&str> = Vec::new();
    for (i, ix) in ixs.iter().enumerate() {
        let pid = ix.program_id.as_str();
        if is_native(pid) {
            continue;
        }
        if !programs.contains(&pid) {
            if programs.len() >= MAX_PROGRAMS_PER_TX {
                continue;
            }
            programs.push(pid);
        }
        if let Some((idl, src)) = reg.get(pid, &mut fetch) {
            if let Some(d) = decode_with(idl, ix, src) {
                out.extend(d.lines(i + 1));
            }
        }
    }
    out
}

/// Parse + resolve a wire transaction (lookup tables through `fetch`) and describe it.
/// Returns nothing when anything fails; the Confirm then shows only the raw decode.
pub fn describe_wire<F>(txb: &[u8], reg: &mut Registry, mut fetch: F) -> Vec<String>
where
    F: FnMut(&str) -> Fetched,
{
    let Ok((_, m)) = crate::txcheck::split_wire(txb) else { return Vec::new() };
    let Ok(parsed) = crate::txcheck::parse_message(m) else { return Vec::new() };
    let Ok(res) = crate::txcheck::resolve(&parsed, &mut fetch) else { return Vec::new() };
    describe_instructions(&res.instructions, reg, fetch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keypair::Keypair;
    use crate::tx::AccountMeta;
    use serde_json::json;

    fn zlib(b: &[u8]) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec_zlib(b, 6)
    }

    fn idl_account(json: &Value) -> Vec<u8> {
        let z = zlib(json.to_string().as_bytes());
        let mut d = sighash("internal", "IdlAccount").to_vec();
        d.extend_from_slice(&[7u8; 32]);
        d.extend_from_slice(&(z.len() as u32).to_le_bytes());
        d.extend_from_slice(&z);
        d
    }

    fn new_idl(addr: &str) -> Value {
        json!({
            "address": addr,
            "metadata": {"name": "demo_swap", "version": "0.1.0"},
            "instructions": [{
                "name": "swap",
                "discriminator": [1, 2, 3, 4, 5, 6, 7, 8],
                "accounts": [
                    {"name": "user", "writable": true, "signer": true},
                    {"name": "pool", "accounts": [{"name": "state", "writable": true}, {"name": "vault"}]}
                ],
                "args": [
                    {"name": "amount_in", "type": "u64"},
                    {"name": "min_out", "type": {"option": "u64"}},
                    {"name": "side", "type": {"defined": {"name": "Side"}}},
                    {"name": "route", "type": {"vec": {"defined": {"name": "Hop"}}}},
                    {"name": "memo", "type": "string"},
                    {"name": "owner", "type": "pubkey"}
                ]
            }],
            "types": [
                {"name": "Side", "type": {"kind": "enum", "variants": [{"name": "Buy"}, {"name": "Sell", "fields": [{"name": "limit", "type": "u32"}]}]}},
                {"name": "Hop", "type": {"kind": "struct", "fields": [{"name": "pool", "type": "u8"}, {"name": "pct", "type": "u8"}]}}
            ]
        })
    }

    fn ix_data(owner: &[u8; 32], memo: &str) -> Vec<u8> {
        let mut d = vec![1, 2, 3, 4, 5, 6, 7, 8];
        d.extend_from_slice(&1_000_000u64.to_le_bytes());
        d.push(1);
        d.extend_from_slice(&990u64.to_le_bytes());
        d.push(1); // Sell
        d.extend_from_slice(&42u32.to_le_bytes());
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&[3, 60, 4, 40]);
        d.extend_from_slice(&(memo.len() as u32).to_le_bytes());
        d.extend_from_slice(memo.as_bytes());
        d.extend_from_slice(owner);
        d
    }

    #[test]
    fn snake_case_matches_anchor_sighash_rules() {
        assert_eq!(snake_case("swapBaseIn"), "swap_base_in");
        assert_eq!(snake_case("initializeV2"), "initialize_v2");
        assert_eq!(snake_case("already_snake"), "already_snake");
        assert_eq!(snake_case("setAuthorityURI"), "set_authority_uri");
        // Anchor's well-known `global:initialize` discriminator.
        assert_eq!(sighash("global", "initialize"), [175, 175, 109, 31, 13, 152, 155, 237]);
    }

    #[test]
    fn idl_address_matches_known_program() {
        // Checked against mainnet (2026-09-28): these accounts exist, are owned by the
        // program and start with the IdlAccount discriminator [24, 70, 98, 191, ...].
        assert_eq!(
            idl_address("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4").unwrap(),
            "C88XWfp26heEmDkmfSzeXP7Fd7GQJ2j9dDTUsyiZbUTa"
        );
        assert_eq!(
            idl_address("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo").unwrap(),
            "7UZRobkzaKVm1RbCH5WdFaYCGzCRjnu3prziHAsYiSyr"
        );
        assert_eq!(sighash("internal", "IdlAccount"), [24, 70, 98, 191, 58, 144, 123, 158]);
        assert!(idl_address("not-a-key").is_err());
    }

    #[test]
    fn decodes_new_format_args_and_nested_accounts() {
        let prog = Keypair::generate().address();
        let idl = parse_idl(&new_idl(&prog), &prog).unwrap();
        let owner = Keypair::generate().public_bytes();
        let accts: Vec<AccountMeta> = (0..4).map(|_| AccountMeta::writable(&Keypair::generate().address())).collect();
        let ix = Instruction { program_id: prog.clone(), accounts: accts.clone(), data: ix_data(&owner, "hi") };
        let d = decode_with(&idl, &ix, "on-chain").unwrap();
        assert_eq!(d.program, "Demo Swap");
        assert_eq!(d.instruction, "swap");
        let args: HashMap<_, _> = d.args.iter().cloned().collect();
        assert_eq!(args["amount_in"], "1000000");
        assert_eq!(args["min_out"], "990");
        assert_eq!(args["side"], "Sell{limit: 42}");
        assert_eq!(args["route"], "[{pool: 3, pct: 60}, {pool: 4, pct: 40}]");
        assert_eq!(args["memo"], "\"hi\"");
        assert_eq!(args["owner"], tx::encode_pubkey(&owner));
        let names: Vec<&str> = d.accounts.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["user", "pool.state", "pool.vault", "remaining[0]"]);
        assert!(d.notes.is_empty(), "{:?}", d.notes);
        let lines = d.lines(2);
        assert!(lines[0].starts_with("#2 Demo Swap · swap") && lines[0].contains("display only"));
    }

    #[test]
    fn legacy_idl_uses_snake_case_sighash() {
        let prog = Keypair::generate().address();
        let v = json!({"name": "old_prog", "instructions": [{"name": "depositAll", "accounts": [{"name": "user", "isMut": true, "isSigner": true}],
            "args": [{"name": "amount", "type": "u64"}, {"name": "who", "type": "publicKey"}, {"name": "cfg", "type": {"defined": "Cfg"}}]}],
            "types": [{"name": "Cfg", "type": {"kind": "struct", "fields": [{"name": "flag", "type": "bool"}]}}]});
        let idl = parse_idl(&v, &prog).unwrap();
        assert_eq!(idl.instructions[0].disc, sighash("global", "deposit_all").to_vec());
        let mut data = sighash("global", "deposit_all").to_vec();
        data.extend_from_slice(&5u64.to_le_bytes());
        data.extend_from_slice(&[9u8; 32]);
        data.push(1);
        let ix = Instruction { program_id: prog, accounts: vec![], data };
        let d = decode_with(&idl, &ix, "bundled").unwrap();
        assert_eq!(d.args[0], ("amount".into(), "5".into()));
        assert_eq!(d.args[2], ("cfg".into(), "{flag: true}".into()));
    }

    #[test]
    fn bad_or_short_data_never_panics_and_is_flagged() {
        let prog = Keypair::generate().address();
        let idl = parse_idl(&new_idl(&prog), &prog).unwrap();
        let owner = [1u8; 32];
        let full = ix_data(&owner, "x");
        for cut in 0..full.len() {
            let ix = Instruction { program_id: prog.clone(), accounts: vec![], data: full[..cut].to_vec() };
            if let Some(d) = decode_with(&idl, &ix, "on-chain") {
                assert!(!d.notes.is_empty(), "cut {cut} should be flagged");
            }
        }
        let mut extra = full.clone();
        extra.extend_from_slice(&[0, 0, 0]);
        let d = decode_with(&idl, &Instruction { program_id: prog.clone(), accounts: vec![], data: extra }, "x").unwrap();
        assert!(d.notes.iter().any(|n| n.contains("3 trailing byte")));
        // A huge vec length can't allocate or loop.
        let mut evil = vec![1, 2, 3, 4, 5, 6, 7, 8];
        evil.extend_from_slice(&1u64.to_le_bytes());
        evil.push(0);
        evil.push(0);
        evil.extend_from_slice(&u32::MAX.to_le_bytes());
        let d = decode_with(&idl, &Instruction { program_id: prog.clone(), accounts: vec![], data: evil }, "x").unwrap();
        assert!(d.notes.iter().any(|n| n.contains("route")), "{:?}", d.notes);
        // Unknown discriminator → no decode at all (raw display stays).
        assert!(decode_with(&idl, &Instruction { program_id: prog, accounts: vec![], data: vec![9; 16] }, "x").is_none());
        // Random bytes through every path.
        let mut seed = 0x1234_5678u32;
        for _ in 0..500 {
            let mut d = vec![1, 2, 3, 4, 5, 6, 7, 8];
            for _ in 0..(seed % 90) {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                d.push((seed >> 16) as u8);
            }
            let _ = decode_with(&idl, &Instruction { program_id: "x".into(), accounts: vec![], data: d }, "x");
        }
    }

    #[test]
    fn untrusted_strings_and_names_are_neutralised() {
        assert_eq!(quote("a\u{202E}b\nc"), "\"a\u{FFFD}b\u{FFFD}c\"");
        assert!(quote(&"x".repeat(200)).ends_with("…\""));
        assert_eq!(sanitize_name("transfer\u{202E}<b>all</b>"), "transferballb");
        assert_eq!(sanitize_name("\u{200B}"), "?");
    }

    #[test]
    fn onchain_account_parse_checks_layout_owner_and_size() {
        let prog = Keypair::generate().address();
        let v = new_idl(&prog);
        let data = idl_account(&v);
        assert_eq!(parse_idl_account(&data).unwrap(), v);
        let mut bad = data.clone();
        bad[0] ^= 1;
        assert!(parse_idl_account(&bad).is_err());
        let mut short = data.clone();
        short.truncate(50);
        assert!(parse_idl_account(&short).is_err());
        // Zip bomb: 8 MiB of zeros compresses tiny but must be refused.
        let bomb = zlib(&vec![b' '; 8 << 20]);
        let mut d = sighash("internal", "IdlAccount").to_vec();
        d.extend_from_slice(&[0u8; 32]);
        d.extend_from_slice(&(bomb.len() as u32).to_le_bytes());
        d.extend_from_slice(&bomb);
        assert!(parse_idl_account(&d).unwrap_err().contains("4 MiB"));
        // Owner must be the program.
        let addr = idl_address(&prog).unwrap();
        let mut f = |k: &str| -> Fetched {
            assert_eq!(k, addr);
            Ok(Some(("11111111111111111111111111111111".into(), 1, data.clone())))
        };
        assert!(fetch_onchain(&prog, &mut f).is_err());
    }

    #[test]
    fn registry_fetches_once_caches_and_falls_back_to_bundled() {
        let dir = std::env::temp_dir().join(format!("vibe-idl-test-{}", Keypair::generate().address()));
        let prog = Keypair::generate().address();
        let data = idl_account(&new_idl(&prog));
        let addr = idl_address(&prog).unwrap();
        let mut calls = 0;
        let owner = [5u8; 32];
        let ix = Instruction { program_id: prog.clone(), accounts: vec![], data: ix_data(&owner, "ok") };
        {
            let mut reg = Registry::new(Some(dir.clone()));
            let lines = describe_instructions(std::slice::from_ref(&ix), &mut reg, |k: &str| {
                calls += 1;
                assert_eq!(k, addr);
                Ok(Some((prog.clone(), 1, data.clone())))
            });
            assert!(lines[0].contains("Demo Swap · swap") && lines[0].contains("on-chain"), "{lines:?}");
        }
        assert_eq!(calls, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(format!("{prog}.json"))).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // New registry, same disk cache: no network.
        let mut reg = Registry::new(Some(dir.clone()));
        let lines = describe_instructions(std::slice::from_ref(&ix), &mut reg, |_k: &str| -> Fetched { panic!("should use the cache") });
        assert!(!lines.is_empty());
        // Bundled fallback when the RPC fails (Jupiter v6 `route` = first bundled ix name check).
        let mut reg = Registry::new(None);
        let jup = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
        let (idl, src) = reg.get(jup, &mut |_k: &str| -> Fetched { Err("offline".into()) }).unwrap();
        assert_eq!(*src, "bundled");
        let route = idl.instructions.iter().find(|i| i.name == "route").unwrap().clone();
        let mut d = route.disc.clone();
        d.extend_from_slice(&[0u8; 20]);
        let lines = describe_instructions(
            &[Instruction { program_id: jup.into(), accounts: vec![AccountMeta::writable(jup)], data: d }],
            &mut reg,
            |_k: &str| -> Fetched { Err("offline".into()) },
        );
        assert!(lines[0].contains("Jupiter · route") && lines[0].contains("bundled"), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("lists no argument types")));
        // Native programs are never looked up.
        let mut reg = Registry::new(None);
        let native = describe_instructions(
            &[crate::tx::system_transfer(jup, jup, 1)],
            &mut reg,
            |_k: &str| -> Fetched { panic!("native programs are decoded natively") },
        );
        assert!(native.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn malicious_idl_types_are_bounded() {
        let prog = Keypair::generate().address();
        // Nested fixed arrays of a zero-size struct: 16^8 visits without the step budget.
        let mut nested = json!({"defined": {"name": "Z"}});
        for _ in 0..8 {
            nested = json!({"array": [nested, 16]});
        }
        let v = json!({"address": prog, "metadata": {"name": "evil"}, "instructions": [
            {"name": "boom", "discriminator": [9], "accounts": [],
             "args": [{"name": "x", "type": nested}]},
            {"name": "loop", "discriminator": [8], "accounts": [],
             "args": [{"name": "y", "type": {"defined": {"name": "R"}}}]},
            {"name": "bad", "discriminator": [7, 300], "accounts": []}
        ], "types": [
            {"name": "Z", "type": {"kind": "struct", "fields": []}},
            {"name": "R", "type": {"kind": "struct", "fields": [{"name": "a", "type": {"defined": {"name": "R"}}}, {"name": "b", "type": {"defined": {"name": "R"}}}]}}
        ]});
        let idl = parse_idl(&v, &prog).unwrap();
        assert_eq!(idl.instructions.len(), 2, "out-of-range discriminator byte is skipped");
        let t = std::time::Instant::now();
        let d = decode_with(&idl, &Instruction { program_id: prog.clone(), accounts: vec![], data: vec![9, 0, 0] }, "on-chain").unwrap();
        assert!(d.notes.iter().any(|n| n.contains("too complex")), "{:?}", d.notes);
        let d = decode_with(&idl, &Instruction { program_id: prog.clone(), accounts: vec![], data: vec![8] }, "on-chain").unwrap();
        assert!(!d.notes.is_empty());
        assert!(t.elapsed() < std::time::Duration::from_secs(2));
        // Names can't smuggle markup or a fake "System Program" label without the program id.
        let lines = d.lines(1);
        assert!(lines[0].contains(&crate::format::shorten_address(&prog)), "{lines:?}");
        // Non-object garbage IDLs are rejected, not panicked on.
        for g in [json!(null), json!([1, 2]), json!({"instructions": 5}), json!({"instructions": [{"name": 3}]})] {
            assert!(parse_idl(&g, &prog).is_err());
        }
    }

    #[test]
    fn fetch_budget_caps_reads_and_failed_fetch_is_retried_later() {
        let mut b = FetchBudget::new(2, std::time::Duration::from_secs(60));
        let mut n = 0;
        let mut f = |_k: &str| -> Fetched {
            n += 1;
            Ok(None)
        };
        assert!(b.fetch("a", &mut f).is_ok());
        assert!(b.fetch("b", &mut f).is_ok());
        assert!(b.fetch("c", &mut f).is_err());
        assert_eq!(n, 2);
        let mut expired = FetchBudget::new(5, std::time::Duration::from_secs(0));
        assert!(expired.fetch("a", &mut |_k: &str| -> Fetched { panic!("no fetch after deadline") }).is_err());
        // A failed (transient) fetch isn't remembered: the next Confirm tries again.
        let prog = Keypair::generate().address();
        let data = idl_account(&new_idl(&prog));
        let mut reg = Registry::new(None);
        assert!(reg.get(&prog, &mut |_k: &str| -> Fetched { Err("offline".into()) }).is_none());
        let got = reg.get(&prog, &mut |_k: &str| -> Fetched { Ok(Some((prog.clone(), 1, data.clone()))) });
        assert_eq!(got.map(|(_, s)| *s), Some("on-chain"));
        // Oversized cache files are ignored.
        let dir = std::env::temp_dir().join(format!("vibe-idl-big-{}", Keypair::generate().address()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = Keypair::generate().address();
        std::fs::write(dir.join(format!("{big}.json")), vec![b' '; MAX_IDL_JSON * 2 + 1]).unwrap();
        let reg = Registry::new(Some(dir.clone()));
        assert!(reg.read_cache(&big).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn signing_code_never_uses_the_idl_decode() {
        // Display-only contract: no rule, fee gate, agent check or signing path may depend
        // on this module (an IDL must never block, unblock or change a signature).
        for (name, src) in [
            ("txcheck.rs", include_str!("txcheck.rs")),
            ("tx.rs", include_str!("tx.rs")),
            ("fees.rs", include_str!("fees.rs")),
            ("agents.rs", include_str!("agents.rs")),
            ("policy.rs", include_str!("policy.rs")),
            ("signer.rs", include_str!("signer.rs")),
        ] {
            assert!(!src.contains("anchor_idl"), "{name} must not use anchor_idl");
        }
    }

    /// `cargo test -p vibe-core live_mainnet -- --ignored` (network).
    #[test]
    #[ignore]
    fn live_mainnet_idl_fetch() {
        let rpc = crate::Rpc::new("https://api.mainnet-beta.solana.com").with_timeout(10);
        for prog in ["JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4", "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"] {
            let v = fetch_onchain(prog, &mut |k: &str| rpc.get_account(k).map_err(|e| e.to_string())).unwrap().unwrap();
            let idl = parse_idl(&v, prog).unwrap();
            eprintln!("{prog}: {} · {} instructions, args={}", idl.display_name(), idl.instructions.len(), idl.instructions[0].args.is_some());
            assert!(!idl.instructions.is_empty());
        }
    }

    #[test]
    fn every_bundled_idl_parses_and_matches_its_address() {
        for (id, text) in BUNDLED {
            let v: Value = serde_json::from_str(text).unwrap();
            let idl = parse_idl(&v, id).unwrap();
            assert_eq!(&idl.address, id);
            assert!(!idl.instructions.is_empty());
            assert!(idl.instructions.iter().all(|i| i.disc.len() == 8));
        }
    }

    #[test]
    fn describe_wire_never_changes_the_bytes_and_skips_on_bad_input() {
        let payer = Keypair::generate().address();
        let prog = Keypair::generate().address();
        let ix = Instruction { program_id: prog.clone(), accounts: vec![AccountMeta::writable(&payer)], data: ix_data(&[2u8; 32], "z") };
        let bh = Keypair::generate().address();
        let msg = tx::compile_legacy_message(&payer, &bh, &[ix]).unwrap();
        let mut wire = vec![1u8];
        wire.extend_from_slice(&[0u8; 64]);
        wire.extend_from_slice(&msg);
        let before = wire.clone();
        let data = idl_account(&new_idl(&prog));
        let mut reg = Registry::new(None);
        let lines = describe_wire(&wire, &mut reg, |_k: &str| Ok(Some((prog.clone(), 1, data.clone()))));
        assert!(lines.iter().any(|l| l.contains("swap")));
        assert_eq!(wire, before);
        let mut reg = Registry::new(None);
        assert!(describe_wire(&[1, 2, 3], &mut reg, |_k: &str| Ok(None)).is_empty());
    }
}
