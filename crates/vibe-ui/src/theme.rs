use gtk::prelude::*;
use gtk::{gdk, CssProvider, STYLE_PROVIDER_PRIORITY_APPLICATION};

/// Load VIBE CSS and register the hicolor icon search path.
pub fn apply(display: &gdk::Display) {
    crate::fonts::register_bundled_fonts();
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_font_name(Some("Adwaita Sans 12"));
    }
    adw::StyleManager::for_display(display).set_color_scheme(adw::ColorScheme::ForceDark);

    let css = crate::paths::css_path();
    let provider = CssProvider::new();
    if css.exists() {
        provider.load_from_path(&css);
    } else {
        eprintln!(
            "vibe-vault: theme CSS missing at {} (set VIBE_DATA_DIR)",
            css.display()
        );
    }
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let icons = gtk::IconTheme::for_display(display);
    icons.add_search_path(crate::paths::icon_search_path());
}
