use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use vibe_core::rpc::Snapshot;
use vibe_core::store::{self, WalletStore};
use vibe_core::i18n::{n_, tr};
use vibe_core::{Cluster, APP_NAME};

use crate::ctx::{AppState, Ctx};
use crate::pages;

struct NavItem {
    id: &'static str,
    label: &'static str,
    icon: &'static str,
}

struct NavGroup {
    kicker: &'static str,
    items: &'static [NavItem],
}

const NAV: &[NavGroup] = &[
    NavGroup {
        kicker: n_("Wallet"),
        items: &[
            NavItem {
                id: "dashboard",
                label: n_("Dashboard"),
                icon: "view-grid-symbolic",
            },
            NavItem {
                id: "wallets",
                label: n_("Wallets"),
                icon: "network-workgroup-symbolic",
            },
            NavItem {
                id: "activity",
                label: n_("Activity"),
                icon: "view-list-symbolic",
            },
            NavItem {
                id: "assets",
                label: n_("Assets"),
                icon: "package-x-generic-symbolic",
            },
        ],
    },
    NavGroup {
        kicker: n_("Move"),
        items: &[
            NavItem {
                id: "send",
                label: n_("Send / Receive"),
                icon: "mail-send-symbolic",
            },
            NavItem {
                id: "swap",
                label: n_("Swap"),
                icon: "emblem-synchronizing-symbolic",
            },
            NavItem {
                id: "staking",
                label: n_("Staking"),
                icon: "network-server-symbolic",
            },
        ],
    },
    NavGroup {
        kicker: n_("Tools"),
        items: &[
            NavItem {
                id: "addressbook",
                label: n_("Address Book"),
                icon: "x-office-address-book-symbolic",
            },
            NavItem {
                id: "explorer",
                label: n_("Explorer"),
                icon: "edit-find-symbolic",
            },
            NavItem {
                id: "terminal",
                label: n_("Terminal"),
                icon: "utilities-terminal-symbolic",
            },
            NavItem {
                id: "desk",
                label: n_("Net worth desk"),
                icon: "view-paged-symbolic",
            },
        ],
    },
    NavGroup {
        kicker: n_("System"),
        items: &[NavItem {
            id: "settings",
            label: n_("Settings"),
            icon: "preferences-system-symbolic",
        }],
    },
];

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

pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let signer = signer_mode();
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some(if signer { signer_title() } else { APP_NAME }));
    window.set_icon_name(Some(vibe_core::APP_ID));
    let manage = env_flag("VIBE_CREATE") || env_flag("VIBE_IMPORT") || env_flag("VIBE_REVEAL");
    window.set_default_size(
        if manage {
            360
        } else if signer {
            320
        } else {
            1280
        },
        if manage {
            560
        } else if signer {
            430
        } else {
            820
        },
    );
    window.add_css_class("og-window");
    if signer {
        window.add_css_class("og-signer");
        window.set_decorated(true);
        if manage {
            window.set_resizable(true);
            window.set_size_request(340, 480);
        } else {
            window.set_resizable(false);
            window.set_size_request(340, 440);
        }
    }

    let mut settings = store::load_settings(&store::settings_path());
    if let Ok(raw) = std::env::var("VIBE_CLUSTER") {
        let label = match raw.trim().to_ascii_lowercase().as_str() {
            "devnet" => Some("Devnet"),
            "testnet" => Some("Testnet"),
            "mainnet" => Some("Mainnet"),
            _ => None,
        };
        if let Some(label) = label {
            settings.cluster = label.to_string();
        }
    }
    let store_data = store::load_store(&store::wallets_path()).unwrap_or(WalletStore {
        version: 1,
        ..Default::default()
    });
    let contacts = store::load_contacts(&store::contacts_path());

    let toasts = adw::ToastOverlay::new();
    let root = gtk::Stack::new();
    root.set_transition_type(gtk::StackTransitionType::Crossfade);

    let ctx = Ctx {
        window: window.clone(),
        toasts: toasts.clone(),
        root: root.clone(),
        state: Rc::new(RefCell::new(AppState {
            settings,
            store: store_data,
            session: None,
            snapshot: Snapshot::default(),
            contacts,
            validators: Vec::new(),
            stakes: Vec::new(),
            busy: false,
        })),
        shell: Rc::new(RefCell::new(None)),
    };

    if signer {
        return build_signer(app, window, ctx, toasts, root);
    }

    let welcome = pages::welcome::build(&ctx);
    let lock = pages::lock::build(&ctx);

    let pages_stack = gtk::Stack::new();
    pages_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    pages_stack.set_hexpand(true);
    pages_stack.set_vexpand(true);

    let (dash, handles) = pages::dashboard::build(&ctx, &pages_stack);

    let cluster_drop = handles.cluster_drop.clone();
    let selected = match ctx.cluster() {
        Cluster::Devnet => 1,
        Cluster::Testnet => 2,
        _ => 0,
    };
    cluster_drop.set_selected(selected);
    let ctx_cluster = ctx.clone();
    cluster_drop.connect_selected_notify(move |drop| {
        let label = match drop.selected() {
            1 => "Devnet",
            2 => "Testnet",
            _ => "Mainnet",
        };
        ctx_cluster.state.borrow_mut().settings.cluster = label.into();
        if label != "Custom" {
            // keep custom rpc stored but don't use it unless Custom
        }
        ctx_cluster.persist();
        if ctx_cluster.state.borrow().session.is_some() {
            ctx_cluster.refresh();
        }
        ctx_cluster.toast(format!("Cluster: {label}"));
    });

    pages_stack.add_named(&dash, Some("dashboard"));
    pages_stack.add_named(&pages::wallets::build(&ctx, &handles), Some("wallets"));
    pages_stack.add_named(&pages::assets::activity(&ctx, &handles), Some("activity"));
    pages_stack.add_named(&pages::assets::build(&ctx, &handles), Some("assets"));
    pages_stack.add_named(&pages::send::build(&ctx, &handles), Some("send"));
    pages_stack.add_named(&pages::swap::build(&ctx), Some("swap"));
    pages_stack.add_named(&pages::staking::build(&ctx), Some("staking"));
    pages_stack.add_named(&pages::tools::address_book(&ctx), Some("addressbook"));
    pages_stack.add_named(&pages::tools::explorer(&ctx), Some("explorer"));
    pages_stack.add_named(&pages::tools::terminal(&ctx), Some("terminal"));
    pages_stack.add_named(&pages::tools::desk(&ctx), Some("desk"));
    pages_stack.add_named(&pages::settings::build(&ctx, &handles), Some("settings"));
    let start_page = "dashboard";
    pages_stack.set_visible_child_name(start_page);

    *ctx.shell.borrow_mut() = Some(handles);

    let title_text = tr("Dashboard");
    let title = gtk::Label::new(Some(title_text));
    title.add_css_class("og-heading");

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_css_classes(&["og-nav", "navigation-sidebar"]);

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);

    {
        let split = adw::OverlaySplitView::new();
        split.set_sidebar_width_fraction(0.20);
        split.set_min_sidebar_width(232.0);
        split.set_max_sidebar_width(280.0);
        split.set_show_sidebar(true);
        split.set_pin_sidebar(true);
        let sidebar = build_sidebar(&list);
        split.set_sidebar(Some(&sidebar));
        split.set_content(Some(&pages_stack));

        let sidebar_btn = gtk::ToggleButton::new();
        sidebar_btn.set_icon_name("sidebar-show-right-symbolic");
        sidebar_btn.set_tooltip_text(Some(tr("Toggle sidebar")));
        sidebar_btn.set_active(true);
        let split_for_toggle = split.clone();
        sidebar_btn.connect_toggled(move |btn| {
            split_for_toggle.set_show_sidebar(btn.is_active());
        });
        header.pack_start(&sidebar_btn);
        cluster_drop.set_tooltip_text(Some(tr("Solana cluster")));
        header.pack_end(&cluster_drop);
        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_menu_model(Some(&app_menu()));
        header.pack_end(&menu_btn);
        toolbar.set_content(Some(&split));
    }

    root.add_named(&welcome, Some("welcome"));
    root.add_named(&lock, Some("lock"));
    root.add_named(&toolbar, Some("main"));

    if ctx.has_wallets() {
        root.set_visible_child_name("lock");
    } else {
        root.set_visible_child_name("welcome");
    }

    toasts.set_child(Some(&root));
    window.set_content(Some(&toasts));

    let title_for_nav = title.clone();
    let stack_for_nav = pages_stack.clone();
    list.connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let id = row.widget_name();
        if id.is_empty() {
            return;
        }
        stack_for_nav.set_visible_child_name(&id);
        if let Some(label) = nav_label(&id) {
            title_for_nav.set_text(label);
        }
    });

    let list_sync = list.clone();
    pages_stack.connect_visible_child_notify(move |stack| {
        if let Some(name) = stack.visible_child_name() {
            select_nav_row(&list_sync, &name);
        }
    });

    glib::idle_add_local_once({
        let list = list.clone();
        let page = start_page.to_string();
        move || select_nav_row(&list, &page)
    });

    let lock_action = gio::SimpleAction::new("lock", None);
    let ctx_lock = ctx.clone();
    lock_action.connect_activate(move |_, _| ctx_lock.lock());
    app.add_action(&lock_action);

    if let Ok(path) = std::env::var("VIBE_SCREENSHOT") {
        let win_shot = window.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(800), move || {
            if let Err(err) = capture_window(&win_shot, &path) {
                eprintln!("vibe-vault: screenshot failed: {err}");
            }
            win_shot.close();
            glib::ControlFlow::Break
        });
    }

    window
}

fn signer_title() -> &'static str {
    if env_flag("VIBE_REVEAL") {
        return tr("View keys");
    }
    if env_flag("VIBE_CREATE") {
        return tr("Create wallet");
    }
    if env_flag("VIBE_IMPORT") {
        return tr("Import wallet");
    }
    let wc = std::env::var("VIBE_WC_REQUEST").unwrap_or_default();
    let swap_out = std::env::var("VIBE_SWAP_OUT").unwrap_or_default();
    let swap_in = std::env::var("VIBE_SWAP_IN").unwrap_or_default();
    let stake = std::env::var("VIBE_STAKE_ACTION").unwrap_or_default();
    if !wc.trim().is_empty() {
        tr("Confirm dApp")
    } else if !swap_out.trim().is_empty() || !swap_in.trim().is_empty() {
        tr("Confirm swap")
    } else if !stake.trim().is_empty() {
        tr("Confirm stake")
    } else {
        tr("Confirm send")
    }
}

fn build_signer(
    app: &adw::Application,
    window: adw::ApplicationWindow,
    ctx: Ctx,
    toasts: adw::ToastOverlay,
    root: gtk::Stack,
) -> adw::ApplicationWindow {
    let title_text = signer_title();
    window.set_title(Some(title_text));
    let page = pages::signer::build(&ctx);
    root.add_named(&page, Some("main"));
    root.set_visible_child_name("main");
    toasts.set_child(Some(&root));
    window.set_content(Some(&toasts));
    window.present();

    let close_action = gio::SimpleAction::new("lock", None);
    let win_close = window.clone();
    close_action.connect_activate(move |_, _| win_close.close());
    app.add_action(&close_action);

    if let Ok(path) = std::env::var("VIBE_SCREENSHOT") {
        let win_shot = window.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(800), move || {
            if let Err(err) = capture_window(&win_shot, &path) {
                eprintln!("vibe-vault: screenshot failed: {err}");
            }
            win_shot.close();
            glib::ControlFlow::Break
        });
    }

    window
}

fn capture_window(window: &adw::ApplicationWindow, path: &str) -> Result<(), String> {
    use gtk::gdk::prelude::PaintableExt;
    use gtk::gsk::prelude::GskRendererExt;
    use gtk::prelude::{NativeExt, WidgetExt};

    let width = window.width().max(1);
    let height = window.height().max(1);
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot.to_node().ok_or("empty render node")?;
    let native = WidgetExt::native(window).ok_or("no native surface")?;
    let renderer = native.renderer().ok_or("no GSK renderer")?;
    let texture = renderer.render_texture(&node, None);
    texture
        .save_to_png(path)
        .map_err(|e| format!("png write: {e}"))?;
    eprintln!("vibe-vault: wrote screenshot {path}");
    Ok(())
}

fn app_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some(tr("Lock vault")), Some("app.lock"));
    menu.append(Some(tr("About VIBE Wallet")), Some("app.about"));
    menu.append(Some(tr("Quit")), Some("app.quit"));
    menu
}

fn build_sidebar(list: &gtk::ListBox) -> gtk::Widget {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
    box_.add_css_class("og-sidebar");
    let brand = gtk::Label::new(Some("VIBE"));
    brand.add_css_class("og-brand");
    brand.set_xalign(0.0);
    let sub = gtk::Label::new(Some("WALLET"));
    sub.add_css_class("og-brand-sub");
    sub.set_xalign(0.0);
    box_.append(&brand);
    box_.append(&sub);

    let mut first_row: Option<gtk::ListBoxRow> = None;
    for group in NAV {
        let kicker = gtk::Label::new(Some(tr(group.kicker)));
        kicker.add_css_class("og-nav-kicker");
        kicker.set_xalign(0.0);
        let kicker_row = gtk::ListBoxRow::new();
        kicker_row.set_child(Some(&kicker));
        kicker_row.set_selectable(false);
        kicker_row.set_activatable(false);
        kicker_row.set_sensitive(false);
        list.append(&kicker_row);
        for item in group.items {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(item.id);
            let inner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            inner.set_margin_top(6);
            inner.set_margin_bottom(6);
            inner.set_margin_end(8);
            let icon = gtk::Image::from_icon_name(item.icon);
            icon.set_pixel_size(16);
            let label = gtk::Label::new(Some(tr(item.label)));
            label.add_css_class("og-nav-label");
            label.set_xalign(0.0);
            label.set_hexpand(true);
            inner.append(&icon);
            inner.append(&label);
            row.set_child(Some(&inner));
            if first_row.is_none() {
                first_row = Some(row.clone());
            }
            list.append(&row);
        }
    }
    let nav_scroll = gtk::ScrolledWindow::new();
    nav_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    nav_scroll.set_vexpand(true);
    nav_scroll.set_child(Some(list));
    box_.append(&nav_scroll);
    if let Some(row) = first_row {
        list.select_row(Some(&row));
    }
    box_.upcast()
}

fn nav_label(id: &str) -> Option<&'static str> {
    for group in NAV {
        for item in group.items {
            if item.id == id {
                return Some(tr(item.label));
            }
        }
    }
    None
}

fn select_nav_row(list: &gtk::ListBox, id: &str) {
    let mut i = 0;
    while let Some(row) = list.row_at_index(i) {
        if row.widget_name() == id {
            list.select_row(Some(&row));
            return;
        }
        i += 1;
    }
}
