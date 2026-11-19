use adw::prelude::*;
use vibe_core::store;
use vibe_core::{mnemonic, Cluster};

use crate::ctx::{Ctx, Session, Unlocked};
use crate::task;

fn password_entry(placeholder: &str) -> gtk::PasswordEntry {
    let e = gtk::PasswordEntry::new();
    e.set_show_peek_icon(true);
    e.set_placeholder_text(Some(placeholder));
    e.set_hexpand(true);
    e
}

fn env_flag(key: &str) -> bool {
    matches!(
        std::env::var(key).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes")
    )
}

pub fn compact_window() -> bool {
    env_flag("VIBE_SIGNER")
        || env_flag("VIBE_CREATE")
        || env_flag("VIBE_IMPORT")
        || env_flag("VIBE_REVEAL")
}

pub fn require_vault_password(ctx: &Ctx, password: &str) -> Result<(), String> {
    if password.len() < 8 {
        return Err("Password must be at least 8 characters".into());
    }
    let store = ctx.state.borrow().store.clone();
    if store.wallets.is_empty() {
        return Ok(());
    }
    store::unlock_store(&store, password)
        .map(|_| ())
        .map_err(|_| "Use this vault's existing password".to_string())
}

fn attach_wallet(
    ctx: &Ctx,
    store: store::WalletStore,
    rec: store::WalletRecord,
    kp: vibe_core::Keypair,
) {
    let mut existing = ctx
        .state
        .borrow()
        .session
        .as_ref()
        .map(|s| s.unlocked.clone())
        .unwrap_or_default();
    existing.push(Unlocked {
        id: rec.id.clone(),
        name: rec.name.clone(),
        address: rec.address.clone(),
        keypair: Some(kp),
        key_type: rec.key_type.clone(),
        derivation_path: rec.derivation_path.clone(),
        hw_device_id: rec.hw_device_id.clone(),
    });
    let active = existing.len().saturating_sub(1);
    ctx.state.borrow_mut().store = store;
    ctx.state.borrow_mut().session = Some(Session {
        unlocked: existing,
        active,
    });
    ctx.persist();
    if !compact_window() {
        ctx.show_root("main");
        ctx.refresh_wallets_list();
        ctx.refresh();
    }
}

pub fn save_mnemonic_wallet(
    ctx: &Ctx,
    name: &str,
    phrase: &str,
    password: &str,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Name this wallet".into());
    }
    require_vault_password(ctx, password)?;
    let mut store = ctx.state.borrow().store.clone();
    let rec = store::add_mnemonic_wallet(&mut store, name, phrase, password)
        .map_err(|e| e.to_string())?;
    let kp = vibe_core::Keypair::from_mnemonic_default(phrase).map_err(|e| e.to_string())?;
    attach_wallet(ctx, store, rec, kp);
    Ok(())
}

pub fn save_import_wallet(ctx: &Ctx, name: &str, raw: &str, password: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Name this wallet".into());
    }
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Paste a seed or secret key".into());
    }
    require_vault_password(ctx, password)?;
    let mut store = ctx.state.borrow().store.clone();
    let rec = if mnemonic::validate(raw) {
        store::add_mnemonic_wallet(&mut store, name, raw, password)
    } else {
        store::add_secret_wallet(&mut store, name, raw, password)
    }
    .map_err(|e| e.to_string())?;
    let kp = if mnemonic::validate(raw) {
        vibe_core::Keypair::from_mnemonic_default(raw)
    } else {
        rec.secret
            .as_ref()
            .ok_or_else(|| vibe_core::Error::msg("missing vault blob"))
            .and_then(|blob| vibe_core::vault::decrypt(blob, password))
            .and_then(|b| vibe_core::Keypair::from_secret64(&b))
    }
    .map_err(|e| e.to_string())?;
    attach_wallet(ctx, store, rec, kp);
    Ok(())
}

pub fn create_wallet(ctx: &Ctx) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Create wallet");
    dialog.set_content_width(360);
    dialog.set_content_height(340);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);

    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    form.set_margin_top(18);
    form.set_margin_bottom(18);
    form.set_margin_start(22);
    form.set_margin_end(22);
    let intro = gtk::Label::new(Some(
        "A 12-word seed is generated on this machine. It is encrypted with Argon2id and never sent anywhere.",
    ));
    intro.set_wrap(true);
    intro.add_css_class("og-muted");
    intro.set_xalign(0.0);
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Wallet name"));
    name.set_text("Main");
    let pass = password_entry("Password (min 8 characters)");
    let pass2 = password_entry("Confirm password");
    let next = gtk::Button::with_label("Generate seed");
    next.add_css_class("og-primary");
    form.append(&intro);
    form.append(&name);
    form.append(&pass);
    form.append(&pass2);
    form.append(&next);

    let reveal = gtk::Box::new(gtk::Orientation::Vertical, 12);
    reveal.set_margin_top(18);
    reveal.set_margin_bottom(18);
    reveal.set_margin_start(22);
    reveal.set_margin_end(22);
    let warn = gtk::Label::new(Some(
        "Write these 12 words down. Anyone with this seed can spend your SOL. VIBE Wallet cannot recover it.",
    ));
    warn.set_wrap(true);
    warn.add_css_class("og-muted");
    warn.set_xalign(0.0);
    let words = gtk::Label::new(None);
    words.add_css_class("og-mnemonic");
    words.set_wrap(true);
    words.set_selectable(true);
    words.set_xalign(0.0);
    let saved = gtk::CheckButton::with_label("I wrote this seed down and stored it offline");
    let finish = gtk::Button::with_label("Encrypt and open wallet");
    finish.add_css_class("og-primary");
    finish.set_sensitive(false);
    reveal.append(&warn);
    reveal.append(&words);
    reveal.append(&saved);
    reveal.append(&finish);

    stack.add_named(&form, Some("form"));
    stack.add_named(&reveal, Some("seed"));
    dialog.set_child(Some(&stack));

    let phrase = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let saved_c = saved.clone();
    let finish_c = finish.clone();
    saved.connect_toggled(move |btn| finish_c.set_sensitive(btn.is_active()));

    let stack_c = stack.clone();
    let words_c = words.clone();
    let phrase_c = phrase.clone();
    let ctx_err = ctx.clone();
    let name_n = name.clone();
    let pass_n = pass.clone();
    let pass2_n = pass2.clone();
    next.connect_clicked(move |_| {
        let n = name_n.text();
        let p1 = pass_n.text();
        let p2 = pass2_n.text();
        if n.trim().is_empty() {
            ctx_err.toast("Name this wallet");
            return;
        }
        if p1.len() < 8 {
            ctx_err.toast("Password must be at least 8 characters");
            return;
        }
        if p1 != p2 {
            ctx_err.toast("Passwords do not match");
            return;
        }
        match mnemonic::generate(12) {
            Ok(ph) => {
                words_c.set_text(&ph);
                *phrase_c.borrow_mut() = ph;
                stack_c.set_visible_child_name("seed");
            }
            Err(e) => ctx_err.toast(format!("{e}")),
        }
    });

    let ctx_f = ctx.clone();
    let dialog_f = dialog.clone();
    let name_f = name.clone();
    let pass_f = pass.clone();
    finish.connect_clicked(move |_| {
        if !saved_c.is_active() {
            ctx_f.toast("Confirm you saved the seed");
            return;
        }
        let ph = phrase.borrow().clone();
        let password = pass_f.text().to_string();
        let wallet_name = name_f.text().to_string();
        match save_mnemonic_wallet(&ctx_f, &wallet_name, &ph, &password) {
            Ok(()) => {
                ctx_f.toast("Wallet created");
                dialog_f.close();
            }
            Err(e) => ctx_f.toast(e),
        }
        drop(ph);
    });

    dialog.present(Some(&ctx.window));
}

pub fn import_wallet(ctx: &Ctx) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Import wallet");
    dialog.set_content_width(360);
    dialog.set_content_height(420);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    form.set_margin_top(18);
    form.set_margin_bottom(18);
    form.set_margin_start(22);
    form.set_margin_end(22);
    let intro = gtk::Label::new(Some(
        "Paste a 12/24-word seed (Phantom path m/44'/501'/0'/0') or a 64-byte JSON / base58 secret key.",
    ));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("og-muted");
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Wallet name"));
    name.set_text("Imported");
    let secret = gtk::TextView::new();
    secret.set_wrap_mode(gtk::WrapMode::Word);
    secret.set_hexpand(true);
    secret.set_vexpand(true);
    let sw = gtk::ScrolledWindow::new();
    sw.set_min_content_height(140);
    sw.set_child(Some(&secret));
    sw.add_css_class("og-card");
    let pass = password_entry("Password to encrypt this wallet");
    let pass2 = password_entry("Confirm password");
    let go = gtk::Button::with_label("Import");
    go.add_css_class("og-primary");
    form.append(&intro);
    form.append(&name);
    form.append(&sw);
    form.append(&pass);
    form.append(&pass2);
    form.append(&go);
    dialog.set_child(Some(&form));

    let ctx_c = ctx.clone();
    let dialog_c = dialog.clone();
    go.connect_clicked(move |_| {
        let buffer = secret.buffer();
        let (a, b) = (buffer.start_iter(), buffer.end_iter());
        let raw = buffer.text(&a, &b, false).to_string();
        let password = pass.text().to_string();
        let p2 = pass2.text().to_string();
        let wallet_name = name.text().to_string();
        if wallet_name.trim().is_empty() {
            ctx_c.toast("Name this wallet");
            return;
        }
        if password.len() < 8 {
            ctx_c.toast("Password must be at least 8 characters");
            return;
        }
        if password != p2 {
            ctx_c.toast("Passwords do not match");
            return;
        }
        match save_import_wallet(&ctx_c, &wallet_name, &raw, &password) {
            Ok(()) => {
                ctx_c.toast("Wallet imported");
                dialog_c.close();
            }
            Err(e) => ctx_c.toast(e),
        }
    });
    dialog.present(Some(&ctx.window));
}

pub fn connect_ledger(ctx: &Ctx) {
    connect_hardware(ctx);
}

#[derive(Clone)]
enum FoundHw {
    Ledger { path: String, model: String },
    Trezor { path: String, model: String },
}

impl FoundHw {
    fn plugin(&self) -> &'static str {
        match self {
            FoundHw::Ledger { .. } => "ledger",
            FoundHw::Trezor { .. } => "trezor",
        }
    }
    fn path(&self) -> &str {
        match self {
            FoundHw::Ledger { path, .. } | FoundHw::Trezor { path, .. } => path,
        }
    }
    fn model(&self) -> &str {
        match self {
            FoundHw::Ledger { model, .. } | FoundHw::Trezor { model, .. } => model,
        }
    }
    fn default_path(&self) -> &'static str {
        match self {
            FoundHw::Ledger { .. } => vibe_core::ledger::DEFAULT_PATH,
            FoundHw::Trezor { .. } => vibe_core::trezor::DEFAULT_PATH,
        }
    }
}

pub fn connect_hardware(ctx: &Ctx) {
    let mut found: Vec<FoundHw> = Vec::new();
    match vibe_core::ledger::scan_devices() {
        Ok(list) => {
            for d in list {
                found.push(FoundHw::Ledger {
                    path: d.path,
                    model: d.model,
                });
            }
        }
        Err(e) => ctx.toast(e.to_string()),
    }
    match vibe_core::trezor::scan_devices() {
        Ok(list) => {
            for d in list {
                found.push(FoundHw::Trezor {
                    path: d.path,
                    model: d.model,
                });
            }
        }
        Err(e) => ctx.toast(e.to_string()),
    }
    if found.is_empty() {
        ctx.toast("Plug in a Ledger (Solana app open) or a Trezor Safe 3 / T. Quit Trezor Suite if it has the device.");
        return;
    }
    let device = found[0].clone();
    let dialog = adw::Dialog::new();
    dialog.set_title("Connect Hardware");
    dialog.set_content_width(380);
    dialog.set_content_height(360);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    form.set_margin_top(18);
    form.set_margin_bottom(18);
    form.set_margin_start(22);
    form.set_margin_end(22);
    let intro = gtk::Label::new(Some(
        "Confirm the address on the device. The seed never leaves the Ledger or Trezor.",
    ));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("og-muted");
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Wallet name"));
    name.set_text(device.model());
    let path = gtk::Entry::new();
    path.set_text(device.default_path());
    path.set_placeholder_text(Some("Derivation path"));
    let go = gtk::Button::with_label("Confirm on device");
    go.add_css_class("og-primary");
    let status = gtk::Label::new(Some(&format!(
        "{} · {} device{}",
        device.model(),
        found.len(),
        if found.len() == 1 { "" } else { "s" }
    )));
    status.add_css_class("og-muted");
    let ctx_c = ctx.clone();
    let dialog_c = dialog.clone();
    let name_c = name.clone();
    let path_c = path.clone();
    let status_c = status.clone();
    go.connect_clicked(move |_| {
        let wallet_name = name_c.text().to_string();
        let deriv = path_c.text().to_string();
        status_c.set_text(&format!("Confirm the address on the {}…", device.model()));
        let ctx_d = ctx_c.clone();
        let dialog_d = dialog_c.clone();
        let status_d = status_c.clone();
        let device_path = device.path().to_string();
        let plugin = device.plugin().to_string();
        let plugin_bg = plugin.clone();
        let model = device.model().to_string();
        let deriv_bg = deriv.clone();
        task::spawn(
            move || -> Result<(String, String), String> {
                let addr = if plugin_bg == "trezor" {
                    vibe_core::trezor::get_address(&device_path, &deriv_bg, true)
                        .map_err(|e| e.to_string())?
                } else {
                    vibe_core::ledger::get_address(&device_path, &deriv_bg, true)
                        .map_err(|e| e.to_string())?
                };
                Ok((addr, device_path))
            },
            move |res| match res {
                Ok((addr, device_path)) => {
                    let mut store = ctx_d.state.borrow().store.clone();
                    match store::add_hardware_wallet(
                        &mut store,
                        &wallet_name,
                        &addr,
                        &plugin,
                        &device_path,
                        &deriv,
                    ) {
                        Ok(rec) => {
                            if store::save_store(&store::wallets_path(), &store).is_ok() {
                                ctx_d.state.borrow_mut().store = store;
                                ctx_d.persist();
                                ctx_d.refresh_wallets_list();
                                ctx_d.toast(format!("{model} {}", vibe_core::shorten_address(&rec.address)));
                                dialog_d.close();
                            }
                        }
                        Err(e) => {
                            status_d.set_text(&e.to_string());
                            ctx_d.toast(e.to_string());
                        }
                    }
                }
                Err(e) => {
                    status_d.set_text(&e);
                    ctx_d.toast(e);
                }
            },
        );
    });
    form.append(&intro);
    form.append(&name);
    form.append(&path);
    form.append(&go);
    form.append(&status);
    dialog.set_child(Some(&form));
    dialog.present(Some(&ctx.window));
}

pub fn unlock(ctx: &Ctx) {
    let pass = password_entry("Password");
    let store = ctx.state.borrow().store.clone();
    match store::unlock_store(&store, &pass.text()) {
        Ok(_) => {}
        Err(_) => {}
    }
    // real unlock used from lock page
}

fn unlocked_row(rec: &store::WalletRecord, kp: Option<vibe_core::Keypair>) -> Unlocked {
    Unlocked {
        id: rec.id.clone(),
        name: rec.name.clone(),
        address: rec.address.clone(),
        keypair: kp,
        key_type: rec.key_type.clone(),
        derivation_path: rec.derivation_path.clone(),
        hw_device_id: rec.hw_device_id.clone(),
    }
}

fn finish_session(ctx: &Ctx, unlocked: Vec<Unlocked>) -> Result<(), String> {
    if unlocked.is_empty() {
        return Err("no wallets on disk".into());
    }
    let want = std::env::var("VIBE_ACTIVE_ID").unwrap_or_default();
    let stored = ctx.state.borrow().store.active_id.clone();
    let target = if !want.is_empty() { want } else { stored };
    let active = unlocked
        .iter()
        .position(|u| u.id == target)
        .or_else(|| unlocked.iter().position(|u| u.address == target))
        .unwrap_or(0);
    ctx.state.borrow_mut().session = Some(Session { unlocked, active });
    if compact_window() {
        return Ok(());
    }
    ctx.show_root("main");
    ctx.refresh_wallets_list();
    ctx.refresh();
    ctx.toast("Unlocked");
    Ok(())
}

pub fn try_unlock(ctx: &Ctx, password: &str) -> Result<(), String> {
    let store = ctx.state.borrow().store.clone();
    if store.wallets.is_empty() {
        return Err("no wallets on disk".into());
    }
    let want = std::env::var("VIBE_ACTIVE_ID").unwrap_or_default();
    let target = if !want.is_empty() {
        want
    } else {
        store.active_id.clone()
    };
    let active_rec = store
        .wallets
        .iter()
        .find(|w| w.id == target || w.address == target)
        .or_else(|| store.wallets.first())
        .cloned();
    if let Some(rec) = &active_rec {
        if rec.key_kind().is_hardware() || rec.key_kind().is_watch() {
            let mut unlocked = Vec::new();
            if !password.is_empty() && store::software_wallet_count(&store) > 0 {
                if let Ok(pairs) = store::unlock_store(&store, password) {
                    for (r, kp) in pairs {
                        unlocked.push(unlocked_row(&r, Some(kp)));
                    }
                }
            }
            for rec in &store.wallets {
                if rec.has_secret() {
                    continue;
                }
                if unlocked.iter().any(|u| u.id == rec.id) {
                    continue;
                }
                unlocked.push(unlocked_row(rec, None));
            }
            return finish_session(ctx, unlocked);
        }
    }
    match store::unlock_store(&store, password) {
        Ok(pairs) => {
            let mut unlocked: Vec<Unlocked> = pairs
                .into_iter()
                .map(|(rec, kp)| unlocked_row(&rec, Some(kp)))
                .collect();
            for rec in &store.wallets {
                if rec.has_secret() {
                    continue;
                }
                if unlocked.iter().any(|u| u.id == rec.id) {
                    continue;
                }
                unlocked.push(unlocked_row(rec, None));
            }
            finish_session(ctx, unlocked)
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn confirm_send(
    ctx: &Ctx,
    to: &str,
    amount: &str,
    symbol: &str,
    on_ok: impl FnOnce() + 'static,
) {
    let from = ctx.active_address().unwrap_or_default();
    let cluster = ctx.cluster();
    let body = format!(
        "From: {from}\nTo: {to}\nAmount: {amount} {symbol}\nCluster: {}\n\nThis will be simulated, then sent if you confirm.",
        cluster.as_label()
    );
    let d = adw::AlertDialog::new(Some("Review send"), Some(&body));
    d.add_response("cancel", "Cancel");
    d.add_response("send", "Simulate and send");
    d.set_response_appearance("send", adw::ResponseAppearance::Destructive);
    d.set_default_response(Some("cancel"));
    let on_ok = std::cell::RefCell::new(Some(on_ok));
    d.connect_response(None, move |_, resp| {
        if resp == "send" {
            if let Some(cb) = on_ok.borrow_mut().take() {
                cb();
            }
        }
    });
    d.present(Some(&ctx.window));
}

pub fn confirm_swap(
    ctx: &Ctx,
    pay: &str,
    recv: &str,
    in_mint: &str,
    out_mint: &str,
    impact: Option<f64>,
    route: &str,
    on_ok: impl FnOnce() + 'static,
) {
    let impact_txt = impact
        .map(|p| format!("{p:.4}%"))
        .unwrap_or_else(|| "—".into());
    let high = impact.map(|p| p >= 3.0).unwrap_or(false);
    let severe = impact.map(|p| p >= 10.0).unwrap_or(false);
    let mut summary = format!(
        "Pay {pay}\nReceive about {recv}\nRoute {route}\nPrice impact {impact_txt}\nCluster Mainnet\n\nSigning stays in this vault. The swap cannot be reversed once it lands."
    );
    if severe {
        summary.push_str("\n\nPrice impact is very high. You may receive far less than expected.");
    } else if high {
        summary.push_str("\n\nPrice impact is elevated. Check the destination mint before signing.");
    }
    let d = adw::AlertDialog::new(Some("Confirm swap"), Some(&summary));
    d.add_response("cancel", "Cancel");
    d.add_response("sign", "Sign and swap");
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");
    d.set_response_appearance(
        "sign",
        if severe {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );

    let extra = gtk::Box::new(gtk::Orientation::Vertical, 8);
    extra.set_margin_top(8);
    let in_lab = gtk::Label::new(Some(&format!("Pay mint  {in_mint}")));
    in_lab.set_wrap(true);
    in_lab.set_xalign(0.0);
    in_lab.set_selectable(true);
    in_lab.add_css_class("monospace");
    let out_lab = gtk::Label::new(Some(&format!("Receive mint  {out_mint}")));
    out_lab.set_wrap(true);
    out_lab.set_xalign(0.0);
    out_lab.set_selectable(true);
    out_lab.add_css_class("monospace");
    let check = gtk::CheckButton::with_label("I verified the destination mint. Sign this swap.");
    extra.append(&in_lab);
    extra.append(&out_lab);
    extra.append(&check);
    d.set_extra_child(Some(&extra));
    d.set_response_enabled("sign", false);
    let dlg = d.clone();
    check.connect_toggled(move |btn| {
        dlg.set_response_enabled("sign", btn.is_active());
    });

    let on_ok = std::cell::RefCell::new(Some(on_ok));
    d.connect_response(None, move |_, resp| {
        if resp == "sign" {
            if let Some(cb) = on_ok.borrow_mut().take() {
                cb();
            }
        }
    });
    d.present(Some(&ctx.window));
}

pub fn swap_landed(ctx: &Ctx, signature: &str, pay: &str, recv: &str) {
    let url = ctx.cluster().explorer_tx_url(signature);
    let body = format!("{pay} → {recv}\n\n{url}");
    let d = adw::AlertDialog::new(Some("Swap confirmed"), Some(&body));
    d.add_response("close", "Close");
    d.add_response("solscan", "Open Vibe Explorer");
    d.set_response_appearance("solscan", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("solscan"));
    let url_c = url.clone();
    d.connect_response(None, move |_, resp| {
        if resp == "solscan" {
            let _ = gio::AppInfo::launch_default_for_uri(&url_c, gio::AppLaunchContext::NONE);
        }
    });
    d.present(Some(&ctx.window));
}

pub fn notify_swap(ctx: &Ctx, signature: &str, pay: &str, recv: &str) {
    notify_tx(ctx, "swap", signature, pay, recv);
}

pub fn notify_tx(ctx: &Ctx, kind: &str, signature: &str, pay: &str, recv: &str) {
    let url = ctx.cluster().explorer_tx_url(signature);
    let title = match kind {
        "send" => "VIBE Wallet — send confirmed",
        "stake" | "unstake" | "withdraw" => "VIBE Wallet — stake confirmed",
        "receive" => "VIBE Wallet — received",
        _ => "VIBE Wallet — swap confirmed",
    };
    let line = if pay.is_empty() {
        recv.to_string()
    } else if recv.is_empty() {
        pay.to_string()
    } else {
        format!("{pay} → {recv}")
    };
    if let Some(app) = ctx.window.application() {
        let n = gio::Notification::new(title);
        n.set_body(Some(&format!("{line}\nTap to open Vibe Explorer")));
        n.set_priority(gio::NotificationPriority::High);
        n.set_default_action_and_target_value("app.open-solscan", Some(&signature.to_variant()));
        let id = format!("vibe-{kind}-{signature}");
        app.send_notification(Some(&id), &n);
    }
    let toast = adw::Toast::new(&format!("{line}  confirmed"));
    toast.set_button_label(Some("Vibe Explorer"));
    toast.set_timeout(0);
    let url_t = url.clone();
    toast.connect_button_clicked(move |_| {
        let _ = gio::AppInfo::launch_default_for_uri(&url_t, gio::AppLaunchContext::NONE);
    });
    ctx.toasts.add_toast(toast);
}

pub fn confirm_generic(
    ctx: &Ctx,
    title: &str,
    body: &str,
    action: &str,
    on_ok: impl FnOnce() + 'static,
) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("cancel", "Cancel");
    d.add_response("ok", action);
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    let on_ok = std::cell::RefCell::new(Some(on_ok));
    d.connect_response(None, move |_, resp| {
        if resp == "ok" {
            if let Some(cb) = on_ok.borrow_mut().take() {
                cb();
            }
        }
    });
    d.present(Some(&ctx.window));
}

pub fn airdrop(ctx: &Ctx) {
    if !matches!(ctx.cluster(), Cluster::Devnet | Cluster::Testnet) {
        ctx.toast("Airdrop only works on Devnet or Testnet. Switch cluster first.");
        return;
    }
    let Some(addr) = ctx.active_address() else {
        ctx.toast("Unlock a wallet first");
        return;
    };
    let rpc = ctx.rpc();
    let ctx_c = ctx.clone();
    ctx.toast("Requesting 1 SOL airdrop…");
    task::spawn(
        move || rpc.request_airdrop(&addr, vibe_core::LAMPORTS_PER_SOL),
        move |res| match res {
            Ok(sig) => {
                ctx_c.toast(format!("Airdrop submitted: {}", vibe_core::shorten_address(&sig)));
                ctx_c.refresh();
            }
            Err(e) => ctx_c.toast(format!("{e}")),
        },
    );
}
