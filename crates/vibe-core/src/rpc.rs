// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
// Portions derived from Cinder (https://github.com/gregnb/cinder),
// Copyright (c) 2026 Gregory Nowakowski, MIT License (see LICENSES/MIT-Cinder.txt).

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::programs::ProgramIds;
use crate::{format, LAMPORTS_PER_SOL};

const UA: &str = "VIBEWallet/1.0 (+https://oldgrowthcrypto.com)";

#[derive(Clone, Debug)]
pub struct Rpc {
    pub url: String,
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
        Self { url: url.into() }
    }

    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(UA)
            .build();
        let resp = agent
            .post(&self.url)
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| Error::Rpc(e.to_string()))?;
        let v: Value = resp.into_json().map_err(|e| Error::Rpc(e.to_string()))?;
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("rpc error");
            return Err(Error::msg(explain_rpc_text(msg)));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
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

    pub fn send_transaction(&self, tx_bytes: &[u8]) -> Result<String> {
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, tx_bytes);
        let r = self.call(
            "sendTransaction",
            json!([
                b64,
                {
                    "encoding": "base64",
                    "preflightCommitment": "confirmed",
                    "skipPreflight": false
                }
            ]),
        )?;
        r.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| Error::Rpc("sendTransaction returned no signature".into()))
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
