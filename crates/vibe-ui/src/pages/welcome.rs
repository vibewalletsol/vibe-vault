// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use adw::prelude::*;

use crate::ctx::Ctx;
use crate::dialogs;

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(560);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 16);
    col.set_valign(gtk::Align::Center);
    col.set_margin_top(48);
    col.set_margin_bottom(48);
    col.set_margin_start(24);
    col.set_margin_end(24);

    let kicker = gtk::Label::new(Some("SOLANA · SELF-CUSTODY"));
    kicker.add_css_class("og-hero-kicker");
    let title = gtk::Label::new(Some("VIBE Wallet"));
    title.add_css_class("og-page-title");
    let sub = gtk::Label::new(Some(
        "Keys stay on this Ubuntu machine, encrypted with Argon2id. Create a wallet or import a seed to get started.",
    ));
    sub.set_wrap(true);
    sub.add_css_class("og-muted");

    let create = gtk::Button::with_label("Create wallet");
    create.add_css_class("og-primary");
    let import = gtk::Button::with_label("Import seed or secret key");
    import.add_css_class("og-ghost");

    let ctx_c = ctx.clone();
    create.connect_clicked(move |_| dialogs::create_wallet(&ctx_c));
    let ctx_i = ctx.clone();
    import.connect_clicked(move |_| dialogs::import_wallet(&ctx_i));

    col.append(&kicker);
    col.append(&title);
    col.append(&sub);
    col.append(&create);
    col.append(&import);
    clamp.set_child(Some(&col));
    clamp.upcast()
}
