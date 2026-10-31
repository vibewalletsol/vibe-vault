use adw::prelude::*;
use gio::prelude::*;
use vibe_core::{APP_ID, APP_NAME, APP_VERSION, DEVELOPER, HOMEPAGE};

fn env_flag(key: &str) -> bool {
    matches!(
        std::env::var(key).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes")
    )
}

fn signer_mode() -> bool {
    env_flag("VIBE_SIGNER")
        || env_flag("VIBE_CREATE")
        || env_flag("VIBE_IMPORT")
        || env_flag("VIBE_REVEAL")
}

pub fn run() -> glib::ExitCode {
    let flags = if signer_mode() {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::default()
    };
    let app = adw::Application::new(Some(APP_ID), flags);

    app.connect_startup(|app| {
        if let Some(display) = gtk::gdk::Display::default() {
            crate::theme::apply(&display);
        } else {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        }
        install_actions(app);
        app.set_accels_for_action("app.quit", &["<primary>q"]);
        app.set_accels_for_action("app.about", &["F1"]);
        app.set_accels_for_action("app.lock", &["<primary>l"]);
    });

    app.connect_activate(|app| {
        let existing = app.windows();
        if let Some(win) = existing.first() {
            win.present();
            return;
        }
        let win = crate::window::build(app);
        win.present();
    });

    app.run()
}

fn install_actions(app: &adw::Application) {
    let quit = gio::SimpleAction::new("quit", None);
    let app_quit = app.clone();
    quit.connect_activate(move |_, _| {
        app_quit.quit();
    });
    app.add_action(&quit);

    let about = gio::SimpleAction::new("about", None);
    let app_about = app.clone();
    about.connect_activate(move |_, _| {
        show_about(&app_about);
    });
    app.add_action(&about);

    let open_scan = gio::SimpleAction::new("open-solscan", Some(&glib::VariantTy::STRING));
    open_scan.connect_activate(move |_, param| {
        let Some(sig) = param.and_then(|v| v.str()) else {
            return;
        };
        let sig = sig.trim();
        if sig.is_empty() || sig.len() < 32 || sig.len() > 128 {
            return;
        }
        if !sig
            .chars()
            .all(|c| c.is_ascii_alphanumeric())
        {
            return;
        }
        let url = format!("https://vibewallet.net/tx.html?sig={sig}");
        let _ = gio::AppInfo::launch_default_for_uri(&url, gio::AppLaunchContext::NONE);
    });
    app.add_action(&open_scan);
}

fn show_about(app: &adw::Application) {
    let dialog = adw::AboutDialog::builder()
        .application_name(APP_NAME)
        .application_icon(APP_ID)
        .developer_name(DEVELOPER)
        .version(APP_VERSION)
        .website(HOMEPAGE)
        .issue_url("https://oldgrowthcrypto.com")
        .comments(
            "VIBE Wallet — self-custody Solana vault for Linux, plus the net-worth desk.\n\
             GTK4 + libadwaita. Native vault on this machine.",
        )
        .license_type(gtk::License::Gpl30Only)
        .copyright("© 2026 Old Growth Crypto")
        .build();
    dialog.add_legal_section(
        "Cinder",
        Some("vibe-vault is derived from Cinder (https://github.com/gregnb/cinder)\nCopyright (c) 2026 Gregory Nowakowski"),
        gtk::License::MitX11,
        None,
    );
    dialog.add_legal_section(
        "Third-party notices",
        Some("Rust crate licenses: /usr/share/doc/vibe-wallet/THIRD_PARTY_NOTICES"),
        gtk::License::Unknown,
        None,
    );
    dialog.add_legal_section(
        "Adwaita Sans",
        Some("© The GNOME Project, based on Inter (SIL Open Font License 1.1)"),
        gtk::License::Unknown,
        None,
    );
    dialog.add_legal_section(
        "Inter",
        Some("© The Inter Project Authors (SIL Open Font License 1.1)"),
        gtk::License::Unknown,
        None,
    );

    let parent = app.active_window();
    dialog.present(parent.as_ref());
}
