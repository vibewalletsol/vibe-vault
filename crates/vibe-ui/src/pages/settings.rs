use vibe_core::i18n::tr;
use adw::prelude::*;
use vibe_core::store;
use vibe_core::Cluster;

use crate::ctx::{Ctx, ShellWidgets};

pub fn build(ctx: &Ctx, shell: &ShellWidgets) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("og-content");
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(640);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 14);
    col.set_margin_top(24);
    col.set_margin_bottom(32);
    col.set_margin_start(24);
    col.set_margin_end(24);

    let title = gtk::Label::new(Some("Settings"));
    title.add_css_class("og-page-title");
    title.set_xalign(0.0);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.add_css_class("og-card");
    let rpc = gtk::Entry::new();
    rpc.set_placeholder_text(Some(tr("Custom RPC URL (optional)")));
    rpc.set_text(&ctx.state.borrow().settings.custom_rpc);
    let fallbacks = gtk::Entry::new();
    fallbacks.set_placeholder_text(Some(tr("Fallback RPCs, in order (https://…, comma-separated)")));
    fallbacks.set_text(&ctx.state.borrow().settings.rpc_fallbacks.join(", "));
    fallbacks.set_tooltip_text(Some(tr("Tried in order when the main RPC is down. Must be the same cluster (the vault checks the genesis hash). https only.")));
    let rpc_status = gtk::Label::new(Some(&format!("Active RPC: {}", ctx.rpc().active_label())));
    rpc_status.set_xalign(0.0);
    rpc_status.set_wrap(true);
    rpc_status.add_css_class("og-muted");
    let check = gtk::Button::with_label(tr("Check RPCs"));
    check.add_css_class("og-ghost");
    {
        let ctx_h = ctx.clone();
        let status_h = rpc_status.clone();
        check.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            status_h.set_text("Checking…");
            let rpc = ctx_h.rpc();
            let btn = btn.clone();
            let status_h = status_h.clone();
            crate::task::spawn(
                move || {
                    let rows = rpc.health();
                    let mut lines: Vec<String> = rows
                        .iter()
                        .enumerate()
                        .map(|(i, (url, r))| {
                            let host = vibe_core::rpc::endpoint_host(url);
                            let tag = if i == 0 { "primary".to_string() } else { format!("fallback {i}") };
                            match r {
                                Ok((slot, ms)) => format!("✓ {host} ({tag}) · slot {slot} · {ms} ms"),
                                Err(e) => format!("✗ {host} ({tag}) · {e}"),
                            }
                        })
                        .collect();
                    lines.push(format!("Active RPC: {}", rpc.active_label()));
                    lines.join("\n")
                },
                move |text: String| {
                    status_h.set_text(&text);
                    btn.set_sensitive(true);
                },
            );
        });
    }
    let slip = gtk::Entry::new();
    slip.set_placeholder_text(Some(tr("Jupiter slippage bps (0 = auto / V2 ultra)")));
    slip.set_text(&ctx.state.borrow().settings.slippage_bps.to_string());
    let jup = gtk::PasswordEntry::new();
    jup.set_show_peek_icon(true);
    jup.set_placeholder_text(Some(tr("Jupiter API key (optional, free at developers.jup.ag/portal)")));
    jup.set_text(&ctx.state.borrow().settings.jupiter_api_key);
    let cg = gtk::PasswordEntry::new();
    cg.set_show_peek_icon(true);
    cg.set_placeholder_text(Some(tr("CoinGecko API key (optional demo/pro)")));
    cg.set_text(&ctx.state.borrow().settings.coingecko_api_key);
    let save = gtk::Button::with_label(tr("Save"));
    save.add_css_class("og-primary");
    let lock = gtk::Button::with_label(tr("Lock vault"));
    lock.add_css_class("og-ghost");
    let about = gtk::Button::with_label(tr("About VIBE Wallet"));
    about.add_css_class("og-ghost");

    let ctx_s = ctx.clone();
    let rpc_c = rpc.clone();
    let fb_c = fallbacks.clone();
    let slip_c = slip.clone();
    let jup_c = jup.clone();
    let cg_c = cg.clone();
    save.connect_clicked(move |_| {
        let mut st = ctx_s.state.borrow_mut();
        st.settings.custom_rpc = rpc_c.text().trim().to_string();
        if !st.settings.custom_rpc.is_empty() {
            st.settings.cluster = "Custom".into();
        }
        st.settings.rpc_fallbacks = vibe_core::rpc::parse_endpoint_list(&fb_c.text());
        if let Ok(v) = slip_c.text().trim().parse::<u16>() {
            st.settings.slippage_bps = v.min(5_000);
        }
        st.settings.jupiter_api_key = jup_c.text().trim().to_string();
        st.settings.coingecko_api_key = cg_c.text().trim().to_string();
        drop(st);
        ctx_s.persist();
        ctx_s.toast("Settings saved");
        ctx_s.refresh();
    });
    let ctx_l = ctx.clone();
    lock.connect_clicked(move |_| ctx_l.lock());
    about.connect_clicked(move |btn| {
        if let Some(win) = btn.root().and_downcast::<gtk::Window>() {
            if let Some(app) = win.application() {
                app.activate_action("about", None);
            }
        }
    });

    let cluster_help = gtk::Label::new(Some(
        "Use the header cluster switch. Jupiter Swap V2 is mainnet-only. A free API key from https://developers.jup.ag/portal is optional but recommended (paste above, then Save).",
    ));
    cluster_help.set_wrap(true);
    cluster_help.add_css_class("og-muted");

    card.append(&gtk::Label::new(Some(&format!(
        "Cluster: {}",
        ctx.cluster().as_label()
    ))));
    card.append(&rpc);
    card.append(&fallbacks);
    card.append(&check);
    card.append(&rpc_status);
    card.append(&slip);
    card.append(&jup);
    card.append(&cg);
    card.append(&save);
    card.append(&lock);
    card.append(&about);
    col.append(&title);
    col.append(&cluster_help);
    col.append(&card);
    clamp.set_child(Some(&col));
    outer.append(&clamp);
    let _ = shell;
    let _ = Cluster::Mainnet;
    let _ = store::settings_path();
    outer.upcast()
}
