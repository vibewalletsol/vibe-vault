use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use vibe_core::rpc::{Rpc, Snapshot, StakeAccount, Validator};
use vibe_core::store::{self, ContactBook, Settings, WalletRecord, WalletStore};
use vibe_core::{Cluster, Keypair};

use crate::task;

static RPC_CACHE: std::sync::Mutex<Option<(String, Rpc)>> = std::sync::Mutex::new(None);

fn shared_rpc(primary: &str, fallbacks: &[String], genesis: Option<&str>) -> Rpc {
    let key = format!(
        "{primary}|{}|{}",
        fallbacks.join(","),
        genesis.unwrap_or("")
    );
    let mut g = RPC_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, r)) = g.as_ref() {
        if *k == key {
            return r.clone();
        }
    }
    let r = Rpc::with_endpoints(primary, fallbacks, genesis);
    *g = Some((key, r.clone()));
    r
}

#[derive(Clone)]
pub struct Unlocked {
    pub id: String,
    pub name: String,
    pub address: String,
    pub keypair: Option<Keypair>,
    pub key_type: String,
    pub derivation_path: String,
    pub hw_device_id: String,
}

impl Unlocked {
    pub fn kind(&self) -> vibe_core::WalletKeyType {
        vibe_core::WalletKeyType::parse(&self.key_type)
    }

    pub fn public_bytes(&self) -> Result<[u8; 32], String> {
        if let Some(kp) = &self.keypair {
            return Ok(kp.public_bytes());
        }
        vibe_core::tx::decode_pubkey(&self.address).map_err(|e| e.to_string())
    }

    pub fn sign_message(&self, message: &[u8]) -> Result<[u8; 64], String> {
        if let Some(kp) = &self.keypair {
            return Ok(kp.sign(message));
        }
        if message.first() == Some(&vibe_core::txcheck::V1_PREFIX) {
            if self.kind() == vibe_core::WalletKeyType::Ledger {
                return Err("A Ledger cannot sign a version 1 transaction yet. Use the vault on this computer.".into());
            }
            if self.kind() == vibe_core::WalletKeyType::Trezor {
                return Err("A Trezor cannot sign a version 1 transaction yet. Use the vault on this computer.".into());
            }
        }
        if self.kind() == vibe_core::WalletKeyType::Ledger {
            let path = if self.derivation_path.is_empty() {
                vibe_core::ledger::DEFAULT_PATH
            } else {
                self.derivation_path.as_str()
            };
            let device = if self.hw_device_id.is_empty() {
                vibe_core::ledger::scan_devices()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .next()
                    .map(|d| d.path)
                    .ok_or_else(|| "Plug in a Ledger with the Solana app open".to_string())?
            } else {
                self.hw_device_id.clone()
            };
            return vibe_core::ledger::sign_message(&device, path, message)
                .map_err(|e| e.to_string());
        }
        if self.kind() == vibe_core::WalletKeyType::Trezor {
            let path = if self.derivation_path.is_empty() {
                vibe_core::trezor::DEFAULT_PATH
            } else {
                self.derivation_path.as_str()
            };
            let device = if self.hw_device_id.is_empty() {
                vibe_core::trezor::scan_devices()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .next()
                    .map(|d| d.path)
                    .ok_or_else(|| {
                        "Plug in a Trezor Safe 3 / T and quit Trezor Suite if it has the device"
                            .to_string()
                    })?
            } else {
                self.hw_device_id.clone()
            };
            return vibe_core::trezor::sign_message(&device, path, message)
                .map_err(|e| e.to_string());
        }
        if self.kind().is_watch() {
            return Err("Watch-only account — it cannot sign".into());
        }
        Err("This account cannot sign".into())
    }

    /// Sign an already-built version 1 off-chain message.
    /// Ledger uses the Solana app's off-chain instruction so the stick can show the text.
    pub fn sign_offchain(&self, message: &[u8]) -> Result<[u8; 64], String> {
        if let Some(kp) = &self.keypair {
            return Ok(kp.sign(message));
        }
        if self.kind() == vibe_core::WalletKeyType::Ledger {
            let path = if self.derivation_path.is_empty() {
                vibe_core::ledger::DEFAULT_PATH
            } else {
                self.derivation_path.as_str()
            };
            let device = if self.hw_device_id.is_empty() {
                vibe_core::ledger::scan_devices()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .next()
                    .map(|d| d.path)
                    .ok_or_else(|| "Plug in a Ledger with the Solana app open".to_string())?
            } else {
                self.hw_device_id.clone()
            };
            return vibe_core::ledger::sign_offchain_message(&device, path, message)
                .map_err(|e| e.to_string());
        }
        if self.kind() == vibe_core::WalletKeyType::Trezor {
            return Err(
                "A Trezor cannot clear-sign this message yet. Use the vault on this computer, or a Ledger with Solana app 1.14 or newer."
                    .into(),
            );
        }
        if self.kind().is_watch() {
            return Err("Watch-only account — it cannot sign".into());
        }
        Err("This account cannot sign".into())
    }

    pub fn sign_legacy(
        &self,
        fee_payer: &str,
        blockhash: &str,
        ixs: &[vibe_core::tx::Instruction],
    ) -> Result<Vec<u8>, String> {
        let pk = self.public_bytes()?;
        vibe_core::tx::build_signed_legacy_with(fee_payer, blockhash, ixs, &[pk], |_, msg| {
            self.sign_message(msg).map_err(|e| vibe_core::Error::msg(e))
        })
        .map_err(|e| e.to_string())
    }

    pub fn sign_wire(&self, tx: &mut [u8]) -> Result<(), String> {
        let pk = self.public_bytes()?;
        vibe_core::tx::sign_wire_transaction_with(tx, &pk, |msg| {
            self.sign_message(msg).map_err(|e| vibe_core::Error::msg(e))
        })
        .map_err(|e| e.to_string())
    }
}

pub struct Session {
    pub unlocked: Vec<Unlocked>,
    pub active: usize,
}

impl Session {
    pub fn current(&self) -> Option<&Unlocked> {
        self.unlocked.get(self.active)
    }

    pub fn current_mut(&mut self) -> Option<&mut Unlocked> {
        self.unlocked.get_mut(self.active)
    }
}

pub struct AppState {
    pub settings: Settings,
    pub store: WalletStore,
    pub session: Option<Session>,
    pub snapshot: Snapshot,
    pub contacts: ContactBook,
    pub validators: Vec<Validator>,
    pub stakes: Vec<StakeAccount>,
    pub busy: bool,
}

pub struct ShellWidgets {
    pub balance: gtk::Label,
    pub fiat: gtk::Label,
    pub address: gtk::Label,
    pub chip: gtk::Label,
    pub hint: gtk::Label,
    pub hero_chg: gtk::Label,
    pub metric_sol: gtk::Label,
    pub metric_tokens: gtk::Label,
    pub metric_day: gtk::Label,
    pub donut: gtk::DrawingArea,
    pub donut_slices: Rc<RefCell<Vec<crate::donut::Slice>>>,
    pub legend: gtk::Box,
    pub holdings: gtk::Box,
    pub activity: gtk::Box,
    pub assets: gtk::Box,
    pub wallets: gtk::Box,
    pub receive_addr: gtk::Label,
    pub qr_addr: Rc<RefCell<String>>,
    pub qr_area: gtk::DrawingArea,
    pub cluster_drop: gtk::DropDown,
}

#[derive(Clone)]
pub struct Ctx {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub root: gtk::Stack,
    pub state: Rc<RefCell<AppState>>,
    pub shell: Rc<RefCell<Option<ShellWidgets>>>,
}

impl Ctx {
    pub fn toast(&self, msg: impl AsRef<str>) {
        task::toast(&self.toasts, msg);
    }

    /// Vault RPC with ordered failover (1.9). Primary = `VIBE_RPC_URL` from the desk or the
    /// vault setting; fallbacks = `VIBE_RPC_FALLBACKS` from the desk or the vault setting.
    /// One shared instance per (primary, fallbacks) so the active endpoint sticks.
    pub fn rpc(&self) -> Rpc {
        let st = self.state.borrow();
        let primary = std::env::var("VIBE_RPC_URL")
            .ok()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| st.settings.rpc_url());
        let fallbacks = match std::env::var("VIBE_RPC_FALLBACKS") {
            Ok(raw) if !raw.trim().is_empty() => vibe_core::rpc::parse_endpoint_list(&raw),
            _ => st.settings.rpc_fallbacks.clone(),
        };
        // window.rs already folds the desk's VIBE_CLUSTER into settings.cluster.
        let genesis = st.settings.cluster().genesis_hash();
        drop(st);
        shared_rpc(&primary, &fallbacks, genesis)
    }

    pub fn cluster(&self) -> Cluster {
        self.state.borrow().settings.cluster()
    }

    pub fn locked(&self) -> bool {
        self.state.borrow().session.is_none()
    }

    pub fn has_wallets(&self) -> bool {
        !self.state.borrow().store.wallets.is_empty()
    }

    pub fn active_address(&self) -> Option<String> {
        self.state
            .borrow()
            .session
            .as_ref()
            .and_then(|s| s.current())
            .map(|u| u.address.clone())
    }

    pub fn active_wallet(&self) -> Option<Unlocked> {
        self.state
            .borrow()
            .session
            .as_ref()
            .and_then(|s| s.current())
            .cloned()
    }

    pub fn show_root(&self, name: &str) {
        self.root.set_visible_child_name(name);
    }

    pub fn persist(&self) {
        let st = self.state.borrow();
        let _ = store::save_store(&store::wallets_path(), &st.store);
        let _ = store::save_settings(&store::settings_path(), &st.settings);
        let _ = store::save_contacts(&store::contacts_path(), &st.contacts);
        let _ = store::save_contacts(&store::desk_contacts_path(), &st.contacts);
    }

    pub fn apply_snapshot(&self, snap: Snapshot) {
        if let Some(shell) = self.shell.borrow().as_ref() {
            let cluster = self.cluster();
            let total = vibe_core::coingecko::total_usd(&snap.holdings);
            let usd = if total > 0.0 {
                total
            } else {
                vibe_core::rpc::usd_from_lamports(snap.lamports, snap.sol_usd)
            };
            shell.balance.set_text(&vibe_core::format_usd(usd));
            shell.fiat.set_text(&format!(
                "{} SOL · {}",
                vibe_core::format_sol(snap.lamports),
                vibe_core::shorten_address(&snap.address)
            ));
            shell
                .address
                .set_text(&vibe_core::shorten_address(&snap.address));
            shell.address.set_tooltip_text(Some(&snap.address));
            shell.chip.set_text(cluster.as_label());
            if matches!(cluster, Cluster::Devnet | Cluster::Testnet) {
                shell.chip.add_css_class("og-chip-teal");
            } else {
                shell.chip.remove_css_class("og-chip-teal");
            }
            apply_change_label(&shell.hero_chg, snap.change_24h);
            apply_change_label(&shell.metric_day, snap.change_24h);
            shell
                .metric_sol
                .set_text(&format!("{} SOL", vibe_core::format_sol(snap.lamports)));
            let token_usd: f64 = snap
                .holdings
                .iter()
                .filter(|h| h.coin_id != "solana")
                .map(|h| h.value_usd)
                .sum();
            shell
                .metric_tokens
                .set_text(&vibe_core::format_usd(token_usd));
            shell.hint.set_text(&format!(
                "{} coins · epoch {} · slot {}",
                snap.holdings.len(),
                snap.epoch,
                snap.slot
            ));

            let mut slices: Vec<crate::donut::Slice> = Vec::new();
            if usd > 0.0 {
                for h in snap.holdings.iter().take(8) {
                    slices.push(crate::donut::Slice {
                        symbol: h.symbol.clone(),
                        value: h.value_usd,
                        pct: (h.value_usd / usd) * 100.0,
                    });
                }
            }
            fill_legend(&shell.legend, &slices);
            *shell.donut_slices.borrow_mut() = slices;
            shell.donut.queue_draw();

            crate::holdings::fill_list(&shell.holdings, &snap.holdings);
            crate::holdings::fill_list(&shell.assets, &snap.holdings);
            fill_activity(&shell.activity, &snap);
            shell.receive_addr.set_text(&snap.address);
            *shell.qr_addr.borrow_mut() = snap.address.clone();
            shell.qr_area.queue_draw();
        }
        self.state.borrow_mut().snapshot = snap;
        self.refresh_wallets_list();
    }

    pub fn refresh_wallets_list(&self) {
        let Some(shell) = self.shell.borrow().as_ref().map(|s| s.wallets.clone()) else {
            return;
        };
        while let Some(c) = shell.first_child() {
            shell.remove(&c);
        }
        let store_wallets = self.state.borrow().store.wallets.clone();
        let active = self.active_address();
        for w in store_wallets {
            shell.append(&wallet_row(self, &w, active.as_deref() == Some(&w.address)));
        }
        if self.state.borrow().store.wallets.is_empty() {
            let empty = gtk::Label::new(Some("No wallets yet."));
            empty.add_css_class("og-muted");
            empty.set_xalign(0.0);
            shell.append(&empty);
        }
    }

    pub fn refresh(&self) {
        let Some(addr) = self.active_address() else {
            return;
        };
        let rpc = self.rpc();
        let gecko = self.state.borrow().settings.coingecko_api_key.clone();
        let ctx = self.clone();
        self.state.borrow_mut().busy = true;
        task::spawn(
            move || rpc.snapshot_with_gecko(&addr, Some(gecko.as_str()).filter(|s| !s.is_empty())),
            move |result| {
                ctx.state.borrow_mut().busy = false;
                match result {
                    Ok(snap) => ctx.apply_snapshot(snap),
                    Err(e) => ctx.toast(format!("Refresh failed: {e}")),
                }
            },
        );
    }

    pub fn lock(&self) {
        self.state.borrow_mut().session = None;
        self.state.borrow_mut().snapshot = Snapshot::default();
        self.show_root("lock");
    }
}

fn fill_activity(box_: &gtk::Box, snap: &Snapshot) {
    while let Some(c) = box_.first_child() {
        box_.remove(&c);
    }
    if snap.activity.is_empty() {
        let l = gtk::Label::new(Some("No signatures yet on this cluster."));
        l.add_css_class("og-muted");
        l.set_xalign(0.0);
        l.set_wrap(true);
        box_.append(&l);
        return;
    }
    for sig in snap.activity.iter().take(20) {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let top = gtk::Label::new(Some(&vibe_core::shorten_address(&sig.signature)));
        top.set_xalign(0.0);
        top.add_css_class("og-mono");
        let status = if sig.err { "failed" } else { "confirmed" };
        let sub = gtk::Label::new(Some(&format!(
            "{status} · slot {}{}",
            sig.slot,
            sig.memo
                .as_ref()
                .map(|m| format!(" · {m}"))
                .unwrap_or_default()
        )));
        sub.set_xalign(0.0);
        sub.add_css_class(if sig.err { "og-muted" } else { "og-teal" });
        row.append(&top);
        row.append(&sub);
        box_.append(&row);
    }
}

fn apply_change_label(lab: &gtk::Label, chg: Option<f64>) {
    lab.remove_css_class("og-up");
    lab.remove_css_class("og-down");
    lab.remove_css_class("og-flat");
    match chg {
        Some(v) if v > 0.05 => {
            lab.set_text(&format!("▲ {v:.2}%  24h"));
            lab.add_css_class("og-up");
        }
        Some(v) if v < -0.05 => {
            lab.set_text(&format!("▼ {v:.2}%  24h"));
            lab.add_css_class("og-down");
        }
        Some(v) => {
            lab.set_text(&format!("{v:.2}%  24h"));
            lab.add_css_class("og-flat");
        }
        None => {
            lab.set_text("Live prices from CoinGecko");
            lab.add_css_class("og-flat");
        }
    }
}

fn fill_legend(box_: &gtk::Box, slices: &[crate::donut::Slice]) {
    while let Some(c) = box_.first_child() {
        box_.remove(&c);
    }
    if slices.is_empty() {
        let hint = gtk::Label::new(Some("Allocation appears\nwhen coins have prices"));
        hint.add_css_class("og-muted");
        hint.set_xalign(0.0);
        box_.append(&hint);
        return;
    }
    for (i, s) in slices.iter().take(6).enumerate() {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let swatch = gtk::Label::new(None);
        swatch.set_markup(&format!(
            "<span foreground=\"{}\" size=\"large\">●</span>",
            crate::donut::color_hex(i)
        ));
        let name = gtk::Label::new(Some(&format!("{}  {:.0}%", s.symbol, s.pct)));
        name.add_css_class("og-legend-lab");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        line.append(&swatch);
        line.append(&name);
        box_.append(&line);
    }
}

fn wallet_row(ctx: &Ctx, rec: &WalletRecord, active: bool) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("og-card");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    let name = gtk::Label::new(Some(&rec.name));
    name.set_xalign(0.0);
    name.add_css_class("og-heading");
    let addr = gtk::Label::new(Some(&rec.address));
    addr.set_xalign(0.0);
    addr.add_css_class("og-mono");
    addr.add_css_class("og-muted");
    addr.set_selectable(true);
    col.append(&name);
    col.append(&addr);
    row.append(&col);
    if active {
        let chip = gtk::Label::new(Some("Active"));
        chip.add_css_class("og-chip");
        row.append(&chip);
    } else if ctx.state.borrow().session.is_some() {
        let btn = gtk::Button::with_label("Use");
        btn.add_css_class("og-ghost");
        let id = rec.id.clone();
        let ctx = ctx.clone();
        btn.connect_clicked(move |_| {
            if let Some(session) = ctx.state.borrow_mut().session.as_mut() {
                if let Some(i) = session.unlocked.iter().position(|u| u.id == id) {
                    session.active = i;
                }
            }
            {
                let mut st = ctx.state.borrow_mut();
                store::set_active_wallet(&mut st.store, &id);
            }
            ctx.persist();
            ctx.refresh_wallets_list();
            ctx.refresh();
        });
        row.append(&btn);
    }
    let copy = gtk::Button::with_label("Copy");
    copy.add_css_class("og-ghost");
    let address = rec.address.clone();
    let ctx_c = ctx.clone();
    copy.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&address);
            ctx_c.toast("Address copied");
        }
    });
    row.append(&copy);
    row.upcast()
}
