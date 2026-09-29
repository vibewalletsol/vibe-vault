use adw::prelude::*;

use crate::ctx::{Ctx, ShellWidgets};

pub fn build(_ctx: &Ctx, shell: &ShellWidgets) -> gtk::Widget {
    page("ASSETS", "Tokens", &shell.assets)
}

pub fn activity(_ctx: &Ctx, shell: &ShellWidgets) -> gtk::Widget {
    page("HISTORY", "Activity", &shell.activity)
}

fn page(kicker: &str, title: &str, body: &impl glib::object::IsA<gtk::Widget>) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_vexpand(true);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(1360);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
    col.set_margin_top(24);
    col.set_margin_bottom(32);
    col.set_margin_start(24);
    col.set_margin_end(24);
    let k = gtk::Label::new(Some(kicker));
    k.add_css_class("og-hero-kicker");
    k.set_xalign(0.0);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("og-page-title");
    t.set_xalign(0.0);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("og-card");
    card.append(body);
    col.append(&k);
    col.append(&t);
    col.append(&card);
    clamp.set_child(Some(&col));
    scroller.set_child(Some(&clamp));
    outer.append(&scroller);
    outer.upcast()
}
