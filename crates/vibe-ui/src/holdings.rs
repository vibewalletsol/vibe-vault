// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use adw::prelude::*;
use vibe_core::coingecko::Holding;
use vibe_core::format::{format_ui_amount, format_usd};
use pango::EllipsizeMode;

pub fn table_header() -> gtk::Widget {
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    box_.add_css_class("og-cg-head");
    box_.append(&head("Coin", 200, false));
    box_.append(&head("Price", 96, true));
    box_.append(&head("24h", 72, true));
    box_.append(&head("Holdings", 140, true));
    box_.upcast()
}

fn head(title: &str, width: i32, end: bool) -> gtk::Widget {
    let lab = gtk::Label::new(Some(title));
    lab.add_css_class("og-cg-th");
    lab.set_xalign(if end { 1.0 } else { 0.0 });
    lab.set_width_request(width);
    if !end {
        lab.set_hexpand(true);
    }
    lab.upcast()
}

pub fn asset_row(h: &Holding) -> gtk::Widget {
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    box_.add_css_class("og-cg-row");
    box_.set_hexpand(true);

    let coin = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    coin.add_css_class("og-cg-coin");
    coin.set_hexpand(true);
    coin.set_width_request(200);
    coin.set_valign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name("image-x-generic-symbolic");
    icon.add_css_class("og-coin-icon");
    icon.set_pixel_size(28);
    if let Some(path) = &h.icon_path {
        if path.exists() {
            icon.set_from_file(Some(path));
        }
    }

    let names = gtk::Box::new(gtk::Orientation::Vertical, 1);
    names.set_hexpand(true);
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = gtk::Label::new(Some(&h.name));
    name.add_css_class("og-coin-name");
    name.set_xalign(0.0);
    name.set_ellipsize(EllipsizeMode::End);
    let ticker = gtk::Label::new(Some(&h.symbol));
    ticker.add_css_class("og-cg-ticker");
    ticker.set_xalign(0.0);
    title.append(&name);
    title.append(&ticker);
    names.append(&title);
    coin.append(&icon);
    coin.append(&names);
    box_.append(&coin);

    box_.append(&num_cell(
        if h.price_usd > 0.0 {
            format_price(h.price_usd)
        } else {
            "—".into()
        },
        96,
        true,
    ));
    box_.append(&chg_cell(h.change_24h, 72));
    box_.append(&stack_cell(
        format_usd(h.value_usd),
        format!("{} {}", h.ui_amount, h.symbol),
        140,
    ));
    box_.upcast()
}

fn num_cell(text: String, width: i32, bold: bool) -> gtk::Widget {
    let lab = gtk::Label::new(Some(&text));
    lab.add_css_class("og-cg-td");
    if bold {
        lab.add_css_class("og-cg-price");
    }
    lab.set_xalign(1.0);
    lab.set_width_request(width);
    lab.upcast()
}

fn chg_cell(chg: Option<f64>, width: i32) -> gtk::Widget {
    let (text, class) = match chg {
        Some(v) if v > 0.05 => (format!("▲ {v:.2}%"), "og-up"),
        Some(v) if v < -0.05 => (format!("▼ {v:.2}%"), "og-down"),
        Some(v) => (format!("{v:.2}%"), "og-flat"),
        None => ("—".into(), "og-flat"),
    };
    let lab = gtk::Label::new(Some(&text));
    lab.add_css_class("og-cg-td");
    lab.add_css_class("og-cg-chg");
    lab.add_css_class(class);
    lab.set_xalign(1.0);
    lab.set_width_request(width);
    lab.upcast()
}

fn stack_cell(top: String, bot: String, width: i32) -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 1);
    col.set_width_request(width);
    col.set_halign(gtk::Align::End);
    let a = gtk::Label::new(Some(&top));
    a.add_css_class("og-cg-td");
    a.add_css_class("og-cg-hold");
    a.set_xalign(1.0);
    let b = gtk::Label::new(Some(&bot));
    b.add_css_class("og-cg-sub");
    b.set_xalign(1.0);
    col.append(&a);
    col.append(&b);
    col.upcast()
}

fn format_price(v: f64) -> String {
    if v >= 1000.0 {
        format!("${v:.2}")
    } else if v >= 1.0 {
        format!("${v:.4}")
    } else if v >= 0.01 {
        format!("${v:.6}")
    } else {
        format!("${v:.8}")
    }
}

pub fn fill_list(box_: &gtk::Box, holdings: &[Holding]) {
    while let Some(c) = box_.first_child() {
        box_.remove(&c);
    }
    if holdings.is_empty() {
        let empty = gtk::Label::new(Some("No coins yet. Unlock a wallet and refresh."));
        empty.add_css_class("og-muted");
        empty.set_xalign(0.0);
        box_.append(&empty);
        return;
    }
    box_.append(&table_header());
    let total: f64 = holdings.iter().map(|h| h.value_usd).sum();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    head.add_css_class("og-group-head");
    let t = gtk::Label::new(Some("Assets"));
    t.add_css_class("og-group-title");
    t.set_hexpand(true);
    t.set_xalign(0.0);
    let s = gtk::Label::new(Some(&format_usd(total)));
    s.add_css_class("og-group-sum");
    s.set_xalign(1.0);
    head.append(&t);
    head.append(&s);
    box_.append(&head);
    for h in holdings {
        box_.append(&asset_row(h));
    }
    let _ = format_ui_amount(0, 0);
}
