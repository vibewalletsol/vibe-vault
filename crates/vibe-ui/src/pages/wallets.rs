use vibe_core::i18n::tr;
use adw::prelude::*;

use crate::ctx::{Ctx, ShellWidgets};
use crate::dialogs;

pub fn build(ctx: &Ctx, shell: &ShellWidgets) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_vexpand(true);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(760);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 14);
    col.set_margin_top(24);
    col.set_margin_bottom(32);
    col.set_margin_start(24);
    col.set_margin_end(24);

    let kicker = gtk::Label::new(Some("ACCOUNTS"));
    kicker.add_css_class("og-hero-kicker");
    kicker.set_xalign(0.0);
    let title = gtk::Label::new(Some("Wallets"));
    title.add_css_class("og-page-title");
    title.set_xalign(0.0);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let create = gtk::Button::with_label(tr("Create"));
    create.add_css_class("og-primary");
    let import = gtk::Button::with_label(tr("Import"));
    import.add_css_class("og-ghost");
    let hw = gtk::Button::with_label(tr("Hardware"));
    hw.add_css_class("og-ghost");
    let ctx_c = ctx.clone();
    create.connect_clicked(move |_| dialogs::create_wallet(&ctx_c));
    let ctx_i = ctx.clone();
    import.connect_clicked(move |_| dialogs::import_wallet(&ctx_i));
    let ctx_h = ctx.clone();
    hw.connect_clicked(move |_| {
        dialogs::connect_hardware(&ctx_h);
    });
    row.append(&create);
    row.append(&import);
    row.append(&hw);

    col.append(&kicker);
    col.append(&title);
    col.append(&row);
    col.append(&shell.wallets);
    clamp.set_child(Some(&col));
    scroller.set_child(Some(&clamp));
    outer.append(&scroller);
    outer.upcast()
}
