use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::hd::DEFAULT_PATH;
use crate::keypair::Keypair;
use crate::vault::{self, EncryptedBlob};
use crate::wallet::WalletKeyType;
use crate::{mnemonic, Cluster};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletRecord {
    pub id: String,
    pub name: String,
    pub address: String,
    pub key_type: String,
    #[serde(default = "default_path")]
    pub derivation_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<EncryptedBlob>,
    #[serde(default)]
    pub mnemonic: Option<EncryptedBlob>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hw_plugin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hw_device_id: String,
    /// 0 = off. Enforced in vibe-vault at sign time; the GTK UI cannot skip it.
    #[serde(default)]
    pub max_sol_per_tx: f64,
    /// Extra Confirm checkbox when the mint can freeze, mint, hook, or permanently delegate.
    #[serde(default = "default_t22_extra")]
    pub token2022_extra_confirm: bool,
    /// 0 = off. USD notional cap per transaction (GTK + vault).
    #[serde(default)]
    pub max_usd_per_tx: f64,
    /// 0 = off. Rolling UTC-day USD cap.
    #[serde(default)]
    pub daily_usd_cap: f64,
    /// Block unlimited SPL Approve unless the desk marks a known router.
    #[serde(default = "default_block_approve")]
    pub block_infinite_approve: bool,
}

impl WalletRecord {
    pub fn has_secret(&self) -> bool {
        self.secret
            .as_ref()
            .map(|b| !b.ciphertext.is_empty())
            .unwrap_or(false)
    }

    pub fn key_kind(&self) -> WalletKeyType {
        WalletKeyType::parse(&self.key_type)
    }
}

fn default_path() -> String {
    DEFAULT_PATH.to_string()
}

fn default_t22_extra() -> bool {
    true
}

fn default_block_approve() -> bool {
    true
}

pub const MAX_WALLETS: usize = 30;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WalletStore {
    pub version: u32,
    #[serde(default)]
    pub wallets: Vec<WalletRecord>,
    #[serde(default)]
    pub active_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub cluster: String,
    #[serde(default)]
    pub custom_rpc: String,
    #[serde(default = "default_slippage")]
    pub slippage_bps: u16,
    /// Optional Jupiter Developer Platform key (`x-api-key`).
    #[serde(default)]
    pub jupiter_api_key: String,
    /// Optional CoinGecko demo/pro API key.
    #[serde(default)]
    pub coingecko_api_key: String,
    /// 1.9: ordered fallback RPC endpoints (same cluster), tried after the primary.
    #[serde(default)]
    pub rpc_fallbacks: Vec<String>,
}

fn default_slippage() -> u16 {
    0 // 0 = Jupiter auto slippage (Swap V2 ultra / all routers)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cluster: "Mainnet".into(),
            custom_rpc: String::new(),
            slippage_bps: 0,
            jupiter_api_key: String::new(),
            coingecko_api_key: String::new(),
            rpc_fallbacks: Vec::new(),
        }
    }
}

impl Settings {
    pub fn cluster(&self) -> Cluster {
        match self.cluster.as_str() {
            "Devnet" => Cluster::Devnet,
            "Testnet" => Cluster::Testnet,
            "Custom" if !self.custom_rpc.is_empty() => Cluster::Custom(self.custom_rpc.clone()),
            _ => Cluster::Mainnet,
        }
    }

    pub fn set_cluster(&mut self, cluster: Cluster) {
        match cluster {
            Cluster::Mainnet => {
                self.cluster = "Mainnet".into();
            }
            Cluster::Devnet => {
                self.cluster = "Devnet".into();
            }
            Cluster::Testnet => {
                self.cluster = "Testnet".into();
            }
            Cluster::Custom(url) => {
                self.cluster = "Custom".into();
                self.custom_rpc = url;
            }
        }
    }

    pub fn rpc_url(&self) -> String {
        if !self.custom_rpc.is_empty() && self.cluster == "Custom" {
            self.custom_rpc.clone()
        } else {
            self.cluster().rpc_url().to_string()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub name: String,
    pub address: String,
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub sent_count: u32,
    #[serde(default)]
    pub last_sent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContactBook {
    #[serde(default)]
    pub contacts: Vec<Contact>,
}

pub fn load_store(path: &Path) -> Result<WalletStore> {
    if !path.exists() {
        return Ok(WalletStore {
            version: 1,
            ..Default::default()
        });
    }
    let text = fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(WalletStore {
            version: 1,
            ..Default::default()
        });
    }
    Ok(serde_json::from_str(&text)?)
}

pub fn save_store(path: &Path, store: &WalletStore) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(store)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_settings(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_settings(path: &Path, settings: &Settings) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(settings)?)?;
    Ok(())
}

pub fn load_contacts(path: &Path) -> ContactBook {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_contacts(path: &Path, book: &ContactBook) -> Result<()> {
    write_private(path, &serde_json::to_string_pretty(book)?)
}

pub(crate) fn write_private(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn last_mint_path() -> PathBuf {
    if let Ok(raw) = std::env::var("VIBE_LAST_MINT_PATH") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    config_dir().join("last-mint.json")
}

pub fn write_last_mint(mint: &str, signature: &str, name: &str, symbol: &str) -> Result<()> {
    let path = last_mint_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let program = std::env::var("VIBE_MINT_PROGRAM").unwrap_or_default();
    let mut body = serde_json::json!({
        "mint": mint,
        "signature": signature,
        "name": name,
        "symbol": symbol,
        "ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    if let Some(obj) = body.as_object_mut() {
        let trimmed = program.trim();
        if !trimmed.is_empty() {
            obj.insert("program".into(), serde_json::Value::String(trimmed.to_string()));
        }
    }
    fs::write(&path, serde_json::to_vec_pretty(&body)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn config_dir() -> PathBuf {
    if let Ok(raw) = std::env::var("XDG_CONFIG_HOME") {
        if !raw.is_empty() {
            return PathBuf::from(raw).join("vibe-wallet");
        }
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(home).join(".config/vibe-wallet")
}

pub fn desk_contacts_path() -> PathBuf {
    config_dir().join("contacts.json")
}

pub fn sent_ledger_path() -> PathBuf {
    config_dir().join("sent.json")
}

pub fn swap_events_path() -> PathBuf {
    config_dir().join("swap-events.jsonl")
}

pub fn append_swap_event(
    signature: &str,
    solscan: &str,
    pay: &str,
    recv: &str,
    in_mint: &str,
    out_mint: &str,
    kind: &str,
) -> Result<()> {
    use std::io::Write;

    let path = swap_events_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let kind = if kind.trim().is_empty() { "swap" } else { kind };
    let line = serde_json::json!({
        "signature": signature,
        "solscan": solscan,
        "pay": pay,
        "recv": recv,
        "in_mint": in_mint,
        "out_mint": out_mint,
        "kind": kind,
        "ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{line}")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn is_untrusted_program(address: &str) -> bool {
    use crate::programs::ProgramIds;
    matches!(
        address,
        ProgramIds::SYSTEM
            | ProgramIds::COMPUTE_BUDGET
            | ProgramIds::TOKEN
            | ProgramIds::TOKEN_2022
            | ProgramIds::ASSOCIATED_TOKEN
            | ProgramIds::MEMO
            | ProgramIds::STAKE
            | ProgramIds::VOTE
    )
}

fn shorten(address: &str) -> String {
    crate::format::shorten_address(address)
}

pub fn remember_sent(
    book: &mut ContactBook,
    address: &str,
    amount: &str,
    mint: &str,
    own: &[String],
) -> bool {
    if !crate::format::looks_like_pubkey(address) {
        return false;
    }
    if is_untrusted_program(address) {
        return false;
    }
    if own.iter().any(|item| item == address) {
        return false;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let at = now.to_string();
    let mut ledger: serde_json::Value =
        fs::read_to_string(sent_ledger_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| serde_json::json!({ "sent": [] }));
    if let Some(rows) = ledger.get_mut("sent").and_then(|v| v.as_array_mut()) {
        rows.push(serde_json::json!({
            "address": address,
            "amount": amount,
            "mint": mint,
            "at": at,
        }));
        if rows.len() > 500 {
            let extra = rows.len() - 500;
            rows.drain(0..extra);
        }
    }
    let _ = write_private(
        &sent_ledger_path(),
        &serde_json::to_string_pretty(&ledger).unwrap_or_else(|_| "{\"sent\":[]}".into()),
    );
    if let Some(found) = book.contacts.iter_mut().find(|c| c.address == address) {
        found.trusted = true;
        found.sent_count = found.sent_count.saturating_add(1);
        found.last_sent = at;
        if found.source != "manual" {
            found.source = "sent".into();
        }
    } else {
        book.contacts.push(Contact {
            name: shorten(address),
            address: address.to_string(),
            trusted: true,
            source: "sent".into(),
            sent_count: 1,
            last_sent: at,
        });
    }
    let _ = save_contacts(&contacts_path(), book);
    let _ = save_contacts(&desk_contacts_path(), book);
    true
}

pub fn user_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    let share = PathBuf::from(&home).join(".local/share");
    for name in ["vibe-wallet", "vibe-vault", "oldgrowth-wallet"] {
        let dir = share.join(name);
        if dir.join("wallets.json").exists() {
            return dir;
        }
    }
    let dir = share.join("vibe-wallet");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn wallets_path() -> PathBuf {
    user_data_dir().join("wallets.json")
}

pub fn settings_path() -> PathBuf {
    user_data_dir().join("settings.json")
}

pub fn contacts_path() -> PathBuf {
    user_data_dir().join("contacts.json")
}

fn new_id() -> String {
    let mut b = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut b);
    format!(
        "{:x}{:x}",
        u64::from_be_bytes(b),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

fn ensure_cap(store: &WalletStore) -> Result<()> {
    if store.wallets.len() >= MAX_WALLETS {
        return Err(Error::msg(format!("{MAX_WALLETS} wallets is the limit")));
    }
    Ok(())
}

pub fn set_active_wallet(store: &mut WalletStore, id: &str) -> bool {
    if store.wallets.iter().any(|w| w.id == id) {
        store.active_id = id.to_string();
        true
    } else {
        false
    }
}

pub fn remove_wallet(store: &mut WalletStore, id: &str) -> Result<WalletRecord> {
    let pos = store
        .wallets
        .iter()
        .position(|w| w.id == id)
        .ok_or_else(|| Error::msg("Wallet not found"))?;
    if store.wallets[pos].has_secret() {
        return Err(Error::msg("Remove seed wallets from the vault, not here"));
    }
    let rec = store.wallets.remove(pos);
    if store.active_id == rec.id {
        store.active_id = store
            .wallets
            .first()
            .map(|w| w.id.clone())
            .unwrap_or_default();
    }
    Ok(rec)
}

pub fn rename_wallet(store: &mut WalletStore, id: &str, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    if let Some(rec) = store.wallets.iter_mut().find(|w| w.id == id) {
        rec.name = name.to_string();
        true
    } else {
        false
    }
}

pub fn active_wallet_index(store: &WalletStore) -> usize {
    store
        .wallets
        .iter()
        .position(|w| w.id == store.active_id)
        .unwrap_or(0)
}

pub fn next_solana_account(store: &WalletStore) -> u32 {
    let mut used = std::collections::BTreeSet::new();
    for rec in &store.wallets {
        if let Some(n) = crate::hd::account_index(&rec.derivation_path) {
            used.insert(n);
        } else if rec.mnemonic.is_some() && rec.derivation_path.is_empty() {
            used.insert(0);
        }
    }
    (0u32..)
        .find(|n| !used.contains(n))
        .unwrap_or(store.wallets.len() as u32)
}

pub fn mnemonic_account_count(store: &WalletStore) -> usize {
    store
        .wallets
        .iter()
        .filter(|w| w.mnemonic.is_some())
        .count()
}

pub fn add_mnemonic_wallet(
    store: &mut WalletStore,
    name: &str,
    phrase: &str,
    password: &str,
) -> Result<WalletRecord> {
    add_mnemonic_wallet_at_path(store, name, phrase, password, DEFAULT_PATH, "")
}

pub fn add_mnemonic_wallet_at_path(
    store: &mut WalletStore,
    name: &str,
    phrase: &str,
    password: &str,
    path: &str,
    passphrase: &str,
) -> Result<WalletRecord> {
    ensure_cap(store)?;
    if !mnemonic::validate(phrase) {
        return Err(Error::InvalidMnemonic);
    }
    let path = if path.trim().is_empty() {
        DEFAULT_PATH
    } else {
        path.trim()
    };
    let kp = Keypair::from_mnemonic(phrase, passphrase, path)?;
    let secret64 = kp.secret64();
    let secret = vault::encrypt(&secret64, password)?;
    let mnemonic_blob = vault::encrypt(phrase.as_bytes(), password)?;
    let rec = WalletRecord {
        id: new_id(),
        name: name.trim().to_string(),
        address: kp.address(),
        key_type: WalletKeyType::Mnemonic.as_storage().to_string(),
        derivation_path: path.to_string(),
        secret: Some(secret),
        mnemonic: Some(mnemonic_blob),
        hw_plugin: String::new(),
        hw_device_id: String::new(),
        max_sol_per_tx: 0.0,
        token2022_extra_confirm: true,
        max_usd_per_tx: 0.0,
        daily_usd_cap: 0.0,
        block_infinite_approve: true,
    };
    store.version = 1;
    store.active_id = rec.id.clone();
    store.wallets.push(rec.clone());
    Ok(rec)
}

pub fn add_derived_account(
    store: &mut WalletStore,
    name: &str,
    password: &str,
) -> Result<WalletRecord> {
    ensure_cap(store)?;
    if password.len() < 8 {
        return Err(Error::msg("Password must be at least 8 characters"));
    }
    if !store.wallets.is_empty() {
        unlock_store(store, password)?;
    }
    let seed = store
        .wallets
        .iter()
        .find(|w| w.mnemonic.is_some())
        .cloned()
        .ok_or_else(|| Error::msg("No recovery phrase on this vault"))?;
    let phrase = export_mnemonic(&seed, password)?;
    let idx = next_solana_account(store);
    let path = crate::hd::solana_account_path(idx);
    add_mnemonic_wallet_at_path(store, name, &phrase, password, &path, "")
}

pub fn add_secret_wallet(
    store: &mut WalletStore,
    name: &str,
    secret_b58_or_json: &str,
    password: &str,
) -> Result<WalletRecord> {
    ensure_cap(store)?;
    let secret = parse_secret(secret_b58_or_json)?;
    let kp = Keypair::from_secret64(&secret)?;
    let blob = vault::encrypt(&secret, password)?;
    let rec = WalletRecord {
        id: new_id(),
        name: name.trim().to_string(),
        address: kp.address(),
        key_type: WalletKeyType::PrivateKey.as_storage().to_string(),
        derivation_path: String::new(),
        secret: Some(blob),
        mnemonic: None,
        hw_plugin: String::new(),
        hw_device_id: String::new(),
        max_sol_per_tx: 0.0,
        token2022_extra_confirm: true,
        max_usd_per_tx: 0.0,
        daily_usd_cap: 0.0,
        block_infinite_approve: true,
    };
    store.version = 1;
    store.active_id = rec.id.clone();
    store.wallets.push(rec.clone());
    Ok(rec)
}

fn parse_secret(input: &str) -> Result<[u8; 64]> {
    let t = input.trim();
    if t.starts_with('[') {
        let arr: Vec<u8> = serde_json::from_str(t)
            .map_err(|_| Error::msg("JSON secret key must be a byte array"))?;
        return secret_from_bytes(&arr);
    }
    let decoded = bs58::decode(t)
        .into_vec()
        .map_err(|_| Error::msg("invalid base58 secret key"))?;
    secret_from_bytes(&decoded)
}

fn secret_from_bytes(arr: &[u8]) -> Result<[u8; 64]> {
    if arr.len() == 64 {
        let mut out = [0u8; 64];
        out.copy_from_slice(arr);
        return Ok(out);
    }
    if arr.len() == 32 {
        let kp = Keypair::from_seed32(arr)?;
        return Ok(kp.secret64());
    }
    Err(Error::msg("secret key must be 32 or 64 bytes (JSON array or base58)"))
}

pub fn software_wallet_count(store: &WalletStore) -> usize {
    store.wallets.iter().filter(|w| w.has_secret()).count()
}

pub fn add_watch_wallet(store: &mut WalletStore, name: &str, address: &str) -> Result<WalletRecord> {
    ensure_cap(store)?;
    let address = address.trim();
    if !crate::format::looks_like_pubkey(address) {
        return Err(Error::InvalidAddress);
    }
    let rec = WalletRecord {
        id: new_id(),
        name: name.trim().to_string(),
        address: address.to_string(),
        key_type: WalletKeyType::Watch.as_storage().to_string(),
        derivation_path: String::new(),
        secret: None,
        mnemonic: None,
        hw_plugin: String::new(),
        hw_device_id: String::new(),
        max_sol_per_tx: 0.0,
        token2022_extra_confirm: true,
        max_usd_per_tx: 0.0,
        daily_usd_cap: 0.0,
        block_infinite_approve: true,
    };
    store.version = 1;
    store.active_id = rec.id.clone();
    store.wallets.push(rec.clone());
    Ok(rec)
}

pub fn add_hardware_wallet(
    store: &mut WalletStore,
    name: &str,
    address: &str,
    plugin: &str,
    device_id: &str,
    path: &str,
) -> Result<WalletRecord> {
    ensure_cap(store)?;
    let address = address.trim();
    if !crate::format::looks_like_pubkey(address) {
        return Err(Error::InvalidAddress);
    }
    let kind = WalletKeyType::parse(plugin);
    if kind == WalletKeyType::Lattice {
        return Err(Error::msg("Lattice signing is not in this build"));
    }
    if !kind.is_hardware() {
        return Err(Error::msg("Unknown hardware plugin"));
    }
    let path = if path.trim().is_empty() {
        if kind == WalletKeyType::Trezor {
            crate::trezor::DEFAULT_PATH
        } else {
            crate::ledger::DEFAULT_PATH
        }
    } else {
        path.trim()
    };
    let rec = WalletRecord {
        id: new_id(),
        name: name.trim().to_string(),
        address: address.to_string(),
        key_type: kind.as_storage().to_string(),
        derivation_path: path.to_string(),
        secret: None,
        mnemonic: None,
        hw_plugin: kind.as_storage().to_string(),
        hw_device_id: device_id.trim().to_string(),
        max_sol_per_tx: 0.0,
        token2022_extra_confirm: true,
        max_usd_per_tx: 0.0,
        daily_usd_cap: 0.0,
        block_infinite_approve: true,
    };
    store.version = 1;
    store.active_id = rec.id.clone();
    store.wallets.push(rec.clone());
    Ok(rec)
}

pub fn unlock_store(store: &WalletStore, password: &str) -> Result<Vec<(WalletRecord, Keypair)>> {
    if store.wallets.is_empty() {
        return Err(Error::msg("no wallets on disk"));
    }
    let mut out = Vec::new();
    let mut saw_software = false;
    for rec in &store.wallets {
        if !rec.has_secret() {
            continue;
        }
        saw_software = true;
        let blob = rec
            .secret
            .as_ref()
            .ok_or_else(|| Error::msg("missing vault blob"))?;
        let secret = vault::decrypt(blob, password)?;
        let kp = Keypair::from_secret64(&secret)?;
        if kp.address() != rec.address {
            return Err(Error::msg("wallet blob does not match stored address"));
        }
        out.push((rec.clone(), kp));
    }
    if saw_software && out.is_empty() {
        return Err(Error::WrongPassword);
    }
    if out.is_empty() && software_wallet_count(store) == 0 {
        return Ok(out);
    }
    if out.is_empty() {
        return Err(Error::msg("no software wallets to unlock"));
    }
    Ok(out)
}

pub fn export_mnemonic(rec: &WalletRecord, password: &str) -> Result<String> {
    let Some(blob) = &rec.mnemonic else {
        return Err(Error::msg("this wallet has no seed phrase on disk"));
    };
    let bytes = vault::decrypt(blob, password)?;
    String::from_utf8(bytes).map_err(|_| Error::msg("corrupt mnemonic blob"))
}

pub fn export_secret_base58(rec: &WalletRecord, password: &str) -> Result<String> {
    if rec.key_kind().is_watch() {
        return Err(Error::msg("watch-only accounts have no secret"));
    }
    if rec.key_kind().is_hardware() {
        return Err(Error::msg("hardware wallets keep the secret on the device"));
    }
    let blob = rec
        .secret
        .as_ref()
        .ok_or_else(|| Error::msg("this wallet has no secret on disk"))?;
    let secret = vault::decrypt(blob, password)?;
    let kp = Keypair::from_secret64(&secret)?;
    Ok(bs58::encode(kp.secret64()).into_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NonceBook {
    #[serde(default)]
    pub nonces: Vec<NonceRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonceRow {
    pub address: String,
    pub authority: String,
    #[serde(default)]
    pub wallet_id: String,
}

pub fn nonces_path() -> PathBuf {
    config_dir().join("nonces.json")
}

pub fn load_nonces() -> NonceBook {
    fs::read_to_string(nonces_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_nonces(book: &NonceBook) -> Result<()> {
    write_private(&nonces_path(), &serde_json::to_string_pretty(book)?)
}

pub fn remember_nonce(address: &str, authority: &str, wallet_id: &str) -> Result<()> {
    let mut book = load_nonces();
    if !book.nonces.iter().any(|n| n.address == address) {
        book.nonces.push(NonceRow {
            address: address.to_string(),
            authority: authority.to_string(),
            wallet_id: wallet_id.to_string(),
        });
        save_nonces(&book)?;
    }
    Ok(())
}

pub fn nonce_for_authority(authority: &str) -> Option<NonceRow> {
    load_nonces()
        .nonces
        .into_iter()
        .find(|n| n.authority == authority)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mnemonic;

    #[test]
    fn create_unlock_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ogw-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("wallets.json");
        let mut store = WalletStore {
            version: 1,
            ..Default::default()
        };
        let phrase = mnemonic::generate(12).unwrap();
        let rec = add_mnemonic_wallet(&mut store, "Test", &phrase, "correct-horse").unwrap();
        save_store(&path, &store).unwrap();
        let loaded = load_store(&path).unwrap();
        let unlocked = unlock_store(&loaded, "correct-horse").unwrap();
        assert_eq!(unlocked.len(), 1);
        assert_eq!(unlocked[0].0.address, rec.address);
        assert!(unlock_store(&loaded, "wrong-password").is_err());
        let exported = export_mnemonic(&loaded.wallets[0], "correct-horse").unwrap();
        assert_eq!(exported, phrase);
        let secret = export_secret_base58(&loaded.wallets[0], "correct-horse").unwrap();
        assert!(!secret.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wallet_cap() {
        let mut store = WalletStore {
            version: 1,
            ..Default::default()
        };
        let phrase = mnemonic::generate(12).unwrap();
        for i in 0..MAX_WALLETS {
            add_mnemonic_wallet(&mut store, &format!("W{i}"), &phrase, "correct-horse").unwrap();
        }
        assert_eq!(store.wallets.len(), MAX_WALLETS);
        assert!(add_mnemonic_wallet(&mut store, "Overflow", &phrase, "correct-horse").is_err());
        let last = store.wallets.last().unwrap().id.clone();
        assert_eq!(store.active_id, last);
        let first = store.wallets[0].id.clone();
        assert!(set_active_wallet(&mut store, &first));
        assert_eq!(active_wallet_index(&store), 0);
        assert!(rename_wallet(&mut store, &first, "Main"));
        assert_eq!(store.wallets[0].name, "Main");
    }

    #[test]
    fn derived_accounts_use_next_path() {
        let mut store = WalletStore {
            version: 1,
            ..Default::default()
        };
        let phrase = mnemonic::generate(12).unwrap();
        let a = add_mnemonic_wallet(&mut store, "Main", &phrase, "correct-horse").unwrap();
        let b = add_derived_account(&mut store, "Trading", "correct-horse").unwrap();
        assert_ne!(a.address, b.address);
        assert_eq!(a.derivation_path, crate::hd::DEFAULT_PATH);
        assert_eq!(b.derivation_path, crate::hd::solana_account_path(1));
        assert_eq!(next_solana_account(&store), 2);
    }

    #[test]
    fn secret_accepts_32_or_64_bytes() {
        let kp = crate::keypair::Keypair::generate();
        let s64 = kp.secret64();
        let json64 = serde_json::to_string(&s64.to_vec()).unwrap();
        let json32 = serde_json::to_string(&s64[..32].to_vec()).unwrap();
        let mut a = WalletStore {
            version: 1,
            ..Default::default()
        };
        let rec64 = add_secret_wallet(&mut a, "A", &json64, "correct-horse").unwrap();
        let rec32 = add_secret_wallet(&mut a, "B", &json32, "correct-horse").unwrap();
        assert_eq!(rec64.address, kp.address());
        assert_eq!(rec32.address, kp.address());
        let b58 = bs58::encode(&s64[..32]).into_string();
        let rec_b58 = add_secret_wallet(&mut a, "C", &b58, "correct-horse").unwrap();
        assert_eq!(rec_b58.address, kp.address());
    }

    #[test]
    fn passphrase_changes_derived_address() {
        let mut store = WalletStore {
            version: 1,
            ..Default::default()
        };
        let phrase = mnemonic::generate(12).unwrap();
        let plain = add_mnemonic_wallet_at_path(
            &mut store,
            "A",
            &phrase,
            "correct-horse",
            DEFAULT_PATH,
            "",
        )
        .unwrap();
        let extra = add_mnemonic_wallet_at_path(
            &mut store,
            "B",
            &phrase,
            "correct-horse",
            DEFAULT_PATH,
            "secret",
        )
        .unwrap();
        assert_ne!(plain.address, extra.address);
    }

    #[test]
    fn watch_and_ledger_have_no_secret() {
        let mut store = WalletStore {
            version: 1,
            ..Default::default()
        };
        let kp = crate::keypair::Keypair::generate();
        let rec = add_watch_wallet(&mut store, "Watch", &kp.address()).unwrap();
        assert_eq!(rec.key_type, "watch");
        assert!(!rec.has_secret());
        assert!(unlock_store(&store, "").unwrap().is_empty());
        let trezor = add_hardware_wallet(&mut store, "T", &kp.address(), "trezor", "x", "").unwrap();
        assert_eq!(trezor.key_type, "trezor");
        assert_eq!(trezor.hw_plugin, "trezor");
        assert_eq!(trezor.derivation_path, crate::trezor::DEFAULT_PATH);
        assert!(!trezor.has_secret());
        let hw = add_hardware_wallet(&mut store, "Ledger", &kp.address(), "ledger", "/dev/hidraw0", "").unwrap();
        assert_eq!(hw.key_type, "ledger");
        assert!(!hw.has_secret());
        assert!(export_secret_base58(&hw, "correct-horse").is_err());
        assert!(export_secret_base58(&trezor, "correct-horse").is_err());
        let gone = remove_wallet(&mut store, &trezor.id).unwrap();
        assert_eq!(gone.id, trezor.id);
        assert!(store.wallets.iter().all(|w| w.id != trezor.id));
        assert!(remove_wallet(&mut store, "missing").is_err());
    }

    #[test]
    fn old_secret_object_still_loads() {
        let json = r#"{"version":1,"wallets":[{"id":"a","name":"Main","address":"11111111111111111111111111111111","key_type":"mnemonic","derivation_path":"m/44'/501'/0'/0'","secret":{"salt":"YQ==","nonce":"YQ==","ciphertext":"YQ=="}}],"active_id":"a"}"#;
        let store: WalletStore = serde_json::from_str(json).unwrap();
        assert!(store.wallets[0].secret.is_some());
    }
}
