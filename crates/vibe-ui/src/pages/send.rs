// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use vibe_core::store;
use vibe_core::{parse_ui_amount, tx, Cluster, ProgramIds, WSOL_MINT};

use crate::ctx::{Ctx, ShellWidgets};
use crate::dialogs;
use crate::task;

#[derive(Clone, Debug)]
struct SendAsset {
    mint: String,
    symbol: String,
    decimals: u8,
    program: String,
    ui_amount: String,
    account: String,
}

fn is_native_mint(mint: &str) -> bool {
    mint.is_empty()
        || mint.eq_ignore_ascii_case("native")
        || mint.eq_ignore_ascii_case("sol")
        || mint == WSOL_MINT
}

fn env_trim(key: &str) -> String {
    std::env::var(key)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub fn build(ctx: &Ctx, shell: &ShellWidgets) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_vexpand(true);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(720);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 16);
    col.set_margin_top(24);
    col.set_margin_bottom(32);
    col.set_margin_start(24);
    col.set_margin_end(24);

    let views = adw::ViewStack::new();
    views.add_titled(&send_form(ctx), Some("send"), "Send");
    views.add_titled(&receive_form(ctx, shell), Some("receive"), "Receive");
    let switcher = adw::ViewSwitcher::new();
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    switcher.set_stack(Some(&views));

    col.append(&switcher);
    col.append(&views);
    clamp.set_child(Some(&col));
    scroller.set_child(Some(&clamp));
    outer.append(&scroller);
    outer.upcast()
}

fn send_form(ctx: &Ctx) -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("og-card");
    let title = gtk::Label::new(Some("Send"));
    title.add_css_class("og-heading");
    title.set_xalign(0.0);
    let asset_lab = gtk::Label::new(Some("Asset"));
    asset_lab.add_css_class("og-muted");
    asset_lab.set_xalign(0.0);
    let assets = Rc::new(RefCell::new(Vec::<SendAsset>::new()));
    let drop = gtk::DropDown::from_strings(&["SOL"]);
    let bal = gtk::Label::new(Some(""));
    bal.add_css_class("og-muted");
    bal.set_xalign(0.0);
    let dest = gtk::Entry::new();
    dest.set_placeholder_text(Some("Destination address"));
    let amount = gtk::Entry::new();
    amount.set_placeholder_text(Some("Amount"));
    let pref_dest = env_trim("VIBE_SEND_DEST");
    if !pref_dest.is_empty() {
        dest.set_text(&pref_dest);
    }
    let pref_amount = env_trim("VIBE_SEND_AMOUNT");
    if !pref_amount.is_empty() {
        amount.set_text(&pref_amount);
    }
    refill_assets(ctx, &drop, &assets, &bal, &amount);
    let status = gtk::Label::new(None);
    status.add_css_class("og-muted");
    status.set_wrap(true);
    status.set_xalign(0.0);
    let go = gtk::Button::with_label("Review send");
    go.add_css_class("og-primary");

    let ctx_map = ctx.clone();
    let drop_map = drop.clone();
    let assets_map = assets.clone();
    let bal_map = bal.clone();
    let amount_map = amount.clone();
    card.connect_map(move |_| {
        refill_assets(&ctx_map, &drop_map, &assets_map, &bal_map, &amount_map);
    });

    let drop_ch = drop.clone();
    let assets_ch = assets.clone();
    let bal_ch = bal.clone();
    let amount_ch = amount.clone();
    drop.connect_notify_local(Some("selected"), move |_, _| {
        sync_asset_labels(&drop_ch, &assets_ch, &bal_ch, &amount_ch);
    });

    let ctx_c = ctx.clone();
    let dest_c = dest.clone();
    let amount_c = amount.clone();
    let status_c = status.clone();
    let drop_c = drop.clone();
    let assets_c = assets.clone();
    go.connect_clicked(move |_| {
        let to = dest_c.text().to_string();
        if !vibe_core::format::looks_like_pubkey(&to) {
            ctx_c.toast("That does not look like a Solana address");
            return;
        }
        let Some(wallet) = ctx_c.active_wallet() else {
            ctx_c.toast("Unlock a wallet first");
            return;
        };
        let asset = {
            let list = assets_c.borrow();
            let idx = drop_c.selected() as usize;
            list.get(idx).cloned().unwrap_or_else(|| SendAsset {
                mint: "native".into(),
                symbol: "SOL".into(),
                decimals: 9,
                program: String::new(),
                ui_amount: String::new(),
                account: String::new(),
            })
        };
        let raw = match parse_ui_amount(&amount_c.text(), asset.decimals) {
            Ok(v) if v > 0 => v,
            Ok(_) => {
                ctx_c.toast("Enter an amount greater than 0");
                return;
            }
            Err(e) => {
                ctx_c.toast(e.to_string());
                return;
            }
        };
        let from = wallet.address.clone();
        let cluster = ctx_c.cluster();
        let ctx_s = ctx_c.clone();
        let status_s = status_c.clone();
        let to_copy = to.clone();
        let wallet_copy = wallet.clone();
        let amount_label = amount_c.text().to_string();
        let symbol = asset.symbol.clone();
        dialogs::confirm_send(&ctx_c, &to, &amount_label, &symbol, move || {
            status_s.set_text("Simulating…");
            let rpc = ctx_s.rpc();
            let to_s = to_copy.clone();
            let to_done = to_s.clone();
            let from_s = from.clone();
            let ctx_done = ctx_s.clone();
            let status_d = status_s.clone();
            let asset_s = asset.clone();
            task::spawn(
                move || -> Result<String, String> {
                    let blockhash = rpc.get_latest_blockhash().map_err(|e| e.to_string())?;
                    let ixs = if is_native_mint(&asset_s.mint) {
                        vec![tx::system_transfer(&from_s, &to_s, raw)]
                    } else {
                        spl_send_ixs(&from_s, &to_s, raw, &asset_s)?
                    };
                    let bytes = wallet_copy
                        .sign_legacy(&from_s, &blockhash, &ixs)?;
                    let b64 =
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
                    let sim = rpc.simulate(&b64).map_err(|e| e.to_string())?;
                    let sig = rpc.send_transaction(&bytes).map_err(|e| e.to_string())?;
                    Ok(format!("{sig}\n\n{sim}"))
                },
                move |res| match res {
                    Ok(out) => {
                        save_contact_from_send(&ctx_done, &to_done);
                        ctx_done.persist();
                        let sig = out.lines().next().unwrap_or("");
                        status_d.set_text(&format!("Sent. {out}"));
                        ctx_done.toast(format!(
                            "Sent {} — {}",
                            vibe_core::shorten_address(sig),
                            cluster_explorer(&cluster, sig)
                        ));
                        ctx_done.refresh();
                    }
                    Err(e) => {
                        status_d.set_text(&e);
                        ctx_done.toast(e);
                    }
                },
            );
        });
    });

    card.append(&title);
    card.append(&asset_lab);
    card.append(&drop);
    card.append(&bal);
    card.append(&dest);
    card.append(&amount);
    card.append(&go);
    card.append(&status);
    card.upcast()
}

fn refill_assets(
    ctx: &Ctx,
    drop: &gtk::DropDown,
    assets: &Rc<RefCell<Vec<SendAsset>>>,
    bal: &gtk::Label,
    amount: &gtk::Entry,
) {
    let mut list = Vec::new();
    let snap = ctx.state.borrow().snapshot.clone();
    let sol_ui = vibe_core::format_sol(snap.lamports);
    list.push(SendAsset {
        mint: "native".into(),
        symbol: "SOL".into(),
        decimals: 9,
        program: String::new(),
        ui_amount: sol_ui,
        account: snap.address.clone(),
    });
    for tok in &snap.tokens {
        if tok.mint.is_empty() || tok.mint == WSOL_MINT {
            continue;
        }
        list.push(SendAsset {
            mint: tok.mint.clone(),
            symbol: vibe_core::tokens::display_symbol(&tok.mint),
            decimals: tok.decimals,
            program: if tok.program.is_empty() {
                ProgramIds::TOKEN.to_string()
            } else {
                tok.program.clone()
            },
            ui_amount: tok.ui_amount.clone(),
            account: tok.account.clone(),
        });
    }
    let pref = env_trim("VIBE_SEND_MINT");
    if !pref.is_empty() && !is_native_mint(&pref) && !list.iter().any(|a| a.mint == pref) {
        let decimals = env_trim("VIBE_SEND_DECIMALS")
            .parse::<u8>()
            .unwrap_or(9);
        let symbol = env_trim("VIBE_SEND_SYMBOL");
        let program = env_trim("VIBE_SEND_PROGRAM");
        list.push(SendAsset {
            mint: pref.clone(),
            symbol: if symbol.is_empty() {
                vibe_core::tokens::display_symbol(&pref)
            } else {
                symbol
            },
            decimals,
            program: if program.is_empty() {
                ProgramIds::TOKEN.to_string()
            } else {
                program
            },
            ui_amount: String::new(),
            account: String::new(),
        });
    }
    let labels: Vec<String> = list
        .iter()
        .map(|a| {
            if a.ui_amount.is_empty() {
                a.symbol.clone()
            } else {
                format!("{} · {}", a.symbol, a.ui_amount)
            }
        })
        .collect();
    let model = gtk::StringList::new(&labels.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    drop.set_model(Some(&model));
    let mut selected = 0u32;
    if !pref.is_empty() {
        if let Some(i) = list.iter().position(|a| {
            a.mint == pref || (is_native_mint(&pref) && is_native_mint(&a.mint))
        }) {
            selected = i as u32;
        }
    }
    drop.set_selected(selected);
    *assets.borrow_mut() = list;
    sync_asset_labels(drop, assets, bal, amount);
}

fn sync_asset_labels(
    drop: &gtk::DropDown,
    assets: &Rc<RefCell<Vec<SendAsset>>>,
    bal: &gtk::Label,
    amount: &gtk::Entry,
) {
    let list = assets.borrow();
    let Some(asset) = list.get(drop.selected() as usize) else {
        bal.set_text("");
        amount.set_placeholder_text(Some("Amount"));
        return;
    };
    if asset.ui_amount.is_empty() {
        bal.set_text(&format!("Sending {}", asset.symbol));
    } else {
        bal.set_text(&format!("Balance: {} {}", asset.ui_amount, asset.symbol));
    }
    amount.set_placeholder_text(Some(&format!("Amount in {}", asset.symbol)));
}

fn spl_send_ixs(
    from: &str,
    to: &str,
    raw: u64,
    asset: &SendAsset,
) -> Result<Vec<tx::Instruction>, String> {
    let program = if asset.program.is_empty() {
        ProgramIds::TOKEN
    } else {
        asset.program.as_str()
    };
    let source = if asset.account.is_empty() {
        tx::associated_token_address(from, &asset.mint, program).map_err(|e| e.to_string())?
    } else {
        asset.account.clone()
    };
    let dest = tx::associated_token_address(to, &asset.mint, program).map_err(|e| e.to_string())?;
    let create =
        tx::create_associated_token_account_idempotent(from, to, &asset.mint, program)
            .map_err(|e| e.to_string())?;
    let xfer = tx::token_transfer_checked(
        &source,
        &asset.mint,
        &dest,
        from,
        raw,
        asset.decimals,
        program,
    );
    Ok(vec![create, xfer])
}

fn cluster_explorer(cluster: &Cluster, sig: &str) -> String {
    cluster.explorer_tx_url(sig)
}

fn receive_form(ctx: &Ctx, shell: &ShellWidgets) -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("og-card");
    let title = gtk::Label::new(Some("Receive"));
    title.add_css_class("og-heading");
    title.set_xalign(0.0);
    let hint = gtk::Label::new(Some(
        "This is your public address. Share it to receive SOL or SPL tokens on the selected cluster.",
    ));
    hint.set_wrap(true);
    hint.add_css_class("og-muted");
    hint.set_xalign(0.0);
    shell.receive_addr.add_css_class("og-mono");
    shell.receive_addr.set_selectable(true);
    shell.receive_addr.set_wrap(true);
    shell.receive_addr.set_xalign(0.0);

    let copy = gtk::Button::with_label("Copy address");
    copy.add_css_class("og-primary");
    let ctx_c = ctx.clone();
    copy.connect_clicked(move |_| {
        if let Some(addr) = ctx_c.active_address() {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&addr);
                ctx_c.toast("Address copied");
            }
        }
    });

    card.append(&title);
    card.append(&hint);
    card.append(&shell.qr_area);
    card.append(&shell.receive_addr);
    card.append(&copy);
    card.upcast()
}

pub fn execute_signed(
    ctx: &Ctx,
    work: impl FnOnce() -> Result<String, String> + Send + 'static,
    ok_msg: &str,
) {
    let ctx_c = ctx.clone();
    let ok = ok_msg.to_string();
    task::spawn(work, move |res| match res {
        Ok(sig) => {
            ctx_c.toast(format!("{ok} {}", vibe_core::shorten_address(&sig)));
            ctx_c.refresh();
        }
        Err(e) => ctx_c.toast(e),
    });
}

pub fn save_contact_from_send(ctx: &Ctx, address: &str) {
    let own: Vec<String> = ctx
        .state
        .borrow()
        .store
        .wallets
        .iter()
        .map(|w| w.address.clone())
        .collect();
    let mut st = ctx.state.borrow_mut();
    store::remember_sent(&mut st.contacts, address, "", "SOL", &own);
}
