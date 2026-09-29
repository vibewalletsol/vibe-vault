// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
//! Jupiter Swap V2 meta-aggregator: `/swap/v2/order` + `/swap/v2/execute`.
//!
//! Base URL is `https://api.jup.ag`. An API key (`x-api-key`) is recommended
//! (free at https://developers.jup.ag/portal). Keyless still works at a low
//! rate limit.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::keypair::Keypair;
use crate::tx;

static REFERRAL_ENABLED: AtomicBool = AtomicBool::new(true);

const BASE: &str = "https://api.jup.ag/swap/v2";
const UA: &str = "VIBEWallet/1.0 (+https://oldgrowthcrypto.com)";
/// Ultra / Swap V2 referral account from referral.jup.ag.
pub const REFERRAL_ACCOUNT: &str = "BXr7VFmHHgxRjijszzf3upW8yRRthEC53V65coJr6HXz";
/// Ultra / Swap V2 minimum integrator fee (50 bps = 0.50%).
pub const REFERRAL_FEE_BPS: u16 = 50;

#[derive(Clone, Debug, Default)]
pub struct Client {
    pub api_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Order {
    pub input_mint: String,
    pub output_mint: String,
    pub in_amount: String,
    pub out_amount: String,
    pub other_amount_threshold: String,
    pub in_usd: Option<f64>,
    pub out_usd: Option<f64>,
    pub price_impact: Option<f64>,
    pub slippage_bps: Option<u16>,
    pub router: String,
    pub mode: String,
    pub fee_bps: Option<f64>,
    pub fee_mint: String,
    pub route_labels: Vec<String>,
    pub request_id: String,
    pub transaction_b64: Option<String>,
    pub gasless: bool,
    pub error_code: Option<i64>,
    pub error_message: String,
}

impl Order {
    pub fn can_execute(&self) -> bool {
        self.transaction_b64
            .as_ref()
            .map(|t| !t.is_empty())
            .unwrap_or(false)
    }

    pub fn route_summary(&self) -> String {
        if self.route_labels.is_empty() {
            self.router.clone()
        } else {
            format!("{} · {}", self.router, self.route_labels.join(" → "))
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExecuteResult {
    pub status: String,
    pub signature: String,
    pub code: i64,
    pub total_in: String,
    pub total_out: String,
    pub error: String,
}

impl Client {
    pub fn new(api_key: Option<String>) -> Self {
        let api_key = api_key
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Self { api_key }
    }

    pub fn from_env_or(settings_key: &str) -> Self {
        let env = std::env::var("VIBE_JUPITER_API_KEY")
            .ok()
            .or_else(|| std::env::var("JUPITER_API_KEY").ok());
        let key = env
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                let t = settings_key.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            });
        Self::new(key)
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(45))
            .user_agent(UA)
            .build()
    }

    fn apply_key<'a>(&self, req: ureq::Request) -> ureq::Request {
        match &self.api_key {
            Some(k) => req.set("x-api-key", k),
            None => req,
        }
    }

    fn read_json(resp: ureq::Response) -> Result<Value> {
        resp.into_json()
            .map_err(|e| Error::msg(format!("jupiter json: {e}")))
    }

    fn map_http(err: ureq::Error) -> Error {
        match err {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                let detail = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v.get("error")
                            .and_then(|e| e.as_str())
                            .map(|s| s.to_string())
                            .or_else(|| v.get("message").and_then(|e| e.as_str()).map(|s| s.to_string()))
                    })
                    .unwrap_or(body);
                http_fail_msg(code, &detail)
            }
            other => Error::msg(format!("jupiter: {other}")),
        }
    }

    fn order_json(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        taker: Option<&str>,
        slippage_bps: Option<u16>,
        with_referral: bool,
    ) -> Result<Value> {
        let input_mint = jup_mint(input_mint);
        let output_mint = jup_mint(output_mint);
        let mut url = format!(
            "{BASE}/order?inputMint={input_mint}&outputMint={output_mint}&amount={amount}"
        );
        if let Some(t) = taker {
            url.push_str("&taker=");
            url.push_str(t);
        }
        // 0 / None = Jupiter auto slippage (ultra mode, all routers compete).
        if let Some(bps) = slippage_bps.filter(|b| *b > 0) {
            url.push_str("&slippageBps=");
            url.push_str(&bps.to_string());
        }
        if with_referral {
            url.push_str("&referralAccount=");
            url.push_str(REFERRAL_ACCOUNT);
            url.push_str("&referralFee=");
            url.push_str(&REFERRAL_FEE_BPS.to_string());
        }
        let req = self.apply_key(self.agent().get(&url));
        let v = match req.call() {
            Ok(r) => Self::read_json(r)?,
            Err(e) => return Err(Self::map_http(e)),
        };
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            if v.get("requestId").is_none() && v.get("outAmount").is_none() {
                return Err(Error::msg(format!("jupiter: {err}")));
            }
        }
        Ok(v)
    }

    /// Quote (and optionally an assembled transaction when `taker` is set).
    pub fn order(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        taker: Option<&str>,
        slippage_bps: Option<u16>,
    ) -> Result<Order> {
        let try_ref = REFERRAL_ENABLED.load(Ordering::Relaxed);
        match self.order_json(
            input_mint,
            output_mint,
            amount,
            taker,
            slippage_bps,
            try_ref,
        ) {
            Ok(v) => parse_order(v),
            Err(e) if try_ref && referral_rejected(&e) => {
                REFERRAL_ENABLED.store(false, Ordering::Relaxed);
                let v = self.order_json(
                    input_mint,
                    output_mint,
                    amount,
                    taker,
                    slippage_bps,
                    false,
                )?;
                parse_order(v)
            }
            Err(e) => Err(e),
        }
    }

    pub fn sign_order(order: &Order, keypair: &Keypair) -> Result<String> {
        let pk = keypair.public_bytes();
        Self::sign_order_with(order, &pk, |msg| Ok(keypair.sign(msg)))
    }

    pub fn sign_order_with<F>(order: &Order, pubkey: &[u8; 32], sign: F) -> Result<String>
    where
        F: FnMut(&[u8]) -> Result<[u8; 64]>,
    {
        let b64 = order
            .transaction_b64
            .as_ref()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Error::msg("no swap transaction to sign"))?;
        let mut tx = tx::decode_b64(b64)?;
        tx::sign_wire_transaction_with(&mut tx, pubkey, sign)?;
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &tx,
        ))
    }

    pub fn execute(&self, signed_tx_b64: &str, request_id: &str) -> Result<ExecuteResult> {
        let body = json!({
            "signedTransaction": signed_tx_b64,
            "requestId": request_id,
        });
        let req = self
            .apply_key(self.agent().post(&format!("{BASE}/execute")))
            .set("Content-Type", "application/json");
        let v = match req.send_json(body) {
            Ok(r) => Self::read_json(r)?,
            Err(e) => return Err(Self::map_http(e)),
        };
        let status = v
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let signature = v
            .get("signature")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let error = v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("")
            .to_string();
        if status != "Success" && code != 0 {
            return Err(Error::msg(execute_fail_msg(code, &error, &status)));
        }
        Ok(ExecuteResult {
            status,
            signature,
            code,
            total_in: json_string(&v, "totalInputAmount"),
            total_out: json_string(&v, "totalOutputAmount"),
            error,
        })
    }
}

pub const JITO_TX_URL: &str = "https://mainnet.block-engine.jito.wtf/api/v1/transactions";
pub const JITO_TIP_ACCOUNTS: &[&str] = &[
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXlvLk",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
    "DttWaMuVvTiduZRnguLF7jNxT6dY5um3uAjKbjubovHmw",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
];

/// Race a signed swap at Jito's block engine. Failures are non-fatal.
pub fn jito_send_tx(signed_b64: &str) -> Result<String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendTransaction",
        "params": [signed_b64, { "encoding": "base64" }],
    });
    let resp = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .post(JITO_TX_URL)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| Error::msg(format!("jito: {e}")))?;
    let v: Value = resp
        .into_json()
        .map_err(|e| Error::msg(format!("jito json: {e}")))?;
    if let Some(err) = v.get("error") {
        return Err(Error::msg(format!("jito: {err}")));
    }
    Ok(v.get("result")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string())
}

fn jup_mint(mint: &str) -> String {
    let t = mint.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("native") || t.eq_ignore_ascii_case("sol") {
        crate::WSOL_MINT.to_string()
    } else {
        t.to_string()
    }
}

fn http_fail_msg(code: u16, detail: &str) -> Error {
    let low = detail.to_ascii_lowercase();
    if code == 422 && (low.contains("invalid signature") || low.contains("signature")) {
        return Error::msg(
            "Jupiter rejected the signed swap. The quote may have gone stale, or this route needed a different signature slot. Wait a second and try again.",
        );
    }
    let hint = if code == 401 {
        " Jupiter now requires an API key for higher limits. Get a free key at https://developers.jup.ag/portal and paste it in Settings."
    } else {
        ""
    };
    Error::msg(format!("jupiter HTTP {code}: {detail}{hint}"))
}

fn execute_fail_msg(code: i64, error: &str, status: &str) -> String {
    let err = if error.is_empty() { status } else { error };
    let low = err.to_ascii_lowercase();
    if code == 4615041 || low.contains("program") {
        format!(
            "Solana rejected this swap (code {code}). Often slippage, thin liquidity, or a token rule. Try a smaller amount, or swap again in a few seconds."
        )
    } else {
        format!("swap failed (code {code}): {err}")
    }
}

fn json_string(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn referral_rejected(err: &Error) -> bool {
    let text = err.to_string().to_lowercase();
    text.contains("referralaccount") || text.contains("referral account")
}

fn json_f64(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|n| n.as_f64().or_else(|| n.as_str()?.parse().ok()))
}

fn parse_order(v: Value) -> Result<Order> {
    let mut route_labels = Vec::new();
    if let Some(plan) = v.get("routePlan").and_then(|p| p.as_array()) {
        for step in plan {
            if let Some(label) = step
                .pointer("/swapInfo/label")
                .and_then(|s| s.as_str())
            {
                route_labels.push(label.to_string());
            }
        }
    }
    let tx = v.get("transaction").and_then(|t| {
        if t.is_null() {
            None
        } else {
            t.as_str().map(|s| s.to_string())
        }
    });
    let error_message = v
        .get("errorMessage")
        .and_then(|s| s.as_str())
        .or_else(|| v.get("error").and_then(|s| s.as_str()))
        .unwrap_or("")
        .to_string();
    Ok(Order {
        input_mint: json_string(&v, "inputMint"),
        output_mint: json_string(&v, "outputMint"),
        in_amount: json_string(&v, "inAmount"),
        out_amount: json_string(&v, "outAmount"),
        other_amount_threshold: json_string(&v, "otherAmountThreshold"),
        in_usd: json_f64(&v, "inUsdValue"),
        out_usd: json_f64(&v, "outUsdValue"),
        price_impact: json_f64(&v, "priceImpact"),
        slippage_bps: v.get("slippageBps").and_then(|n| n.as_u64()).map(|n| n as u16),
        router: json_string(&v, "router"),
        mode: json_string(&v, "mode"),
        fee_bps: json_f64(&v, "feeBps"),
        fee_mint: json_string(&v, "feeMint"),
        route_labels,
        request_id: json_string(&v, "requestId"),
        transaction_b64: tx.filter(|s| !s.is_empty()),
        gasless: v.get("gasless").and_then(|g| g.as_bool()).unwrap_or(false),
        error_code: v.get("errorCode").and_then(|c| c.as_i64()),
        error_message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sol_becomes_wsol() {
        assert_eq!(jup_mint("native"), crate::WSOL_MINT);
        assert_eq!(jup_mint("SOL"), crate::WSOL_MINT);
        assert_eq!(jup_mint(crate::WSOL_MINT), crate::WSOL_MINT);
    }

    #[test]
    fn program_fail_is_readable() {
        let msg = execute_fail_msg(4615041, "Program 1111 failed", "Failed");
        assert!(msg.contains("smaller amount"));
        assert!(!msg.starts_with("swap failed (code"));
    }

    #[test]
    fn http_422_signature_is_readable() {
        let msg = http_fail_msg(422, "Invalid signature for transaction").to_string();
        assert!(msg.contains("stale") || msg.contains("signature slot"), "{msg}");
        assert!(!msg.contains("HTTP 422"));
    }
}
