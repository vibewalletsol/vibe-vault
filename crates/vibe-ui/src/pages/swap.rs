use vibe_core::i18n::tr;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use vibe_core::format::{format_ui_amount, format_usd, parse_raw_u64, parse_ui_amount};
use vibe_core::jupiter::{self, Order};
use vibe_core::rpc::Snapshot;
use vibe_core::tokens;
use vibe_core::{Cluster, WSOL_MINT};

use crate::ctx::Ctx;
use crate::dialogs;
use crate::task;

fn env_trim(key: &str) -> String {
    std::env::var(key)
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[derive(Clone, Debug)]
struct Asset {
    mint: String,
    symbol: String,
    decimals: u8,
    balance_raw: u64,
}

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_vexpand(true);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(520);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 14);
    col.set_margin_top(24);
    col.set_margin_bottom(32);
    col.set_margin_start(24);
    col.set_margin_end(24);

    let kicker = gtk::Label::new(Some("JUPITER SWAP V2"));
    kicker.add_css_class("og-hero-kicker");
    kicker.set_xalign(0.0);
    let title = gtk::Label::new(Some("Swap"));
    title.add_css_class("og-page-title");
    title.set_xalign(0.0);

    let from_assets: Rc<RefCell<Vec<Asset>>> = Rc::new(RefCell::new(Vec::new()));
    let to_assets: Rc<RefCell<Vec<Asset>>> = Rc::new(RefCell::new(Vec::new()));

    let from_drop = gtk::DropDown::from_strings(&["SOL"]);
    let to_drop = gtk::DropDown::from_strings(&["USDC"]);
    let amount = gtk::Entry::new();
    amount.set_placeholder_text(Some("0.0"));
    amount.set_editable(true);
    amount.set_can_focus(true);
    let bal_lbl = gtk::Label::new(Some("Balance: —"));
    bal_lbl.add_css_class("og-muted");
    bal_lbl.set_xalign(0.0);
    let max = gtk::Button::with_label(tr("Max"));
    max.add_css_class("og-ghost");

    let out_lbl = gtk::Label::new(Some("You receive —"));
    out_lbl.add_css_class("og-heading");
    out_lbl.set_xalign(0.0);
    out_lbl.set_wrap(true);
    let quote_lbl = gtk::Label::new(Some(
        "Quotes use Jupiter Swap V2 (Metis, JupiterZ, Dflow, OKX). Mainnet only.",
    ));
    quote_lbl.set_wrap(true);
    quote_lbl.set_xalign(0.0);
    quote_lbl.add_css_class("og-muted");

    let pay_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    from_drop.set_hexpand(true);
    pay_row.append(&from_drop);
    pay_row.append(&amount);
    pay_row.append(&max);

    let recv_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    to_drop.set_hexpand(true);
    recv_row.append(&to_drop);

    let flip = gtk::Button::with_label(tr("↕ Flip"));
    flip.add_css_class("og-ghost");

    let qbtn = gtk::Button::with_label(tr("Get quote"));
    qbtn.add_css_class("og-ghost");
    let sbtn = gtk::Button::with_label(tr("Review swap"));
    sbtn.add_css_class("og-primary");
    sbtn.set_sensitive(false);

    let order_cell: Rc<RefCell<Option<Order>>> = Rc::new(RefCell::new(None));

    let from_a = from_assets.clone();
    let to_a = to_assets.clone();
    let from_d = from_drop.clone();
    let to_d = to_drop.clone();
    let bal_c = bal_lbl.clone();
    let amount_fill = amount.clone();
    let ctx_fill = ctx.clone();
    let fill = move || {
        let snap = ctx_fill.state.borrow().snapshot.clone();
        let (mut from, mut to) = build_asset_lists(&snap);
        let pref_in = env_trim("VIBE_SWAP_IN");
        let pref_out = env_trim("VIBE_SWAP_OUT");
        let pref_amt = env_trim("VIBE_SWAP_AMOUNT");
        let pref_out_sym = env_trim("VIBE_SWAP_OUT_SYMBOL");
        let pref_out_dec = env_trim("VIBE_SWAP_OUT_DECIMALS")
            .parse::<u8>()
            .unwrap_or(6);
        if !pref_out.is_empty() && !to.iter().any(|a| a.mint == pref_out) {
            to.push(Asset {
                mint: pref_out.clone(),
                symbol: if pref_out_sym.is_empty() {
                    tokens::display_symbol(&pref_out)
                } else {
                    pref_out_sym.clone()
                },
                decimals: pref_out_dec,
                balance_raw: 0,
            });
        }
        let from_labels: Vec<String> = from
            .iter()
            .map(|a| {
                format!(
                    "{}  {}",
                    a.symbol,
                    format_ui_amount(a.balance_raw, a.decimals)
                )
            })
            .collect();
        let to_labels: Vec<String> = to.iter().map(|a| a.symbol.clone()).collect();
        let from_refs: Vec<&str> = from_labels.iter().map(|s| s.as_str()).collect();
        let to_refs: Vec<&str> = to_labels.iter().map(|s| s.as_str()).collect();
        from_d.set_model(Some(&gtk::StringList::new(&from_refs)));
        to_d.set_model(Some(&gtk::StringList::new(&to_refs)));
        if !pref_in.is_empty() {
            if let Some(i) = from.iter().position(|a| a.mint == pref_in) {
                from_d.set_selected(i as u32);
            } else if !from.is_empty() {
                from_d.set_selected(0);
            }
        } else if !from.is_empty() {
            from_d.set_selected(0);
        }
        if !pref_out.is_empty() {
            if let Some(i) = to.iter().position(|a| a.mint == pref_out) {
                to_d.set_selected(i as u32);
            }
        } else {
            let usdc = to.iter().position(|a| a.symbol == "USDC").unwrap_or(0) as u32;
            if !to.is_empty() {
                to_d.set_selected(usdc.min(to.len() as u32 - 1));
            }
        }
        if !pref_amt.is_empty() {
            amount_fill.set_text(&pref_amt);
        }
        *from_a.borrow_mut() = from;
        *to_a.borrow_mut() = to;
        let fi = from_d.selected() as usize;
        let ti = to_d.selected() as usize;
        let from_mint = from_a.borrow().get(fi).map(|a| a.mint.clone());
        let to_mint = to_a.borrow().get(ti).map(|a| a.mint.clone());
        if let (Some(f), Some(t)) = (from_mint, to_mint) {
            if f == t {
                if let Some(i) = to_a.borrow().iter().position(|a| a.mint != f) {
                    to_d.set_selected(i as u32);
                }
            }
        }
        update_balance_label(&bal_c, &from_a.borrow(), from_d.selected());
    };

    let fill_btn = gtk::Button::with_label(tr("Refresh tokens"));
    fill_btn.add_css_class("og-ghost");
    let fill_c = fill.clone();
    fill_btn.connect_clicked(move |_| fill_c());

    let pair_guard = Rc::new(Cell::new(false));
    let from_assets_b = from_assets.clone();
    let to_assets_b = to_assets.clone();
    let to_drop_b = to_drop.clone();
    let bal_b = bal_lbl.clone();
    let pair_guard_b = pair_guard.clone();
    let ctx_b = ctx.clone();
    from_drop.connect_selected_notify(move |drop| {
        update_balance_label(&bal_b, &from_assets_b.borrow(), drop.selected());
        if pair_guard_b.get() {
            return;
        }
        let from = from_assets_b.borrow().get(drop.selected() as usize).cloned();
        let to = to_assets_b.borrow().get(to_drop_b.selected() as usize).cloned();
        if let (Some(f), Some(t)) = (from, to) {
            if f.mint == t.mint {
                if let Some(i) = to_assets_b.borrow().iter().position(|a| a.mint != f.mint) {
                    pair_guard_b.set(true);
                    to_drop_b.set_selected(i as u32);
                    pair_guard_b.set(false);
                }
                ctx_b.toast("Pick two different tokens");
            }
        }
    });
    let from_assets_t = from_assets.clone();
    let to_assets_t = to_assets.clone();
    let from_drop_t = from_drop.clone();
    let pair_guard_t = pair_guard.clone();
    let ctx_t = ctx.clone();
    to_drop.connect_selected_notify(move |drop| {
        if pair_guard_t.get() {
            return;
        }
        let from = from_assets_t
            .borrow()
            .get(from_drop_t.selected() as usize)
            .cloned();
        let to = to_assets_t.borrow().get(drop.selected() as usize).cloned();
        if let (Some(f), Some(t)) = (from, to) {
            if f.mint == t.mint {
                if let Some(i) = to_assets_t.borrow().iter().position(|a| a.mint != f.mint) {
                    pair_guard_t.set(true);
                    drop.set_selected(i as u32);
                    pair_guard_t.set(false);
                }
                ctx_t.toast("Can't swap a token for itself");
            }
        }
    });

    let from_assets_m = from_assets.clone();
    let from_drop_m = from_drop.clone();
    let amount_m = amount.clone();
    max.connect_clicked(move |_| {
        let list = from_assets_m.borrow();
        let i = from_drop_m.selected() as usize;
        if let Some(a) = list.get(i) {
            amount_m.set_text(&format_ui_amount(a.balance_raw, a.decimals));
        }
    });

    let from_assets_f = from_assets.clone();
    let to_assets_f = to_assets.clone();
    let from_drop_f = from_drop.clone();
    let to_drop_f = to_drop.clone();
    flip.connect_clicked(move |_| {
        let fi = from_drop_f.selected() as usize;
        let ti = to_drop_f.selected() as usize;
        let from = from_assets_f.borrow();
        let to = to_assets_f.borrow();
        let Some(f) = from.get(fi) else { return };
        let Some(t) = to.get(ti) else { return };
        if let Some(new_from) = from.iter().position(|a| a.mint == t.mint) {
            from_drop_f.set_selected(new_from as u32);
        }
        if let Some(new_to) = to.iter().position(|a| a.mint == f.mint) {
            to_drop_f.set_selected(new_to as u32);
        }
    });

    let ctx_q = ctx.clone();
    let amount_q = amount.clone();
    let from_drop_q = from_drop.clone();
    let to_drop_q = to_drop.clone();
    let from_assets_q = from_assets.clone();
    let to_assets_q = to_assets.clone();
    let quote_lbl_q = quote_lbl.clone();
    let out_lbl_q = out_lbl.clone();
    let sbtn_q = sbtn.clone();
    let order_q = order_cell.clone();
    qbtn.connect_clicked(move |_| {
        if !matches!(ctx_q.cluster(), Cluster::Mainnet) {
            ctx_q.toast("Jupiter Swap V2 is mainnet-only. Switch the header to Mainnet.");
            return;
        }
        let Some(wallet) = ctx_q.active_wallet() else {
            ctx_q.toast("Unlock a wallet first");
            return;
        };
        let from = {
            let list = from_assets_q.borrow();
            list.get(from_drop_q.selected() as usize).cloned()
        };
        let to = {
            let list = to_assets_q.borrow();
            list.get(to_drop_q.selected() as usize).cloned()
        };
        let (Some(from), Some(to)) = (from, to) else {
            ctx_q.toast("Pick tokens to swap");
            return;
        };
        if from.mint == to.mint {
            ctx_q.toast("Pick two different tokens");
            return;
        }
        let amount_raw = match parse_ui_amount(&amount_q.text(), from.decimals) {
            Ok(v) if v > 0 => v,
            Ok(_) => {
                ctx_q.toast("Enter an amount greater than 0");
                return;
            }
            Err(e) => {
                ctx_q.toast(e.to_string());
                return;
            }
        };
        let slip = ctx_q.state.borrow().settings.slippage_bps;
        let slip = if slip == 0 { None } else { Some(slip) };
        let key = ctx_q.state.borrow().settings.jupiter_api_key.clone();
        let client = jupiter::Client::from_env_or(&key);
        let taker = wallet.address.clone();
        let ctx_d = ctx_q.clone();
        let quote_lbl_d = quote_lbl_q.clone();
        let out_lbl_d = out_lbl_q.clone();
        let sbtn_d = sbtn_q.clone();
        let order_d = order_q.clone();
        let to_sym = to.symbol.clone();
        let to_dec = to.decimals;
        quote_lbl_q.set_text("Fetching Jupiter V2 order…");
        sbtn_q.set_sensitive(false);
        task::spawn(
            move || client.order(&from.mint, &to.mint, amount_raw, Some(&taker), slip),
            move |res| match res {
                Ok(order) => {
                    let out_ui = parse_raw_u64(&order.out_amount);
                    let min_ui = parse_raw_u64(&order.other_amount_threshold);
                    let mut lines = vec![
                        format!("Router: {}", order.route_summary()),
                        format!("Mode: {}", if order.mode.is_empty() { "ultra" } else { &order.mode }),
                    ];
                    if let Some(p) = order.price_impact {
                        lines.push(format!("Price impact: {p:.4}%"));
                    }
                    if let (Some(i), Some(o)) = (order.in_usd, order.out_usd) {
                        lines.push(format!("{} → {}", format_usd(i), format_usd(o)));
                    }
                    if let Some(fee) = order.fee_bps.filter(|f| *f >= 50.0) {
                        lines.push(format!("App fee: {fee} bps"));
                    }
                    if order.gasless {
                        lines.push("Gasless: yes".into());
                    }
                    if min_ui > 0 {
                        lines.push(format!(
                            "Min received: {} {to_sym}",
                            format_ui_amount(min_ui, to_dec)
                        ));
                    }
                    if !order.error_message.is_empty() {
                        lines.push(order.error_message.clone());
                    }
                    out_lbl_d.set_text(&format!(
                        "You receive {} {to_sym}",
                        format_ui_amount(out_ui, to_dec)
                    ));
                    quote_lbl_d.set_text(&lines.join("\n"));
                    let ok = order.can_execute();
                    sbtn_d.set_sensitive(ok);
                    if !ok {
                        ctx_d.toast(
                            order
                                .error_message
                                .clone()
                                .if_empty("Quote ok, but Jupiter could not build a transaction"),
                        );
                    }
                    *order_d.borrow_mut() = Some(order);
                }
                Err(e) => {
                    quote_lbl_d.set_text(&e.to_string());
                    ctx_d.toast(e.to_string());
                    sbtn_d.set_sensitive(false);
                }
            },
        );
    });

    let ctx_s = ctx.clone();
    let order_s = order_cell.clone();
    let from_drop_s = from_drop.clone();
    let to_drop_s = to_drop.clone();
    let from_assets_s = from_assets.clone();
    let to_assets_s = to_assets.clone();
    let amount_s = amount.clone();
    sbtn.connect_clicked(move |_| {
        if !matches!(ctx_s.cluster(), Cluster::Mainnet) {
            ctx_s.toast("Switch to Mainnet to swap.");
            return;
        }
        let Some(wallet) = ctx_s.active_wallet() else {
            ctx_s.toast("Unlock a wallet first");
            return;
        };
        let from = from_assets_s
            .borrow()
            .get(from_drop_s.selected() as usize)
            .cloned();
        let to = to_assets_s
            .borrow()
            .get(to_drop_s.selected() as usize)
            .cloned();
        let (Some(from), Some(to)) = (from, to) else {
            return;
        };
        let amount_raw = match parse_ui_amount(&amount_s.text(), from.decimals) {
            Ok(v) if v > 0 => v,
            _ => {
                ctx_s.toast("Enter an amount");
                return;
            }
        };
        let preview = order_s.borrow().clone();
        let pay = format!(
            "{} {}",
            format_ui_amount(amount_raw, from.decimals),
            from.symbol
        );
        let recv = match &preview {
            Some(o) => format!(
                "{} {}",
                format_ui_amount(parse_raw_u64(&o.out_amount), to.decimals),
                to.symbol
            ),
            None => format!("? {}", to.symbol),
        };
        let route = preview
            .as_ref()
            .map(|o| o.route_summary())
            .unwrap_or_else(|| "Jupiter V2".into());
        let impact = preview.as_ref().and_then(|o| o.price_impact);
        let ctx_d = ctx_s.clone();
        let wallet = wallet.clone();
        let addr = wallet.address.clone();
        let in_mint = from.mint.clone();
        let out_mint = to.mint.clone();
        let pay_done = pay.clone();
        let recv_done = recv.clone();
        dialogs::confirm_swap(
            &ctx_s,
            &pay,
            &recv,
            &from.mint,
            &to.mint,
            impact,
            &route,
            move || {
            let slip = ctx_d.state.borrow().settings.slippage_bps;
            let slip = if slip == 0 { None } else { Some(slip) };
            let key = ctx_d.state.borrow().settings.jupiter_api_key.clone();
            let client = jupiter::Client::from_env_or(&key);
            let ctx_done = ctx_d.clone();
            let pay_n = pay_done.clone();
            let recv_n = recv_done.clone();
            let in_m = in_mint.clone();
            let out_m = out_mint.clone();
            task::spawn(
                move || -> Result<String, String> {
                    let order = client
                        .order(&in_mint, &out_mint, amount_raw, Some(&addr), slip)
                        .map_err(|e| e.to_string())?;
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
                move |res| match res {
                    Ok(sig) => {
                        let url = ctx_done.cluster().explorer_tx_url(&sig);
                        let _ = vibe_core::store::append_swap_event(
                            &sig, &url, &pay_n, &recv_n, &in_m, &out_m, "swap",
                        );
                        dialogs::notify_swap(&ctx_done, &sig, &pay_n, &recv_n);
                        dialogs::swap_landed(&ctx_done, &sig, &pay_n, &recv_n);
                        ctx_done.refresh();
                    }
                    Err(e) => ctx_done.toast(e),
                },
            );
        });
    });

    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("og-card");
    let pay_h = gtk::Label::new(Some("You pay"));
    pay_h.add_css_class("og-muted");
    pay_h.set_xalign(0.0);
    let recv_h = gtk::Label::new(Some("You receive"));
    recv_h.add_css_class("og-muted");
    recv_h.set_xalign(0.0);
    card.append(&pay_h);
    card.append(&pay_row);
    card.append(&bal_lbl);
    card.append(&flip);
    card.append(&recv_h);
    card.append(&recv_row);
    card.append(&out_lbl);
    card.append(&qbtn);
    card.append(&quote_lbl);
    card.append(&sbtn);

    let note = gtk::Label::new(Some(
        "Mainnet. Optional free API key: Settings → Jupiter, from https://developers.jup.ag/portal. Keyless works at a low rate limit.",
    ));
    note.set_wrap(true);
    note.add_css_class("og-muted");

    col.append(&kicker);
    col.append(&title);
    col.append(&fill_btn);
    col.append(&card);
    col.append(&note);
    clamp.set_child(Some(&col));
    scroller.set_child(Some(&clamp));
    outer.append(&scroller);

    fill();
    outer.upcast()
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

fn update_balance_label(label: &gtk::Label, assets: &[Asset], selected: u32) {
    if let Some(a) = assets.get(selected as usize) {
        label.set_text(&format!(
            "Balance: {} {}",
            format_ui_amount(a.balance_raw, a.decimals),
            a.symbol
        ));
    }
}

fn build_asset_lists(snap: &Snapshot) -> (Vec<Asset>, Vec<Asset>) {
    let mut from = Vec::new();
    from.push(Asset {
        mint: WSOL_MINT.to_string(),
        symbol: "SOL".into(),
        decimals: 9,
        balance_raw: snap.lamports,
    });
    for t in &snap.tokens {
        let mint = if t.mint.is_empty() {
            continue;
        } else {
            t.mint.clone()
        };
        if mint == WSOL_MINT {
            continue;
        }
        from.push(Asset {
            symbol: tokens::display_symbol(&mint),
            mint,
            decimals: t.decimals,
            balance_raw: parse_raw_u64(&t.amount_raw),
        });
    }

    let mut to = Vec::new();
    to.push(Asset {
        mint: WSOL_MINT.to_string(),
        symbol: "SOL".into(),
        decimals: 9,
        balance_raw: snap.lamports,
    });
    for k in tokens::KNOWN_TOKENS {
        if k.mint == WSOL_MINT {
            continue;
        }
        let symbol = if k.symbol == "WSOL" {
            "SOL".into()
        } else {
            k.symbol.to_string()
        };
        let held = snap.tokens.iter().find(|t| t.mint == k.mint);
        to.push(Asset {
            mint: k.mint.to_string(),
            symbol,
            decimals: held.map(|h| h.decimals).unwrap_or(token_decimals(k.mint)),
            balance_raw: held.map(|h| parse_raw_u64(&h.amount_raw)).unwrap_or(0),
        });
    }
    for t in &snap.tokens {
        if t.mint == WSOL_MINT {
            continue;
        }
        if to.iter().any(|a| a.mint == t.mint) {
            continue;
        }
        to.push(Asset {
            symbol: tokens::display_symbol(&t.mint),
            mint: t.mint.clone(),
            decimals: t.decimals,
            balance_raw: parse_raw_u64(&t.amount_raw),
        });
    }
    (from, to)
}

fn token_decimals(mint: &str) -> u8 {
    match mint {
        vibe_core::USDC_MINT => 6,
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => 6, // USDT
        "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263" => 5, // BONK
        _ => 9,
    }
}
