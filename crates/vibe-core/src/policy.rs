//! Vault rules the GTK UI cannot skip. Enforced in vibe-vault at sign time.

use crate::store::{self, ContactBook, WalletRecord};
use crate::LAMPORTS_PER_SOL;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MintRisks {
    pub freeze: bool,
    pub mint_authority: bool,
    pub permanent_delegate: bool,
    pub transfer_hook: bool,
    pub transfer_fee: bool,
}

impl MintRisks {
    pub fn extra_confirm(&self) -> bool {
        self.freeze || self.mint_authority || self.permanent_delegate || self.transfer_hook
    }

    pub fn is_empty(&self) -> bool {
        !self.freeze
            && !self.mint_authority
            && !self.permanent_delegate
            && !self.transfer_hook
            && !self.transfer_fee
    }

    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.freeze {
            out.push("This mint can still freeze your tokens.".into());
        }
        if self.mint_authority {
            out.push("Mint authority is active — supply can still grow.".into());
        }
        if self.permanent_delegate {
            out.push("Permanent delegate can move tokens without your signature.".into());
        }
        if self.transfer_hook {
            out.push("Transfer hook can run extra code on every send.".into());
        }
        if self.transfer_fee {
            out.push("Token-2022 transfer fee may be taken on this send.".into());
        }
        out
    }
}

pub fn check_max_sol(rec: &WalletRecord, sol: f64) -> Result<(), String> {
    let cap = rec.max_sol_per_tx;
    if cap <= 0.0 || sol <= 0.0 {
        return Ok(());
    }
    if sol > cap + 1e-9 {
        return Err(format!(
            "Vault rule: max {cap} SOL per tx (this is {sol:.4} SOL)"
        ));
    }
    Ok(())
}

pub fn check_max_usd(rec: &WalletRecord, usd: f64) -> Result<(), String> {
    let cap = rec.max_usd_per_tx;
    if cap <= 0.0 || usd <= 0.0 {
        return Ok(());
    }
    if usd > cap + 1e-9 {
        return Err(format!(
            "Vault rule: max ${cap} per tx (this is about ${usd:.2})"
        ));
    }
    Ok(())
}

pub fn lamports_to_sol(lamports: u64) -> f64 {
    lamports as f64 / LAMPORTS_PER_SOL as f64
}

pub fn known_payee(address: &str) -> bool {
    let dest = address.trim();
    if dest.is_empty() {
        return false;
    }
    let book = store::load_contacts(&store::desk_contacts_path());
    if contact_knows(&book, dest) {
        return true;
    }
    let vault_book = store::load_contacts(&store::contacts_path());
    if contact_knows(&vault_book, dest) {
        return true;
    }
    sent_knows(dest)
}

fn contact_knows(book: &ContactBook, dest: &str) -> bool {
    book.contacts.iter().any(|c| {
        c.address == dest && (c.trusted || c.sent_count > 0 || c.source == "sent" || c.source == "own")
    })
}

fn sent_knows(dest: &str) -> bool {
    let text = std::fs::read_to_string(store::sent_ledger_path()).unwrap_or_default();
    let val: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    if let Some(arr) = val.get("sent").and_then(|v| v.as_array()) {
        return arr.iter().any(|row| {
            row.get("address")
                .and_then(|v| v.as_str())
                .map(|s| s == dest)
                .unwrap_or(false)
        });
    }
    false
}

pub fn mint_risks_from_account(value: &serde_json::Value) -> MintRisks {
    let info = value
        .pointer("/data/parsed/info")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let mut risks = MintRisks::default();
    risks.mint_authority = nonempty_authority(info.get("mintAuthority"));
    risks.freeze = nonempty_authority(info.get("freezeAuthority"));
    if let Some(exts) = info.get("extensions").and_then(|v| v.as_array()) {
        for ext in exts {
            let kind = ext
                .get("extension")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let state = ext.get("state").cloned().unwrap_or(serde_json::Value::Null);
            if kind.contains("permanentdelegate") {
                risks.permanent_delegate = nonempty_authority(state.get("delegate"));
            } else if kind.contains("transferhook") {
                risks.transfer_hook = true;
            } else if kind.contains("transferfee") {
                risks.transfer_fee = true;
            } else if kind.contains("defaultaccountstate") {
                let frozen = state
                    .get("accountState")
                    .and_then(|v| v.as_str())
                    .map(|s| s.eq_ignore_ascii_case("frozen"))
                    .unwrap_or(false);
                if frozen {
                    risks.freeze = true;
                }
            }
        }
    }
    risks
}

fn nonempty_authority(v: Option<&serde_json::Value>) -> bool {
    match v {
        None => false,
        Some(x) if x.is_null() => false,
        Some(x) => x.as_str().map(|s| !s.trim().is_empty()).unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::WalletRecord;

    fn rec(cap: f64, extra: bool) -> WalletRecord {
        WalletRecord {
            id: "w1".into(),
            name: "Main".into(),
            address: "11111111111111111111111111111111".into(),
            key_type: "software".into(),
            derivation_path: String::new(),
            secret: None,
            mnemonic: None,
            hw_plugin: String::new(),
            hw_device_id: String::new(),
            max_sol_per_tx: cap,
            token2022_extra_confirm: extra,
            max_usd_per_tx: 0.0,
            daily_usd_cap: 0.0,
            block_infinite_approve: true,
        }
    }

    #[test]
    fn cap_off_allows_any() {
        assert!(check_max_sol(&rec(0.0, true), 50.0).is_ok());
    }

    #[test]
    fn cap_blocks_over() {
        let err = check_max_sol(&rec(1.0, true), 2.0).unwrap_err();
        assert!(err.contains("max 1"));
        assert!(err.contains("2.0000"));
    }

    #[test]
    fn cap_allows_under() {
        assert!(check_max_sol(&rec(1.0, true), 0.5).is_ok());
    }

    #[test]
    fn usd_cap_blocks_over() {
        let mut r = rec(0.0, true);
        r.max_usd_per_tx = 25.0;
        assert!(check_max_usd(&r, 10.0).is_ok());
        let err = check_max_usd(&r, 40.0).unwrap_err();
        assert!(err.contains("max $25"));
    }

    #[test]
    fn freeze_and_delegate_need_extra() {
        let mut r = MintRisks::default();
        assert!(!r.extra_confirm());
        r.permanent_delegate = true;
        assert!(r.extra_confirm());
        assert!(r.lines().iter().any(|l| l.contains("Permanent delegate")));
    }

    #[test]
    fn parse_mint_json() {
        let val = serde_json::json!({
            "data": {"parsed": {"info": {
                "mintAuthority": "Auth111111111111111111111111111111111111111",
                "freezeAuthority": null,
                "extensions": [
                    {"extension": "permanentDelegate", "state": {"delegate": "Del111111111111111111111111111111111111111"}},
                    {"extension": "transferHook", "state": {"programId": "Hook11111111111111111111111111111111111111"}}
                ]
            }}}
        });
        let r = mint_risks_from_account(&val);
        assert!(r.mint_authority);
        assert!(!r.freeze);
        assert!(r.permanent_delegate);
        assert!(r.transfer_hook);
        assert!(r.extra_confirm());
    }
}
