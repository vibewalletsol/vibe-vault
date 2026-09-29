// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use adw::prelude::*;
use vibe_core::store::{self, Contact};
use vibe_core::{format, parse_sol, tx};

use crate::ctx::Ctx;
use crate::dialogs;
use crate::task;

pub fn address_book(ctx: &Ctx) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(720);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
    col.set_margin_top(24);
    col.set_margin_start(24);
    col.set_margin_end(24);
    col.set_margin_bottom(24);
    let title = gtk::Label::new(Some("Address book"));
    title.add_css_class("og-page-title");
    title.set_xalign(0.0);
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Name"));
    let addr = gtk::Entry::new();
    addr.set_placeholder_text(Some("Solana address"));
    let add = gtk::Button::with_label("Add contact");
    add.add_css_class("og-primary");
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    redraw_contacts(ctx, &list);
    let ctx_c = ctx.clone();
    let name_c = name.clone();
    let addr_c = addr.clone();
    let list_c = list.clone();
    add.connect_clicked(move |_| {
        let n = name_c.text().to_string();
        let a = addr_c.text().to_string();
        if n.trim().is_empty() || !format::looks_like_pubkey(&a) {
            ctx_c.toast("Need a name and a valid address");
            return;
        }
        ctx_c
            .state
            .borrow_mut()
            .contacts
            .contacts
            .push(Contact {
                name: n,
                address: a,
                trusted: false,
                source: "manual".into(),
                sent_count: 0,
                last_sent: String::new(),
            });
        ctx_c.persist();
        name_c.set_text("");
        addr_c.set_text("");
        redraw_contacts(&ctx_c, &list_c);
        ctx_c.toast("Contact saved");
    });
    col.append(&title);
    col.append(&name);
    col.append(&addr);
    col.append(&add);
    col.append(&list);
    clamp.set_child(Some(&col));
    outer.append(&clamp);
    outer.upcast()
}

fn redraw_contacts(ctx: &Ctx, list: &gtk::Box) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let contacts = ctx.state.borrow().contacts.contacts.clone();
    if contacts.is_empty() {
        let l = gtk::Label::new(Some("No contacts yet."));
        l.add_css_class("og-muted");
        l.set_xalign(0.0);
        list.append(&l);
        return;
    }
    for c in contacts {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        row.add_css_class("og-card");
        let n = gtk::Label::new(Some(&c.name));
        n.add_css_class("og-heading");
        n.set_xalign(0.0);
        if c.trusted {
            n.set_text(&format!("{}  ·  Trusted", c.name));
        }
        let a = gtk::Label::new(Some(&c.address));
        a.add_css_class("og-mono");
        a.set_selectable(true);
        a.set_xalign(0.0);
        row.append(&n);
        row.append(&a);
        list.append(&row);
    }
}

pub fn explorer(ctx: &Ctx) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(820);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
    col.set_margin_top(24);
    col.set_margin_start(24);
    col.set_margin_end(24);
    col.set_margin_bottom(24);
    let title = gtk::Label::new(Some("Explorer"));
    title.add_css_class("og-page-title");
    title.set_xalign(0.0);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Transaction signature"));
    let go = gtk::Button::with_label("Lookup");
    go.add_css_class("og-primary");
    let out = gtk::TextView::new();
    out.set_editable(false);
    out.set_wrap_mode(gtk::WrapMode::WordChar);
    out.add_css_class("og-mono");
    let sw = gtk::ScrolledWindow::new();
    sw.set_min_content_height(360);
    sw.set_vexpand(true);
    sw.set_child(Some(&out));
    sw.add_css_class("og-card");
    let ctx_c = ctx.clone();
    let entry_c = entry.clone();
    let out_c = out.clone();
    go.connect_clicked(move |_| {
        let sig = entry_c.text().to_string();
        if !format::looks_like_signature(&sig) {
            ctx_c.toast("Paste a transaction signature");
            return;
        }
        let rpc = ctx_c.rpc();
        let ctx_d = ctx_c.clone();
        let buf = out_c.buffer();
        buf.set_text("Fetching…");
        task::spawn(
            move || rpc.get_transaction(&sig),
            move |res| match res {
                Ok(v) => {
                    buf.set_text(&serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()));
                }
                Err(e) => {
                    buf.set_text(&e.to_string());
                    ctx_d.toast(e.to_string());
                }
            },
        );
    });
    col.append(&title);
    col.append(&entry);
    col.append(&go);
    col.append(&sw);
    clamp.set_child(Some(&col));
    outer.append(&clamp);
    outer.upcast()
}

pub fn terminal(ctx: &Ctx) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
    col.set_margin_top(24);
    col.set_margin_start(24);
    col.set_margin_end(24);
    col.set_margin_bottom(24);
    let title = gtk::Label::new(Some("Terminal"));
    title.add_css_class("og-page-title");
    title.set_xalign(0.0);
    let out = gtk::TextView::new();
    out.set_editable(false);
    out.set_wrap_mode(gtk::WrapMode::WordChar);
    let sw = gtk::ScrolledWindow::new();
    sw.set_vexpand(true);
    sw.set_child(Some(&out));
    sw.add_css_class("og-card");
    out.buffer().set_text(
        "VIBE Wallet terminal\nCommands: help, address, balance, cluster, airdrop, rpc, account, slot, epoch, blockhash, fees, stakes, tx\nRead-only. Signing stays in Confirm.\n",
    );
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("help"));
    let ctx_c = ctx.clone();
    let out_c = out.clone();
    entry.connect_activate(move |entry| {
        let cmd = entry.text().to_string();
        entry.set_text("");
        let buf = out_c.buffer();
        let mut end = buf.end_iter();
        buf.insert(&mut end, &format!("\n> {cmd}\n"));
        run_cmd(&ctx_c, &cmd, &out_c);
    });
    col.append(&title);
    col.append(&sw);
    col.append(&entry);
    outer.append(&col);
    outer.upcast()
}

fn run_cmd(ctx: &Ctx, cmd: &str, view: &gtk::TextView) {
    let mut parts = cmd.split_whitespace();
    let Some(head) = parts.next() else { return };
    let rest: Vec<&str> = parts.collect();
    let buf = view.buffer();
    let append = |s: &str| {
        let mut end = buf.end_iter();
        buf.insert(&mut end, s);
    };
    match head {
        "help" => append("help, address, balance, cluster, airdrop, rpc, account, slot, epoch, blockhash, fees, stakes, tx\n"),
        "address" => append(&format!(
            "{}\n",
            ctx.active_address().unwrap_or_else(|| "locked".into())
        )),
        "cluster" => append(&format!("{}\n", ctx.cluster().as_label())),
        "rpc" => append(&format!("{}\n", ctx.state.borrow().settings.rpc_url())),
        "balance" => {
            let Some(addr) = ctx.active_address() else {
                append("unlock first\n");
                return;
            };
            let rpc = ctx.rpc();
            let view = view.clone();
            task::spawn(
                move || rpc.get_balance(&addr),
                move |res| {
                    let mut end = view.buffer().end_iter();
                    match res {
                        Ok(l) => view.buffer().insert(
                            &mut end,
                            &format!("{} SOL\n", vibe_core::format_sol(l)),
                        ),
                        Err(e) => view.buffer().insert(&mut end, &format!("{e}\n")),
                    }
                },
            );
        }
        "airdrop" => dialogs::airdrop(ctx),
        "send" => {
            if rest.len() < 2 {
                append("usage: send <address> <sol>\n");
                return;
            }
            append("Use the Send page for simulated sends.\n");
            let _ = (parse_sol, tx::system_transfer, store::contacts_path);
        }
        other => append(&format!("unknown command: {other}\n")),
    }
}

pub fn desk(_ctx: &Ctx) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(640);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
    col.set_margin_top(32);
    col.set_margin_start(24);
    col.set_margin_end(24);
    let title = gtk::Label::new(Some("Net worth desk"));
    title.add_css_class("og-page-title");
    title.set_xalign(0.0);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.add_css_class("og-card");
    let body = gtk::Label::new(Some(
        "The VIBE Wallet desk tracks crypto lots, metals, cash, real estate, and P/L. Open it beside this vault — keys stay here.",
    ));
    body.set_wrap(true);
    body.set_xalign(0.0);
    let open = gtk::Button::with_label("Open net worth desk");
    open.add_css_class("og-primary");
    open.connect_clicked(|_| {
        let _ = std::process::Command::new("vibe-wallet").spawn();
    });
    card.append(&body);
    card.append(&open);
    col.append(&title);
    col.append(&card);
    clamp.set_child(Some(&col));
    outer.append(&clamp);
    outer.upcast()
}
