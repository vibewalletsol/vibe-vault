// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use adw::prelude::*;
use vibe_core::{parse_sol, tx};

use crate::ctx::Ctx;
use crate::dialogs;
use crate::task;

const EVERSTAKE_VOTE: &str = "9QU2QSxhb24FUX3Tu2FpczXjpK3VYrvRudywSZaM29mF";

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_vexpand(true);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(820);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 14);
    col.set_margin_top(24);
    col.set_margin_bottom(32);
    col.set_margin_start(24);
    col.set_margin_end(24);

    let kicker = gtk::Label::new(Some("VALIDATORS"));
    kicker.add_css_class("og-hero-kicker");
    kicker.set_xalign(0.0);
    let title = gtk::Label::new(Some("Staking"));
    title.add_css_class("og-page-title");
    title.set_xalign(0.0);
    let explain = gtk::Label::new(Some(
        "Staking delegates SOL from this vault to a validator. The coins stay yours and still show as Staked SOL in VIBE Wallet. APY is the estimated yearly reward after the validator's commission. Unstaking deactivates the account; after ~1–2 epochs you can withdraw spendable SOL back here.",
    ));
    explain.add_css_class("og-muted");
    explain.set_wrap(true);
    explain.set_xalign(0.0);

    let form = gtk::Box::new(gtk::Orientation::Vertical, 10);
    form.add_css_class("og-card");
    let vote = gtk::Entry::new();
    vote.set_placeholder_text(Some("Validator vote address"));
    let amount = gtk::Entry::new();
    amount.set_placeholder_text(Some("Amount in SOL (plus rent)"));
    if let Ok(v) = std::env::var("VIBE_STAKE_VOTE") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            vote.set_text(&v);
        }
    }
    if vote.text().is_empty() {
        vote.set_text(EVERSTAKE_VOTE);
    }
    if let Ok(v) = std::env::var("VIBE_STAKE_AMOUNT") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            amount.set_text(&v);
        }
    }
    let delegate = gtk::Button::with_label("Review delegate");
    delegate.add_css_class("og-primary");
    let everstake = gtk::Button::with_label("Use Everstake");
    everstake.add_css_class("og-ghost");
    let vote_e = vote.clone();
    everstake.connect_clicked(move |_| {
        vote_e.set_text(EVERSTAKE_VOTE);
    });

    let unstake = gtk::Entry::new();
    unstake.set_placeholder_text(Some("Stake account to unstake or withdraw"));
    if let Ok(v) = std::env::var("VIBE_UNSTAKE_ACCOUNT") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            unstake.set_text(&v);
        }
    }
    let deactivate = gtk::Button::with_label("Deactivate (unstake)");
    deactivate.add_css_class("og-ghost");
    let withdraw = gtk::Button::with_label("Withdraw to this wallet");
    withdraw.add_css_class("og-ghost");
    let load = gtk::Button::with_label("Load validators & my stakes");
    load.add_css_class("og-ghost");
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    form.append(&vote);
    form.append(&amount);
    form.append(&delegate);
    form.append(&everstake);
    form.append(&unstake);
    form.append(&deactivate);
    form.append(&withdraw);
    form.append(&load);

    let ctx_l = ctx.clone();
    let list_l = list.clone();
    let vote_fill = vote.clone();
    let unstake_fill = unstake.clone();
    load.connect_clicked(move |_| {
        let Some(addr) = ctx_l.active_address() else {
            ctx_l.toast("Unlock a wallet first");
            return;
        };
        let rpc = ctx_l.rpc();
        let ctx_d = ctx_l.clone();
        let list_d = list_l.clone();
        let vote_d = vote_fill.clone();
        let unstake_d = unstake_fill.clone();
        ctx_l.toast("Loading validators…");
        task::spawn(
            move || {
                let vals = rpc.get_vote_accounts();
                let stakes = rpc.get_stake_accounts(&addr);
                (vals, stakes)
            },
            move |(vals, stakes)| {
                while let Some(c) = list_d.first_child() {
                    list_d.remove(&c);
                }
                match stakes {
                    Ok(s) => {
                        ctx_d.state.borrow_mut().stakes = s.clone();
                        let h = gtk::Label::new(Some(&format!("Your stake accounts ({})", s.len())));
                        h.add_css_class("og-heading");
                        h.set_xalign(0.0);
                        list_d.append(&h);
                        for st in s.iter().take(15) {
                            let label = format!(
                                "{} · {} SOL · {} · {}",
                                vibe_core::shorten_address(&st.address),
                                vibe_core::format_sol(st.lamports),
                                st.state,
                                st.voter
                                    .as_ref()
                                    .map(|v| vibe_core::shorten_address(v))
                                    .unwrap_or_else(|| "—".into())
                            );
                            let btn = gtk::Button::with_label(&label);
                            btn.add_css_class("og-ghost");
                            btn.set_tooltip_text(Some(&st.address));
                            let unstake_c = unstake_d.clone();
                            let addr = st.address.clone();
                            btn.connect_clicked(move |_| {
                                unstake_c.set_text(&addr);
                            });
                            list_d.append(&btn);
                        }
                    }
                    Err(e) => ctx_d.toast(format!("stakes: {e}")),
                }
                match vals {
                    Ok(v) => {
                        ctx_d.state.borrow_mut().validators = v.clone();
                        let h = gtk::Label::new(Some("Top validators by stake"));
                        h.add_css_class("og-heading");
                        h.set_xalign(0.0);
                        h.set_margin_top(8);
                        list_d.append(&h);
                        for val in v.iter().take(12) {
                            let label = format!(
                                "{}% comm · {} SOL · {}",
                                val.commission,
                                vibe_core::format_sol(val.activated_stake),
                                vibe_core::shorten_address(&val.vote)
                            );
                            let btn = gtk::Button::with_label(&label);
                            btn.add_css_class("og-ghost");
                            btn.set_tooltip_text(Some(&val.vote));
                            let vote_c = vote_d.clone();
                            let vote_addr = val.vote.clone();
                            btn.connect_clicked(move |_| {
                                vote_c.set_text(&vote_addr);
                            });
                            list_d.append(&btn);
                        }
                    }
                    Err(e) => ctx_d.toast(format!("validators: {e}")),
                }
            },
        );
    });

    let ctx_d = ctx.clone();
    let vote_c = vote.clone();
    let amount_c = amount.clone();
    delegate.connect_clicked(move |_| {
        let vote_addr = vote_c.text().to_string();
        if !vibe_core::format::looks_like_pubkey(&vote_addr) {
            ctx_d.toast("Enter a vote account address (click a validator row to copy)");
            return;
        }
        let lamports = match parse_sol(&amount_c.text()) {
            Ok(v) if v > 0 => v,
            _ => {
                ctx_d.toast("Enter a SOL amount to stake");
                return;
            }
        };
        let Some(wallet) = ctx_d.active_wallet() else {
            ctx_d.toast("Unlock a wallet first");
            return;
        };
        let body = format!(
            "Delegate {} SOL to {}\nFrom {}\nCluster {}\n\nCreates a new stake account, initializes it, and delegates. Simulated first. Staked SOL remains yours and still shows in VIBE Wallet.",
            vibe_core::format_sol(lamports),
            vote_addr,
            wallet.address,
            ctx_d.cluster().as_label()
        );
        let ctx_s = ctx_d.clone();
        let wallet = wallet.clone();
        let from = wallet.address.clone();
        dialogs::confirm_generic(&ctx_d, "Review stake", &body, "Simulate and delegate", move || {
            let rpc = ctx_s.rpc();
            let vote_addr = vote_addr.clone();
            let ctx_done = ctx_s.clone();
            task::spawn(
                move || -> Result<String, String> {
                    let min_del = rpc
                        .get_stake_minimum_delegation()
                        .unwrap_or(vibe_core::LAMPORTS_PER_SOL);
                    if lamports < min_del {
                        return Err(format!(
                            "Solana requires at least {} SOL to stake",
                            vibe_core::format_sol(min_del)
                        ));
                    }
                    let rent = rpc.get_min_rent(200).map_err(|e| e.to_string())?;
                    let blockhash = rpc.get_latest_blockhash().map_err(|e| e.to_string())?;
                    let stake_kp = vibe_core::Keypair::generate();
                    let stake = stake_kp.address();
                    let create = tx::create_account(
                        &from,
                        &stake,
                        lamports.saturating_add(rent),
                        200,
                        vibe_core::ProgramIds::STAKE,
                    )
                    .map_err(|e| e.to_string())?;
                    let init = tx::stake_initialize(&stake, &from, &from).map_err(|e| e.to_string())?;
                    let del = tx::stake_delegate(&stake, &vote_addr, &from);
                    let user_pk = wallet.public_bytes()?;
                    let extra_pk = stake_kp.public_bytes();
                    let bytes = tx::build_signed_legacy_with(
                        &from,
                        &blockhash,
                        &[create, init, del],
                        &[user_pk, extra_pk],
                        |i, msg| {
                            if i == 0 {
                                wallet.sign_message(msg).map_err(|e| vibe_core::Error::msg(e))
                            } else {
                                Ok(stake_kp.sign(msg))
                            }
                        },
                    )
                    .map_err(|e| e.to_string())?;
                    let b64 =
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
                    rpc.simulate(&b64).map_err(|e| e.to_string())?;
                    rpc.send_transaction(&bytes).map_err(|e| e.to_string())
                },
                move |res| match res {
                    Ok(sig) => {
                        ctx_done.toast(format!("Stake tx {}", vibe_core::shorten_address(&sig)));
                        ctx_done.refresh();
                    }
                    Err(e) => ctx_done.toast(e),
                },
            );
        });
    });

    let ctx_u = ctx.clone();
    let unstake_c = unstake.clone();
    deactivate.connect_clicked(move |_| {
        let stake_acc = unstake_c.text().to_string();
        if !vibe_core::format::looks_like_pubkey(&stake_acc) {
            ctx_u.toast("Enter a stake account (click one of yours above)");
            return;
        }
        let Some(wallet) = ctx_u.active_wallet() else {
            ctx_u.toast("Unlock a wallet first");
            return;
        };
        let body = format!(
            "Deactivate stake account {}\nAuthority {}\nCluster {}\n\nSOL stays in the stake account for ~1–2 epochs, then you can withdraw it back to this wallet.",
            stake_acc,
            wallet.address,
            ctx_u.cluster().as_label()
        );
        let ctx_s = ctx_u.clone();
        let wallet = wallet.clone();
        let from = wallet.address.clone();
        dialogs::confirm_generic(&ctx_u, "Review unstake", &body, "Simulate and deactivate", move || {
            let rpc = ctx_s.rpc();
            let ctx_done = ctx_s.clone();
            task::spawn(
                move || -> Result<String, String> {
                    let blockhash = rpc.get_latest_blockhash().map_err(|e| e.to_string())?;
                    let ix = tx::stake_deactivate(&stake_acc, &from);
                    let bytes = wallet.sign_legacy(&from, &blockhash, &[ix])?;
                    let b64 =
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
                    rpc.simulate(&b64).map_err(|e| e.to_string())?;
                    rpc.send_transaction(&bytes).map_err(|e| e.to_string())
                },
                move |res| match res {
                    Ok(sig) => {
                        ctx_done.toast(format!("Unstake tx {}", vibe_core::shorten_address(&sig)));
                        ctx_done.refresh();
                    }
                    Err(e) => ctx_done.toast(e),
                },
            );
        });
    });

    let ctx_w = ctx.clone();
    let unstake_w = unstake.clone();
    withdraw.connect_clicked(move |_| {
        let stake_acc = unstake_w.text().to_string();
        if !vibe_core::format::looks_like_pubkey(&stake_acc) {
            ctx_w.toast("Enter a cooled-down stake account to withdraw");
            return;
        }
        let Some(wallet) = ctx_w.active_wallet() else {
            ctx_w.toast("Unlock a wallet first");
            return;
        };
        let body = format!(
            "Withdraw all lamports from {}\nTo {}\nCluster {}\n\nWorks after deactivation cooldown. SOL returns to this vault and shows as spendable in VIBE Wallet.",
            stake_acc,
            wallet.address,
            ctx_w.cluster().as_label()
        );
        let ctx_s = ctx_w.clone();
        let wallet = wallet.clone();
        let from = wallet.address.clone();
        dialogs::confirm_generic(&ctx_w, "Review withdraw", &body, "Simulate and withdraw", move || {
            let rpc = ctx_s.rpc();
            let ctx_done = ctx_s.clone();
            task::spawn(
                move || -> Result<String, String> {
                    let lamports = rpc.get_balance(&stake_acc).map_err(|e| e.to_string())?;
                    if lamports == 0 {
                        return Err("stake account is empty".into());
                    }
                    let blockhash = rpc.get_latest_blockhash().map_err(|e| e.to_string())?;
                    let ix = tx::stake_withdraw(&stake_acc, &from, &from, lamports);
                    let bytes = wallet.sign_legacy(&from, &blockhash, &[ix])
                        .map_err(|e| e.to_string())?;
                    let b64 =
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
                    rpc.simulate(&b64).map_err(|e| e.to_string())?;
                    rpc.send_transaction(&bytes).map_err(|e| e.to_string())
                },
                move |res| match res {
                    Ok(sig) => {
                        ctx_done.toast(format!("Withdraw tx {}", vibe_core::shorten_address(&sig)));
                        ctx_done.refresh();
                    }
                    Err(e) => ctx_done.toast(e),
                },
            );
        });
    });

    if let Ok(action) = std::env::var("VIBE_STAKE_ACTION") {
        let action = action.to_ascii_lowercase();
        if action == "unstake" {
            ctx.toast("Review deactivate below — SOL returns after the epoch cooldown");
        } else if action == "withdraw" {
            ctx.toast("Review withdraw below — cooled-down SOL returns to this wallet");
        } else if action == "stake" {
            ctx.toast("Review the delegate amount and validator, then confirm");
        }
    }

    col.append(&kicker);
    col.append(&title);
    col.append(&explain);
    col.append(&form);
    col.append(&list);
    clamp.set_child(Some(&col));
    scroller.set_child(Some(&clamp));
    outer.append(&scroller);
    outer.upcast()
}
