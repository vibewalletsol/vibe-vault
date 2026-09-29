use vibe_core::i18n::tr;
use adw::prelude::*;

use crate::ctx::Ctx;
use crate::dialogs;

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    shell.set_halign(gtk::Align::Fill);
    shell.set_valign(gtk::Align::Fill);
    shell.add_css_class("og-content");

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(340);
    clamp.set_halign(gtk::Align::Center);
    clamp.set_valign(gtk::Align::Center);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
    col.set_valign(gtk::Align::Center);
    col.set_margin_top(18);
    col.set_margin_bottom(18);
    col.set_margin_start(18);
    col.set_margin_end(18);
    col.add_css_class("og-card");

    let title = gtk::Label::new(Some("Unlock"));
    title.add_css_class("og-heading");
    let sub = gtk::Label::new(Some("Enter the password for this vault."));
    sub.add_css_class("og-muted");
    sub.set_wrap(true);

    let pass = gtk::PasswordEntry::new();
    pass.set_show_peek_icon(true);
    pass.set_hexpand(true);
    pass.set_placeholder_text(Some(tr("Password")));

    let go = gtk::Button::with_label(tr("Unlock"));
    go.add_css_class("og-primary");
    let forgot = gtk::Label::new(Some(
        "Forgot the password? The seed phrase is the only recovery.",
    ));
    forgot.set_wrap(true);
    forgot.add_css_class("og-muted");

    let ctx_c = ctx.clone();
    let pass_c = pass.clone();
    let do_unlock = move || {
        match dialogs::try_unlock(&ctx_c, &pass_c.text()) {
            Ok(()) => pass_c.set_text(""),
            Err(e) => ctx_c.toast(e),
        }
    };
    let unlock_click = do_unlock.clone();
    go.connect_clicked(move |_| unlock_click());
    pass.connect_activate(move |_| do_unlock());

    col.append(&title);
    col.append(&sub);
    col.append(&pass);
    col.append(&go);
    col.append(&forgot);
    clamp.set_child(Some(&col));
    shell.append(&clamp);
    shell.upcast()
}
