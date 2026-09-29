//! UI translations for vibe-vault (1.9 batch 4). gettext `.mo` catalogs, domain `vibe-vault`.
//!
//! * `tr("…")` translates at call time (returns `&'static str`); `n_("…")` only marks a
//!   string for extraction (usable in `const` tables) and is translated where it is shown.
//!   `tools/i18n.py extract` collects both from `crates/vibe-ui/src`.
//! * Language: `LANGUAGE` (colon list), else `LC_ALL`, `LC_MESSAGES`, `LANG`. The desk exports
//!   `LANGUAGE` when Options → General → Language is set, and the vault inherits it.
//! * **Security:** catalogs are only read from root-managed install dirs
//!   (`<exe>/../share/locale`, `/usr/share/locale`, `/usr/local/share/locale`) and, only with
//!   `VIBE_DEV=1`, from the source tree's `po/vault/<lang>.po`. Never from `$HOME` or XDG
//!   dirs, never world-writable files, max 1 MiB. A translation whose `{}` placeholders differ
//!   from the English string, or that contains control / bidi-override characters, is dropped.
//! * Only window chrome (navigation, buttons, headings) is marked. Transaction review text —
//!   vault rule findings, blocked reasons, amounts, addresses — is intentionally NOT
//!   translated, so a wrong or tampered catalog can never soften a signing warning.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const DOMAIN: &str = crate::GETTEXT_DOMAIN;
const MAX_CATALOG: u64 = 1 << 20;

/// Extraction marker for `const` tables; translate with [`tr`] where the text is shown.
pub const fn n_(s: &'static str) -> &'static str {
    s
}

struct Catalog {
    lang: String,
    map: HashMap<String, &'static str>,
}

fn catalog() -> &'static Catalog {
    static CAT: OnceLock<Catalog> = OnceLock::new();
    CAT.get_or_init(|| {
        let env = |k: &str| std::env::var(k).ok();
        let langs = languages(env);
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        let dev = std::env::var("VIBE_DEV").map(|v| v.trim() == "1").unwrap_or(false);
        for lang in langs {
            if lang == "en" || lang.starts_with("en_") {
                break;
            }
            if let Some(map) = load_for(&lang, exe_dir.as_deref(), dev) {
                let map = map.into_iter().map(|(k, v)| (k, &*Box::leak(v.into_boxed_str()))).collect();
                return Catalog { lang, map };
            }
        }
        Catalog { lang: "en".into(), map: HashMap::new() }
    })
}

/// Translate `msgid` (English source text). Unknown → the English text.
pub fn tr(msgid: &'static str) -> &'static str {
    catalog().map.get(msgid).copied().unwrap_or(msgid)
}

/// Language actually in use ("en" when no catalog matched).
pub fn active_language() -> &'static str {
    &catalog().lang
}

fn expand(tag: &str) -> Vec<String> {
    let tag = tag.trim();
    if tag.contains(['/', '\\']) || tag.starts_with('.') {
        return Vec::new();
    }
    let t = tag.split(['.', '@']).next().unwrap_or("").trim();
    if t.is_empty() || t == "C" || t == "POSIX" {
        return vec!["en".into()];
    }
    if !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Vec::new();
    }
    let mut out = vec![t.to_string()];
    if let Some((base, _)) = t.split_once('_') {
        out.push(base.to_string());
    }
    out
}

pub fn languages<F: Fn(&str) -> Option<String>>(env: F) -> Vec<String> {
    for var in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        let Some(val) = env(var).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) else { continue };
        let parts: Vec<&str> = if var == "LANGUAGE" { val.split(':').collect() } else { vec![val.as_str()] };
        let mut out: Vec<String> = Vec::new();
        for p in parts {
            for l in expand(p) {
                if !out.contains(&l) {
                    out.push(l);
                }
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    vec!["en".into()]
}

fn trusted_file(p: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() || meta.len() > MAX_CATALOG {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o002 != 0 {
            return None;
        }
    }
    std::fs::read(p).ok()
}

fn load_for(lang: &str, exe_dir: Option<&Path>, dev: bool) -> Option<HashMap<String, String>> {
    let rel = format!("{lang}/LC_MESSAGES/{DOMAIN}.mo");
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = exe_dir {
        dirs.push(d.join("../share/locale"));
    }
    dirs.push(PathBuf::from("/usr/share/locale"));
    dirs.push(PathBuf::from("/usr/local/share/locale"));
    for d in dirs {
        if let Some(bytes) = trusted_file(&d.join(&rel)) {
            if let Ok(m) = parse_mo(&bytes) {
                return Some(m);
            }
        }
    }
    if dev {
        // target/release/vibe-vault → <repo>/po/vault/<lang>.po
        if let Some(d) = exe_dir {
            let po = d.join("../../po/vault").join(format!("{lang}.po"));
            if let Some(bytes) = trusted_file(&po) {
                if let Ok(text) = String::from_utf8(bytes) {
                    return Some(parse_po(&text));
                }
            }
        }
    }
    None
}

fn placeholders(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c == '{' {
            if let Some(end) = s[i..].find('}') {
                out.push(s[i..i + end + 1].to_string());
            }
        }
    }
    out.sort();
    out
}

fn acceptable(id: &str, tr: &str) -> bool {
    !id.is_empty()
        && !tr.is_empty()
        && placeholders(id) == placeholders(tr)
        && !tr.chars().any(|c| {
            (c.is_control() && c != '\n')
                || matches!(c as u32, 0x202A..=0x202E | 0x2066..=0x2069 | 0x200E | 0x200F | 0x061C)
        })
}

/// GNU `.mo` (either byte order). Plural entries use their first form. Bounds-checked.
pub fn parse_mo(b: &[u8]) -> Result<HashMap<String, String>, String> {
    if b.len() < 28 {
        return Err("mo too short".into());
    }
    let le = match &b[..4] {
        [0xde, 0x12, 0x04, 0x95] => true,
        [0x95, 0x04, 0x12, 0xde] => false,
        _ => return Err("not a .mo file".into()),
    };
    let u32at = |o: usize| -> Result<usize, String> {
        let s: [u8; 4] = b.get(o..o + 4).ok_or("mo truncated")?.try_into().unwrap();
        Ok(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) } as usize)
    };
    let n = u32at(8)?;
    let (oi, os) = (u32at(12)?, u32at(16)?);
    if n > 100_000 {
        return Err("mo has too many entries".into());
    }
    let get = |tab: usize, i: usize| -> Result<&[u8], String> {
        let len = u32at(tab + 8 * i)?;
        let off = u32at(tab + 8 * i + 4)?;
        b.get(off..off.checked_add(len).ok_or("overflow")?).ok_or_else(|| "mo string out of range".to_string())
    };
    let mut map = HashMap::new();
    for i in 0..n {
        let (Ok(k), Ok(v)) = (get(oi, i), get(os, i)) else { return Err("mo table out of range".into()) };
        let (Ok(k), Ok(v)) = (std::str::from_utf8(k), std::str::from_utf8(v)) else { continue };
        if k.is_empty() {
            continue; // header
        }
        let k = k.split('\0').next().unwrap_or("");
        let v = v.split('\0').next().unwrap_or("");
        if acceptable(k, v) {
            map.insert(k.to_string(), v.to_string());
        }
    }
    Ok(map)
}

fn unquote(s: &str) -> Option<String> {
    let s = s.trim();
    let body = s.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut it = body.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                o => out.push(o),
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Minimal `.po` reader for dev runs (msgid/msgstr, continuation lines; fuzzy and
/// obsolete entries skipped; plural entries ignored).
pub fn parse_po(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let (mut id, mut st, mut key, mut fuzzy, mut plural) = (String::new(), String::new(), 0u8, false, false);
    let mut flush = |id: &mut String, st: &mut String, fuzzy: bool, plural: bool| {
        if !fuzzy && !plural && acceptable(id, st) {
            map.insert(std::mem::take(id), std::mem::take(st));
        }
        id.clear();
        st.clear();
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("#,") {
            flush(&mut id, &mut st, fuzzy, plural);
            key = 0;
            fuzzy = line.contains("fuzzy");
            plural = false;
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("msgid_plural") {
            let _ = rest;
            plural = true;
            key = 3;
        } else if let Some(rest) = line.strip_prefix("msgid") {
            if key == 2 || key == 3 {
                flush(&mut id, &mut st, fuzzy, plural);
                fuzzy = false;
                plural = false;
            }
            id = unquote(rest).unwrap_or_default();
            key = 1;
        } else if line.starts_with("msgstr[") {
            key = 3;
        } else if let Some(rest) = line.strip_prefix("msgstr") {
            st = unquote(rest).unwrap_or_default();
            key = 2;
        } else if line.starts_with('"') {
            let v = unquote(line).unwrap_or_default();
            match key {
                1 => id.push_str(&v),
                2 => st.push_str(&v),
                _ => {}
            }
        }
    }
    flush(&mut id, &mut st, fuzzy, plural);
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mo(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut pairs: Vec<(&str, &str)> = pairs.to_vec();
        pairs.sort();
        let n = pairs.len();
        let (oi, os) = (28, 28 + 8 * n);
        let mut ids = Vec::new();
        let mut strs = Vec::new();
        let mut it = Vec::new();
        let mut st = Vec::new();
        let base_i = 28 + 16 * n;
        let total_ids: usize = pairs.iter().map(|(k, _)| k.len() + 1).sum();
        let base_s = base_i + total_ids;
        for (k, v) in &pairs {
            it.push((k.len() as u32, (base_i + ids.len()) as u32));
            ids.extend_from_slice(k.as_bytes());
            ids.push(0);
            st.push((v.len() as u32, (base_s + strs.len()) as u32));
            strs.extend_from_slice(v.as_bytes());
            strs.push(0);
        }
        let mut out = Vec::new();
        for x in [0x950412deu32, 0, n as u32, oi as u32, os as u32, 0, 0] {
            out.extend_from_slice(&x.to_le_bytes());
        }
        for (a, b) in it.iter().chain(st.iter()) {
            out.extend_from_slice(&a.to_le_bytes());
            out.extend_from_slice(&b.to_le_bytes());
        }
        out.extend(ids);
        out.extend(strs);
        out
    }

    #[test]
    fn mo_parse_and_guards() {
        let b = mo(&[
            ("", "Content-Type: text/plain; charset=UTF-8\n"),
            ("Cancel", "Cancelar"),
            ("{n} wallets", "{n} carteras"),
            ("Send {amount}", "Enviar"),
            ("Sign", "Fir\u{202E}mar"),
        ]);
        let m = parse_mo(&b).unwrap();
        assert_eq!(m.get("Cancel").map(String::as_str), Some("Cancelar"));
        assert_eq!(m.get("{n} wallets").map(String::as_str), Some("{n} carteras"));
        assert!(!m.contains_key("Send {amount}"), "placeholder mismatch dropped");
        assert!(!m.contains_key("Sign"), "bidi override dropped");
        assert!(!m.contains_key(""));
        // Truncated / garbage input never panics.
        for cut in 0..b.len() {
            let _ = parse_mo(&b[..cut]);
        }
        let mut evil = b.clone();
        evil[8..12].copy_from_slice(&50_000u32.to_le_bytes());
        assert!(parse_mo(&evil).is_err());
        assert!(parse_mo(b"not a catalog at all, clearly").is_err());
    }

    #[test]
    fn po_parse_skips_fuzzy_plural_and_bad() {
        let po = r#"
msgid ""
msgstr "Content-Type: text/plain; charset=UTF-8\n"

#: a.rs:1
msgid "Lock vault"
msgstr "Bloquear la bóveda"

#, fuzzy
msgid "Quit"
msgstr "Salir"

msgid "Long "
"line"
msgstr "Línea "
"larga"

msgid "{} files"
msgid_plural "{} files"
msgstr[0] "{} archivo"
msgstr[1] "{} archivos"

msgid "Settings"
msgstr ""
"#;
        let m = parse_po(po);
        assert_eq!(m.get("Lock vault").map(String::as_str), Some("Bloquear la bóveda"));
        assert_eq!(m.get("Long line").map(String::as_str), Some("Línea larga"));
        assert!(!m.contains_key("Quit"));
        assert!(!m.contains_key("{} files"));
        assert!(!m.contains_key("Settings"));
    }

    #[test]
    fn language_resolution() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(a, _)| *a == k).map(|(_, v)| v.to_string())
        };
        assert_eq!(languages(env(&[("LANG", "es_ES.UTF-8")])), ["es_ES", "es"]);
        assert_eq!(languages(env(&[("LANGUAGE", "de:es"), ("LANG", "fr_FR.UTF-8")])), ["de", "es"]);
        assert_eq!(languages(env(&[("LC_ALL", "C")])), ["en"]);
        assert_eq!(languages(env(&[])), ["en"]);
        // Path tricks in a locale name are ignored.
        assert_eq!(languages(env(&[("LANGUAGE", "../../tmp/x"), ("LANG", "es_MX")])), ["es_MX", "es"]);
        // 1.9: regional locales fall back to the shipped language catalog.
        assert_eq!(languages(env(&[("LANG", "fr_CA.UTF-8")])), ["fr_CA", "fr"]);
        assert_eq!(languages(env(&[("LC_MESSAGES", "de_AT.UTF-8")])), ["de_AT", "de"]);
        assert_eq!(languages(env(&[("LANG", "pt_PT.UTF-8")])), ["pt_PT", "pt"]);
        assert_eq!(languages(env(&[("LANG", "it_CH.UTF-8@euro")])), ["it_CH", "it"]);
        // What the desk exports for an explicit Options → Language pick.
        assert_eq!(languages(env(&[("LANGUAGE", "pt_BR:pt"), ("LANG", "en_US.UTF-8")])), ["pt_BR", "pt"]);
    }

    #[test]
    fn shipped_vault_catalogs_are_complete() {
        // Every po/vault/<lang>.po must translate every vault string and pass the guards
        // (placeholders, no bidi/control characters) — parse_po drops anything that fails.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../po/vault");
        let pot = std::fs::read_to_string(dir.join("vibe-vault.pot")).expect("vibe-vault.pot");
        let total = pot.lines().filter(|l| l.starts_with("msgid \"") && *l != "msgid \"\"").count()
            + pot.matches("msgid \"\"\n\"").count();
        assert!(total >= 60, "unexpected pot size {total}");
        for lang in ["es", "fr", "de", "it", "pt"] {
            let text = std::fs::read_to_string(dir.join(format!("{lang}.po"))).expect(lang);
            let map = parse_po(&text);
            assert_eq!(map.len(), total, "{lang}.po: {} of {total} usable", map.len());
        }
    }

    #[test]
    fn catalogs_only_from_install_dirs_or_dev_tree() {
        let root = std::env::temp_dir().join(format!("vibe-i18n-{}", std::process::id()));
        let bin = root.join("target/release");
        std::fs::create_dir_all(&bin).unwrap();
        // Installed layout: <exe>/../share/locale/<lang>/LC_MESSAGES/vibe-vault.mo
        let modir = root.join("target/share/locale/xx/LC_MESSAGES");
        std::fs::create_dir_all(&modir).unwrap();
        let mofile = modir.join(format!("{DOMAIN}.mo"));
        std::fs::write(&mofile, mo(&[("Quit", "Salir")])).unwrap();
        let m = load_for("xx", Some(&bin), false).unwrap();
        assert_eq!(m.get("Quit").map(String::as_str), Some("Salir"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&mofile, std::fs::Permissions::from_mode(0o666)).unwrap();
            assert!(load_for("xx", Some(&bin), false).is_none(), "world-writable catalog refused");
        }
        // Dev tree po/vault/<lang>.po only with VIBE_DEV.
        std::fs::create_dir_all(root.join("po/vault")).unwrap();
        std::fs::write(root.join("po/vault/yy.po"), "msgid \"Quit\"\nmsgstr \"Beenden\"\n").unwrap();
        assert!(load_for("yy", Some(&bin), false).is_none());
        assert_eq!(load_for("yy", Some(&bin), true).unwrap().get("Quit").map(String::as_str), Some("Beenden"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_strings_stay_english() {
        // No catalog for the test process language → English source text back.
        assert_eq!(tr("Some string that is not in any catalog"), "Some string that is not in any catalog");
        assert_eq!(n_("Dashboard"), "Dashboard");
    }

    #[test]
    fn review_code_is_not_translated() {
        // Vault rule findings and the signing path stay English (see module docs).
        for (name, src) in [("txcheck.rs", include_str!("txcheck.rs")), ("tx.rs", include_str!("tx.rs")), ("policy.rs", include_str!("policy.rs"))] {
            assert!(!src.contains("i18n::") && !src.contains("tr(\""), "{name} must not be translated");
        }
    }
}
