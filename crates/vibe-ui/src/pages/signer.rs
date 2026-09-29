//! Compact confirm + password sheet. Visual transfer, little copy.

use std::path::PathBuf;

use adw::prelude::*;
use vibe_core::format::parse_ui_amount;
use vibe_core::jupiter;
use vibe_core::{tx, ProgramIds, WSOL_MINT};

use crate::ctx::Ctx;
use crate::dialogs;
use crate::paths;
use crate::task;

fn env_trim(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

fn env_flag(key: &str) -> bool {
    matches!(
        std::env::var(key).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes")
    )
}

fn env_u32(key: &str, fallback: u32) -> u32 {
    env_trim(key).parse().ok().filter(|v| *v > 0).unwrap_or(fallback)
}

fn env_u64(key: &str, fallback: u64) -> u64 {
    env_trim(key).parse().ok().unwrap_or(fallback)
}

fn mint_token_program() -> &'static str {
    let raw = env_trim("VIBE_MINT_PROGRAM");
    if raw == ProgramIds::TOKEN
        || raw.eq_ignore_ascii_case("spl")
        || raw.eq_ignore_ascii_case("token")
    {
        ProgramIds::TOKEN
    } else {
        ProgramIds::TOKEN_2022
    }
}

fn mint_keypair_from_env() -> Result<vibe_core::Keypair, String> {
    let raw = env_trim("VIBE_MINT_SECRET");
    if raw.is_empty() {
        return Ok(vibe_core::Keypair::generate());
    }
    if let Ok(arr) = serde_json::from_str::<Vec<u8>>(&raw) {
        if arr.len() == 64 {
            return vibe_core::Keypair::from_secret64(&arr).map_err(|e| e.to_string());
        }
        if arr.len() == 32 {
            return vibe_core::Keypair::from_seed32(&arr).map_err(|e| e.to_string());
        }
        return Err("Mint secret must be 32 or 64 bytes".into());
    }
    let bytes = bs58::decode(raw.trim())
        .into_vec()
        .map_err(|_| "Mint secret is not valid base58".to_string())?;
    if bytes.len() == 64 {
        return vibe_core::Keypair::from_secret64(&bytes).map_err(|e| e.to_string());
    }
    if bytes.len() == 32 {
        return vibe_core::Keypair::from_seed32(&bytes).map_err(|e| e.to_string());
    }
    Err("Mint secret must be a 64-byte keypair".into())
}

fn mint_max_fee(decimals: u8, supply: u64) -> u64 {
    let raw = env_trim("VIBE_MINT_FEE_MAX");
    if raw.is_empty() || raw == "0" {
        return u64::MAX;
    }
    parse_ui_amount(&raw, decimals).unwrap_or(supply.max(1))
}

fn with_budget(ixs: Vec<tx::Instruction>) -> Vec<tx::Instruction> {
    with_budget_units(ixs, 80_000)
}

/// Priority fee picked in the Confirm window (µ-lamports/CU). `FEE_UNSET` = not picked here
/// (use the desk's env suggestion; for swaps, let Jupiter decide).
static CHOSEN_MICRO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(FEE_UNSET);
const FEE_UNSET: u64 = u64::MAX;
/// CU budget used to turn a µ-lamport price into Jupiter's `priorityFeeLamports`.
const SWAP_FEE_UNITS: u32 = 400_000;
/// Confirm warns above this total priority fee (0.001 SOL) or above this price.
const CONFIRM_WARN_LAMPORTS: u64 = 1_000_000;
const CONFIRM_WARN_MICRO: u64 = 1_000_000;

fn chosen_micro() -> Option<u64> {
    match CHOSEN_MICRO.load(std::sync::atomic::Ordering::SeqCst) {
        FEE_UNSET => None,
        v => Some(v),
    }
}

fn set_chosen_micro(v: Option<u64>) {
    CHOSEN_MICRO.store(v.unwrap_or(FEE_UNSET), std::sync::atomic::Ordering::SeqCst);
}

/// Jupiter `priorityFeeLamports` for the Confirm choice (None = Jupiter auto).
fn swap_priority_lamports() -> Option<u64> {
    chosen_micro().map(|m| vibe_core::fees::priority_fee_lamports(SWAP_FEE_UNITS, m))
}

fn with_budget_units(mut ixs: Vec<tx::Instruction>, default_units: u32) -> Vec<tx::Instruction> {
    let micro = chosen_micro().unwrap_or_else(|| env_u64("VIBE_PRIORITY_MICROLAMPORTS", 25_000));
    if micro == 0 {
        return ixs;
    }
    let units = env_u32("VIBE_COMPUTE_UNITS", default_units);
    let mut out = vec![
        tx::set_compute_unit_limit(units),
        tx::set_compute_unit_price(micro),
    ];
    out.append(&mut ixs);
    out
}

fn send_memo_ix(payer: &str) -> Option<tx::Instruction> {
    let raw = env_trim("VIBE_SEND_MEMO");
    if raw.is_empty() {
        return None;
    }
    let clipped: String = raw.chars().take(200).collect();
    Some(tx::memo(payer, &clipped))
}

fn is_native_mint(mint: &str) -> bool {
    mint.is_empty()
        || mint.eq_ignore_ascii_case("native")
        || mint.eq_ignore_ascii_case("sol")
        || mint == WSOL_MINT
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Send,
    Swap,
    Stake,
    Burn,
    Reclaim,
    Wc,
    Mint,
    Nonce,
    Mass,
    Bot,
}

fn kind() -> Kind {
    if !env_trim("VIBE_WC_REQUEST").is_empty() {
        Kind::Wc
    } else if !env_trim("VIBE_BOT_TASK").is_empty() {
        Kind::Bot
    } else if !env_trim("VIBE_MINT_ACTION").is_empty()
        || !env_trim("VIBE_MINT_SYMBOL").is_empty()
        || !env_trim("VIBE_MINT_NAME").is_empty()
        || !env_trim("VIBE_MINT_MINT").is_empty()
    {
        Kind::Mint
    } else if !env_trim("VIBE_NONCE_ACTION").is_empty() {
        Kind::Nonce
    } else if !env_trim("VIBE_MASS").is_empty()
        || !env_trim("VIBE_MASS_DESTS").is_empty()
        || env_trim("VIBE_MASS_COUNT").parse::<u32>().unwrap_or(0) > 0
    {
        Kind::Mass
    } else if !env_trim("VIBE_SWAP_OUT").is_empty() || !env_trim("VIBE_SWAP_IN").is_empty() {
        Kind::Swap
    } else if !env_trim("VIBE_STAKE_ACTION").is_empty() {
        Kind::Stake
    } else if !env_trim("VIBE_BURN_MINT").is_empty() {
        Kind::Burn
    } else if !env_trim("VIBE_RECLAIM_ACCOUNTS").is_empty() {
        Kind::Reclaim
    } else {
        Kind::Send
    }
}

#[derive(Clone, Default)]
struct ReviewPaint {
    blocked: Option<String>,
    reds: Vec<String>,
    warns: Vec<String>,
    infos: Vec<String>,
    extra: bool,
}

fn env_sol_amount(k: Kind) -> f64 {
    let raw = match k {
        Kind::Send if is_native_mint(&env_trim("VIBE_SEND_MINT")) => env_trim("VIBE_SEND_AMOUNT"),
        Kind::Mass if is_native_mint(&env_trim("VIBE_SEND_MINT")) => {
            let each = env_trim("VIBE_SEND_AMOUNT").replace(',', "").parse::<f64>().unwrap_or(0.0);
            let n = env_trim("VIBE_MASS_COUNT").parse::<f64>().unwrap_or(0.0);
            return each * n;
        }
        Kind::Stake => env_trim("VIBE_STAKE_AMOUNT"),
        Kind::Swap if is_native_mint(&env_trim("VIBE_SWAP_IN")) => env_trim("VIBE_SWAP_AMOUNT"),
        Kind::Reclaim => env_trim("VIBE_RECLAIM_SOL"),
        Kind::Bot => env_trim("VIBE_BOT_TOTAL"),
        _ => String::new(),
    };
    raw.trim().replace(',', "").parse::<f64>().unwrap_or(0.0)
}

fn env_usd_amount() -> f64 {
    let raw = env_trim("VIBE_TX_USD");
    if raw.is_empty() {
        env_trim("VIBE_SEND_USD")
            .trim()
            .replace(',', "")
            .parse::<f64>()
            .unwrap_or(0.0)
    } else {
        raw.trim().replace(',', "").parse::<f64>().unwrap_or(0.0)
    }
}

fn review_mints(k: Kind) -> Vec<String> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<String>, raw: String| {
        let s = raw.trim().to_string();
        if !s.is_empty() && !is_native_mint(&s) && vibe_core::format::looks_like_pubkey(&s) {
            if !out.iter().any(|m| m == &s) {
                out.push(s);
            }
        }
    };
    match k {
        Kind::Send | Kind::Mass => push(&mut out, env_trim("VIBE_SEND_MINT")),
        Kind::Swap => {
            push(&mut out, env_trim("VIBE_SWAP_IN"));
            push(&mut out, env_trim("VIBE_SWAP_OUT"));
        }
        Kind::Burn => push(&mut out, env_trim("VIBE_BURN_MINT")),
        Kind::Bot => push(&mut out, env_trim("VIBE_BOT_MINT")),
        _ => {}
    }
    out
}

fn collect_review(k: Kind, rec: vibe_core::store::WalletRecord, rpc: vibe_core::Rpc) -> ReviewPaint {
    let mut paint = ReviewPaint::default();
    match agent_task_check(&rec) {
        Ok(Some(task)) => paint.warns.push(format!(
            "Requested by AI agent “{}” (task {}, {}). Check every detail before you sign.",
            task.agent_name,
            task.id,
            if task.approved_by_human { "approved on the desk" } else { "within its caps" }
        )),
        Ok(None) => {}
        Err(e) => {
            paint.blocked = Some(e);
            return paint;
        }
    }
    let sol = env_sol_amount(k);
    if let Err(e) = vibe_core::policy::check_max_sol(&rec, sol) {
        paint.blocked = Some(e);
    }
    if let Err(e) = vibe_core::policy::check_max_usd(&rec, env_usd_amount()) {
        paint.blocked = Some(e);
    }
    if k == Kind::Send {
        let dest = env_trim("VIBE_SEND_DEST");
        if vibe_core::format::looks_like_pubkey(&dest) && !vibe_core::policy::known_payee(&dest) {
            paint.warns.push("You've never paid this pubkey.".into());
        }
    }
    if k == Kind::Mass {
        let n = env_trim("VIBE_MASS_COUNT").if_empty("0");
        let each = env_trim("VIBE_SEND_AMOUNT").if_empty("0");
        let sym = env_trim("VIBE_SEND_SYMBOL").if_empty("TOKEN");
        paint.infos.push(format!("{n} wallets × {each} {sym} each."));
        paint.warns.push("Each batch is its own transaction. Failed batches after the first still spend the ones that landed.".into());
    }
    if k == Kind::Bot {
        let n = env_trim("VIBE_BOT_COUNT").if_empty("0");
        let mode = env_trim("VIBE_BOT_MODE").if_empty("volume");
        let sym = env_trim("VIBE_BOT_SYMBOL").if_empty("TOKEN");
        paint.infos.push(format!("Trading bot · {mode} · {n} trades · {sym}."));
        paint.warns.push("Leave this vault window open while the bot runs. Closing it stops the task.".into());
    }
    if k == Kind::Reclaim {
        let n = env_trim("VIBE_RECLAIM_COUNT").if_empty("0");
        let sol_s = env_trim("VIBE_RECLAIM_SOL").if_empty("0");
        paint.infos.push(format!("You get back {sol_s} SOL from {n} account(s)."));
    }
    if k == Kind::Wc {
        match wc_request() {
            Ok(req) => {
                let reviews = review_wc_request(&req, &rec, &rpc);
                paint_wc_reviews(&mut paint, &reviews);
                // 1.9 F8: extra, display-only Anchor IDL lines (never part of the review
                // or the pre-sign re-check above).
                let idl = anchor_idl_lines(&req, &rpc);
                if !idl.is_empty() {
                    paint.infos.push(
                        "Anchor IDL decode — names come from the program's own IDL. Display only: the vault checks above are unchanged.".into(),
                    );
                    paint.infos.extend(idl);
                }
            }
            Err(e) => paint.blocked = Some(format!("Blocked by the vault: {e}")),
        }
        return paint;
    }
    let mut extra = false;
    for mint in review_mints(k) {
        match rpc.mint_risks(&mint) {
            Ok(risks) => {
                if rec.token2022_extra_confirm && risks.extra_confirm() {
                    extra = true;
                }
                paint.reds.extend(risks.lines());
            }
            Err(e) => paint.warns.push(format!("Could not inspect mint: {e}")),
        }
    }
    paint.extra = extra;
    if k == Kind::Send && is_native_mint(&env_trim("VIBE_SEND_MINT")) {
        match simulate_native_send(&rpc, &rec.address) {
            Ok(msg) => paint.infos.push(msg),
            Err(e) => {
                let low = e.to_lowercase();
                if low.contains("insufficient") || low.contains("custom program error") {
                    paint.blocked = Some(format!("Simulation failed: {e}"));
                } else {
                    paint.warns.push(format!("Could not simulate yet: {e}"));
                }
            }
        }
    }
    // 1.9: which RPC this Confirm is using (host only, never the API key).
    paint.infos.push(format!("RPC · {}", rpc.active_label()));
    paint
}

fn simulate_native_send(rpc: &vibe_core::Rpc, from: &str) -> Result<String, String> {
    let dest = env_trim("VIBE_SEND_DEST");
    let amount = env_trim("VIBE_SEND_AMOUNT");
    let lamports = vibe_core::parse_sol(&amount).map_err(|e| e.to_string())?;
    if !vibe_core::format::looks_like_pubkey(&dest) {
        return Err("Invalid destination".into());
    }
    let hash = rpc.get_latest_blockhash().map_err(|e| e.to_string())?;
    let ixs = vec![tx::system_transfer(from, &dest, lamports)];
    let bytes = tx::unsigned_legacy(from, &hash, &ixs).map_err(|e| e.to_string())?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    let logs = rpc.simulate(&b64).map_err(|e| e.to_string())?;
    let units = logs
        .lines()
        .next()
        .unwrap_or("simulated")
        .trim()
        .to_string();
    Ok(format!("Simulated · {units}"))
}

fn reload_active_record(ctx: &Ctx) -> Option<vibe_core::store::WalletRecord> {
    let store = vibe_core::store::load_store(&vibe_core::store::wallets_path()).ok()?;
    let want = env_trim("VIBE_ACTIVE_ID");
    let target = if !want.is_empty() {
        want
    } else {
        store.active_id.clone()
    };
    store
        .wallets
        .into_iter()
        .find(|w| w.id == target || w.address == target)
}

/// 1.9: a Confirm opened for an AI agent task (`VIBE_AGENT_TASK`) is re-checked against the
/// vault agent policy: agent not revoked, bound to this wallet, same parameters, caps.
fn agent_task_check(rec: &vibe_core::store::WalletRecord) -> Result<Option<vibe_core::agents::Task>, String> {
    let task = env_trim("VIBE_AGENT_TASK");
    if task.is_empty() {
        return Ok(None);
    }
    let flow = match kind() {
        Kind::Send => "send",
        Kind::Swap => "swap",
        Kind::Stake => "stake",
        Kind::Nonce => "nonce",
        Kind::Mint => "mint",
        _ => "other",
    };
    vibe_core::agents::verify_for_confirm(&task, &rec.id, flow, |k| env_trim(k))
        .map(Some)
        .map_err(|e| format!("Blocked by the vault agent policy: {e}"))
}

fn agent_record_result(res: &Result<String, String>) {
    let task = env_trim("VIBE_AGENT_TASK");
    if !task.is_empty() {
        let _ = vibe_core::agents::record_result(&task, res.clone());
    }
}

fn enforce_vault_rules(ctx: &Ctx, k: Kind, extra_ack: bool) -> Result<(), String> {
    let rec = reload_active_record(ctx).ok_or_else(|| "No wallet in this vault".to_string())?;
    agent_task_check(&rec)?;
    vibe_core::policy::check_max_sol(&rec, env_sol_amount(k))?;
    vibe_core::policy::check_max_usd(&rec, env_usd_amount())?;
    if !rec.token2022_extra_confirm {
        return Ok(());
    }
    let rpc = ctx.rpc();
    let mut need = false;
    for mint in review_mints(k) {
        if let Ok(risks) = rpc.mint_risks(&mint) {
            if risks.extra_confirm() {
                need = true;
            }
        }
    }
    if need && !extra_ack {
        return Err("Tick the extra confirm — this mint can freeze, mint, or take tokens".into());
    }
    Ok(())
}

fn label_line(text: &str, class: &str) -> gtk::Label {
    let lab = gtk::Label::new(Some(text));
    lab.add_css_class(class);
    lab.set_wrap(true);
    lab.set_xalign(0.0);
    lab.set_max_width_chars(42);
    lab
}

fn send_signed(
    rpc: &vibe_core::Rpc,
    wallet: &crate::ctx::Unlocked,
    ixs: Vec<tx::Instruction>,
) -> Result<String, String> {
    let from = wallet.address.clone();
    let nonce = env_trim("VIBE_SEND_NONCE");
    let mut ixs = with_budget(ixs);
    let blockhash = if !nonce.is_empty() {
        let na = rpc.get_nonce(&nonce).map_err(|e| e.to_string())?;
        if !na.is_initialized() {
            return Err("Nonce account is not initialized".into());
        }
        ixs = tx::with_durable_nonce(&nonce, &from, ixs);
        na.nonce
    } else {
        rpc.get_latest_blockhash().map_err(|e| e.to_string())?
    };
    let bytes = wallet.sign_legacy(&from, &blockhash, &ixs)?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    rpc.simulate(&b64).map_err(|e| e.to_string())?;
    rpc.send_transaction(&bytes).map_err(|e| e.to_string())
}

fn send_signed_extra(
    rpc: &vibe_core::Rpc,
    wallet: &crate::ctx::Unlocked,
    ixs: Vec<tx::Instruction>,
    extra: &vibe_core::Keypair,
) -> Result<String, String> {
    let from = wallet.address.clone();
    let ixs = with_budget_units(ixs, 400_000);
    let blockhash = rpc.get_latest_blockhash().map_err(|e| e.to_string())?;
    let user_pk = wallet.public_bytes()?;
    let extra_pk = extra.public_bytes();
    let bytes = tx::build_signed_legacy_with(
        &from,
        &blockhash,
        &ixs,
        &[user_pk, extra_pk],
        |i, msg| {
            if i == 0 {
                wallet
                    .sign_message(msg)
                    .map_err(|e| vibe_core::Error::msg(e))
            } else {
                Ok(extra.sign(msg))
            }
        },
    )
    .map_err(|e| e.to_string())?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    rpc.simulate(&b64).map_err(|e| e.to_string())?;
    rpc.send_transaction(&bytes).map_err(|e| e.to_string())
}

pub fn build(ctx: &Ctx) -> gtk::Widget {
    if env_flag("VIBE_REVEAL") {
        return build_reveal(ctx);
    }
    if env_flag("VIBE_CREATE") {
        return build_manage(ctx, false);
    }
    if env_flag("VIBE_IMPORT") {
        return build_manage(ctx, true);
    }
    build_sign(ctx)
}

fn copy_text(text: &str) {
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(text);
    }
}

fn build_reveal(ctx: &Ctx) -> gtk::Widget {
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shell.add_css_class("og-signer-shell");
    shell.set_hexpand(true);
    shell.set_vexpand(true);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("og-card");
    card.add_css_class("og-signer-card");
    card.set_hexpand(true);
    card.set_vexpand(true);

    let kicker = gtk::Label::new(Some("VIEW KEYS"));
    kicker.add_css_class("og-signer-kicker");
    kicker.set_halign(gtk::Align::Center);

    let hint = gtk::Label::new(Some(
        "Confirm with the vault password. Anyone with this seed or key can spend. This window closes itself.",
    ));
    hint.add_css_class("og-signer-hint");
    hint.set_wrap(true);
    hint.set_justify(gtk::Justification::Center);
    hint.set_halign(gtk::Align::Center);
    hint.set_max_width_chars(36);

    let pass = gtk::PasswordEntry::new();
    pass.set_show_peek_icon(true);
    pass.set_placeholder_text(Some("Vault password"));
    pass.set_hexpand(true);
    pass.add_css_class("og-signer-pass");

    let status = gtk::Label::new(Some("On-device · never sent"));
    status.add_css_class("og-signer-secure");
    status.set_wrap(true);

    let go = gtk::Button::with_label("Reveal");
    go.add_css_class("og-primary");
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("og-ghost");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_homogeneous(true);
    row.append(&cancel);
    row.append(&go);

    let keys = gtk::Box::new(gtk::Orientation::Vertical, 10);
    keys.set_visible(false);
    keys.set_hexpand(true);

    let name_l = gtk::Label::new(None);
    name_l.add_css_class("og-signer-hint");
    name_l.set_wrap(true);
    name_l.set_selectable(true);
    name_l.set_justify(gtk::Justification::Center);
    let seed_l = gtk::Label::new(None);
    seed_l.add_css_class("og-signer-hint");
    seed_l.set_wrap(true);
    seed_l.set_selectable(true);
    seed_l.set_justify(gtk::Justification::Center);
    seed_l.set_max_width_chars(36);
    let secret_l = gtk::Label::new(None);
    secret_l.add_css_class("og-signer-hint");
    secret_l.set_wrap(true);
    secret_l.set_selectable(true);
    secret_l.set_justify(gtk::Justification::Center);
    secret_l.set_max_width_chars(36);
    let copy_seed = gtk::Button::with_label("Copy seed");
    copy_seed.add_css_class("og-ghost");
    copy_seed.set_visible(false);
    let copy_key = gtk::Button::with_label("Copy private key");
    copy_key.add_css_class("og-ghost");
    let copies = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    copies.set_homogeneous(true);
    copies.append(&copy_seed);
    copies.append(&copy_key);
    keys.append(&name_l);
    keys.append(&seed_l);
    keys.append(&secret_l);
    keys.append(&copies);

    let win = ctx.window.clone();
    cancel.connect_clicked(move |_| {
        win.close();
    });
    let xbtn = gtk::Button::from_icon_name("window-close-symbolic");
    xbtn.add_css_class("flat");
    xbtn.add_css_class("circular");
    xbtn.add_css_class("og-signer-close");
    xbtn.set_tooltip_text(Some("Close"));
    xbtn.set_halign(gtk::Align::End);
    xbtn.set_valign(gtk::Align::Start);
    xbtn.set_margin_top(6);
    xbtn.set_margin_end(6);
    let win_x = ctx.window.clone();
    xbtn.connect_clicked(move |_| {
        win_x.close();
    });

    let ctx_s = ctx.clone();
    let pass_s = pass.clone();
    let status_s = status.clone();
    let go_s = go.clone();
    let keys_s = keys.clone();
    let name_s = name_l.clone();
    let seed_s = seed_l.clone();
    let secret_s = secret_l.clone();
    let copy_seed_s = copy_seed.clone();
    let copy_key_s = copy_key.clone();
    let hint_s = hint.clone();
    let win_close = ctx.window.clone();
    let reveal = move || {
        if !go_s.is_sensitive() {
            return;
        }
        status_s.set_text("Unlocking…");
        let password = pass_s.text().to_string();
        match dialogs::try_unlock(&ctx_s, &password) {
            Ok(()) => {}
            Err(e) => {
                status_s.set_text(&e);
                ctx_s.toast(e);
                return;
            }
        }
        let store = ctx_s.state.borrow().store.clone();
        let want = env_trim("VIBE_ACTIVE_ID");
        let rec = store
            .wallets
            .iter()
            .find(|w| !want.is_empty() && w.id == want)
            .or_else(|| store.wallets.first());
        let Some(rec) = rec.cloned() else {
            status_s.set_text("No account in this vault");
            return;
        };
        let phrase = vibe_core::store::export_mnemonic(&rec, &password).ok();
        let secret = match vibe_core::store::export_secret_base58(&rec, &password) {
            Ok(s) => s,
            Err(e) => {
                status_s.set_text(&e.to_string());
                return;
            }
        };
        pass_s.set_text("");
        pass_s.set_visible(false);
        go_s.set_visible(false);
        name_s.set_text(&format!("{} · {}", rec.name, rec.address));
        if let Some(phrase) = phrase.clone() {
            seed_s.set_text(&format!("Seed\n{phrase}"));
            seed_s.set_visible(true);
            copy_seed_s.set_visible(true);
            let phrase_c = phrase;
            copy_seed_s.connect_clicked(move |_| {
                copy_text(&phrase_c);
                glib::timeout_add_seconds_local(30, || {
                    copy_text("");
                    glib::ControlFlow::Break
                });
            });
        } else {
            seed_s.set_visible(false);
            copy_seed_s.set_visible(false);
        }
        secret_s.set_text(&format!("Private key\n{secret}"));
        let secret_c = secret.clone();
        copy_key_s.connect_clicked(move |_| {
            copy_text(&secret_c);
            glib::timeout_add_seconds_local(30, || {
                copy_text("");
                glib::ControlFlow::Break
            });
        });
        keys_s.set_visible(true);
        hint_s.set_text("Write it down offline. This window closes in 60 seconds.");
        status_s.set_text("Closes in 60s · clipboard wipe 30s after copy");
        let win_t = win_close.clone();
        glib::timeout_add_seconds_local(60, move || {
            win_t.close();
            glib::ControlFlow::Break
        });
    };
    let reveal_click = reveal.clone();
    go.connect_clicked(move |_| reveal_click());
    pass.connect_activate(move |_| reveal());

    card.append(&kicker);
    card.append(&hint);
    card.append(&pass);
    card.append(&row);
    card.append(&keys);
    card.append(&status);
    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    overlay.set_child(Some(&card));
    overlay.add_overlay(&xbtn);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&overlay));
    shell.append(&scroll);
    glib::idle_add_local_once({
        let pass = pass.clone();
        move || {
            pass.grab_focus();
        }
    });
    shell.upcast()
}

fn default_wallet_name(ctx: &Ctx) -> String {
    let n = ctx.state.borrow().store.wallets.len();
    if n == 0 {
        "Main".into()
    } else {
        format!("Wallet {}", n + 1)
    }
}

fn build_manage(ctx: &Ctx, import: bool) -> gtk::Widget {
    let existing = !ctx.state.borrow().store.wallets.is_empty();
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shell.add_css_class("og-signer-shell");
    shell.set_hexpand(true);
    shell.set_vexpand(true);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("og-card");
    card.add_css_class("og-signer-card");
    card.set_hexpand(true);
    card.set_vexpand(true);

    let kicker = gtk::Label::new(Some(if import {
        "IMPORT WALLET"
    } else {
        "CREATE WALLET"
    }));
    kicker.add_css_class("og-signer-kicker");
    kicker.set_halign(gtk::Align::Center);

    let hint = gtk::Label::new(Some(if import {
        "Paste a 12/24-word seed or a JSON / base58 secret key. Encrypted on this machine."
    } else if existing {
        "New 12-word seed, encrypted with this vault's password."
    } else {
        "A 12-word seed is generated here and encrypted on this machine."
    }));
    hint.add_css_class("og-signer-hint");
    hint.set_wrap(true);
    hint.set_justify(gtk::Justification::Center);
    hint.set_halign(gtk::Align::Center);
    hint.set_max_width_chars(36);

    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Wallet name"));
    name.set_text(&default_wallet_name(ctx));
    name.set_hexpand(true);
    name.add_css_class("og-signer-pass");

    let secret = gtk::TextView::new();
    secret.set_wrap_mode(gtk::WrapMode::Word);
    secret.set_hexpand(true);
    secret.add_css_class("og-signer-secret");
    let sw = gtk::ScrolledWindow::new();
    sw.set_min_content_height(88);
    sw.set_child(Some(&secret));
    sw.add_css_class("og-signer-secret-wrap");
    sw.set_visible(import);

    let pass = gtk::PasswordEntry::new();
    pass.set_show_peek_icon(true);
    pass.set_placeholder_text(Some(if existing {
        "Vault password"
    } else {
        "Password (min 8 characters)"
    }));
    pass.set_hexpand(true);
    pass.add_css_class("og-signer-pass");

    let pass2 = gtk::PasswordEntry::new();
    pass2.set_show_peek_icon(true);
    pass2.set_placeholder_text(Some("Confirm password"));
    pass2.set_hexpand(true);
    pass2.add_css_class("og-signer-pass");
    pass2.set_visible(!existing);

    let status = gtk::Label::new(Some("On-device · keys stay in the vault"));
    status.add_css_class("og-signer-secure");
    status.set_wrap(true);
    status.set_justify(gtk::Justification::Center);
    status.set_halign(gtk::Align::Center);

    let go = gtk::Button::with_label(if import { "Import" } else { "Generate seed" });
    go.add_css_class("og-primary");
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("og-ghost");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_homogeneous(true);
    row.append(&cancel);
    row.append(&go);

    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    form.append(&kicker);
    form.append(&hint);
    form.append(&name);
    form.append(&sw);
    form.append(&pass);
    form.append(&pass2);
    form.append(&row);
    form.append(&status);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.add_named(&form, Some("form"));

    if !import {
        let reveal = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let save_kicker = gtk::Label::new(Some("SAVE YOUR SEED"));
        save_kicker.add_css_class("og-signer-kicker");
        save_kicker.set_halign(gtk::Align::Center);
        let warn = gtk::Label::new(Some(
            "Write these 12 words down. Anyone with this seed can spend. VIBE cannot recover it.",
        ));
        warn.add_css_class("og-signer-hint");
        warn.set_wrap(true);
        warn.set_justify(gtk::Justification::Center);
        warn.set_halign(gtk::Align::Center);
        warn.set_max_width_chars(36);
        let words = gtk::Label::new(None);
        words.add_css_class("og-mnemonic");
        words.set_wrap(true);
        words.set_selectable(true);
        words.set_justify(gtk::Justification::Center);
        words.set_xalign(0.5);
        let saved = gtk::CheckButton::with_label("I wrote this seed down and stored it offline");
        saved.add_css_class("og-signer-check");
        let finish = gtk::Button::with_label("Encrypt and save");
        finish.add_css_class("og-primary");
        finish.set_sensitive(false);
        let close_seed = gtk::Button::with_label("Cancel");
        close_seed.add_css_class("og-ghost");
        let seed_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        seed_row.set_homogeneous(true);
        seed_row.append(&close_seed);
        seed_row.append(&finish);
        reveal.append(&save_kicker);
        reveal.append(&warn);
        reveal.append(&words);
        reveal.append(&saved);
        reveal.append(&seed_row);
        stack.add_named(&reveal, Some("seed"));

        let finish_c = finish.clone();
        saved.connect_toggled(move |btn| finish_c.set_sensitive(btn.is_active()));

        let phrase = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let ctx_g = ctx.clone();
        let stack_c = stack.clone();
        let words_c = words.clone();
        let phrase_c = phrase.clone();
        let name_n = name.clone();
        let pass_n = pass.clone();
        let pass2_n = pass2.clone();
        let status_n = status.clone();
        let existing_n = existing;
        go.connect_clicked(move |_| {
            let n = name_n.text().to_string();
            let p1 = pass_n.text().to_string();
            let p2 = pass2_n.text().to_string();
            if n.trim().is_empty() {
                status_n.set_text("Name this wallet");
                ctx_g.toast("Name this wallet");
                return;
            }
            if !existing_n && p1 != p2 {
                status_n.set_text("Passwords do not match");
                ctx_g.toast("Passwords do not match");
                return;
            }
            match dialogs::require_vault_password(&ctx_g, &p1) {
                Ok(()) => {}
                Err(e) => {
                    status_n.set_text(&e);
                    ctx_g.toast(e);
                    return;
                }
            }
            match vibe_core::mnemonic::generate(12) {
                Ok(ph) => {
                    words_c.set_text(&ph);
                    *phrase_c.borrow_mut() = ph;
                    stack_c.set_visible_child_name("seed");
                }
                Err(e) => {
                    let msg = format!("{e}");
                    status_n.set_text(&msg);
                    ctx_g.toast(msg);
                }
            }
        });

        let ctx_f = ctx.clone();
        let name_f = name.clone();
        let pass_f = pass.clone();
        let saved_f = saved.clone();
        let finish_go = finish.clone();
        finish.connect_clicked(move |_| {
            if !saved_f.is_active() {
                ctx_f.toast("Confirm you saved the seed");
                return;
            }
            finish_go.set_sensitive(false);
            let ph = phrase.borrow().clone();
            match dialogs::save_mnemonic_wallet(&ctx_f, &name_f.text(), &ph, &pass_f.text()) {
                Ok(()) => {
                    ctx_f.toast("Wallet created");
                    ctx_f.window.close();
                }
                Err(e) => {
                    finish_go.set_sensitive(true);
                    ctx_f.toast(e);
                }
            }
            drop(ph);
        });

        let win_seed = ctx.window.clone();
        close_seed.connect_clicked(move |_| {
            win_seed.close();
        });
    } else {
        let ctx_i = ctx.clone();
        let name_i = name.clone();
        let pass_i = pass.clone();
        let pass2_i = pass2.clone();
        let status_i = status.clone();
        let go_i = go.clone();
        let existing_i = existing;
        let secret_i = secret.clone();
        let do_import = move || {
            if !go_i.is_sensitive() {
                return;
            }
            let n = name_i.text().to_string();
            let p1 = pass_i.text().to_string();
            let p2 = pass2_i.text().to_string();
            if !existing_i && p1 != p2 {
                status_i.set_text("Passwords do not match");
                ctx_i.toast("Passwords do not match");
                return;
            }
            let buffer = secret_i.buffer();
            let (a, b) = (buffer.start_iter(), buffer.end_iter());
            let raw = buffer.text(&a, &b, false).to_string();
            go_i.set_sensitive(false);
            status_i.set_text("Importing…");
            match dialogs::save_import_wallet(&ctx_i, &n, &raw, &p1) {
                Ok(()) => {
                    ctx_i.toast("Wallet imported");
                    ctx_i.window.close();
                }
                Err(e) => {
                    go_i.set_sensitive(true);
                    status_i.set_text(&e);
                    ctx_i.toast(e);
                }
            }
        };
        let import_click = do_import.clone();
        go.connect_clicked(move |_| import_click());
        pass.connect_activate({
            let do_import = do_import.clone();
            move |_| do_import()
        });
    }

    let win = ctx.window.clone();
    cancel.connect_clicked(move |_| {
        win.close();
    });

    let xbtn = gtk::Button::from_icon_name("window-close-symbolic");
    xbtn.add_css_class("flat");
    xbtn.add_css_class("circular");
    xbtn.add_css_class("og-signer-close");
    xbtn.set_tooltip_text(Some("Close"));
    xbtn.set_halign(gtk::Align::End);
    xbtn.set_valign(gtk::Align::Start);
    xbtn.set_margin_top(6);
    xbtn.set_margin_end(6);
    let win_x = ctx.window.clone();
    xbtn.connect_clicked(move |_| {
        win_x.close();
    });

    card.append(&stack);
    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    overlay.set_child(Some(&card));
    overlay.add_overlay(&xbtn);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&overlay));
    shell.append(&scroll);
    glib::idle_add_local_once({
        let pass = pass.clone();
        let name = name.clone();
        move || {
            if name.text().is_empty() {
                name.grab_focus();
            } else {
                pass.grab_focus();
            }
        }
    });
    shell.upcast()
}

fn build_sign(ctx: &Ctx) -> gtk::Widget {
    let k = kind();
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shell.add_css_class("og-signer-shell");
    shell.set_hexpand(true);
    shell.set_vexpand(true);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("og-card");
    card.add_css_class("og-signer-card");
    card.set_hexpand(true);
    card.set_vexpand(true);
    card.set_valign(gtk::Align::Fill);

    let kicker = gtk::Label::new(Some(match k {
        Kind::Swap => "CONFIRM SWAP",
        Kind::Send => "CONFIRM SEND",
        Kind::Stake => "CONFIRM STAKE",
        Kind::Burn => "CONFIRM BURN",
        Kind::Reclaim => "CONFIRM RECLAIM",
        Kind::Wc => "CONFIRM DAPP",
        Kind::Mint => "CONFIRM TOKEN",
        Kind::Nonce => "CONFIRM NONCE",
        Kind::Mass => "CONFIRM MASS SEND",
        Kind::Bot => "CONFIRM TRADING BOT",
    }));
    kicker.add_css_class("og-signer-kicker");
    kicker.set_halign(gtk::Align::Center);

    let flow = match k {
        Kind::Swap => flow_swap(),
        Kind::Send => flow_send(),
        Kind::Stake => flow_stake(),
        Kind::Burn => flow_burn(),
        Kind::Reclaim => flow_reclaim(),
        Kind::Wc => flow_wc(),
        Kind::Mint => flow_mint(),
        Kind::Nonce => flow_nonce(),
        Kind::Mass => flow_mass(),
        Kind::Bot => flow_bot(),
    };

    let active_kind = {
        let store = ctx.state.borrow().store.clone();
        let want = env_trim("VIBE_ACTIVE_ID");
        let target = if !want.is_empty() {
            want
        } else {
            store.active_id.clone()
        };
        store
            .wallets
            .iter()
            .find(|w| w.id == target || w.address == target)
            .map(|w| w.key_kind())
            .unwrap_or(vibe_core::WalletKeyType::Unknown)
    };
    let hardware = active_kind.is_hardware();
    let watch = active_kind.is_watch();
    let hw_name = match active_kind {
        vibe_core::WalletKeyType::Trezor => "Trezor",
        _ => "Ledger",
    };

    let pass = gtk::PasswordEntry::new();
    pass.set_show_peek_icon(true);
    pass.set_placeholder_text(Some("Vault password"));
    pass.set_hexpand(true);
    pass.add_css_class("og-signer-pass");

    let secure = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    secure.set_halign(gtk::Align::Center);
    let secure_ic = gtk::Image::from_icon_name("changes-prevent-symbolic");
    secure_ic.set_pixel_size(14);
    secure_ic.add_css_class("og-signer-lock");
    let status = gtk::Label::new(Some("On-device · irreversible"));
    status.add_css_class("og-signer-secure");
    status.set_xalign(0.0);
    status.set_wrap(true);
    secure.append(&secure_ic);
    secure.append(&status);

    let go_label = if hardware {
        format!("Confirm on {hw_name}")
    } else {
        "Sign".to_string()
    };
    let go = gtk::Button::with_label(&go_label);
    go.add_css_class("og-primary");
    go.set_sensitive(false);
    let extra_ack = std::rc::Rc::new(std::cell::Cell::new(false));
    let extra_need = std::rc::Rc::new(std::cell::Cell::new(false));
    let blocked = std::rc::Rc::new(std::cell::Cell::new(false));
    let fee_bad = std::rc::Rc::new(std::cell::Cell::new(false));
    let fee_box = if k == Kind::Send || k == Kind::Swap {
        Some(fee_picker(k, fee_bad.clone()))
    } else {
        None
    };
    let extra_box = gtk::CheckButton::with_label(if k == Kind::Wc {
        "I understand the warnings above"
    } else {
        "I understand this mint can freeze, mint, or take tokens"
    });
    extra_box.add_css_class("og-signer-extra");
    extra_box.set_visible(false);
    let review = gtk::Box::new(gtk::Orientation::Vertical, 6);
    review.add_css_class("og-signer-review");
    let review_status = gtk::Label::new(Some(if k == Kind::Wc {
        "Vault is decoding the transaction and resolving lookup tables…"
    } else {
        "Checking simulation and mint rules…"
    }));
    review_status.add_css_class("og-signer-hint");
    review_status.set_wrap(true);
    review_status.set_xalign(0.0);
    review.append(&review_status);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("og-ghost");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_homogeneous(true);
    row.append(&cancel);
    row.append(&go);

    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let win = ctx.window.clone();
    let done_c = done.clone();
    cancel.connect_clicked(move |_| {
        if k == Kind::Wc && !done_c.get() {
            write_wc_reject();
            done_c.set(true);
        }
        win.close();
    });
    let xbtn = gtk::Button::from_icon_name("window-close-symbolic");
    xbtn.add_css_class("flat");
    xbtn.add_css_class("circular");
    xbtn.add_css_class("og-signer-close");
    xbtn.set_tooltip_text(Some("Close"));
    xbtn.set_halign(gtk::Align::End);
    xbtn.set_valign(gtk::Align::Start);
    xbtn.set_margin_top(6);
    xbtn.set_margin_end(6);
    let win_x = ctx.window.clone();
    let done_x = done.clone();
    xbtn.connect_clicked(move |_| {
        if k == Kind::Wc && !done_x.get() {
            write_wc_reject();
            done_x.set(true);
        }
        win_x.close();
    });

    let ctx_s = ctx.clone();
    let pass_s = pass.clone();
    let status_s = status.clone();
    let go_s = go.clone();
    let extra_ack_s = extra_ack.clone();
    let extra_need_s = extra_need.clone();
    let blocked_s = blocked.clone();
    let fee_bad_s = fee_bad.clone();
    let sign = move || {
        if go_s.is_sensitive() == false {
            return;
        }
        if fee_bad_s.get() {
            status_s.set_text("Fix the priority fee first");
            return;
        }
        if watch {
            status_s.set_text("Watch-only — cannot sign");
            return;
        }
        if blocked_s.get() {
            status_s.set_text("Vault rule blocked this transaction");
            return;
        }
        if extra_need_s.get() && !extra_ack_s.get() {
            status_s.set_text("Tick the extra confirm first");
            return;
        }
        status_s.set_text(&if hardware {
            format!("Waiting for {hw_name}…")
        } else {
            "Unlocking…".into()
        });
        match dialogs::try_unlock(&ctx_s, &pass_s.text()) {
            Ok(()) => {}
            Err(e) => {
                status_s.set_text(&e);
                ctx_s.toast(e);
                return;
            }
        }
        if let Err(e) = enforce_vault_rules(&ctx_s, k, extra_ack_s.get()) {
            status_s.set_text(&e);
            ctx_s.toast(&e);
            go_s.set_sensitive(true);
            return;
        }
        pass_s.set_text("");
        go_s.set_sensitive(false);
        status_s.set_text(&if hardware {
            format!("Confirm on the {hw_name}…")
        } else {
            "Signing…".into()
        });
        run_tx(&ctx_s, k, status_s.clone(), go_s.clone(), done.clone(), extra_ack_s.get());
    };
    let sign_click = sign.clone();
    go.connect_clicked(move |_| sign_click());
    pass.connect_activate(move |_| sign());

    card.append(&kicker);
    card.append(&flow);
    card.append(&review);
    if let Some(fb) = &fee_box {
        card.append(fb);
    }
    card.append(&extra_box);
    {
        let store = ctx.state.borrow().store.clone();
        let want = env_trim("VIBE_ACTIVE_ID");
        let rec = store
            .wallets
            .iter()
            .find(|w| {
                if !want.is_empty() {
                    w.id == want || w.address == want
                } else {
                    w.id == store.active_id || store.active_id.is_empty()
                }
            })
            .cloned();
        if rec.is_none() && !watch && k != Kind::Wc {
            go.set_sensitive(true);
        }
        if let Some(rec) = rec {
            let rpc = ctx.rpc();
            let go_r = go.clone();
            let extra_box_r = extra_box.clone();
            let extra_ack_r = extra_ack.clone();
            let extra_need_r = extra_need.clone();
            let blocked_r = blocked.clone();
            let review_r = review.clone();
            let review_status_r = review_status.clone();
            let watch_r = watch;
            task::spawn(
                move || collect_review(k, rec, rpc),
                move |paint| {
                    review_status_r.set_visible(false);
                    if let Some(err) = &paint.blocked {
                        review_r.append(&label_line(err, "og-signer-block"));
                        blocked_r.set(true);
                        if !watch_r {
                            go_r.set_sensitive(false);
                        }
                    }
                    for line in &paint.reds {
                        review_r.append(&label_line(line, "og-signer-risk"));
                    }
                    for line in &paint.warns {
                        review_r.append(&label_line(line, "og-signer-warn"));
                    }
                    for line in &paint.infos {
                        review_r.append(&label_line(line, "og-signer-info"));
                    }
                    if paint.extra {
                        extra_need_r.set(true);
                        extra_box_r.set_visible(true);
                        let ack = extra_ack_r.clone();
                        let go_e = go_r.clone();
                        let blocked_e = blocked_r.clone();
                        extra_box_r.connect_toggled(move |btn| {
                            ack.set(btn.is_active());
                            if !blocked_e.get() {
                                go_e.set_sensitive(btn.is_active());
                            }
                        });
                    } else if !blocked_r.get() && !watch_r {
                        go_r.set_sensitive(true);
                    }
                },
            );
        }
    }
    if hardware {
        pass.set_visible(false);
        status.set_text(if active_kind == vibe_core::WalletKeyType::Trezor {
            "Unlock the Trezor · confirm on the device"
        } else {
            "Solana app open · confirm on the device"
        });
    } else if watch {
        pass.set_visible(false);
        status.set_text("Watch-only account — it cannot sign");
    }
    card.append(&pass);
    card.append(&row);
    card.append(&secure);
    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    overlay.set_child(Some(&card));
    overlay.add_overlay(&xbtn);
    shell.append(&overlay);
    glib::idle_add_local_once({
        let pass = pass.clone();
        move || {
            pass.grab_focus();
        }
    });
    shell.upcast()
}

/// Presets + custom µ-lamports for the Send / Swap Confirm. The choice is stored in
/// `CHOSEN_MICRO`; the vault re-checks the built bytes against R14b before signing.
fn fee_picker(k: Kind, fee_bad: std::rc::Rc<std::cell::Cell<bool>>) -> gtk::Widget {
    use vibe_core::fees::{self, FeeVerdict};
    let units = if k == Kind::Swap {
        SWAP_FEE_UNITS
    } else {
        env_u32("VIBE_COMPUTE_UNITS", 80_000)
    };
    let suggested = env_trim("VIBE_PRIORITY_MICROLAMPORTS").parse::<u64>().ok();
    let mode = env_trim("VIBE_PRIORITY_MODE").to_ascii_lowercase();
    // (label, value) — None value = Jupiter auto (swap) / custom sentinel handled below.
    let mut items: Vec<(String, Option<u64>)> = Vec::new();
    if k == Kind::Swap {
        items.push(("Auto (Jupiter)".into(), None));
    } else if let Some(v) = suggested.filter(|v| fees::PRESETS.iter().all(|(_, _, p)| p != v)) {
        items.push((format!("Suggested · {v} µL"), Some(v)));
    }
    for (_, label, micro) in fees::PRESETS {
        items.push((format!("{label} · {micro} µL"), Some(*micro)));
    }
    let custom_idx = items.len() as u32;
    let mut labels: Vec<&str> = items.iter().map(|(l, _)| l.as_str()).collect();
    labels.push("Custom…");
    let drop = gtk::DropDown::from_strings(&labels);
    drop.set_tooltip_text(Some("Priority fee (µ-lamports per compute unit)"));
    // Preselect: desk preset name, else the desk's number, else Normal / Auto.
    let pick = if k == Kind::Swap && suggested.is_none() {
        0
    } else if let Some(i) = items.iter().position(|(l, _)| {
        !mode.is_empty() && l.to_ascii_lowercase().starts_with(&mode)
    }) {
        i as u32
    } else if let Some(i) = suggested.and_then(|v| items.iter().position(|(_, x)| *x == Some(v))) {
        i as u32
    } else if k == Kind::Swap {
        0
    } else {
        items.iter().position(|(l, _)| l.starts_with("Normal")).unwrap_or(0) as u32
    };
    drop.set_selected(pick);

    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("µ-lamports per CU, e.g. 50000"));
    entry.set_input_purpose(gtk::InputPurpose::Digits);
    entry.set_visible(false);
    let detail = label_line("", "og-signer-info");
    let warn = label_line("", "og-signer-warn");
    warn.set_visible(false);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some("Priority fee"));
    title.add_css_class("og-signer-hint");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    head.append(&title);
    head.append(&drop);

    let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
    col.add_css_class("og-signer-fee");
    col.append(&head);
    col.append(&entry);
    col.append(&detail);
    col.append(&warn);

    let items = std::rc::Rc::new(items);
    let refresh = {
        let drop = drop.clone();
        let entry = entry.clone();
        let detail = detail.clone();
        let warn = warn.clone();
        let items = items.clone();
        std::rc::Rc::new(move || {
            let idx = drop.selected();
            let custom = idx == custom_idx;
            entry.set_visible(custom);
            let micro: Result<Option<u64>, String> = if custom {
                match fees::parse_micro(&entry.text()) {
                    Some(v) => Ok(Some(v)),
                    None => Err("Type a whole number of µ-lamports per CU.".into()),
                }
            } else {
                Ok(items.get(idx as usize).and_then(|(_, v)| *v))
            };
            warn.set_visible(false);
            warn.remove_css_class("og-signer-block");
            match micro {
                Err(e) => {
                    set_chosen_micro(None);
                    fee_bad.set(true);
                    detail.set_text("Priority fee · —");
                    warn.set_text(&e);
                    warn.add_css_class("og-signer-block");
                    warn.set_visible(true);
                }
                Ok(None) => {
                    set_chosen_micro(None);
                    fee_bad.set(false);
                    detail.set_text("Priority fee · Jupiter picks it (auto, capped by the vault at 0.05 SOL)");
                }
                Ok(Some(m)) => {
                    let verdict = if custom { fees::check_custom(units, m) } else { Ok(fees::check_fee(units, m)) };
                    let fee = fees::priority_fee_lamports(units, m);
                    detail.set_text(&if k == Kind::Swap {
                        format!("{} (sent to Jupiter as a max of {fee} lamports)", fees::describe(units, m))
                    } else {
                        fees::describe(units, m)
                    });
                    match verdict {
                        Err(e) | Ok(FeeVerdict::Block(e)) => {
                            set_chosen_micro(None);
                            fee_bad.set(true);
                            warn.set_text(&format!("Blocked by the vault [R14b]: {e}"));
                            warn.add_css_class("og-signer-block");
                            warn.set_visible(true);
                        }
                        Ok(v) => {
                            set_chosen_micro(Some(m));
                            fee_bad.set(false);
                            let high = matches!(v, FeeVerdict::Warn(_))
                                || fee > CONFIRM_WARN_LAMPORTS
                                || m > CONFIRM_WARN_MICRO;
                            if high {
                                warn.set_text(&format!(
                                    "High priority fee: about {:.6} SOL. Most transfers land with Normal.",
                                    fee as f64 / vibe_core::LAMPORTS_PER_SOL as f64
                                ));
                                warn.set_visible(true);
                            }
                        }
                    }
                }
            }
        })
    };
    refresh();
    let r = refresh.clone();
    drop.connect_selected_notify(move |_| r());
    let r = refresh.clone();
    entry.connect_changed(move |_| r());
    col.upcast()
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}
impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.trim().is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

fn shorten(addr: &str) -> String {
    vibe_core::shorten_address(addr)
}

fn glyph(rel: &str, fallback: &str, size: i32) -> gtk::Image {
    let img = gtk::Image::from_icon_name(fallback);
    img.set_pixel_size(size);
    if let Some(path) = paths::brand_asset(rel) {
        img.set_from_file(Some(path));
    }
    img
}

fn token_chip(kicker: &str, amount: &str, symbol: &str, leaving: bool, icon_path: &str, mint: &str) -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_halign(gtk::Align::Fill);
    col.set_hexpand(true);
    let kick = gtk::Label::new(Some(kicker));
    kick.add_css_class(if leaving {
        "og-signer-dir-out"
    } else {
        "og-signer-dir-in"
    });
    kick.set_halign(gtk::Align::Center);
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    chip.add_css_class("og-signer-chip");
    chip.add_css_class(if leaving {
        "og-signer-chip-out"
    } else {
        "og-signer-chip-in"
    });
    chip.set_halign(gtk::Align::Fill);
    chip.set_hexpand(true);
    chip.set_valign(gtk::Align::Center);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    inner.set_halign(gtk::Align::Center);
    inner.set_valign(gtk::Align::Center);
    inner.set_hexpand(true);
    let icon = token_icon(symbol, icon_path, 40, mint);
    icon.set_valign(gtk::Align::Center);
    let stack = gtk::Box::new(gtk::Orientation::Vertical, 1);
    stack.set_valign(gtk::Align::Center);
    stack.set_halign(gtk::Align::Start);
    let amt = gtk::Label::new(Some(amount));
    amt.add_css_class("og-signer-amt");
    amt.set_xalign(0.0);
    amt.set_ellipsize(pango::EllipsizeMode::End);
    stack.append(&amt);
    if !symbol.is_empty() {
        let sym = gtk::Label::new(Some(symbol));
        sym.add_css_class("og-signer-sym");
        sym.set_xalign(0.0);
        stack.append(&sym);
    }
    inner.append(&icon);
    inner.append(&stack);
    chip.append(&inner);
    col.append(&kick);
    col.append(&chip);
    col.upcast()
}

fn is_sol_token(symbol: &str, mint: &str) -> bool {
    symbol.eq_ignore_ascii_case("SOL") || is_native_mint(mint)
}

fn is_nav_spiral(path: &str) -> bool {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    p.contains("/nav/solana")
}

fn token_icon(symbol: &str, path: &str, size: i32, mint: &str) -> gtk::Widget {
    let img = gtk::Image::from_icon_name("image-x-generic-symbolic");
    img.set_pixel_size(size);
    img.add_css_class("og-signer-token-icon");
    if is_sol_token(symbol, mint) {
        if let Some(p) = paths::coin_asset("solana.png") {
            img.set_from_file(Some(&p));
            return img.upcast();
        }
    }
    let file = path.trim();
    if !file.is_empty() && !is_nav_spiral(file) {
        let p = PathBuf::from(file);
        if p.is_file() {
            img.set_from_file(Some(&p));
            return img.upcast();
        }
    }
    img.upcast()
}

fn mint_line(mint: &str) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("og-signer-mint");
    row.set_halign(gtk::Align::Center);
    let lab = gtk::Label::new(Some("MINT"));
    lab.add_css_class("og-signer-mint-kicker");
    let addr = gtk::Label::new(Some(&shorten(mint)));
    addr.add_css_class("monospace");
    addr.add_css_class("og-signer-mint-addr");
    addr.set_selectable(true);
    addr.set_tooltip_text(Some(mint));
    row.append(&lab);
    row.append(&addr);
    row.upcast()
}

fn swap_pill(text: &str) -> gtk::Widget {
    let lab = gtk::Label::new(Some(text));
    lab.add_css_class("og-signer-pill");
    lab.upcast()
}

fn flow_swap() -> gtk::Widget {
    let pay = env_trim("VIBE_SWAP_AMOUNT").if_empty("—");
    let from = env_trim("VIBE_SWAP_IN_SYMBOL").if_empty("TOKEN");
    let to = env_trim("VIBE_SWAP_OUT_SYMBOL").if_empty("TOKEN");
    let recv = env_trim("VIBE_SWAP_OUT_AMOUNT");
    let recv = if recv.is_empty() {
        format!("~ {to}")
    } else {
        recv
    };
    let in_icon = env_trim("VIBE_SWAP_IN_ICON");
    let out_icon = env_trim("VIBE_SWAP_OUT_ICON");
    let in_mint = env_trim("VIBE_SWAP_IN");
    let out_mint = env_trim("VIBE_SWAP_OUT");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    col.append(&token_chip("SELL", &pay, &from, true, &in_icon, &in_mint));
    let arrow = glyph(
        "actions/swap-glyph.png",
        "emblem-synchronizing-symbolic",
        22,
    );
    arrow.add_css_class("og-signer-arrow");
    arrow.set_halign(gtk::Align::Center);
    arrow.set_valign(gtk::Align::Center);
    col.append(&arrow);
    col.append(&token_chip("BUY", &recv, &to, false, &out_icon, &out_mint));
    let pills = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    pills.set_halign(gtk::Align::Center);
    pills.add_css_class("og-signer-pills");
    let router = env_trim("VIBE_SWAP_ROUTER");
    let impact = env_trim("VIBE_SWAP_IMPACT");
    if !router.is_empty() {
        pills.append(&swap_pill(&router.to_uppercase()));
    }
    if !impact.is_empty() {
        pills.append(&swap_pill(&format!("IMPACT  {impact}")));
    }
    let fee = env_trim("VIBE_SWAP_FEE");
    if !fee.is_empty() {
        pills.append(&swap_pill(&format!("APP FEE  {fee}")));
    }
    if pills.first_child().is_some() {
        col.append(&pills);
    }
    if !out_mint.is_empty() {
        col.append(&mint_line(&out_mint));
    }
    col.upcast()
}

fn flow_send() -> gtk::Widget {
    let pay = env_trim("VIBE_SEND_AMOUNT").if_empty("—");
    let sym = env_trim("VIBE_SEND_SYMBOL").if_empty("SOL");
    let dest = shorten(&env_trim("VIBE_SEND_DEST"));
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    col.append(&token_chip("LEAVING", &pay, &sym, true, "", ""));
    let arrow = glyph("actions/send-glyph-48.png", "mail-send-symbolic", 32);
    arrow.add_css_class("og-signer-arrow");
    arrow.set_halign(gtk::Align::Center);
    col.append(&arrow);
    col.append(&token_chip("TO", &dest, "", false, "", ""));
    let memo = env_trim("VIBE_SEND_MEMO");
    if !memo.is_empty() {
        let clipped: String = memo.chars().take(48).collect();
        col.append(&token_chip("MEMO", &clipped, "", false, "", ""));
    }
    col.upcast()
}

fn flow_mass() -> gtk::Widget {
    let n = env_trim("VIBE_MASS_COUNT").if_empty("0");
    let each = env_trim("VIBE_SEND_AMOUNT").if_empty("—");
    let sym = env_trim("VIBE_SEND_SYMBOL").if_empty("SOL");
    let mint = env_trim("VIBE_SEND_MINT");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    col.append(&token_chip("EACH", &each, &sym, true, "", &mint));
    let arrow = glyph("actions/send-glyph-48.png", "mail-send-symbolic", 32);
    arrow.add_css_class("og-signer-arrow");
    arrow.set_halign(gtk::Align::Center);
    col.append(&arrow);
    col.append(&token_chip("WALLETS", &n, "", false, "", ""));
    col.upcast()
}

fn flow_bot() -> gtk::Widget {
    let mode = env_trim("VIBE_BOT_MODE").if_empty("volume");
    let n = env_trim("VIBE_BOT_COUNT").if_empty("0");
    let sym = env_trim("VIBE_BOT_SYMBOL").if_empty("TOKEN");
    let total = env_trim("VIBE_BOT_TOTAL").if_empty("—");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    col.append(&token_chip("BOT", &mode.to_uppercase(), &sym, false, "", ""));
    col.append(&token_chip("TRADES", &n, "", false, "", ""));
    col.append(&token_chip("PAY", &total, "SOL", true, "", "native"));
    col.upcast()
}

fn flow_stake() -> gtk::Widget {
    let pay = env_trim("VIBE_STAKE_AMOUNT").if_empty("—");
    let action = env_trim("VIBE_STAKE_ACTION").if_empty("stake");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    col.append(&token_chip("LEAVING", &pay, "SOL", true, "", "native"));
    let arrow = glyph("actions/stake-glyph-48.png", "network-server-symbolic", 32);
    arrow.add_css_class("og-signer-arrow");
    arrow.set_halign(gtk::Align::Center);
    col.append(&arrow);
    let action_lab = action.to_uppercase();
    col.append(&token_chip("STAKE", &action_lab, "SOL", false, "", "native"));
    col.upcast()
}

fn flow_burn() -> gtk::Widget {
    let name = env_trim("VIBE_BURN_NAME").if_empty("NFT");
    let mint = env_trim("VIBE_BURN_MINT");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    col.append(&token_chip("BURN", &name, "NFT", true, "", &mint));
    let arrow = glyph("actions/send-glyph-48.png", "user-trash-symbolic", 32);
    arrow.add_css_class("og-signer-arrow");
    arrow.set_halign(gtk::Align::Center);
    col.append(&arrow);
    col.append(&token_chip("GONE", "on-chain", "", false, "", ""));
    if !mint.is_empty() {
        col.append(&mint_line(&mint));
    }
    col.upcast()
}

fn flow_reclaim() -> gtk::Widget {
    let n = env_trim("VIBE_RECLAIM_COUNT").if_empty("0");
    let sol = env_trim("VIBE_RECLAIM_SOL").if_empty("0");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    col.append(&token_chip("ACCOUNTS", &n, "close / burn dust", true, "", ""));
    let arrow = glyph(
        "actions/reclaim-glyph-48.png",
        "emblem-synchronizing-symbolic",
        32,
    );
    arrow.add_css_class("og-signer-arrow");
    arrow.set_halign(gtk::Align::Center);
    col.append(&arrow);
    col.append(&token_chip("RENT BACK", &sol, "SOL", false, "", "native"));
    col.upcast()
}

fn wc_request() -> Result<serde_json::Value, String> {
    let path = env_trim("VIBE_WC_REQUEST");
    if path.is_empty() {
        return Err("Missing WalletConnect request".into());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Bytes a `solana_signMessage` request asks the vault to sign (same logic as run_wc).
fn wc_message_bytes(req: &serde_json::Value) -> Result<Vec<u8>, String> {
    if let Some(b64) = req
        .get("message_b64")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return tx::decode_b64(b64).map_err(|e| e.to_string());
    }
    let message = req.get("message").and_then(|v| v.as_str()).unwrap_or("");
    Ok(match bs58::decode(message.trim()).into_vec() {
        Ok(b) if !b.is_empty() => b,
        _ => message.as_bytes().to_vec(),
    })
}

fn wc_contacts() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in [
        vibe_core::store::desk_contacts_path(),
        vibe_core::store::contacts_path(),
    ] {
        for c in vibe_core::store::load_contacts(&path).contacts {
            if !c.address.is_empty() && !out.contains(&c.address) {
                out.push(c.address);
            }
        }
    }
    out
}

/// Vault-side review of a dApp request, built only from the bytes that will be
/// signed. Lookup tables are fetched through the vault's own RPC. Any table that
/// can't be resolved blocks signing (vault-spec R15).
fn review_wc_request(
    req: &serde_json::Value,
    rec: &vibe_core::store::WalletRecord,
    rpc: &vibe_core::Rpc,
) -> Vec<vibe_core::txcheck::Review> {
    use vibe_core::txcheck::{self, Finding, Level, Review, RuleCtx};
    let contacts = wc_contacts();
    let ctx = RuleCtx {
        user: &rec.address,
        from_dapp: true,
        max_sol_per_tx: rec.max_sol_per_tx,
        block_infinite_approve: rec.block_infinite_approve,
        contacts: &contacts,
        priority_fee_warn_lamports: txcheck::PRIORITY_FEE_WARN_LAMPORTS,
        priority_fee_max_lamports: vibe_core::fees::MAX_PRIORITY_FEE_LAMPORTS,
    };
    let fetch = |k: &str| rpc.get_account(k).map_err(|e| e.to_string());
    let block = |text: String| Review {
        findings: vec![Finding { rule: "R15", level: Level::Block, text }],
        ..Default::default()
    };
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    if method == "solana_signMessage" {
        return match wc_message_bytes(req) {
            Ok(bytes) if txcheck::message_payload_is_transaction(&bytes) => vec![Review {
                findings: vec![Finding {
                    rule: "R11",
                    level: Level::Block,
                    text: "This \"message\" is really a Solana transaction. Signing it could move your funds, so the vault blocks it.".into(),
                }],
                ..Default::default()
            }],
            Ok(_) => vec![Review::default()],
            Err(e) => vec![block(format!("Unreadable message: {e}"))],
        };
    }
    if method == "solana_signAllTransactions" {
        let rows: Vec<Result<Vec<u8>, String>> = req
            .get("transactions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|item| {
                        item.as_str()
                            .ok_or_else(|| "not a string".to_string())
                            .and_then(|s| tx::decode_b64(s).map_err(|e| e.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if rows.is_empty() {
            return vec![block("dApp sent no transactions".into())];
        }
        if let Some(Err(e)) = rows.iter().find(|r| r.is_err()) {
            return vec![block(format!("Unreadable transaction in batch: {e}"))];
        }
        let txs: Vec<Vec<u8>> = rows.into_iter().filter_map(|r| r.ok()).collect();
        return txcheck::review_batch(&txs, &ctx, fetch);
    }
    let raw = req.get("transaction").and_then(|v| v.as_str()).unwrap_or("");
    match tx::decode_b64(raw) {
        Ok(bytes) if !bytes.is_empty() => vec![txcheck::review_wire(&bytes, &ctx, fetch)],
        _ => vec![block("dApp did not send a readable transaction".into())],
    }
}

/// 1.9 F8: Anchor instruction names, arguments and account names for the Confirm screen.
///
/// DISPLAY ONLY. Called from `collect_review` after the vault review has been painted.
/// It is deliberately NOT called from `review_wc_request`, which is also the exact-bytes
/// re-check `run_wc` performs right before signing, so an IDL (or a failure to fetch one)
/// can never block, unblock or change what gets signed. Any error or panic → no lines, and
/// the Confirm shows the existing raw decode only.
fn anchor_idl_lines(req: &serde_json::Value, rpc: &vibe_core::Rpc) -> Vec<String> {
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    if method == "solana_signMessage" {
        return Vec::new();
    }
    let raws: Vec<String> = if method == "solana_signAllTransactions" {
        req.get("transactions")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default()
    } else {
        req.get("transaction").and_then(|v| v.as_str()).map(|s| vec![s.to_string()]).unwrap_or_default()
    };
    let many = raws.len() > 1;
    let mut out = Vec::new();
    let Ok(mut reg) = vibe_core::anchor_idl::global().lock() else {
        return out;
    };
    // Bounded: at most 12 reads and 8 s for the whole Confirm, 6 s per HTTP request.
    let idl_rpc = rpc.clone().with_timeout(6);
    let mut budget = vibe_core::anchor_idl::FetchBudget::confirm_default();
    for (n, raw) in raws.iter().enumerate().take(8) {
        let Ok(bytes) = tx::decode_b64(raw) else { continue };
        let lines = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            vibe_core::anchor_idl::describe_wire(&bytes, &mut reg, |k: &str| {
                budget.fetch(k, &mut |k: &str| idl_rpc.get_account(k).map_err(|e| e.to_string()))
            })
        }))
        .unwrap_or_default();
        for l in lines {
            out.push(if many { format!("Tx {} · {l}", n + 1) } else { l });
        }
    }
    out
}

fn paint_wc_reviews(paint: &mut ReviewPaint, reviews: &[vibe_core::txcheck::Review]) {
    use vibe_core::txcheck::Level;
    let many = reviews.len() > 1;
    for (n, r) in reviews.iter().enumerate() {
        let tag = if many { format!("Tx {} · ", n + 1) } else { String::new() };
        for f in &r.findings {
            let line = format!("{tag}[{}] {}", f.rule, f.text);
            match f.level {
                Level::Block => {
                    if paint.blocked.is_none() {
                        paint.blocked = Some(format!("Blocked by the vault: {line}"));
                    } else {
                        paint.reds.push(line);
                    }
                }
                Level::Warn => paint.warns.push(line),
                Level::Info => paint.infos.push(line),
            }
        }
        if !r.account_lines.is_empty() {
            paint.infos.push(format!(
                "{tag}Vault decode ({}): {} accounts, {} from lookup tables",
                if r.version.is_empty() { "unparsed" } else { r.version.as_str() },
                r.account_lines.len(),
                r.lookup_accounts
            ));
            for l in &r.instruction_lines {
                paint.infos.push(format!("{tag}{l}"));
            }
            for l in &r.account_lines {
                paint.infos.push(format!("{tag}{l}"));
            }
        }
        if r.needs_ack() {
            paint.extra = true;
        }
    }
}

fn write_wc_json(value: serde_json::Value) {
    let path = env_trim("VIBE_WC_RESULT");
    if path.is_empty() {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(&value) {
        let _ = std::fs::write(&path, &text);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

fn write_wc_reject() {
    let req_id = wc_request()
        .ok()
        .and_then(|v| v.get("id").and_then(|x| x.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();
    write_wc_json(serde_json::json!({
        "ok": false,
        "id": req_id,
        "error": "User rejected"
    }));
}

fn flow_wc() -> gtk::Widget {
    let req = wc_request().unwrap_or_else(|_| serde_json::json!({}));
    let origin = req
        .get("origin")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            let env = env_trim("VIBE_WC_ORIGIN");
            if env.is_empty() {
                None
            } else {
                Some(env)
            }
        })
        .unwrap_or_else(|| "dApp".into());
    let method = req
        .get("method")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            let env = env_trim("VIBE_WC_METHOD");
            if env.is_empty() {
                None
            } else {
                Some(env)
            }
        })
        .unwrap_or_else(|| "sign".into());
    let url = req
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    col.add_css_class("og-signer-wc");
    col.set_halign(gtk::Align::Fill);
    let mark = glyph("mark-96.png", "application-x-executable-symbolic", 64);
    mark.add_css_class("og-signer-hero");
    mark.set_halign(gtk::Align::Center);
    mark.set_pixel_size(64);
    let brand = gtk::Label::new(Some("Vibe Network"));
    brand.add_css_class("og-signer-brand");
    brand.set_halign(gtk::Align::Center);
    col.append(&mark);
    col.append(&brand);
    let name = gtk::Label::new(Some(&origin));
    name.add_css_class("og-signer-amt");
    name.set_halign(gtk::Align::Center);
    name.set_wrap(true);
    let headline = req
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| match method.as_str() {
            "solana_signAndSendTransaction"
                if origin.contains("vibewallet.net")
                    || origin.contains("localhost")
                    || origin.contains("127.0.0.1") =>
            {
                "Swap on VibeSwap".into()
            }
            "solana_signAndSendTransaction" if origin.contains("jup.ag") => "Swap on Jupiter".into(),
            "solana_signAndSendTransaction" => "Sign and send".into(),
            "solana_signTransaction"
                if origin.contains("vibewallet.net")
                    || origin.contains("localhost")
                    || origin.contains("127.0.0.1") =>
            {
                "Swap on VibeSwap".into()
            }
            "solana_signTransaction" => "Sign transaction".into(),
            "solana_signMessage" => "Sign message".into(),
            "solana_signAllTransactions" => "Sign transactions".into(),
            other => other.replace("solana_", "").replace('_', " "),
        });
    let meth = gtk::Label::new(Some(&headline));
    meth.add_css_class("og-signer-sym");
    meth.set_halign(gtk::Align::Center);
    col.append(&name);
    col.append(&meth);
    if let Some(arr) = req
        .get("summary")
        .and_then(|v| v.get("lines"))
        .and_then(|v| v.as_array())
    {
        for item in arr {
            let Some(text) = item.as_str() else { continue };
            if text.is_empty() {
                continue;
            }
            let lab = gtk::Label::new(Some(text));
            lab.add_css_class("og-signer-sym");
            lab.set_wrap(true);
            lab.set_justify(gtk::Justification::Center);
            lab.set_halign(gtk::Align::Center);
            col.append(&lab);
        }
    }
    if !url.is_empty() {
        let link = gtk::Label::new(Some(&url));
        link.add_css_class("dim-label");
        link.add_css_class("monospace");
        link.set_halign(gtk::Align::Center);
        link.set_ellipsize(pango::EllipsizeMode::Middle);
        col.append(&link);
    }
    let hint = gtk::Label::new(Some("Review this dApp request, then unlock to sign."));
    hint.add_css_class("dim-label");
    hint.set_wrap(true);
    hint.set_halign(gtk::Align::Center);
    col.append(&hint);
    col.upcast()
}

fn flow_mint() -> gtk::Widget {
    let action = env_trim("VIBE_MINT_ACTION").if_empty("create");
    let name = env_trim("VIBE_MINT_NAME").if_empty("Token");
    let symbol = env_trim("VIBE_MINT_SYMBOL").if_empty("TOKEN");
    let supply = env_trim("VIBE_MINT_SUPPLY").if_empty("0");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    let tag = action.to_uppercase();
    col.append(&token_chip(&tag, &name, &symbol, false, "", ""));
    if action == "create" || action == "mintmore" || action == "burn" {
        col.append(&token_chip("SUPPLY", &supply, &symbol, true, "", ""));
    }
    if action == "create" {
        let std = if mint_token_program() == ProgramIds::TOKEN {
            "SPL Token"
        } else {
            "Token-2022"
        };
        col.append(&token_chip("STANDARD", std, "", false, "", ""));
        if !env_trim("VIBE_MINT_SECRET").is_empty() {
            col.append(&token_chip("MINT", "custom address", "", false, "", ""));
        }
    }
    col.upcast()
}

fn flow_nonce() -> gtk::Widget {
    let action = env_trim("VIBE_NONCE_ACTION").if_empty("create");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.add_css_class("og-signer-flow");
    col.append(&token_chip("NONCE", &action.to_uppercase(), "SOL", false, "", "native"));
    col.upcast()
}

fn run_wc(
    ctx: &Ctx,
    wallet: crate::ctx::Unlocked,
    status: gtk::Label,
    go: gtk::Button,
    done: std::rc::Rc<std::cell::Cell<bool>>,
    extra_ack: bool,
) {
    let req = match wc_request() {
        Ok(v) => v,
        Err(e) => {
            status.set_text(&e);
            go.set_sensitive(true);
            return;
        }
    };
    let method = req
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let req_id = req
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let send = req.get("send").and_then(|v| v.as_bool()).unwrap_or(false)
        || method == "solana_signAndSendTransaction";
    let wallet = wallet.clone();
    let rpc = ctx.rpc();
    let ctx_done = ctx.clone();
    let origin = req
        .get("origin")
        .and_then(|v| v.as_str())
        .unwrap_or("dApp")
        .to_string();
    let rec = reload_active_record(ctx);
    task::spawn(
        move || -> Result<serde_json::Value, String> {
            // Re-run the vault review on the exact request that is about to be
            // signed (lookup tables resolved again). Never sign on a BLOCK.
            let rec = rec.ok_or_else(|| "No wallet in this vault".to_string())?;
            let reviews = review_wc_request(&req, &rec, &rpc);
            if let Some(f) = reviews.iter().find_map(|r| r.first_block()) {
                return Err(format!("Blocked by the vault: [{}] {}", f.rule, f.text));
            }
            if reviews.iter().any(|r| r.needs_ack()) && !extra_ack {
                return Err("Tick “I understand the warnings above” first".into());
            }
            if method == "solana_signMessage" {
                let bytes = wc_message_bytes(&req)?;
                let signature = bs58::encode(wallet.sign_message(&bytes)?).into_string();
                return Ok(serde_json::json!({
                    "ok": true,
                    "id": req_id,
                    "method": method,
                    "signature": signature
                }));
            }
            if method == "solana_signAllTransactions" {
                let rows: Vec<String> = req
                    .get("transactions")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| item.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut signed = Vec::new();
                let mut last_sig = String::new();
                for raw in rows {
                    let pk = wallet.public_bytes()?;
                    let (sig, tx_b64, _) = tx::sign_wire_b64_with(&raw, &pk, |msg| {
                        wallet.sign_message(msg).map_err(|e| vibe_core::Error::msg(e))
                    })
                    .map_err(|e| e.to_string())?;
                    last_sig = sig;
                    signed.push(serde_json::Value::String(tx_b64));
                }
                return Ok(serde_json::json!({
                    "ok": true,
                    "id": req_id,
                    "method": method,
                    "signature": last_sig,
                    "transactions": signed
                }));
            }
            let raw = req
                .get("transaction")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if raw.is_empty() {
                return Err("dApp did not send a serialized transaction".into());
            }
            let pk = wallet.public_bytes()?;
            let (sig, tx_b64, bytes) = tx::sign_wire_b64_with(raw, &pk, |msg| {
                wallet.sign_message(msg).map_err(|e| vibe_core::Error::msg(e))
            })
            .map_err(|e| e.to_string())?;
            if send {
                let sent = rpc.send_transaction(&bytes).map_err(|e| e.to_string())?;
                return Ok(serde_json::json!({
                    "ok": true,
                    "id": req_id,
                    "method": method,
                    "signature": sent,
                    "transaction": tx_b64
                }));
            }
            Ok(serde_json::json!({
                "ok": true,
                "id": req_id,
                "method": method,
                "signature": sig,
                "transaction": tx_b64
            }))
        },
        move |res| match res {
            Ok(value) => {
                write_wc_json(value.clone());
                done.set(true);
                let sig = value
                    .get("signature")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if send && !sig.is_empty() {
                    let url = ctx_done.cluster().explorer_tx_url(&sig);
                    let _ = vibe_core::store::append_swap_event(
                        &sig, &url, &origin, "dApp", "", "", "dapp",
                    );
                    status.set_text("Confirmed on Solana");
                    ctx_done.toast("Confirmed on Solana");
                } else {
                    status.set_text("Signed");
                    ctx_done.toast("Signed");
                }
                let win = ctx_done.window.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                    win.close();
                    glib::ControlFlow::Break
                });
            }
            Err(e) => {
                status.set_text(&e);
                ctx_done.toast(e);
                go.set_sensitive(true);
            }
        },
    );
}

fn run_tx(
    ctx: &Ctx,
    k: Kind,
    status: gtk::Label,
    go: gtk::Button,
    done: std::rc::Rc<std::cell::Cell<bool>>,
    extra_ack: bool,
) {
    if let Err(e) = enforce_vault_rules(ctx, k, extra_ack) {
        status.set_text(&e);
        go.set_sensitive(true);
        return;
    }
    let Some(wallet) = ctx.active_wallet() else {
        status.set_text("Unlock failed");
        go.set_sensitive(true);
        return;
    };
    match k {
        Kind::Swap => run_swap(ctx, wallet, status, go),
        Kind::Send => run_send(ctx, wallet, status, go),
        Kind::Stake => run_stake(ctx, wallet, status, go),
        Kind::Burn => run_burn(ctx, wallet, status, go),
        Kind::Reclaim => run_reclaim(ctx, wallet, status, go),
        Kind::Wc => run_wc(ctx, wallet, status, go, done, extra_ack),
        Kind::Mint => run_mint(ctx, wallet, status, go),
        Kind::Nonce => run_nonce(ctx, wallet, status, go),
        Kind::Mass => run_mass(ctx, wallet, status, go),
        Kind::Bot => run_bot(ctx, wallet, status, go),
    }
}

fn run_swap(ctx: &Ctx, wallet: crate::ctx::Unlocked, status: gtk::Label, go: gtk::Button) {
    let in_mint = env_trim("VIBE_SWAP_IN");
    let out_mint = env_trim("VIBE_SWAP_OUT");
    let amount_s = env_trim("VIBE_SWAP_AMOUNT");
    let in_sym = env_trim("VIBE_SWAP_IN_SYMBOL").if_empty("TOKEN");
    let out_sym = env_trim("VIBE_SWAP_OUT_SYMBOL").if_empty("TOKEN");
    let decimals: u8 = env_trim("VIBE_SWAP_IN_DECIMALS").parse().unwrap_or(9);
    let raw = match parse_ui_amount(&amount_s, decimals) {
        Ok(v) if v > 0 => v,
        _ => {
            status.set_text("Invalid swap amount");
            go.set_sensitive(true);
            return;
        }
    };
    let pay = format!("{amount_s} {in_sym}");
    let out_amt = env_trim("VIBE_SWAP_OUT_AMOUNT");
    let recv = if out_amt.is_empty() {
        format!("~ {out_sym}")
    } else {
        format!("{out_amt} {out_sym}")
    };
    let slip = ctx.state.borrow().settings.slippage_bps;
    let slip = if slip == 0 { None } else { Some(slip) };
    let key = ctx.state.borrow().settings.jupiter_api_key.clone();
    let mut client = jupiter::Client::from_env_or(&key);
    client.priority_fee_lamports = swap_priority_lamports();
    let wallet = wallet.clone();
    let addr = wallet.address.clone();
    let ctx_done = ctx.clone();
    let in_m = in_mint.clone();
    let out_m = out_mint.clone();
    let pay_n = pay.clone();
    let recv_n = recv.clone();
    task::spawn(
        move || -> Result<String, String> {
            let order = client
                .order(&in_mint, &out_mint, raw, Some(&addr), slip)
                .map_err(|e| e.to_string())?;
            // The vault re-reads the fee Jupiter actually put in the bytes (R14b gate runs
            // again inside sign_order_with); a picked fee is also a ceiling.
            if let (Some(want), Some(b64)) = (client.priority_fee_lamports, order.transaction_b64.as_deref()) {
                vibe_core::fees::check_wire_against(b64, want)?;
            }
            if !order.can_execute() {
                return Err(if order.error_message.is_empty() {
                    "Jupiter could not build a swap transaction".into()
                } else {
                    order.error_message
                });
            }
            let pk = wallet.public_bytes()?;
            let signed = jupiter::Client::sign_order_with(&order, &pk, |msg| {
                wallet.sign_message(msg).map_err(|e| vibe_core::Error::msg(e))
            })
            .map_err(|e| e.to_string())?;
            let exec = client
                .execute(&signed, &order.request_id)
                .map_err(|e| e.to_string())?;
            Ok(exec.signature)
        },
        move |res| finish(&ctx_done, &status, &go, res, "swap", &pay_n, &recv_n, &in_m, &out_m),
    );
}

fn run_burn(ctx: &Ctx, wallet: crate::ctx::Unlocked, status: gtk::Label, go: gtk::Button) {
    let mint = env_trim("VIBE_BURN_MINT");
    let name = env_trim("VIBE_BURN_NAME").if_empty("NFT");
    let program = env_trim("VIBE_BURN_PROGRAM");
    let account = env_trim("VIBE_BURN_ACCOUNT");
    let amount: u64 = env_trim("VIBE_BURN_AMOUNT").parse().unwrap_or(1);
    if !vibe_core::format::looks_like_pubkey(&mint) {
        status.set_text("Invalid mint");
        go.set_sensitive(true);
        return;
    }
    let from = wallet.address.clone();
    let rpc = ctx.rpc();
    let ctx_done = ctx.clone();
    let mint_s = mint.clone();
    let name_s = name.clone();
    let wallet = wallet.clone();
    task::spawn(
        move || -> Result<String, String> {
            let program = if program.is_empty() {
                ProgramIds::TOKEN
            } else {
                program.as_str()
            };
            let bag = rpc.get_tokens(&from).map_err(|e| e.to_string())?;
            let hit = bag.iter().find(|t| t.mint == mint);
            let Some(hit) = hit else {
                return Err(
                    "This wallet holds 0 of that mint. Wrong CA, or you already burned it.".into(),
                );
            };
            let held: u64 = hit.amount_raw.parse().unwrap_or(0);
            if held == 0 {
                return Err("This wallet holds 0 of that mint.".into());
            }
            let burn_amt = if amount == 0 || amount > held {
                held
            } else {
                amount
            };
            let source = if vibe_core::format::looks_like_pubkey(&account) {
                account.clone()
            } else if !hit.account.is_empty() {
                hit.account.clone()
            } else {
                tx::associated_token_address(&from, &mint, program).map_err(|e| e.to_string())?
            };
            let program = if hit.program.is_empty() {
                program
            } else {
                hit.program.as_str()
            };
            let burn = tx::token_burn(&source, &mint, &from, burn_amt, program);
            let close = tx::token_close_account(&source, &from, &from, program);
            send_signed(&rpc, &wallet, vec![burn, close])
        },
        move |res| {
            finish(
                &ctx_done,
                &status,
                &go,
                res,
                "burn",
                &name_s,
                "burned",
                &mint_s,
                "",
            )
        },
    );
}

#[derive(Clone)]
struct ReclaimRow {
    account: String,
    program: String,
    mint: String,
    amount: u64,
    withheld: u64,
}

fn parse_reclaim_rows(raw: &str) -> Vec<ReclaimRow> {
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(arr) = parsed.as_array() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for item in arr {
        let account = item
            .get("account")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let program = item
            .get("program")
            .and_then(|v| v.as_str())
            .unwrap_or(ProgramIds::TOKEN)
            .to_string();
        let mint = item
            .get("mint")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let amount = item
            .get("amount")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0);
        if vibe_core::format::looks_like_pubkey(&account) {
            rows.push(ReclaimRow {
                account,
                program,
                mint,
                amount,
                withheld: 0,
            });
        }
    }
    rows
}

fn refresh_reclaim_rows(rpc: &vibe_core::Rpc, rows: Vec<ReclaimRow>) -> Vec<ReclaimRow> {
    let addrs: Vec<String> = rows.iter().map(|r| r.account.clone()).collect();
    let live = match rpc.get_multiple_accounts(&addrs) {
        Ok(v) if v.len() == rows.len() => v,
        _ => return rows,
    };
    let mut out = Vec::new();
    for (i, mut row) in rows.into_iter().enumerate() {
        let Some(slot) = live.get(i) else {
            out.push(row);
            continue;
        };
        let Some((owner, data)) = slot else {
            continue;
        };
        if !ProgramIds::is_token_program(owner) {
            continue;
        }
        row.program = owner.clone();
        if tx::token_account_frozen(data) {
            continue;
        }
        if let Some(amount) = tx::token_account_amount(data) {
            row.amount = amount;
        }
        row.withheld = tx::token_account_withheld(data);
        if row.mint == WSOL_MINT {
            row.amount = 0;
            row.withheld = 0;
        }
        out.push(row);
    }
    out
}

fn reclaim_ixs(from: &str, rows: &[ReclaimRow]) -> Vec<tx::Instruction> {
    let mut ixs = Vec::new();
    for row in rows {
        if row.withheld > 0 && ProgramIds::is_token_2022(&row.program) {
            ixs.push(tx::harvest_withheld_to_mint(
                &row.mint,
                &row.account,
                &row.program,
            ));
        }
        if row.amount > 0 && vibe_core::format::looks_like_pubkey(&row.mint) {
            ixs.push(tx::token_burn(
                &row.account,
                &row.mint,
                from,
                row.amount,
                &row.program,
            ));
        }
        ixs.push(tx::token_close_account(
            &row.account,
            from,
            from,
            &row.program,
        ));
    }
    ixs
}

fn reclaim_chunks(rows: Vec<ReclaimRow>) -> Vec<Vec<ReclaimRow>> {
    let mut chunks: Vec<Vec<ReclaimRow>> = Vec::new();
    let mut cur: Vec<ReclaimRow> = Vec::new();
    let mut weight = 0usize;
    for row in rows {
        let w = 1
            + usize::from(row.amount > 0)
            + usize::from(row.withheld > 0);
        if !cur.is_empty() && weight + w > 12 {
            chunks.push(cur);
            cur = Vec::new();
            weight = 0;
        }
        weight += w;
        cur.push(row);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

fn send_reclaim_chunk(
    rpc: &vibe_core::Rpc,
    wallet: &crate::ctx::Unlocked,
    chunk: &[ReclaimRow],
) -> Result<String, String> {
    send_signed(rpc, wallet, reclaim_ixs(&wallet.address, chunk))
}

fn run_reclaim(ctx: &Ctx, wallet: crate::ctx::Unlocked, status: gtk::Label, go: gtk::Button) {
    let raw = env_trim("VIBE_RECLAIM_ACCOUNTS");
    let count = env_trim("VIBE_RECLAIM_COUNT").if_empty("0");
    let sol = env_trim("VIBE_RECLAIM_SOL").if_empty("0");
    let rows = parse_reclaim_rows(&raw);
    if rows.is_empty() {
        status.set_text("No accounts to close");
        go.set_sensitive(true);
        return;
    }
    let rpc = ctx.rpc();
    let ctx_done = ctx.clone();
    let count_n = count.clone();
    let sol_n = sol.clone();
    let wallet = wallet.clone();
    task::spawn(
        move || -> Result<String, String> {
            let rows = refresh_reclaim_rows(&rpc, rows);
            if rows.is_empty() {
                return Err("Those token accounts are gone or frozen".into());
            }
            let mut last = String::new();
            let mut closed = 0usize;
            let mut last_err = String::new();
            for chunk in reclaim_chunks(rows) {
                match send_reclaim_chunk(&rpc, &wallet, &chunk) {
                    Ok(sig) => {
                        last = sig;
                        closed += chunk.len();
                    }
                    Err(err) => {
                        last_err = err;
                        for row in &chunk {
                            match send_reclaim_chunk(&rpc, &wallet, std::slice::from_ref(row)) {
                                Ok(sig) => {
                                    last = sig;
                                    closed += 1;
                                }
                                Err(e) => last_err = e,
                            }
                        }
                    }
                }
            }
            if closed == 0 {
                return Err(if last_err.is_empty() {
                    "Could not close these accounts".into()
                } else {
                    last_err
                });
            }
            Ok(last)
        },
        move |res| {
            finish(
                &ctx_done,
                &status,
                &go,
                res,
                "reclaim",
                &sol_n,
                &count_n,
                "native",
                "",
            )
        },
    );
}

fn run_send(ctx: &Ctx, wallet: crate::ctx::Unlocked, status: gtk::Label, go: gtk::Button) {
    let dest = env_trim("VIBE_SEND_DEST");
    let amount_s = env_trim("VIBE_SEND_AMOUNT");
    let symbol = env_trim("VIBE_SEND_SYMBOL").if_empty("SOL");
    let mint = env_trim("VIBE_SEND_MINT");
    let decimals: u8 = env_trim("VIBE_SEND_DECIMALS").parse().unwrap_or(9);
    let program = env_trim("VIBE_SEND_PROGRAM");
    if !vibe_core::format::looks_like_pubkey(&dest) {
        status.set_text("Invalid destination");
        go.set_sensitive(true);
        return;
    }
    let raw = match parse_ui_amount(&amount_s, decimals) {
        Ok(v) if v > 0 => v,
        _ => {
            status.set_text("Invalid amount");
            go.set_sensitive(true);
            return;
        }
    };
    let from = wallet.address.clone();
    let rpc = ctx.rpc();
    let ctx_done = ctx.clone();
    let pay = format!("{amount_s} {symbol}");
    let recv = shorten(&dest);
    let dest_s = dest.clone();
    let mint_s = mint.clone();
    let wallet = wallet.clone();
    task::spawn(
        move || -> Result<String, String> {
            let mut ixs = if is_native_mint(&mint) {
                vec![tx::system_transfer(&from, &dest, raw)]
            } else {
                let program = rpc
                    .token_program_for_mint(&mint)
                    .or_else(|_| -> vibe_core::Result<String> {
                        if ProgramIds::is_token_program(&program) {
                            Ok(program.clone())
                        } else {
                            Ok(ProgramIds::TOKEN.to_string())
                        }
                    })
                    .map_err(|e| e.to_string())?;
                let source = tx::associated_token_address(&from, &mint, &program)
                    .map_err(|e| e.to_string())?;
                let dest_ata = tx::associated_token_address(&dest, &mint, &program)
                    .map_err(|e| e.to_string())?;
                let create = tx::create_associated_token_account_idempotent(
                    &from, &dest, &mint, &program,
                )
                .map_err(|e| e.to_string())?;
                let xfer = tx::token_transfer_checked(
                    &source, &mint, &dest_ata, &from, raw, decimals, &program,
                );
                vec![create, xfer]
            };
            if let Some(note) = send_memo_ix(&from) {
                ixs.push(note);
            }
            send_signed(&rpc, &wallet, ixs)
        },
        move |res| {
            finish(
                &ctx_done,
                &status,
                &go,
                res,
                "send",
                &pay,
                &recv,
                &mint_s,
                &dest_s,
            )
        },
    );
}

fn parse_mass_dests() -> Vec<String> {
    let raw = env_trim("VIBE_MASS_DESTS");
    let text = if raw.contains('/') || raw.ends_with(".txt") {
        std::fs::read_to_string(&raw).unwrap_or_default()
    } else {
        raw.replace(',', "\n")
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for part in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let addr = part.trim();
        if vibe_core::format::looks_like_pubkey(addr) && seen.insert(addr.to_string()) {
            out.push(addr.to_string());
        }
    }
    out
}

fn run_mass(ctx: &Ctx, wallet: crate::ctx::Unlocked, status: gtk::Label, go: gtk::Button) {
    let dests = parse_mass_dests();
    let amount_s = env_trim("VIBE_SEND_AMOUNT");
    let symbol = env_trim("VIBE_SEND_SYMBOL").if_empty("SOL");
    let mint = env_trim("VIBE_SEND_MINT");
    let decimals: u8 = env_trim("VIBE_SEND_DECIMALS").parse().unwrap_or(9);
    let program = env_trim("VIBE_SEND_PROGRAM");
    if dests.is_empty() {
        status.set_text("No valid addresses");
        go.set_sensitive(true);
        return;
    }
    let raw = match parse_ui_amount(&amount_s, decimals) {
        Ok(v) if v > 0 => v,
        _ => {
            status.set_text("Invalid amount");
            go.set_sensitive(true);
            return;
        }
    };
    let native = is_native_mint(&mint);
    let chunk = if native { 10usize } else { 4usize };
    let from = wallet.address.clone();
    let rpc = ctx.rpc();
    let ctx_done = ctx.clone();
    let n = dests.len();
    let pay = format!("{amount_s} {symbol} × {n}");
    let wallet = wallet.clone();
    let mint_s = mint.clone();
    task::spawn(
        move || -> Result<String, String> {
            let program = if native {
                String::new()
            } else {
                rpc.token_program_for_mint(&mint)
                    .or_else(|_| -> vibe_core::Result<String> {
                        if ProgramIds::is_token_program(&program) {
                            Ok(program.clone())
                        } else {
                            Ok(ProgramIds::TOKEN.to_string())
                        }
                    })
                    .map_err(|e| e.to_string())?
            };
            let source = if native {
                String::new()
            } else {
                tx::associated_token_address(&from, &mint, &program).map_err(|e| e.to_string())?
            };
            let mut last = String::new();
            let mut sent = 0usize;
            for group in dests.chunks(chunk) {
                let mut ixs = Vec::new();
                for dest in group {
                    if native {
                        ixs.push(tx::system_transfer(&from, dest, raw));
                    } else {
                        let dest_ata = tx::associated_token_address(dest, &mint, &program)
                            .map_err(|e| e.to_string())?;
                        ixs.push(
                            tx::create_associated_token_account_idempotent(
                                &from, dest, &mint, &program,
                            )
                            .map_err(|e| e.to_string())?,
                        );
                        ixs.push(tx::token_transfer_checked(
                            &source, &mint, &dest_ata, &from, raw, decimals, &program,
                        ));
                    }
                }
                last = send_signed(&rpc, &wallet, ixs)?;
                sent += group.len();
            }
            Ok(format!("{last}|{sent}"))
        },
        move |res| {
            let mapped = res.map(|blob| {
                blob.split('|').next().unwrap_or("").to_string()
            });
            finish(
                &ctx_done,
                &status,
                &go,
                mapped,
                "send",
                &pay,
                &format!("{n} wallets"),
                &mint_s,
                "",
            )
        },
    );
}

fn bot_jitter(lo: u64, hi: u64, salt: u64) -> u64 {
    if hi <= lo {
        return lo;
    }
    let mix = salt.wrapping_mul(0x9E3779B97F4A7C15).rotate_left(13);
    lo + mix % (hi - lo + 1)
}

fn bot_write_progress(path: &str, body: serde_json::Value) {
    if path.is_empty() {
        return;
    }
    let _ = std::fs::write(path, body.to_string());
}

fn bot_control(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let Ok(raw) = std::fs::read_to_string(path) else {
        return String::new();
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.get("action").and_then(|a| a.as_str()).map(|s| s.to_string()))
        .unwrap_or_default()
}

fn bot_sleep_ms(ms: u64) {
    if ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

/// Sleep in 200ms slices so Stop/Pause lands without waiting out the full interval.
/// Returns the control action if it is stop or pause.
fn bot_wait(ms: u64, control_path: &str) -> String {
    let mut left = ms;
    while left > 0 {
        let ctl = bot_control(control_path);
        if ctl == "stop" || ctl == "pause" {
            return ctl;
        }
        let slice = left.min(200);
        bot_sleep_ms(slice);
        left = left.saturating_sub(slice);
    }
    String::new()
}

fn bot_shuffle<T>(items: &mut [T], salt: u64) {
    let n = items.len();
    if n < 2 {
        return;
    }
    for i in 0..(n - 1) {
        let span = (n - i) as u64;
        let j = i + (bot_jitter(0, span.saturating_sub(1), salt.wrapping_add(i as u64)) as usize);
        items.swap(i, j.min(n - 1));
    }
}

fn bot_exec_swap(
    client: &jupiter::Client,
    kp: &vibe_core::Keypair,
    addr: &str,
    in_mint: &str,
    out_mint: &str,
    amount: u64,
    slip: Option<u16>,
    dry: bool,
    retries: u32,
    jito: bool,
) -> Result<String, String> {
    if amount == 0 {
        return Err("zero amount".into());
    }
    if dry {
        let order = client
            .order(in_mint, out_mint, amount, None, slip)
            .map_err(|e| e.to_string())?;
        if order.out_amount.is_empty() || order.out_amount == "0" {
            return Err(order.error_message.if_empty("no liquidity"));
        }
        return Ok("sim".into());
    }
    let mut last = "swap failed".to_string();
    for attempt in 0..=retries {
        if attempt > 0 {
            bot_sleep_ms(400);
        }
        match client.order(in_mint, out_mint, amount, Some(addr), slip) {
            Ok(order) if order.can_execute() => match jupiter::Client::sign_order(&order, kp) {
                Ok(signed) => {
                    if jito {
                        let _ = jupiter::jito_send_tx(&signed);
                    }
                    match client.execute(&signed, &order.request_id) {
                        Ok(exec) => return Ok(exec.signature),
                        Err(e) => last = e.to_string(),
                    }
                }
                Err(e) => last = e.to_string(),
            },
            Ok(order) => last = order.error_message.if_empty("no swap tx"),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn bot_wallet_sol(rpc: &vibe_core::Rpc, wallets: &[(String, vibe_core::Keypair)]) -> u64 {
    wallets
        .iter()
        .map(|(addr, _)| rpc.get_balance(addr).unwrap_or(0))
        .sum()
}

fn bot_sweep(
    rpc: &vibe_core::Rpc,
    wallet: &crate::ctx::Unlocked,
    client: &jupiter::Client,
    mint: &str,
    payer: &str,
    wallets: &[(String, vibe_core::Keypair)],
    slip: Option<u16>,
    dry: bool,
    control_path: &str,
) {
    if dry {
        return;
    }
    for (addr, kp) in wallets {
        if bot_control(control_path) == "stop" {
            break;
        }
        if let Ok(bag) = rpc.get_tokens(addr) {
            if let Some(hit) = bag.iter().find(|t| t.mint == mint) {
                let raw: u64 = hit.amount_raw.parse().unwrap_or(0);
                if raw > 0 {
                    let _ = bot_exec_swap(
                        client,
                        kp,
                        addr,
                        mint,
                        vibe_core::WSOL_MINT,
                        raw,
                        slip,
                        false,
                        2,
                        false,
                    );
                }
                if !hit.account.is_empty() {
                    let prog = if hit.program.is_empty() {
                        ProgramIds::TOKEN
                    } else {
                        hit.program.as_str()
                    };
                    let ix = tx::token_close_account(&hit.account, addr, addr, prog);
                    let _ = send_signed_extra(rpc, wallet, vec![ix], kp);
                }
            }
        }
        if let Ok(bal) = rpc.get_balance(addr) {
            if bal > 8_000 {
                let ix = tx::system_transfer(addr, payer, bal.saturating_sub(5_000));
                let _ = send_signed_extra(rpc, wallet, vec![ix], kp);
            }
        }
    }
}

fn run_bot(ctx: &Ctx, wallet: crate::ctx::Unlocked, status: gtk::Label, go: gtk::Button) {
    let path = env_trim("VIBE_BOT_TASK");
    let action = env_trim("VIBE_BOT_ACTION").if_empty("run");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            status.set_text(&format!("Need a bot task file: {e}"));
            go.set_sensitive(true);
            return;
        }
    };
    let task: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            status.set_text(&format!("Bad bot task JSON: {e}"));
            go.set_sensitive(true);
            return;
        }
    };
    let mode = task
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("volume")
        .to_string();
    let mint = task
        .get("mint")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let symbol = task
        .get("symbol")
        .and_then(|v| v.as_str())
        .unwrap_or("TOKEN")
        .to_string();
    let progress_path = task
        .get("progress_path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let control_path = task
        .get("control_path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let count = task.get("count").and_then(|v| v.as_u64()).unwrap_or(10).max(1).min(100);
    let amount_min = task.get("amount_min").and_then(|v| v.as_f64()).unwrap_or(0.005);
    let amount_max = task.get("amount_max").and_then(|v| v.as_f64()).unwrap_or(amount_min).max(amount_min);
    let interval_min = task.get("interval_min").and_then(|v| v.as_f64()).unwrap_or(2.0).max(0.2);
    let interval_max = task.get("interval_max").and_then(|v| v.as_f64()).unwrap_or(interval_min).max(interval_min);
    let transit = task.get("transit").and_then(|v| v.as_bool()).unwrap_or(false);
    let dry = task.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
    let loop_run = task.get("loop").and_then(|v| v.as_bool()).unwrap_or(false);
    let auto_return = task.get("auto_return").and_then(|v| v.as_bool()).unwrap_or(true);
    let threads = task
        .get("threads")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .clamp(1, 8);
    let max_loss = task
        .get("max_loss_sol")
        .and_then(|v| v.as_f64())
        .filter(|n| *n > 0.0)
        .and_then(|n| parse_ui_amount(&n.to_string(), 9).ok())
        .unwrap_or(0);
    let jito_tip = task
        .get("jito_tip_sol")
        .and_then(|v| v.as_f64())
        .filter(|n| *n > 0.0)
        .and_then(|n| parse_ui_amount(&n.to_string(), 9).ok())
        .unwrap_or(0);
    let jito = jito_tip > 0;
    let mut wallets: Vec<(String, vibe_core::Keypair)> = Vec::new();
    if let Some(arr) = task.get("wallets").and_then(|v| v.as_array()) {
        for row in arr {
            let addr = row.get("address").and_then(|v| v.as_str()).unwrap_or("");
            let secret = row.get("secret").and_then(|v| v.as_array());
            let Some(secret) = secret else { continue };
            let bytes: Vec<u8> = secret.iter().filter_map(|x| x.as_u64().map(|n| n as u8)).collect();
            if bytes.len() != 64 {
                continue;
            }
            if let Ok(kp) = vibe_core::Keypair::from_secret64(&bytes) {
                let got = kp.address();
                if addr.is_empty() || got == addr {
                    wallets.push((got, kp));
                }
            }
        }
    }
    if wallets.is_empty() {
        status.set_text("No task wallets in the bot file");
        go.set_sensitive(true);
        return;
    }
    if !vibe_core::format::looks_like_pubkey(&mint) {
        status.set_text("Need a token mint");
        go.set_sensitive(true);
        return;
    }
    let from = wallet.address.clone();
    let rpc = ctx.rpc();
    let key = ctx.state.borrow().settings.jupiter_api_key.clone();
    let slip_settings = ctx.state.borrow().settings.slippage_bps;
    let slip_task = task
        .get("slippage_bps")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u16;
    let slip = if slip_task > 0 {
        Some(slip_task.max(50))
    } else if slip_settings == 0 {
        Some(800)
    } else {
        Some(slip_settings.max(50))
    };
    let wallet = wallet.clone();
    let ctx_done = ctx.clone();
    let mint_n = mint.clone();
    let mode_n = mode.clone();
    let symbol_n = symbol.clone();
    let hold = std::rc::Rc::new(std::cell::RefCell::new(None::<gio::ApplicationHoldGuard>));
    if let Some(app) = ctx.window.application() {
        hold.borrow_mut().replace(app.hold());
    }
    ctx.window.set_hide_on_close(true);
    status.set_text("Bot keeps running after this sheet closes. Control it from Runs.");
    let win_hide = ctx.window.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(900), move || {
        win_hide.set_visible(false);
        glib::ControlFlow::Break
    });
    let hold_done = hold.clone();
    task::spawn(
        move || -> Result<String, String> {
            let client = jupiter::Client::from_env_or(&key);
            let lo = parse_ui_amount(&amount_min.to_string(), 9).unwrap_or(5_000_000);
            let hi = parse_ui_amount(&amount_max.to_string(), 9).unwrap_or(lo).max(lo);
            let ata_rent = 2_039_280u64;
            let per = hi.saturating_add(ata_rent).saturating_add(300_000);
            let mut logs: Vec<String> = Vec::new();
            let mut trades: Vec<serde_json::Value> = Vec::new();
            let mut spent = 0u64;
            let mut bag_sol = 0u64;
            let pid = std::process::id();
            let push = |logs: &mut Vec<String>, line: String| {
                logs.push(line);
                if logs.len() > 40 {
                    logs.remove(0);
                }
            };
            let paint = |status: &str,
                         done: u64,
                         last: &str,
                         logs: &[String],
                         ok: u64,
                         fail: u64,
                         trades: &[serde_json::Value],
                         spent: u64,
                         wallet_sol: u64| {
                let total = if loop_run { 0 } else { count };
                let pct = if total == 0 {
                    0
                } else {
                    (done.saturating_mul(100)) / total
                };
                bot_write_progress(
                    &progress_path,
                    serde_json::json!({
                        "status": status,
                        "done": done,
                        "total": total,
                        "last": last,
                        "logs": logs,
                        "ok": ok,
                        "fail": fail,
                        "pid": pid,
                        "pct": pct,
                        "trades": trades,
                        "mint": mint,
                        "symbol": symbol,
                        "mode": mode,
                        "spent_lamports": spent,
                        "wallet_sol": wallet_sol,
                        "dry_run": dry,
                        "loop": loop_run,
                    }),
                );
            };
            if action == "cleanup" {
                paint("cleanup", 0, "Cleaning up wallets", &logs, 0, 0, &trades, spent, bag_sol);
                bot_sweep(&rpc, &wallet, &client, &mint, &from, &wallets, slip, dry, &control_path);
                bag_sol = bot_wallet_sol(&rpc, &wallets);
                paint("completed", wallets.len() as u64, "Cleanup done", &logs, 0, 0, &trades, spent, bag_sol);
                return Ok("cleanup".into());
            }
            let quote = client
                .order(vibe_core::WSOL_MINT, &mint, lo.max(1), None, slip)
                .map_err(|e| e.to_string())?;
            if quote.out_amount.is_empty() || quote.out_amount == "0" {
                return Err(quote.error_message.if_empty("No Jupiter liquidity for this mint."));
            }
            if let Some(impact) = quote.price_impact {
                if impact > 0.15 {
                    return Err(format!("Price impact {impact:.1}% — pool is too thin."));
                }
            }
            if let Ok(risks) = rpc.mint_risks(&mint) {
                if risks.freeze {
                    push(&mut logs, "Freeze authority is still open.".into());
                }
                if risks.mint_authority {
                    push(&mut logs, "Mint authority is still open.".into());
                }
            }
            paint("funding", 0, "Funding task wallets", &logs, 0, 0, &trades, spent, bag_sol);
            if !dry && transit {
                let hop = vibe_core::Keypair::generate();
                let hop_addr = hop.address();
                let bulk = per.saturating_mul(wallets.len() as u64).saturating_add(50_000);
                send_signed(&rpc, &wallet, vec![tx::system_transfer(&from, &hop_addr, bulk)])?;
                bot_sleep_ms(800);
                for chunk in wallets.chunks(8) {
                    let mut ixs = Vec::new();
                    for (addr, _) in chunk {
                        ixs.push(tx::system_transfer(&hop_addr, addr, per));
                    }
                    send_signed_extra(&rpc, &wallet, ixs, &hop)?;
                }
            } else if !dry {
                for chunk in wallets.chunks(8) {
                    let mut ixs = Vec::new();
                    for (addr, _) in chunk {
                        ixs.push(tx::system_transfer(&from, addr, per));
                    }
                    send_signed(&rpc, &wallet, ixs)?;
                }
            }
            bot_shuffle(&mut wallets, pid as u64);
            push(
                &mut logs,
                if dry {
                    format!("Dry-run · {} wallets", wallets.len())
                } else {
                    format!("Funded {} wallets", wallets.len())
                },
            );
            bag_sol = bot_wallet_sol(&rpc, &wallets);
            paint("running", 0, "Funded", &logs, 0, 0, &trades, spent, bag_sol);
            let mut ok = 0u64;
            let mut fail = 0u64;
            let mut i = 0u64;
            loop {
                if !loop_run && i >= count {
                    break;
                }
                loop {
                    let ctl = bot_control(&control_path);
                    if ctl == "stop" {
                        paint("stopped", i, "Stopped", &logs, ok, fail, &trades, spent, bag_sol);
                        return Ok("stopped".into());
                    }
                    if ctl != "pause" {
                        break;
                    }
                    paint("paused", i, "Paused", &logs, ok, fail, &trades, spent, bag_sol);
                    if bot_wait(400, &control_path) == "stop" {
                        paint("stopped", i, "Stopped", &logs, ok, fail, &trades, spent, bag_sol);
                        return Ok("stopped".into());
                    }
                }
                let wait_ms = (bot_jitter(
                    (interval_min * 1000.0) as u64,
                    (interval_max * 1000.0) as u64,
                    i.wrapping_add(7),
                ))
                .max(200)
                    / threads.max(1);
                let waited = bot_wait(wait_ms, &control_path);
                if waited == "stop" {
                    paint("stopped", i, "Stopped", &logs, ok, fail, &trades, spent, bag_sol);
                    return Ok("stopped".into());
                }
                if waited == "pause" {
                    continue;
                }
                if mode != "holders" && bot_jitter(0, 8, i.wrapping_mul(31)) == 0 {
                    i += 1;
                    trades.push(serde_json::json!({
                        "i": i,
                        "side": "skip",
                        "ok": true,
                        "sig": "",
                        "wallet": "",
                        "amount": 0,
                        "err": "jitter skip",
                    }));
                    continue;
                }
                if i > 0 && i % 5 == 0 {
                    bot_shuffle(&mut wallets, pid as u64 + i);
                }
                let (addr, kp) = &wallets[i as usize % wallets.len()];
                let spend = bot_jitter(lo, hi, i.wrapping_mul(13).wrapping_add(1));
                if max_loss > 0 && spent.saturating_add(spend) > max_loss {
                    push(&mut logs, "Max loss reached.".into());
                    break;
                }
                let buy = mode != "volatility" || bot_jitter(0, 1, i) == 0;
                let side = if buy || mode == "holders" { "buy" } else { "sell" };
                if jito_tip > 0 && !dry {
                    let tip_idx = (i as usize) % jupiter::JITO_TIP_ACCOUNTS.len();
                    let tip_to = jupiter::JITO_TIP_ACCOUNTS[tip_idx];
                    let ix = tx::system_transfer(addr, tip_to, jito_tip);
                    let _ = send_signed_extra(&rpc, &wallet, vec![ix], kp);
                }
                let result = if buy || mode == "holders" {
                    bot_exec_swap(
                        &client,
                        kp,
                        addr,
                        vibe_core::WSOL_MINT,
                        &mint,
                        spend,
                        slip,
                        dry,
                        2,
                        jito,
                    )
                } else {
                    let bag = rpc.get_tokens(addr).unwrap_or_default();
                    let raw = bag
                        .iter()
                        .find(|t| t.mint == mint)
                        .and_then(|t| t.amount_raw.parse::<u64>().ok())
                        .unwrap_or(0);
                    if raw == 0 {
                        bot_exec_swap(
                            &client,
                            kp,
                            addr,
                            vibe_core::WSOL_MINT,
                            &mint,
                            spend,
                            slip,
                            dry,
                            2,
                            jito,
                        )
                    } else {
                        bot_exec_swap(
                            &client, kp, addr, &mint, vibe_core::WSOL_MINT, raw, slip, dry, 2, jito,
                        )
                    }
                };
                match result {
                    Ok(sig) => {
                        ok += 1;
                        spent = spent.saturating_add(spend);
                        push(
                            &mut logs,
                            format!("ok {} {symbol}", &sig[..8.min(sig.len())]),
                        );
                        trades.push(serde_json::json!({
                            "i": i + 1,
                            "side": side,
                            "ok": true,
                            "sig": sig,
                            "wallet": addr,
                            "amount": spend,
                            "err": "",
                            "sim": dry,
                        }));
                        if mode == "volume" {
                            let gap = bot_jitter(800, 1800, i.wrapping_add(3));
                            if bot_wait(gap, &control_path) == "stop" {
                                i += 1;
                                break;
                            }
                            let bag = rpc.get_tokens(addr).unwrap_or_default();
                            if let Some(hit) = bag.iter().find(|t| t.mint == mint) {
                                let raw: u64 = hit.amount_raw.parse().unwrap_or(0);
                                if raw > 0 {
                                    if let Ok(sold) = bot_exec_swap(
                                        &client,
                                        kp,
                                        addr,
                                        &mint,
                                        vibe_core::WSOL_MINT,
                                        raw,
                                        slip,
                                        dry,
                                        2,
                                        jito,
                                    ) {
                                        trades.push(serde_json::json!({
                                            "i": i + 1,
                                            "side": "sell",
                                            "ok": true,
                                            "sig": sold,
                                            "wallet": addr,
                                            "amount": raw,
                                            "err": "",
                                            "sim": dry,
                                        }));
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        fail += 1;
                        let err = e.to_string();
                        push(&mut logs, format!("fail {err}"));
                        trades.push(serde_json::json!({
                            "i": i + 1,
                            "side": side,
                            "ok": false,
                            "sig": "",
                            "wallet": addr,
                            "amount": spend,
                            "err": err,
                        }));
                    }
                }
                i += 1;
                bag_sol = bot_wallet_sol(&rpc, &wallets);
                paint(
                    "running",
                    i,
                    &format!("{ok} ok · {fail} fail"),
                    &logs,
                    ok,
                    fail,
                    &trades,
                    spent,
                    bag_sol,
                );
            }
            if auto_return && !dry && (mode == "volume" || mode == "volatility") {
                push(&mut logs, "Returning leftover SOL…".into());
                paint("cleanup", i, "Returning leftover SOL", &logs, ok, fail, &trades, spent, bag_sol);
                bot_sweep(&rpc, &wallet, &client, &mint, &from, &wallets, slip, dry, &control_path);
                bag_sol = bot_wallet_sol(&rpc, &wallets);
            }
            paint(
                "completed",
                i,
                &format!("{ok} ok · {fail} fail"),
                &logs,
                ok,
                fail,
                &trades,
                spent,
                bag_sol,
            );
            Ok(format!("{ok}/{i}"))
        },
        move |res| {
            match &res {
                Ok(_) => ctx_done.toast("Trading bot finished"),
                Err(e) => ctx_done.toast(e.clone()),
            }
            hold_done.borrow_mut().take();
            if let Some(app) = ctx_done.window.application() {
                glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
                    app.quit();
                    glib::ControlFlow::Break
                });
            }
            let _ = (status, go, mode_n, symbol_n, mint_n);
        },
    );
}

fn run_stake(ctx: &Ctx, wallet: crate::ctx::Unlocked, status: gtk::Label, go: gtk::Button) {
    let action = env_trim("VIBE_STAKE_ACTION");
    let vote = env_trim("VIBE_STAKE_VOTE");
    let amount_s = env_trim("VIBE_STAKE_AMOUNT");
    let unstake = env_trim("VIBE_UNSTAKE_ACCOUNT");
    let from = wallet.address.clone();
    let rpc = ctx.rpc();
    let ctx_done = ctx.clone();
    let action_n = action.clone();
    let amount_n = amount_s.clone();
    let vote_n = vote.clone();
    let wallet = wallet.clone();
    task::spawn(
        move || -> Result<String, String> {
            if action == "unstake" || action == "deactivate" {
                return send_signed(&rpc, &wallet, vec![tx::stake_deactivate(&unstake, &from)]);
            }
            if action == "withdraw" {
                let lamports = rpc
                    .get_balance(&unstake)
                    .map_err(|e| e.to_string())?;
                return send_signed(
                    &rpc,
                    &wallet,
                    vec![tx::stake_withdraw(&unstake, &from, &from, lamports)],
                );
            }
            let lamports = parse_ui_amount(&amount_s, 9).map_err(|e| e.to_string())?;
            if lamports == 0 {
                return Err("Invalid stake amount".into());
            }
            let min_del = rpc
                .get_stake_minimum_delegation()
                .unwrap_or(vibe_core::LAMPORTS_PER_SOL);
            if lamports < min_del {
                return Err(format!(
                    "Solana requires at least {} SOL to stake (you entered {} SOL)",
                    vibe_core::format_sol(min_del),
                    amount_s
                ));
            }
            let rent = rpc.get_min_rent(200).map_err(|e| e.to_string())?;
            let stake_kp = vibe_core::Keypair::generate();
            let stake = stake_kp.address();
            let create = tx::create_account(
                &from,
                &stake,
                lamports.saturating_add(rent),
                200,
                ProgramIds::STAKE,
            )
            .map_err(|e| e.to_string())?;
            let init = tx::stake_initialize(&stake, &from, &from).map_err(|e| e.to_string())?;
            let del = tx::stake_delegate(&stake, &vote, &from);
            send_signed_extra(&rpc, &wallet, vec![create, init, del], &stake_kp)
        },
        move |res| {
            let pay = format!("{} SOL", amount_n);
            finish(
                &ctx_done,
                &status,
                &go,
                res,
                "stake",
                &pay,
                &action_n,
                "native",
                &vote_n,
            )
        },
    );
}

fn run_mint(ctx: &Ctx, wallet: crate::ctx::Unlocked, status: gtk::Label, go: gtk::Button) {
    let action = env_trim("VIBE_MINT_ACTION").if_empty("create");
    let name = env_trim("VIBE_MINT_NAME").if_empty("Token");
    let symbol = env_trim("VIBE_MINT_SYMBOL").if_empty("TOKEN");
    let decimals: u8 = env_trim("VIBE_MINT_DECIMALS").parse().unwrap_or(6);
    let supply_s = env_trim("VIBE_MINT_SUPPLY").if_empty("0");
    let fee_bps: u16 = env_trim("VIBE_MINT_FEE_BPS").parse().unwrap_or(0);
    let uri = env_trim("VIBE_MINT_URI");
    let desc = env_trim("VIBE_MINT_DESC");
    let website = env_trim("VIBE_MINT_WEBSITE");
    let twitter = env_trim("VIBE_MINT_TWITTER");
    let telegram = env_trim("VIBE_MINT_TELEGRAM");
    let discord = env_trim("VIBE_MINT_DISCORD");
    let github = env_trim("VIBE_MINT_GITHUB");
    let tags = env_trim("VIBE_MINT_TAGS");
    let revoke_mint = env_flag("VIBE_MINT_REVOKE") || env_flag("VIBE_MINT_REVOKE_MINT");
    let revoke_freeze = env_flag("VIBE_MINT_REVOKE_FREEZE");
    let revoke_update = env_flag("VIBE_MINT_REVOKE_UPDATE");
    let close_authority = env_flag("VIBE_MINT_CLOSE");
    let permanent_delegate = env_flag("VIBE_MINT_DELEGATE");
    let soulbound = env_flag("VIBE_MINT_SOULBOUND");
    let default_frozen = env_flag("VIBE_MINT_FROZEN");
    let interest_bps: i16 = env_trim("VIBE_MINT_INTEREST").parse().unwrap_or(0);
    let existing = env_trim("VIBE_MINT_MINT");
    let token_program = mint_token_program();
    let is_spl = token_program == ProgramIds::TOKEN;
    let fee_auth_s = env_trim("VIBE_MINT_FEE_AUTH");
    let fee_withdraw_s = env_trim("VIBE_MINT_FEE_WITHDRAW");
    let from = wallet.address.clone();
    let rpc = ctx.rpc();
    let ctx_done = ctx.clone();
    let wallet = wallet.clone();
    let name_n = name.clone();
    let symbol_n = symbol.clone();
    let action_n = action.clone();
    task::spawn(
        move || -> Result<String, String> {
            match action.as_str() {
                "update" => {
                    let mint = existing;
                    if mint.is_empty() {
                        return Err("Need a mint to update".into());
                    }
                    if is_spl {
                        let ix = tx::update_metadata_account_v2(
                            &mint, &from, &name, &symbol, &uri, None,
                        )
                        .map_err(|e| e.to_string())?;
                        let sig = send_signed(&rpc, &wallet, vec![ix])?;
                        return Ok(format!("{sig}\n{mint}"));
                    }
                    let extras: Vec<(&str, &str)> = [
                        ("description", desc.as_str()),
                        ("website", website.as_str()),
                        ("twitter", twitter.as_str()),
                        ("telegram", telegram.as_str()),
                        ("discord", discord.as_str()),
                        ("github", github.as_str()),
                        ("tags", tags.as_str()),
                    ]
                    .into_iter()
                    .filter(|(_, val)| !val.is_empty())
                    .collect();
                    let (has_pointer, has_meta, lamports, space) =
                        rpc.token_2022_metadata(&mint).map_err(|e| e.to_string())?;
                    if !has_pointer && !has_meta {
                        return Err(
                            "This mint has no Token-2022 on-chain metadata. Update only works for tokens minted in this studio."
                                .into(),
                        );
                    }
                    let want = space
                        .saturating_add(tx::token_metadata_tlv_len(&name, &symbol, &uri, &extras) as u64)
                        .saturating_add(128);
                    let need = rpc.get_min_rent(want).map_err(|e| e.to_string())?;
                    let mut ixs = Vec::new();
                    if lamports < need {
                        ixs.push(tx::system_transfer(&from, &mint, need - lamports));
                    }
                    if !has_meta {
                        ixs.push(
                            tx::initialize_token_metadata(
                                &mint, &from, &from, &name, &symbol, &uri,
                            )
                            .map_err(|e| e.to_string())?,
                        );
                    } else {
                        if !uri.is_empty() {
                            ixs.push(
                                tx::update_token_metadata_field(&mint, &from, 2, None, &uri)
                                    .map_err(|e| e.to_string())?,
                            );
                        }
                        if !name.is_empty() && name != "Token" {
                            ixs.push(
                                tx::update_token_metadata_field(&mint, &from, 0, None, &name)
                                    .map_err(|e| e.to_string())?,
                            );
                        }
                        if !symbol.is_empty() && symbol != "TOKEN" {
                            ixs.push(
                                tx::update_token_metadata_field(&mint, &from, 1, None, &symbol)
                                    .map_err(|e| e.to_string())?,
                            );
                        }
                    }
                    for &(key, val) in &extras {
                        ixs.push(
                            tx::update_token_metadata_field(&mint, &from, 3, Some(key), val)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    if ixs.is_empty() {
                        return Err("Nothing to update".into());
                    }
                    let sig = send_signed(&rpc, &wallet, ixs)?;
                    Ok(format!("{sig}\n{mint}"))
                }
                "mintmore" => {
                    let mint = existing;
                    if mint.is_empty() {
                        return Err("Need a mint".into());
                    }
                    let supply = parse_ui_amount(&supply_s, decimals).map_err(|e| e.to_string())?;
                    if supply == 0 {
                        return Err("Enter an amount to mint".into());
                    }
                    let ata = tx::associated_token_address(&from, &mint, token_program)
                        .map_err(|e| e.to_string())?;
                    let ixs = vec![
                        tx::create_associated_token_account_idempotent(
                            &from,
                            &from,
                            &mint,
                            token_program,
                        )
                        .map_err(|e| e.to_string())?,
                        tx::mint_to_checked(
                            &mint,
                            &ata,
                            &from,
                            supply,
                            decimals,
                            token_program,
                        ),
                    ];
                    let sig = send_signed(&rpc, &wallet, ixs)?;
                    Ok(format!("{sig}\n{mint}"))
                }
                "revoke_mint" => {
                    let mint = existing;
                    if mint.is_empty() {
                        return Err("Need a mint".into());
                    }
                    let ix = tx::set_authority(&mint, &from, None, 0, token_program)
                        .map_err(|e| e.to_string())?;
                    let sig = send_signed(&rpc, &wallet, vec![ix])?;
                    Ok(format!("{sig}\n{mint}"))
                }
                "revoke_freeze" => {
                    let mint = existing;
                    if mint.is_empty() {
                        return Err("Need a mint".into());
                    }
                    let ix = tx::set_authority(&mint, &from, None, 1, token_program)
                        .map_err(|e| e.to_string())?;
                    let sig = send_signed(&rpc, &wallet, vec![ix])?;
                    Ok(format!("{sig}\n{mint}"))
                }
                "revoke_update" => {
                    let mint = existing;
                    if mint.is_empty() {
                        return Err("Need a mint".into());
                    }
                    let ix = if is_spl {
                        tx::revoke_metaplex_mutable(&mint, &from).map_err(|e| e.to_string())?
                    } else {
                        tx::update_token_metadata_authority(&mint, &from, None)
                            .map_err(|e| e.to_string())?
                    };
                    let sig = send_signed(&rpc, &wallet, vec![ix])?;
                    Ok(format!("{sig}\n{mint}"))
                }
                "burn" => {
                    let mint = existing;
                    if mint.is_empty() {
                        return Err("Need a mint".into());
                    }
                    let want = parse_ui_amount(&supply_s, decimals).map_err(|e| e.to_string())?;
                    let bag = rpc.get_tokens(&from).map_err(|e| e.to_string())?;
                    let hit = bag.iter().find(|t| t.mint == mint);
                    let Some(hit) = hit else {
                        return Err(
                            "This wallet holds 0 of that mint. Wrong CA, or you already burned it."
                                .into(),
                        );
                    };
                    let held: u64 = hit.amount_raw.parse().unwrap_or(0);
                    if held == 0 {
                        return Err("This wallet holds 0 of that mint.".into());
                    }
                    let amount = if want == 0 || want > held { held } else { want };
                    let ata = if hit.account.is_empty() {
                        tx::associated_token_address(&from, &mint, ProgramIds::TOKEN_2022)
                            .map_err(|e| e.to_string())?
                    } else {
                        hit.account.clone()
                    };
                    let program = if hit.program.is_empty() {
                        token_program
                    } else {
                        hit.program.as_str()
                    };
                    let ix = tx::token_burn(&ata, &mint, &from, amount, program);
                    let close = tx::token_close_account(&ata, &from, &from, program);
                    let sig = send_signed(&rpc, &wallet, vec![ix, close])?;
                    Ok(format!("{sig}\n{mint}"))
                }
                "close" => {
                    let mint = existing;
                    if mint.is_empty() {
                        return Err("Need a mint".into());
                    }
                    let ix = tx::token_close_account(&mint, &from, &from, ProgramIds::TOKEN_2022);
                    let sig = send_signed(&rpc, &wallet, vec![ix])?;
                    Ok(format!("{sig}\n{mint}"))
                }
                _ => {
                    let supply = parse_ui_amount(&supply_s, decimals).map_err(|e| e.to_string())?;
                    let mint_kp = mint_keypair_from_env()?;
                    let mint = mint_kp.address();
                    let extras: Vec<(&str, &str)> = [
                        ("description", desc.as_str()),
                        ("website", website.as_str()),
                        ("twitter", twitter.as_str()),
                        ("telegram", telegram.as_str()),
                        ("discord", discord.as_str()),
                        ("github", github.as_str()),
                        ("tags", tags.as_str()),
                    ]
                    .into_iter()
                    .filter(|(_, val)| !val.is_empty())
                    .collect();
                    if is_spl {
                        let rent = rpc
                            .get_min_rent(tx::SPL_MINT_LEN as u64)
                            .map_err(|e| e.to_string())?;
                        let mut ixs = vec![tx::create_account(
                            &from,
                            &mint,
                            rent,
                            tx::SPL_MINT_LEN as u64,
                            ProgramIds::TOKEN,
                        )
                        .map_err(|e| e.to_string())?];
                        let freeze = if revoke_freeze { None } else { Some(from.as_str()) };
                        ixs.push(
                            tx::initialize_mint2(&mint, decimals, &from, freeze, ProgramIds::TOKEN)
                                .map_err(|e| e.to_string())?,
                        );
                        ixs.push(
                            tx::create_metadata_account_v3(
                                &mint,
                                &from,
                                &from,
                                &from,
                                &name,
                                &symbol,
                                &uri,
                                !revoke_update,
                            )
                            .map_err(|e| e.to_string())?,
                        );
                        let ata = tx::associated_token_address(&from, &mint, ProgramIds::TOKEN)
                            .map_err(|e| e.to_string())?;
                        ixs.push(
                            tx::create_associated_token_account_idempotent(
                                &from,
                                &from,
                                &mint,
                                ProgramIds::TOKEN,
                            )
                            .map_err(|e| e.to_string())?,
                        );
                        if supply > 0 {
                            ixs.push(tx::mint_to_checked(
                                &mint,
                                &ata,
                                &from,
                                supply,
                                decimals,
                                ProgramIds::TOKEN,
                            ));
                        }
                        if revoke_mint {
                            ixs.push(
                                tx::set_authority(&mint, &from, None, 0, ProgramIds::TOKEN)
                                    .map_err(|e| e.to_string())?,
                            );
                        }
                        let sig = send_signed_extra(&rpc, &wallet, ixs, &mint_kp)?;
                        return Ok(format!("{sig}\n{mint}"));
                    }
                    let fee = fee_bps > 0;
                    let flags = tx::MintSpace {
                        transfer_fee: fee,
                        close_authority,
                        permanent_delegate,
                        non_transferable: soulbound,
                        default_frozen,
                        interest: interest_bps != 0,
                    };
                    let space = tx::token_2022_mint_create_space(flags) as u64;
                    let rent_space = tx::token_2022_mint_rent_space(
                        flags,
                        &name,
                        &symbol,
                        &uri,
                        &extras,
                    ) as u64;
                    let rent = rpc.get_min_rent(rent_space).map_err(|e| e.to_string())?;
                    let mut ixs = vec![tx::create_account(
                        &from,
                        &mint,
                        rent,
                        space,
                        ProgramIds::TOKEN_2022,
                    )
                    .map_err(|e| e.to_string())?];
                    ixs.push(
                        tx::initialize_metadata_pointer(&mint, &from, &mint)
                            .map_err(|e| e.to_string())?,
                    );
                    if fee {
                        let cfg = if fee_auth_s.is_empty() {
                            from.as_str()
                        } else {
                            fee_auth_s.as_str()
                        };
                        let wd = if fee_withdraw_s.is_empty() {
                            from.as_str()
                        } else {
                            fee_withdraw_s.as_str()
                        };
                        ixs.push(
                            tx::initialize_transfer_fee_config(
                                &mint,
                                Some(cfg),
                                Some(wd),
                                fee_bps.min(10_000),
                                mint_max_fee(decimals, supply),
                            )
                            .map_err(|e| e.to_string())?,
                        );
                    }
                    if close_authority {
                        ixs.push(
                            tx::initialize_mint_close_authority(&mint, &from)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    if permanent_delegate {
                        ixs.push(
                            tx::initialize_permanent_delegate(&mint, &from)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    if soulbound {
                        ixs.push(tx::initialize_non_transferable_mint(&mint));
                    }
                    if default_frozen {
                        ixs.push(tx::initialize_default_account_state(&mint, 2));
                    }
                    if interest_bps != 0 {
                        ixs.push(
                            tx::initialize_interest_bearing(&mint, &from, interest_bps)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    ixs.push(
                        tx::initialize_mint2(
                            &mint,
                            decimals,
                            &from,
                            Some(&from),
                            ProgramIds::TOKEN_2022,
                        )
                        .map_err(|e| e.to_string())?,
                    );
                    ixs.push(
                        tx::initialize_token_metadata(
                            &mint, &from, &from, &name, &symbol, &uri,
                        )
                        .map_err(|e| e.to_string())?,
                    );
                    for &(key, val) in &extras {
                        ixs.push(
                            tx::update_token_metadata_field(&mint, &from, 3, Some(key), val)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    let ata = tx::associated_token_address(&from, &mint, ProgramIds::TOKEN_2022)
                        .map_err(|e| e.to_string())?;
                    ixs.push(
                        tx::create_associated_token_account_idempotent(
                            &from,
                            &from,
                            &mint,
                            ProgramIds::TOKEN_2022,
                        )
                        .map_err(|e| e.to_string())?,
                    );
                    if supply > 0 {
                        if default_frozen {
                            ixs.push(tx::token_thaw_account(
                                &ata,
                                &mint,
                                &from,
                                ProgramIds::TOKEN_2022,
                            ));
                        }
                        ixs.push(tx::mint_to_checked(
                            &mint,
                            &ata,
                            &from,
                            supply,
                            decimals,
                            ProgramIds::TOKEN_2022,
                        ));
                    }
                    if revoke_mint {
                        ixs.push(
                            tx::set_authority(&mint, &from, None, 0, ProgramIds::TOKEN_2022)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    if revoke_freeze {
                        ixs.push(
                            tx::set_authority(&mint, &from, None, 1, ProgramIds::TOKEN_2022)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    if revoke_update {
                        ixs.push(
                            tx::update_token_metadata_authority(&mint, &from, None)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    let sig = send_signed_extra(&rpc, &wallet, ixs, &mint_kp)?;
                    Ok(format!("{sig}\n{mint}"))
                }
            }
        },
        move |res| {
            if let Ok(ref raw) = res {
                let mut lines = raw.lines();
                let sig = lines.next().unwrap_or("").trim();
                let mint = lines.next().unwrap_or("").trim();
                if !mint.is_empty() {
                    let _ = vibe_core::store::write_last_mint(mint, sig, &name_n, &symbol_n);
                }
            }
            finish(
                &ctx_done,
                &status,
                &go,
                res,
                "mint",
                &name_n,
                &symbol_n,
                "",
                "",
            );
            let _ = action_n;
        },
    );
}

fn run_nonce(ctx: &Ctx, wallet: crate::ctx::Unlocked, status: gtk::Label, go: gtk::Button) {
    let action = env_trim("VIBE_NONCE_ACTION").if_empty("create");
    let account = env_trim("VIBE_NONCE_ACCOUNT");
    let amount_s = env_trim("VIBE_NONCE_AMOUNT");
    let from = wallet.address.clone();
    let rpc = ctx.rpc();
    let ctx_done = ctx.clone();
    let wallet = wallet.clone();
    let action_n = action.clone();
    task::spawn(
        move || -> Result<String, String> {
            match action.as_str() {
                "advance" => {
                    if !vibe_core::format::looks_like_pubkey(&account) {
                        return Err("Need a nonce account".into());
                    }
                    send_signed(&rpc, &wallet, vec![tx::nonce_advance(&account, &from)])
                }
                "withdraw" | "close" => {
                    if !vibe_core::format::looks_like_pubkey(&account) {
                        return Err("Need a nonce account".into());
                    }
                    let lamports = if amount_s.is_empty() || action == "close" {
                        rpc.get_balance(&account).map_err(|e| e.to_string())?
                    } else {
                        parse_ui_amount(&amount_s, 9).map_err(|e| e.to_string())?
                    };
                    send_signed(
                        &rpc,
                        &wallet,
                        vec![tx::nonce_withdraw(&account, &from, &from, lamports)],
                    )
                }
                _ => {
                    let nonce_kp = vibe_core::Keypair::generate();
                    let nonce = nonce_kp.address();
                    let rent = rpc
                        .get_min_rent(tx::NONCE_ACCOUNT_LEN as u64)
                        .map_err(|e| e.to_string())?;
                    let create = tx::create_account(
                        &from,
                        &nonce,
                        rent,
                        tx::NONCE_ACCOUNT_LEN as u64,
                        ProgramIds::SYSTEM,
                    )
                    .map_err(|e| e.to_string())?;
                    let init = tx::nonce_initialize(&nonce, &from).map_err(|e| e.to_string())?;
                    let sig = send_signed_extra(&rpc, &wallet, vec![create, init], &nonce_kp)?;
                    let _ = vibe_core::store::remember_nonce(&nonce, &from, &wallet.id);
                    Ok(format!("{sig}\n{nonce}"))
                }
            }
        },
        move |res| {
            finish(
                &ctx_done,
                &status,
                &go,
                res,
                "nonce",
                &action_n,
                "",
                "native",
                "",
            )
        },
    );
}

fn finish(
    ctx: &Ctx,
    status: &gtk::Label,
    go: &gtk::Button,
    res: Result<String, String>,
    kind: &str,
    pay: &str,
    recv: &str,
    in_mint: &str,
    out_mint: &str,
) {
    agent_record_result(&res);
    match res {
        Ok(sig) => {
            let sig = sig.lines().next().unwrap_or(&sig).trim().to_string();
            let url = ctx.cluster().explorer_tx_url(&sig);
            let _ = vibe_core::store::append_swap_event(&sig, &url, pay, recv, in_mint, out_mint, kind);
            status.set_text("Confirmed on Solana");
            ctx.toast("Confirmed on Solana");
            let win = ctx.window.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(700), move || {
                win.close();
                glib::ControlFlow::Break
            });
        }
        Err(e) => {
            status.set_text(&e);
            ctx.toast(e);
            go.set_sensitive(true);
        }
    }
}
