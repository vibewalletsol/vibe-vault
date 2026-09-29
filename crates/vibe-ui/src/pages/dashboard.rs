use vibe_core::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use vibe_core::Cluster;

use crate::ctx::{Ctx, ShellWidgets};
use crate::donut;
use crate::qr;

pub fn build(ctx: &Ctx, stack: &gtk::Stack) -> (gtk::Widget, ShellWidgets) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    outer.add_css_class("og-dash");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_vexpand(true);
    scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(1360);
    clamp.set_tightening_threshold(880);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 22);
    col.set_margin_top(12);
    col.set_margin_bottom(32);
    col.set_margin_start(8);
    col.set_margin_end(8);

    let chip = gtk::Label::new(Some(ctx.cluster().as_label()));
    chip.add_css_class("og-chip");

    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 28);
    hero.add_css_class("og-hero-card");
    hero.set_hexpand(true);

    let icon = gtk::Image::from_icon_name("com.vibewallet.Vault");
    icon.set_pixel_size(96);
    icon.add_css_class("og-hero-mark");
    hero.append(&icon);

    let left = gtk::Box::new(gtk::Orientation::Vertical, 6);
    left.add_css_class("og-hero-center");
    left.set_hexpand(true);
    left.set_valign(gtk::Align::Center);
    let kicker_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let kicker = gtk::Label::new(Some("Wallet"));
    kicker.add_css_class("og-hero-kicker");
    kicker.set_xalign(0.0);
    kicker.set_hexpand(true);
    kicker_row.append(&kicker);
    kicker_row.append(&chip);
    let balance = gtk::Label::new(Some("—"));
    balance.add_css_class("og-hero-value");
    balance.set_xalign(0.0);
    balance.set_wrap(true);
    let hero_chg = gtk::Label::new(Some("Live prices from CoinGecko"));
    hero_chg.add_css_class("og-hero-change");
    hero_chg.set_xalign(0.0);
    let fiat = gtk::Label::new(Some("Unlock a wallet to load holdings"));
    fiat.add_css_class("og-hero-sub");
    fiat.set_xalign(0.0);
    let address = gtk::Label::new(Some(""));
    address.add_css_class("og-mono");
    address.set_xalign(0.0);
    address.set_selectable(true);
    let hint = gtk::Label::new(Some(""));
    hint.add_css_class("og-muted");
    hint.set_xalign(0.0);

    let cta = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    cta.set_margin_top(8);
    let refresh = gtk::Button::with_label(tr("Refresh"));
    refresh.add_css_class("og-ghost");
    let copy = gtk::Button::with_label(tr("Copy address"));
    copy.add_css_class("og-ghost");
    let ctx_r = ctx.clone();
    refresh.connect_clicked(move |_| ctx_r.refresh());
    let ctx_c = ctx.clone();
    copy.connect_clicked(move |_| {
        if let Some(addr) = ctx_c.active_address() {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&addr);
                ctx_c.toast("Address copied");
            }
        }
    });
    cta.append(&refresh);
    cta.append(&copy);

    left.append(&kicker_row);
    left.append(&balance);
    left.append(&hero_chg);
    left.append(&fiat);
    left.append(&address);
    left.append(&hint);
    left.append(&cta);
    hero.append(&left);

    let donut_slices = Rc::new(RefCell::new(Vec::new()));
    let donut = donut::drawing_area(donut_slices.clone());
    let legend = gtk::Box::new(gtk::Orientation::Vertical, 4);
    legend.set_valign(gtk::Align::Center);
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    right.set_halign(gtk::Align::End);
    right.set_valign(gtk::Align::Center);
    right.append(&donut);
    right.append(&legend);
    hero.append(&right);

    let metrics = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    metrics.set_homogeneous(true);
    let (m_sol_box, metric_sol) = quiet_metric("SOL balance");
    let (m_tok_box, metric_tokens) = quiet_metric("Tokens");
    let (m_day_box, metric_day) = quiet_metric("Market 24h");
    metrics.append(&m_sol_box);
    metrics.append(&m_tok_box);
    metrics.append(&m_day_box);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    actions.set_homogeneous(true);
    for (label, class, page) in [
        ("Send", "og-primary", "send"),
        ("Receive", "og-ghost", "send"),
        ("Swap", "og-teal", "swap"),
        ("Stake", "og-ghost", "staking"),
    ] {
        let btn = gtk::Button::with_label(label);
        btn.add_css_class(class);
        let stack = stack.clone();
        btn.connect_clicked(move |_| stack.set_visible_child_name(page));
        actions.append(&btn);
    }

    let assets_head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let list_title = gtk::Label::new(Some("Holdings"));
    list_title.add_css_class("og-section-title");
    list_title.set_xalign(0.0);
    list_title.set_hexpand(true);
    assets_head.append(&list_title);

    let holdings = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holdings.add_css_class("og-asset-list");
    let activity = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let assets = gtk::Box::new(gtk::Orientation::Vertical, 0);
    assets.add_css_class("og-asset-list");

    col.append(&hero);
    col.append(&metrics);
    col.append(&actions);
    col.append(&assets_head);
    col.append(&holdings);
    clamp.set_child(Some(&col));
    scroller.set_child(Some(&clamp));
    outer.append(&scroller);

    let qr_addr = Rc::new(RefCell::new(String::new()));
    let qr_area = qr::drawing_area(qr_addr.clone());
    let receive_addr = gtk::Label::new(None);
    let wallets = gtk::Box::new(gtk::Orientation::Vertical, 10);

    let cluster_drop = gtk::DropDown::from_strings(&["Mainnet", "Devnet", "Testnet"]);
    let selected = match ctx.cluster() {
        Cluster::Devnet => 1,
        Cluster::Testnet => 2,
        _ => 0,
    };
    cluster_drop.set_selected(selected);

    let handles = ShellWidgets {
        balance,
        fiat,
        address,
        chip,
        hint,
        hero_chg,
        metric_sol,
        metric_tokens,
        metric_day,
        donut,
        donut_slices,
        legend,
        holdings,
        activity,
        assets,
        wallets,
        receive_addr,
        qr_addr,
        qr_area,
        cluster_drop,
    };
    (outer.upcast(), handles)
}

fn quiet_metric(label: &str) -> (gtk::Widget, gtk::Label) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.add_css_class("og-quiet");
    let k = gtk::Label::new(Some(label));
    k.add_css_class("og-group-title");
    k.set_xalign(0.0);
    let v = gtk::Label::new(Some("—"));
    v.add_css_class("og-stat-value");
    v.set_xalign(0.0);
    card.append(&k);
    card.append(&v);
    (card.upcast(), v)
}
