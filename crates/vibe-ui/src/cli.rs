// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
//! Headless create/import. GTK never starts. JSON on stdin/stdout.

use std::io::{Read, Write};

use vibe_core::mnemonic;
use vibe_core::store;

fn fail(msg: impl AsRef<str>) -> i32 {
    let _ = writeln!(
        std::io::stdout(),
        "{}",
        serde_json::json!({"ok": false, "error": msg.as_ref()})
    );
    1
}

fn ok(value: serde_json::Value) -> i32 {
    let _ = writeln!(std::io::stdout(), "{value}");
    0
}

pub fn run(args: &[String]) -> Option<i32> {
    match args.first().map(String::as_str) {
        Some("--new-mnemonic") => Some(new_mnemonic()),
        Some("--add-wallet") => Some(add_wallet()),
        Some("--derive-account") => Some(derive_account()),
        Some("--export-keys") => Some(export_keys()),
        Some("--add-watch") => Some(add_watch()),
        Some("--add-ledger") => Some(add_ledger()),
        Some("--scan-ledger") => Some(scan_ledger()),
        Some("--add-trezor") => Some(add_trezor()),
        Some("--scan-trezor") => Some(scan_trezor()),
        Some("--remove-wallet") => Some(remove_wallet()),
        Some("--rename-wallet") => Some(rename_wallet()),
        Some("--decode-tx") => Some(decode_tx()),
        _ => None,
    }
}

fn new_mnemonic() -> i32 {
    match mnemonic::generate(12) {
        Ok(phrase) => ok(serde_json::json!({"ok": true, "phrase": phrase})),
        Err(e) => fail(e.to_string()),
    }
}

fn add_wallet() -> i32 {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        return fail(e.to_string());
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return fail("Invalid import payload"),
    };
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let password = body
        .get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let phrase = body
        .get("phrase")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let secret = body
        .get("secret")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return fail("Name this wallet");
    }
    if password.len() < 8 {
        return fail("Password must be at least 8 characters");
    }
    let file = store::wallets_path();
    let mut st = match store::load_store(&file) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    if !st.wallets.is_empty() {
        if store::unlock_store(&st, &password).is_err() {
            return fail("Use this vault's existing password");
        }
    }
    let derivation = body
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let passphrase = body
        .get("passphrase")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let rec = if !phrase.is_empty() {
        store::add_mnemonic_wallet_at_path(
            &mut st,
            &name,
            &phrase,
            &password,
            &derivation,
            &passphrase,
        )
    } else if !secret.is_empty() {
        store::add_secret_wallet(&mut st, &name, &secret, &password)
    } else {
        return fail("Paste a seed or secret key");
    };
    match rec {
        Ok(rec) => match store::save_store(&file, &st) {
            Ok(()) => ok(serde_json::json!({
                "ok": true,
                "id": rec.id,
                "name": rec.name,
                "address": rec.address,
            })),
            Err(e) => fail(e.to_string()),
        },
        Err(e) => fail(e.to_string()),
    }
}

fn export_keys() -> i32 {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        return fail(e.to_string());
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return fail("Invalid export payload"),
    };
    let password = body
        .get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if password.len() < 8 {
        return fail("Password must be at least 8 characters");
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
        Err(e) => return fail(e.to_string()),
    };
    if st.wallets.is_empty() {
        return fail("No account in this vault");
    }
    let rec = st
        .wallets
        .iter()
        .find(|w| !ident.is_empty() && (w.id == ident || w.address == ident))
        .or_else(|| st.wallets.first());
    let Some(rec) = rec else {
        return fail("No account in this vault");
    };
    let phrase = store::export_mnemonic(rec, &password).ok();
    match store::export_secret_base58(rec, &password) {
        Ok(secret) => ok(serde_json::json!({
            "ok": true,
            "id": rec.id,
            "name": rec.name,
            "address": rec.address,
            "key_type": rec.key_type,
            "phrase": phrase,
            "secret": secret,
        })),
        Err(_) => fail("Wrong password"),
    }
}

fn derive_account() -> i32 {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        return fail(e.to_string());
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return fail("Invalid import payload"),
    };
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let password = body
        .get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        return fail("Name this wallet");
    }
    let file = store::wallets_path();
    let mut st = match store::load_store(&file) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    match store::add_derived_account(&mut st, &name, &password) {
        Ok(rec) => match store::save_store(&file, &st) {
            Ok(()) => ok(serde_json::json!({
                "ok": true,
                "id": rec.id,
                "name": rec.name,
                "address": rec.address,
                "path": rec.derivation_path,
            })),
            Err(e) => fail(e.to_string()),
        },
        Err(e) => fail(e.to_string()),
    }
}

fn rec_ok(rec: &store::WalletRecord) -> i32 {
    ok(serde_json::json!({
        "ok": true,
        "id": rec.id,
        "name": rec.name,
        "address": rec.address,
        "key_type": rec.key_type,
        "path": rec.derivation_path,
        "hw_device_id": rec.hw_device_id,
    }))
}

fn add_watch() -> i32 {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        return fail(e.to_string());
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return fail("Invalid payload"),
    };
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let address = body.get("address").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return fail("Name this wallet");
    }
    let file = store::wallets_path();
    let mut st = match store::load_store(&file) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    match store::add_watch_wallet(&mut st, &name, &address) {
        Ok(rec) => match store::save_store(&file, &st) {
            Ok(()) => rec_ok(&rec),
            Err(e) => fail(e.to_string()),
        },
        Err(e) => fail(e.to_string()),
    }
}

fn add_ledger() -> i32 {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        return fail(e.to_string());
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return fail("Invalid payload"),
    };
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let address = body.get("address").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let device = body.get("device_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return fail("Name this wallet");
    }
    let file = store::wallets_path();
    let mut st = match store::load_store(&file) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    match store::add_hardware_wallet(&mut st, &name, &address, "ledger", &device, &path) {
        Ok(rec) => match store::save_store(&file, &st) {
            Ok(()) => rec_ok(&rec),
            Err(e) => fail(e.to_string()),
        },
        Err(e) => fail(e.to_string()),
    }
}

fn add_trezor() -> i32 {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        return fail(e.to_string());
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return fail("Invalid payload"),
    };
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let address = body.get("address").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let device = body.get("device_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return fail("Name this wallet");
    }
    let file = store::wallets_path();
    let mut st = match store::load_store(&file) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    match store::add_hardware_wallet(&mut st, &name, &address, "trezor", &device, &path) {
        Ok(rec) => match store::save_store(&file, &st) {
            Ok(()) => rec_ok(&rec),
            Err(e) => fail(e.to_string()),
        },
        Err(e) => fail(e.to_string()),
    }
}

fn scan_ledger() -> i32 {
    let devices = match vibe_core::ledger::scan_devices() {
        Ok(d) => d,
        Err(e) => return fail(e.to_string()),
    };
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let req: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::json!({}));
    let path = req
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(vibe_core::ledger::DEFAULT_PATH);
    let display = req.get("display").and_then(|v| v.as_bool()).unwrap_or(false);
    let confirm = req.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false);
    let device_id = req.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    let mut address = String::new();
    if (display || confirm) && !devices.is_empty() {
        let pick = devices
            .iter()
            .find(|d| {
                !device_id.is_empty()
                    && (d.path == device_id || d.hidraw == device_id || d.serial == device_id)
            })
            .unwrap_or(&devices[0]);
        match vibe_core::ledger::get_address(&pick.path, path, display || confirm) {
            Ok(a) => address = a,
            Err(e) => return fail(e.to_string()),
        }
    }
    ok(serde_json::json!({
        "ok": true,
        "devices": devices.iter().map(|d| serde_json::json!({
            "path": d.path,
            "hidraw": d.hidraw,
            "model": d.model,
            "serial": d.serial,
        })).collect::<Vec<_>>(),
        "address": address,
        "path": path,
    }))
}

fn scan_trezor() -> i32 {
    let devices = match vibe_core::trezor::scan_devices() {
        Ok(d) => d,
        Err(e) => return fail(e.to_string()),
    };
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let req: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::json!({}));
    let path = req
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(vibe_core::trezor::DEFAULT_PATH);
    let display = req.get("display").and_then(|v| v.as_bool()).unwrap_or(false);
    let confirm = req.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false);
    let device_id = req.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    let mut address = String::new();
    if (display || confirm) && !devices.is_empty() {
        let pick = devices
            .iter()
            .find(|d| {
                !device_id.is_empty()
                    && (d.path == device_id || d.hidraw == device_id || d.serial == device_id)
            })
            .unwrap_or(&devices[0]);
        match vibe_core::trezor::get_address(&pick.path, path, display || confirm) {
            Ok(a) => address = a,
            Err(e) => return fail(e.to_string()),
        }
    }
    ok(serde_json::json!({
        "ok": true,
        "devices": devices.iter().map(|d| serde_json::json!({
            "path": d.path,
            "hidraw": d.hidraw,
            "model": d.model,
            "serial": d.serial,
            "transport": d.transport,
            "plugin": "trezor",
        })).collect::<Vec<_>>(),
        "address": address,
        "path": path,
    }))
}

fn remove_wallet() -> i32 {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        return fail(e.to_string());
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return fail("Invalid payload"),
    };
    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if id.is_empty() {
        return fail("Need a wallet id");
    }
    let file = store::wallets_path();
    let mut st = match store::load_store(&file) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    match store::remove_wallet(&mut st, &id) {
        Ok(rec) => match store::save_store(&file, &st) {
            Ok(()) => rec_ok(&rec),
            Err(e) => fail(e.to_string()),
        },
        Err(e) => fail(e.to_string()),
    }
}

fn rename_wallet() -> i32 {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        return fail(e.to_string());
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return fail("Invalid payload"),
    };
    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if id.is_empty() || name.is_empty() {
        return fail("Need a wallet id and name");
    }
    let file = store::wallets_path();
    let mut st = match store::load_store(&file) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    if !store::rename_wallet(&mut st, &id, &name) {
        return fail("Wallet not found");
    }
    match store::save_store(&file, &st) {
        Ok(()) => ok(serde_json::json!({"ok": true, "id": id, "name": name})),
        Err(e) => fail(e.to_string()),
    }
}

fn decode_tx() -> i32 {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        return fail(e.to_string());
    }
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return fail("Invalid payload"),
    };
    if let Some(sig) = body.get("signature").and_then(|v| v.as_str()) {
        let rpc_url = body
            .get("rpc")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| store::load_settings(&store::settings_path()).rpc_url());
        let rpc = vibe_core::Rpc::new(rpc_url);
        return match rpc.get_transaction(sig) {
            Ok(tx) => {
                let ixs = vibe_core::decode::decode_json_parsed(&tx);
                ok(serde_json::json!({
                    "ok": true,
                    "kind": vibe_core::decode::classify(&ixs),
                    "instructions": ixs.iter().map(|i| i.to_json()).collect::<Vec<_>>(),
                }))
            }
            Err(e) => fail(e.to_string()),
        };
    }
    if let Some(b64) = body.get("message_b64").and_then(|v| v.as_str()) {
        let rpc_url = body
            .get("rpc")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| store::load_settings(&store::settings_path()).rpc_url());
        let rpc = vibe_core::Rpc::new(rpc_url);
        return match vibe_core::tx::decode_b64(b64).and_then(|b| {
            vibe_core::decode::decode_compiled_message_with(&b, |k| {
                rpc.get_account(k).map_err(|e| e.to_string())
            })
        }) {
            Ok(ixs) => ok(serde_json::json!({
                "ok": true,
                "kind": vibe_core::decode::classify(&ixs),
                "instructions": ixs.iter().map(|i| i.to_json()).collect::<Vec<_>>(),
            })),
            Err(e) => fail(e.to_string()),
        };
    }
    fail("Need signature or message_b64")
}
