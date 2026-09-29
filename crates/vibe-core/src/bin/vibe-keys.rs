//! Headless key export. JSON on stdin/stdout. No GTK.

use std::io::{Read, Write};

use vibe_core::store;

fn fail(msg: impl AsRef<str>) -> i32 {
    let _ = writeln!(
        std::io::stdout(),
        "{}",
        serde_json::json!({"ok": false, "error": msg.as_ref()})
    );
    1
}

fn main() {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        std::process::exit(fail(e.to_string()));
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => std::process::exit(fail("Invalid export payload")),
    };
    let password = body
        .get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if password.len() < 8 {
        std::process::exit(fail("Password must be at least 8 characters"));
    }
    let ident = body
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let file = store::wallets_path();
    let st = match store::load_store(&file) {
        Ok(s) => s,
        Err(e) => std::process::exit(fail(e.to_string())),
    };
    if st.wallets.is_empty() {
        std::process::exit(fail("No account in this vault"));
    }
    let rec = st
        .wallets
        .iter()
        .find(|w| !ident.is_empty() && (w.id == ident || w.address == ident))
        .or_else(|| st.wallets.first());
    let Some(rec) = rec else {
        std::process::exit(fail("No account in this vault"));
    };
    let phrase = store::export_mnemonic(rec, &password).ok();
    let secret = match store::export_secret_base58(rec, &password) {
        Ok(s) => s,
        Err(_) => std::process::exit(fail("Wrong password")),
    };
    let _ = writeln!(
        std::io::stdout(),
        "{}",
        serde_json::json!({
            "ok": true,
            "id": rec.id,
            "name": rec.name,
            "address": rec.address,
            "key_type": rec.key_type,
            "phrase": phrase,
            "secret": secret,
        })
    );
}
