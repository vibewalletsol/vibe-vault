use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::programs::ProgramIds;
use crate::{format, LAMPORTS_PER_SOL};

const UA: &str = "VIBEWallet/1.0 (+https://oldgrowthcrypto.com)";

/// Ordered RPC endpoints with failover (1.9). `url` is the primary (index 0).
///
/// Reads fail over on transport errors, HTTP 408/429/5xx/401/403, bad JSON and
/// "node is behind" style JSON-RPC errors. Sends never re-sign: the same signed bytes
/// (same signature, same blockhash) go to the next endpoint only after a transport-level
/// failure, and only after `getSignatureStatuses` on that endpoint shows the signature
/// has not landed. A JSON-RPC rejection (preflight) is final and is never retried
/// elsewhere. A fallback endpoint is only used after its genesis hash matches the
/// expected cluster, so a failover can never hop to another cluster.
#[derive(Clone)]
pub struct Rpc {
    pub url: String,
    endpoints: Vec<String>,
    active: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    expected_genesis: Option<String>,
    genesis_ok: std::sync::Arc<std::sync::Mutex<Vec<Option<bool>>>>,
    transport: Transport,
}

/// Low-level JSON-RPC POST. Swappable for tests.
pub type Transport = std::sync::Arc<dyn Fn(&str, &Value) -> std::result::Result<Value, Fail> + Send + Sync>;

/// Why one endpoint attempt failed.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    /// Connection, DNS, TLS, timeout, HTTP 408/429/5xx/401/403, bad JSON. The request may
    /// or may not have reached the node.
    Transport(String),
    /// JSON-RPC error object (code, message). The node answered.
    Rpc(i64, String),
}

impl std::fmt::Debug for Rpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rpc")
            .field("url", &self.url)
            .field("endpoints", &self.endpoints.len())
            .field("active", &self.active_url())
            .finish()
    }
}

pub const GENESIS_MAINNET: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
pub const GENESIS_DEVNET: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
pub const GENESIS_TESTNET: &str = "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY";

/// Only https endpoints (http is allowed for localhost / 127.0.0.1 dev validators).
pub fn endpoint_allowed(url: &str) -> bool {
    let u = url.trim();
    if u.starts_with("https://") && u.len() > 8 {
        return true;
    }
    u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost")
}

/// Parse a user list (commas, newlines, spaces), keep allowed URLs, drop duplicates.
pub fn parse_endpoint_list(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split(|c: char| c == ',' || c == '\n' || c == ' ' || c == ';') {
        let u = part.trim().trim_end_matches('/').to_string();
        if !u.is_empty() && endpoint_allowed(&u) && !out.contains(&u) {
            out.push(u);
        }
    }
    out
}

/// Host part of an endpoint, without any query (API keys stay out of the UI).
pub fn endpoint_host(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or(rest).to_string()
}

fn is_retryable_rpc_code(code: i64) -> bool {
    // -32005 node unhealthy/behind, -32004 block not available, -32016 min context slot,
    // -32603 internal error, -32009/-32007 slot skipped/ledger jump.
    matches!(code, -32005 | -32004 | -32016 | -32603 | -32009 | -32007)
}

fn ureq_transport() -> Transport {
    ureq_transport_secs(30)
}

fn ureq_transport_secs(secs: u64) -> Transport {
    std::sync::Arc::new(move |url: &str, body: &Value| {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(secs))
            .user_agent(UA)
            .build();
        let resp = match agent.post(url).set("Content-Type", "application/json").send_json(body.clone()) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                // Some nodes put a JSON-RPC error in a 4xx body; treat those as answers.
                if code != 429 && code < 500 && code != 401 && code != 403 && code != 408 {
                    if let Ok(v) = r.into_json::<Value>() {
                        if let Some(err) = v.get("error") {
                            let c = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                            let m = err.get("message").and_then(|m| m.as_str()).unwrap_or("rpc error");
                            return Err(Fail::Rpc(c, m.to_string()));
                        }
                    }
                }
                return Err(Fail::Transport(format!("HTTP {code}")));
            }
            Err(e) => return Err(Fail::Transport(e.to_string())),
        };
        resp.into_json::<Value>().map_err(|e| Fail::Transport(format!("bad JSON: {e}")))
    })
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TokenBalance {
    pub account: String,
    pub mint: String,
    pub amount_raw: String,
    pub ui_amount: String,
    pub decimals: u8,
    pub program: String,
}

#[derive(Clone, Debug)]
pub struct SigInfo {
    pub signature: String,
    pub slot: u64,
    pub err: bool,
    pub memo: Option<String>,
    pub block_time: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Validator {
    pub vote: String,
    pub node: String,
    pub activated_stake: u64,
    pub commission: u8,
    pub delinquent: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct StakeAccount {
    pub address: String,
    pub lamports: u64,
    pub voter: Option<String>,
    pub state: String,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub address: String,
    pub lamports: u64,
    pub sol_usd: f64,
    pub tokens: Vec<TokenBalance>,
    pub holdings: Vec<crate::coingecko::Holding>,
    pub activity: Vec<SigInfo>,
    pub epoch: u64,
    pub slot: u64,
    pub change_24h: Option<f64>,
}

impl Rpc {
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_endpoints(url, &[], None)
    }

    /// Primary + ordered fallbacks. `expected_genesis` = the cluster's genesis hash when
    /// known (mainnet/devnet/testnet); for a custom cluster it is learned from the primary.
    pub fn with_endpoints(primary: impl Into<String>, fallbacks: &[String], expected_genesis: Option<&str>) -> Self {
        let url: String = primary.into();
        let mut endpoints = vec![url.clone()];
        for f in fallbacks {
            let f = f.trim().trim_end_matches('/').to_string();
            if endpoint_allowed(&f) && !endpoints.iter().any(|e| e.trim_end_matches('/') == f) {
                endpoints.push(f);
            }
        }
        let n = endpoints.len();
        Self {
            url,
            endpoints,
            active: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            expected_genesis: expected_genesis.map(|s| s.to_string()),
            genesis_ok: std::sync::Arc::new(std::sync::Mutex::new(vec![None; n])),
            transport: ureq_transport(),
        }
    }

    /// Test hook: replace the HTTP layer.
    pub fn with_transport(mut self, t: Transport) -> Self {
        self.transport = t;
        self
    }

    /// Same endpoints and failover state, shorter per-request HTTP timeout (1.9 F8: the
    /// display-only IDL fetch in the Confirm must not stall the review).
    pub fn with_timeout(self, secs: u64) -> Self {
        self.with_transport(ureq_transport_secs(secs.max(1)))
    }

    pub fn endpoints(&self) -> &[String] {
        &self.endpoints
    }

    pub fn active_index(&self) -> usize {
        self.active.load(std::sync::atomic::Ordering::SeqCst).min(self.endpoints.len().saturating_sub(1))
    }

    pub fn active_url(&self) -> String {
        self.endpoints.get(self.active_index()).cloned().unwrap_or_else(|| self.url.clone())
    }

    /// "api.mainnet-beta.solana.com (fallback 1 of 2)" — no API keys.
    pub fn active_label(&self) -> String {
        let i = self.active_index();
        let host = endpoint_host(&self.active_url());
        let n = self.endpoints.len().saturating_sub(1);
        match (i, n) {
            (_, 0) => host,
            (0, n) => format!("{host} (primary, {n} fallback{})", if n == 1 { "" } else { "s" }),
            (i, n) => format!("{host} (fallback {i} of {n})"),
        }
    }

    fn post(&self, idx: usize, method: &str, params: &Value) -> std::result::Result<Value, Fail> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let v = (self.transport)(&self.endpoints[idx], &body)?;
        if let Some(err) = v.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
            let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("rpc error");
            return Err(Fail::Rpc(code, msg.to_string()));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    /// May endpoint `idx` be used? Index 0 always; fallbacks must be on the same cluster.
    fn cluster_ok(&self, idx: usize) -> bool {
        if idx == 0 {
            return true;
        }
        if let Some(v) = self.genesis_ok.lock().ok().and_then(|g| g.get(idx).copied().flatten()) {
            return v;
        }
        let want = match &self.expected_genesis {
            Some(g) => Some(g.clone()),
            None => self.post(0, "getGenesisHash", &json!([])).ok().and_then(|v| v.as_str().map(|s| s.to_string())),
        };
        // Custom cluster with the primary down: we can't prove the fallback is the same chain.
        let Some(w) = want else { return false };
        let ok = match self.post(idx, "getGenesisHash", &json!([])) {
            Ok(v) => v.as_str() == Some(w.as_str()),
            // Fallback unreachable right now: say no, but don't remember it.
            Err(_) => return false,
        };
        if let Ok(mut g) = self.genesis_ok.lock() {
            if let Some(slot) = g.get_mut(idx) {
                *slot = Some(ok);
            }
        }
        ok
    }

    /// Endpoint order for one call: the active one first, then the rest in list order.
    fn order(&self) -> Vec<usize> {
        let n = self.endpoints.len();
        let a = self.active_index();
        let mut out = vec![a];
        out.extend((0..n).filter(|i| *i != a));
        out
    }

    fn set_active(&self, idx: usize) {
        self.active.store(idx, std::sync::atomic::Ordering::SeqCst);
    }

    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut last: Option<Fail> = None;
        for idx in self.order() {
            if !self.cluster_ok(idx) {
                continue;
            }
            match self.post(idx, method, &params) {
                Ok(v) => {
                    self.set_active(idx);
                    return Ok(v);
                }
                Err(Fail::Rpc(code, msg)) if !is_retryable_rpc_code(code) => {
                    return Err(Error::msg(explain_rpc_text(&msg)));
                }
                Err(f) => last = Some(f),
            }
        }
        Err(match last {
            Some(Fail::Rpc(_, m)) => Error::msg(explain_rpc_text(&m)),
            Some(Fail::Transport(m)) => Error::Rpc(m),
            None => Error::Rpc("no usable RPC endpoint (fallbacks are on a different cluster?)".into()),
        })
    }

    /// Health of every endpoint: (url, Ok(slot, latency_ms) | Err(reason)). Also checks the
    /// cluster (genesis hash).
    pub fn health(&self) -> Vec<(String, std::result::Result<(u64, u128), String>)> {
        (0..self.endpoints.len())
            .map(|i| {
                let t = std::time::Instant::now();
                let r = self
                    .post(i, "getSlot", &json!([{ "commitment": "confirmed" }]))
                    .map_err(|e| match e {
                        Fail::Transport(m) => m,
                        Fail::Rpc(c, m) => format!("{m} ({c})"),
                    })
                    .and_then(|v| v.as_u64().ok_or_else(|| "no slot".to_string()))
                    .and_then(|slot| {
                        if i > 0 && !self.cluster_ok(i) {
                            Err("different cluster (genesis hash mismatch)".into())
                        } else {
                            Ok((slot, t.elapsed().as_millis()))
                        }
                    });
                (self.endpoints[i].clone(), r)
            })
            .collect()
    }

    pub fn get_balance(&self, address: &str) -> Result<u64> {
        let r = self.call("getBalance", json!([address]))?;
        Ok(r.get("value").and_then(|v| v.as_u64()).unwrap_or(0))
    }

    /// Owner program id and raw account bytes. `None` if the account is missing.
    pub fn get_multiple_accounts(
        &self,
        addresses: &[String],
    ) -> Result<Vec<Option<(String, Vec<u8>)>>> {
        if addresses.is_empty() {
            return Ok(vec![]);
        }
        let mut out = Vec::with_capacity(addresses.len());
        for chunk in addresses.chunks(100) {
            let r = self.call(
                "getMultipleAccounts",
                json!([chunk, { "encoding": "base64" }]),
            )?;
            let arr = r
                .get("value")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for acc in arr {
                if acc.is_null() {
                    out.push(None);
                    continue;
                }
                let owner = acc
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let b64 = acc
                    .get("data")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let bytes = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    b64,
                )
                .unwrap_or_default();
                out.push(Some((owner, bytes)));
            }
        }
        while out.len() < addresses.len() {
            out.push(None);
        }
        Ok(out)
    }

    pub fn get_latest_blockhash(&self) -> Result<String> {
        let r = self.call(
            "getLatestBlockhash",
            json!([{ "commitment": "confirmed" }]),
        )?;
        r.get("value")
            .and_then(|v| v.get("blockhash"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| Error::Rpc("missing blockhash".into()))
    }

    pub fn get_latest_blockhash_and_height(&self) -> Result<(String, u64)> {
        let r = self.call(
            "getLatestBlockhash",
            json!([{ "commitment": "confirmed" }]),
        )?;
        let value = r.get("value").cloned().unwrap_or(Value::Null);
        let hash = value
            .get("blockhash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Rpc("missing blockhash".into()))?
            .to_string();
        let height = value
            .get("lastValidBlockHeight")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Ok((hash, height))
    }

    pub fn simulate(&self, tx_b64: &str) -> Result<String> {
        let r = self.call(
            "simulateTransaction",
            json!([
                tx_b64,
                {
                    "encoding": "base64",
                    "sigVerify": false,
                    "replaceRecentBlockhash": true,
                    "commitment": "confirmed"
                }
            ]),
        )?;
        let value = r.get("value").cloned().unwrap_or(Value::Null);
        if let Some(err) = value.get("err") {
            if !err.is_null() {
                let logs = value
                    .get("logs")
                    .and_then(|l| l.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str())
                            .filter(|s| s.contains("Error") || s.contains("failed") || s.contains("Instruction"))
                            .rev()
                            .take(3)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let mut msg = explain_sim_err(err);
                let blob = logs.join(" ");
                if blob.contains("GetAccountDataSize") {
                    msg = "Could not open a token account for the destination. This mint is likely Token-2022 while Send used classic SPL, or the mint CA is wrong.".into();
                }
                if !logs.is_empty() && !blob.contains("GetAccountDataSize") {
                    msg = format!("{msg} ({})", logs.join(" · "));
                }
                return Err(Error::msg(msg));
            }
        }
        let logs = value
            .get("logs")
            .and_then(|l| l.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let units = value
            .get("unitsConsumed")
            .and_then(|u| u.as_u64())
            .unwrap_or(0);
        Ok(format!("compute units: {units}\n{logs}"))
    }

    /// Broadcast signed bytes. Failover rules (never double-send):
    /// * The signature is computed locally from the bytes; every retry sends the identical
    ///   transaction, so the cluster can land it at most once.
    /// * A JSON-RPC error (preflight / simulation / blockhash rejection) is final.
    /// * After a transport failure the next endpoint is first asked whether the signature
    ///   already exists; if it does, that signature is returned without resending.
    /// * Nothing here ever fetches a new blockhash or re-signs.
    pub fn send_transaction(&self, tx_bytes: &[u8]) -> Result<String> {
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, tx_bytes);
        let local_sig = crate::tx::first_signature(tx_bytes).ok().map(|s| bs58::encode(s).into_string());
        let params = json!([
            b64,
            {
                "encoding": "base64",
                "preflightCommitment": "confirmed",
                "skipPreflight": false,
                "maxRetries": 5
            }
        ]);
        let mut last: Option<String> = None;
        let mut tried_any = false;
        for idx in self.order() {
            if !self.cluster_ok(idx) {
                continue;
            }
            if tried_any {
                if let Some(sig) = &local_sig {
                    let seen = self
                        .post(idx, "getSignatureStatuses", &json!([[sig], { "searchTransactionHistory": false }]))
                        .ok()
                        .and_then(|v| v.get("value").and_then(|a| a.as_array()).and_then(|a| a.first().cloned()))
                        .map(|st| !st.is_null())
                        .unwrap_or(false);
                    if seen {
                        self.set_active(idx);
                        return Ok(sig.clone());
                    }
                }
            }
            tried_any = true;
            match self.post(idx, "sendTransaction", &params) {
                Ok(r) => {
                    self.set_active(idx);
                    return r
                        .as_str()
                        .map(|s| s.to_string())
                        .or(local_sig.clone())
                        .ok_or_else(|| Error::Rpc("sendTransaction returned no signature".into()));
                }
                Err(Fail::Rpc(_, msg)) => return Err(Error::msg(explain_rpc_text(&msg))),
                Err(Fail::Transport(m)) => last = Some(m),
            }
        }
        Err(Error::Rpc(format!(
            "could not reach any RPC to send ({}). The transaction was not re-signed; check History before trying again.",
            last.unwrap_or_else(|| "no usable endpoint".into())
        )))
    }

    pub fn request_airdrop(&self, address: &str, lamports: u64) -> Result<String> {
        let r = self.call("requestAirdrop", json!([address, lamports]))?;
        r.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| Error::Rpc("airdrop returned no signature".into()))
    }

    pub fn get_signature_statuses(&self, sigs: &[String]) -> Result<Vec<Option<bool>>> {
        let r = self.call(
            "getSignatureStatuses",
            json!([sigs, { "searchTransactionHistory": true }]),
        )?;
        let arr = r
            .get("value")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .iter()
            .map(|v| {
                if v.is_null() {
                    None
                } else {
                    Some(v.get("err").map(|e| e.is_null()).unwrap_or(true))
                }
            })
            .collect())
    }

    pub fn get_signatures(&self, address: &str, limit: usize) -> Result<Vec<SigInfo>> {
        let r = self.call(
            "getSignaturesForAddress",
            json!([address, { "limit": limit }]),
        )?;
        let arr = r.as_array().cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .map(|v| SigInfo {
                signature: v
                    .get("signature")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                slot: v.get("slot").and_then(|s| s.as_u64()).unwrap_or(0),
                err: v.get("err").map(|e| !e.is_null()).unwrap_or(false),
                memo: v
                    .get("memo")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string()),
                block_time: v.get("blockTime").and_then(|t| t.as_i64()),
            })
            .collect())
    }

    pub fn get_transaction(&self, signature: &str) -> Result<Value> {
        self.call(
            "getTransaction",
            json!([
                signature,
                {
                    "encoding": "jsonParsed",
                    "maxSupportedTransactionVersion": 0,
                    "commitment": "confirmed"
                }
            ]),
        )
    }

    pub fn get_epoch_info(&self) -> Result<(u64, u64)> {
        let r = self.call("getEpochInfo", json!([]))?;
        Ok((
            r.get("epoch").and_then(|v| v.as_u64()).unwrap_or(0),
            r.get("absoluteSlot").and_then(|v| v.as_u64()).unwrap_or(0),
        ))
    }

    /// Freeze / mint / permanent-delegate / hook / transfer-fee flags for Confirm.
    pub fn mint_risks(&self, mint: &str) -> Result<crate::policy::MintRisks> {
        if mint.is_empty()
            || mint.eq_ignore_ascii_case("native")
            || mint.eq_ignore_ascii_case("sol")
            || mint == crate::WSOL_MINT
        {
            return Ok(crate::policy::MintRisks::default());
        }
        let r = self.call(
            "getAccountInfo",
            json!([mint, { "encoding": "jsonParsed" }]),
        )?;
        let value = r.get("value").cloned().unwrap_or(Value::Null);
        if value.is_null() {
            return Ok(crate::policy::MintRisks::default());
        }
        Ok(crate::policy::mint_risks_from_account(&value))
    }

    /// `(has_metadata_pointer, has_token_metadata, lamports, space)`.
    pub fn token_2022_metadata(&self, mint: &str) -> Result<(bool, bool, u64, u64)> {
        let r = self.call(
            "getAccountInfo",
            json!([mint, { "encoding": "jsonParsed" }]),
        )?;
        let value = r.get("value").cloned().unwrap_or(Value::Null);
        if value.is_null() {
            return Err(Error::msg("mint account not found"));
        }
        let lamports = value.get("lamports").and_then(|v| v.as_u64()).unwrap_or(0);
        let space = value
            .get("data")
            .and_then(|d| d.get("space"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let exts = value
            .get("data")
            .and_then(|d| d.get("parsed"))
            .and_then(|p| p.get("info"))
            .and_then(|i| i.get("extensions"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut pointer = false;
        let mut meta = false;
        for ext in &exts {
            let kind = ext
                .get("extension")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if kind.contains("metadatapointer") {
                pointer = true;
            }
            if kind == "tokenmetadata" {
                meta = true;
            }
        }
        Ok((pointer, meta, lamports, space))
    }

    pub fn get_min_rent(&self, space: u64) -> Result<u64> {
        let r = self.call("getMinimumBalanceForRentExemption", json!([space]))?;
        Ok(r.as_u64().unwrap_or(0))
    }

    /// Owner, lamports, and raw data. `None` if missing.
    pub fn get_account(&self, address: &str) -> Result<Option<(String, u64, Vec<u8>)>> {
        let r = self.call(
            "getAccountInfo",
            json!([address, { "encoding": "base64" }]),
        )?;
        let value = r.get("value").cloned().unwrap_or(Value::Null);
        if value.is_null() {
            return Ok(None);
        }
        let owner = value
            .get("owner")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let lamports = value.get("lamports").and_then(|v| v.as_u64()).unwrap_or(0);
        let b64 = value
            .get("data")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
            .unwrap_or_default();
        Ok(Some((owner, lamports, data)))
    }

    /// Owner of the mint account: classic SPL Token or Token-2022.
    pub fn token_program_for_mint(&self, mint: &str) -> Result<String> {
        match self.get_account(mint)? {
            Some((owner, _, _)) if crate::programs::ProgramIds::is_token_program(&owner) => Ok(owner),
            Some(_) => Err(Error::msg("That mint is not an SPL token on this cluster.")),
            None => Err(Error::msg("Mint not found on this cluster. Check the token CA.")),
        }
    }

    pub fn get_nonce(&self, address: &str) -> Result<crate::tx::NonceAccount> {
        match self.get_account(address)? {
            Some((_owner, _lamports, data)) => crate::tx::NonceAccount::from_account_data(&data),
            None => Err(Error::msg("nonce account not found")),
        }
    }

    pub fn recent_priority_fees(&self) -> Result<Vec<u64>> {
        let r = self.call("getRecentPrioritizationFees", json!([]))?;
        let arr = r.as_array().cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .filter_map(|row| row.get("prioritizationFee").and_then(|v| v.as_u64()))
            .collect())
    }

    pub fn get_stake_minimum_delegation(&self) -> Result<u64> {
        let r = self.call("getStakeMinimumDelegation", json!([]))?;
        if let Some(v) = r.as_u64() {
            return Ok(v);
        }
        Ok(r.get("value").and_then(|v| v.as_u64()).unwrap_or(LAMPORTS_PER_SOL))
    }

    fn token_accounts(&self, owner: &str, program: &str) -> Result<Vec<TokenBalance>> {
        let r = self.call(
            "getTokenAccountsByOwner",
            json!([
                owner,
                { "programId": program },
                { "encoding": "jsonParsed" }
            ]),
        )?;
        let arr = r
            .get("value")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for acc in arr {
            let pubkey = acc
                .get("pubkey")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            let info = acc
                .pointer("/account/data/parsed/info")
                .cloned()
                .unwrap_or(Value::Null);
            let mint = info
                .get("mint")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let ta = info.get("tokenAmount").cloned().unwrap_or(Value::Null);
            let amount_raw = ta
                .get("amount")
                .and_then(|a| a.as_str())
                .unwrap_or("0")
                .to_string();
            let ui = ta
                .get("uiAmountString")
                .and_then(|a| a.as_str())
                .unwrap_or("0")
                .to_string();
            let decimals = ta.get("decimals").and_then(|d| d.as_u64()).unwrap_or(0) as u8;
            if amount_raw == "0" {
                continue;
            }
            out.push(TokenBalance {
                account: pubkey,
                mint,
                amount_raw,
                ui_amount: ui,
                decimals,
                program: program.to_string(),
            });
        }
        Ok(out)
    }

    pub fn get_tokens(&self, owner: &str) -> Result<Vec<TokenBalance>> {
        let mut a = self.token_accounts(owner, ProgramIds::TOKEN)?;
        let b = self.token_accounts(owner, ProgramIds::TOKEN_2022)?;
        a.extend(b);
        a.sort_by(|x, y| y.ui_amount.partial_cmp(&x.ui_amount).unwrap_or(std::cmp::Ordering::Equal));
        Ok(a)
    }

    pub fn get_vote_accounts(&self) -> Result<Vec<Validator>> {
        let r = self.call("getVoteAccounts", json!([]))?;
        let mut out = Vec::new();
        for (key, delinquent) in [("current", false), ("delinquent", true)] {
            if let Some(arr) = r.get(key).and_then(|v| v.as_array()) {
                for v in arr {
                    out.push(Validator {
                        vote: v
                            .get("votePubkey")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string(),
                        node: v
                            .get("nodePubkey")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string(),
                        activated_stake: v
                            .get("activatedStake")
                            .and_then(|s| s.as_u64())
                            .unwrap_or(0),
                        commission: v.get("commission").and_then(|c| c.as_u64()).unwrap_or(0)
                            as u8,
                        delinquent,
                    });
                }
            }
        }
        out.sort_by(|a, b| b.activated_stake.cmp(&a.activated_stake));
        Ok(out)
    }

    pub fn get_stake_accounts(&self, withdrawer: &str) -> Result<Vec<StakeAccount>> {
        let r = self.call(
            "getProgramAccounts",
            json!([
                ProgramIds::STAKE,
                {
                    "encoding": "jsonParsed",
                    "filters": [
                        { "memcmp": { "offset": 44, "bytes": withdrawer } }
                    ]
                }
            ]),
        )?;
        let arr = r.as_array().cloned().unwrap_or_default();
        let mut out = Vec::new();
        for acc in arr {
            let address = acc
                .get("pubkey")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            let lamports = acc
                .pointer("/account/lamports")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let info = acc.pointer("/account/data/parsed/info").cloned().unwrap_or(Value::Null);
            let stake = info.get("stake").cloned().unwrap_or(Value::Null);
            let voter = stake
                .pointer("/delegation/voter")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let type_name = acc
                .pointer("/account/data/parsed/type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            out.push(StakeAccount {
                address,
                lamports,
                voter,
                state: type_name,
            });
        }
        Ok(out)
    }

    pub fn snapshot(&self, address: &str) -> Result<Snapshot> {
        self.snapshot_with_gecko(address, None)
    }

    pub fn snapshot_with_gecko(
        &self,
        address: &str,
        gecko_key: Option<&str>,
    ) -> Result<Snapshot> {
        if !format::looks_like_pubkey(address) {
            return Err(Error::InvalidAddress);
        }
        let lamports = self.get_balance(address)?;
        let tokens = self.get_tokens(address).unwrap_or_default();
        let activity = self.get_signatures(address, 25).unwrap_or_default();
        let (epoch, slot) = self.get_epoch_info().unwrap_or((0, 0));
        let gecko = crate::coingecko::Client::new(gecko_key.unwrap_or(""), false);
        let holdings = gecko.enrich(lamports, &tokens);
        let sol_usd = holdings
            .iter()
            .find(|h| h.coin_id == "solana")
            .map(|h| h.price_usd)
            .filter(|p| *p > 0.0)
            .or_else(|| fetch_sol_usd().ok())
            .unwrap_or(0.0);
        let change_24h = crate::coingecko::weighted_change_24h(&holdings);
        Ok(Snapshot {
            address: address.to_string(),
            lamports,
            sol_usd,
            tokens,
            holdings,
            activity,
            epoch,
            slot,
            change_24h,
        })
    }
}

fn explain_rpc_text(msg: &str) -> String {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("0x1771") {
        return "Price moved past slippage. Raise slippage and get a new quote.".into();
    }
    if lower.contains("0x1778") {
        return "This swap tx is missing accounts. Get a new quote and try again.".into();
    }
    if lower.contains("0x177a") {
        return "This swap route is stale or the pool rejected it. Get a new quote and try again."
            .into();
    }
    if lower.contains("0x177e") {
        return "This Token-2022 mint cannot take the VibeSwap fee that way. Try another token."
            .into();
    }
    if lower.contains("0x1788") {
        return "Not enough SOL or tokens for this swap plus fees.".into();
    }
    if lower.contains("simulation failed") {
        return "This swap would fail on-chain. Get a new quote and try again.".into();
    }
    msg.to_string()
}

fn explain_sim_err(err: &Value) -> String {
    if let Some(arr) = err.get("InstructionError").and_then(|v| v.as_array()) {
        let n = arr.first().and_then(|v| v.as_u64()).unwrap_or(0);
        let detail = arr.get(1).unwrap_or(&Value::Null);
        if let Some(s) = detail.as_str() {
            return match s {
                "ProgramFailedToComplete" | "ComputationalBudgetExceeded" => {
                    "Token create ran out of compute. Close this window and sign again.".into()
                }
                "InsufficientFunds" => "Not enough SOL for rent plus fees.".into(),
                "InvalidInstructionData" => {
                    format!("Instruction {n} rejected. Check ticker, URI, and extras.")
                }
                "InvalidAccountData" => format!(
                    "Instruction {n}: mint account space did not match the Token-2022 extensions."
                ),
                other => format!("Instruction {n} failed ({other})."),
            };
        }
        if detail.get("InsufficientFundsForRent").is_some() {
            return "Not enough SOL left for rent. Keep a little SOL in the wallet.".into();
        }
        if let Some(custom) = detail.get("Custom").and_then(|v| v.as_u64()) {
            return match custom {
                6001 => "Price moved past slippage. Raise slippage and get a new quote.".into(),
                6008 => "This swap tx is missing accounts. Get a new quote and try again.".into(),
                6010 => {
                    "This swap route is stale or the pool rejected it. Get a new quote and try again."
                        .into()
                }
                6014 => {
                    "This Token-2022 mint cannot take the VibeSwap fee that way. Try another token."
                        .into()
                }
                6017 => "Price moved. Raise slippage and try again.".into(),
                6024 => "Not enough SOL or tokens for this swap plus fees.".into(),
                4 => format!(
                    "Instruction {n}: token authority does not match (SetAuthority)."
                ),
                901952961 => {
                    "This mint has no Token-2022 metadata to edit, or the mint needs more SOL for rent. Tokens created here can be updated after a successful Sign."
                        .into()
                },
                12 => "Solana requires at least 1 SOL to stake. Raise the amount and try again."
                    .into(),
                11 => "Cannot close a token account that still has tokens. Dust burn did not empty it."
                    .into(),
                17 => "That token account is frozen. It cannot be closed.".into(),
                1 => "Not enough of that mint in this wallet (wrong CA, or amount bigger than you hold).".into(),
                other => format!("Instruction {n} rejected (custom {other})."),
            };
        }
    }
    format!("simulation failed: {err}")
}

#[cfg(test)]
mod failover_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(String, String)>>>;

    /// Fake cluster: `down` hosts fail at transport level; `genesis` per host; `landed`
    /// tracks signatures the "cluster" has accepted (shared across hosts).
    fn fake(down: &'static [&'static str], wrong_cluster: &'static [&'static str], log: Log, landed: Arc<Mutex<Vec<String>>>, reject_send: bool) -> Transport {
        Arc::new(move |url: &str, body: &Value| {
            let method = body["method"].as_str().unwrap_or("").to_string();
            log.lock().unwrap().push((endpoint_host(url), method.clone()));
            if down.iter().any(|d| url.contains(d)) {
                return Err(Fail::Transport("connection refused".into()));
            }
            match method.as_str() {
                "getGenesisHash" => Ok(json!({"result": if wrong_cluster.iter().any(|d| url.contains(d)) { GENESIS_DEVNET } else { GENESIS_MAINNET }})),
                "getBalance" => Ok(json!({"result": {"value": 42}})),
                "getSlot" => Ok(json!({"result": 1000})),
                "getSignatureStatuses" => {
                    let sig = body["params"][0][0].as_str().unwrap_or("").to_string();
                    let hit = landed.lock().unwrap().contains(&sig);
                    Ok(json!({"result": {"value": [if hit { json!({"err": null}) } else { Value::Null }]}}))
                }
                "sendTransaction" => {
                    if reject_send {
                        return Ok(json!({"error": {"code": -32002, "message": "Transaction simulation failed: Blockhash not found"}}));
                    }
                    let b64 = body["params"][0].as_str().unwrap();
                    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap();
                    let sig = bs58::encode(crate::tx::first_signature(&bytes).unwrap()).into_string();
                    landed.lock().unwrap().push(sig.clone());
                    Ok(json!({"result": sig}))
                }
                _ => Ok(json!({"result": null})),
            }
        })
    }

    fn rpc(down: &'static [&'static str], wrong: &'static [&'static str], reject: bool) -> (Rpc, Log, Arc<Mutex<Vec<String>>>) {
        let log: Log = Arc::new(Mutex::new(vec![]));
        let landed = Arc::new(Mutex::new(vec![]));
        let r = Rpc::with_endpoints(
            "https://a.example",
            &["https://b.example".into(), "https://c.example".into(), "http://evil.example".into()],
            Some(GENESIS_MAINNET),
        )
        .with_transport(fake(down, wrong, log.clone(), landed.clone(), reject));
        (r, log, landed)
    }

    fn signed_tx() -> Vec<u8> {
        let kp = crate::keypair::Keypair::generate();
        let from = kp.address();
        let hash = crate::keypair::Keypair::generate().address();
        crate::tx::build_signed_legacy(&from, &hash, &[crate::tx::system_transfer(&from, &from, 1)], &[&kp]).unwrap()
    }

    #[test]
    fn plain_http_fallbacks_are_dropped() {
        let (r, _, _) = rpc(&[], &[], false);
        assert_eq!(r.endpoints().len(), 3);
        assert!(!endpoint_allowed("http://evil.example"));
        assert!(endpoint_allowed("http://127.0.0.1:8899"));
        assert_eq!(parse_endpoint_list("https://x.io/, https://x.io\nhttp://bad  https://y.io?api-key=1"), vec!["https://x.io", "https://y.io?api-key=1"]);
        assert_eq!(endpoint_host("https://y.io/rpc?api-key=SECRET"), "y.io");
    }

    #[test]
    fn reads_fail_over_and_stick() {
        let (r, log, _) = rpc(&["a.example"], &[], false);
        assert_eq!(r.get_balance("x").unwrap(), 42);
        assert_eq!(r.active_url(), "https://b.example");
        assert!(r.active_label().contains("fallback 1 of 2"));
        log.lock().unwrap().clear();
        r.get_balance("x").unwrap();
        // Sticky: next call goes straight to b (genesis already verified, cached).
        assert_eq!(log.lock().unwrap()[0], ("b.example".into(), "getBalance".into()));
    }

    #[test]
    fn never_fails_over_to_another_cluster() {
        let (r, _, _) = rpc(&["a.example"], &["b.example"], false);
        r.get_balance("x").unwrap();
        assert_eq!(r.active_url(), "https://c.example");
        let h = r.health();
        assert!(h[0].1.is_err());
        assert!(h[1].1.as_ref().unwrap_err().contains("different cluster"));
        assert!(h[2].1.is_ok());
    }

    #[test]
    fn send_fails_over_with_identical_bytes_once() {
        let (r, log, landed) = rpc(&["a.example"], &[], false);
        let tx = signed_tx();
        let want = bs58::encode(crate::tx::first_signature(&tx).unwrap()).into_string();
        assert_eq!(r.send_transaction(&tx).unwrap(), want);
        assert_eq!(landed.lock().unwrap().as_slice(), &[want.clone()]);
        let sends = log.lock().unwrap().iter().filter(|(_, m)| m == "sendTransaction").count();
        assert_eq!(sends, 2); // a (down) + b
    }

    #[test]
    fn send_skips_resend_when_signature_already_landed() {
        // a accepted the tx but the response was lost (modelled: landed before, a down).
        let (r, log, landed) = rpc(&["a.example"], &[], false);
        let tx = signed_tx();
        let sig = bs58::encode(crate::tx::first_signature(&tx).unwrap()).into_string();
        landed.lock().unwrap().push(sig.clone());
        assert_eq!(r.send_transaction(&tx).unwrap(), sig);
        let calls = log.lock().unwrap().clone();
        assert!(!calls.iter().any(|(h, m)| h == "b.example" && m == "sendTransaction"), "{calls:?}");
        assert_eq!(landed.lock().unwrap().len(), 1);
    }

    #[test]
    fn send_rejection_is_final() {
        let (r, log, _) = rpc(&[], &[], true);
        assert!(r.send_transaction(&signed_tx()).is_err());
        let sends = log.lock().unwrap().iter().filter(|(_, m)| m == "sendTransaction").count();
        assert_eq!(sends, 1);
    }

    #[test]
    fn all_down_says_not_resigned() {
        let (r, _, _) = rpc(&["a.example", "b.example", "c.example"], &[], false);
        let e = r.send_transaction(&signed_tx()).unwrap_err().to_string();
        assert!(e.contains("not re-signed"), "{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explain_jup_stale_route() {
        let err = json!({"InstructionError":[2,{"Custom":6010}]});
        let msg = explain_sim_err(&err);
        assert!(msg.to_lowercase().contains("stale") || msg.to_lowercase().contains("quote"), "{msg}");
        assert!(!msg.contains("simulation failed"));
        assert_eq!(
            explain_rpc_text("Transaction simulation failed: custom program error: 0x177a"),
            "This swap route is stale or the pool rejected it. Get a new quote and try again."
        );
    }

    #[test]
    fn explain_close_with_balance() {
        let err = json!({"InstructionError":[5,{"Custom":11}]});
        let msg = explain_sim_err(&err);
        assert!(msg.contains("still has tokens"), "{msg}");
        assert!(!msg.contains("simulation failed"));
    }

    #[test]
    fn explain_missing_token_metadata() {
        let err = json!({"InstructionError":[2,{"Custom":901952961}]});
        let msg = explain_sim_err(&err);
        assert!(msg.to_lowercase().contains("metadata"), "{msg}");
    }

    #[test]
    fn explain_token_owner_mismatch() {
        let err = json!({"InstructionError":[12,{"Custom":4}]});
        let msg = explain_sim_err(&err);
        assert!(msg.contains("authority does not match"), "{msg}");
        assert!(!msg.contains("stake account"));
    }

    #[test]
    fn explain_ata_get_account_data_size_is_not_insufficient_mint() {
        let err = json!({"InstructionError":[0,{"Custom":1}]});
        let logs = [
            "Program ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL failed: custom program error: 0x1",
            "Program log: Instruction: GetAccountDataSize",
        ];
        let blob = logs.join(" ");
        let mut msg = explain_sim_err(&err);
        if blob.contains("GetAccountDataSize") {
            msg = "Could not open a token account for the destination. This mint is likely Token-2022 while Send used classic SPL, or the mint CA is wrong.".into();
        }
        assert!(msg.to_lowercase().contains("token-2022") || msg.to_lowercase().contains("mint ca"), "{msg}");
        assert!(!msg.to_lowercase().contains("not enough of that mint"), "{msg}");
    }

    #[test]
    fn explain_mint_out_of_compute() {
        let err = json!({"InstructionError":[6,"ProgramFailedToComplete"]});
        let msg = explain_sim_err(&err);
        assert!(msg.contains("compute"), "{msg}");
        assert!(!msg.contains("simulation failed"));
    }
}

pub fn fetch_sol_usd() -> Result<f64> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(UA)
        .build();
    let v: Value = agent
        .get("https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd")
        .call()
        .map_err(|e| Error::Rpc(e.to_string()))?
        .into_json()
        .map_err(|e| Error::Rpc(e.to_string()))?;
    v.pointer("/solana/usd")
        .and_then(|n| n.as_f64())
        .ok_or_else(|| Error::Rpc("no sol usd price".into()))
}

pub fn usd_from_lamports(lamports: u64, sol_usd: f64) -> f64 {
    (lamports as f64 / LAMPORTS_PER_SOL as f64) * sol_usd
}
