use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("VIBE_DATA_DIR") {
        return PathBuf::from(p);
    }

    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    if css_exists(&dev) {
        return dev;
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for cand in [
                dir.join("../share/vibe-wallet"),
                dir.join("../share/vibe-vault"),
                dir.join("data"),
                dir.join("../../data"),
            ] {
                if css_exists(&cand) {
                    return cand;
                }
            }
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        for name in ["vibe-wallet", "vibe-vault", "oldgrowth-wallet"] {
            let cand = PathBuf::from(&home).join(".local/share").join(name);
            if css_exists(&cand) {
                return cand;
            }
        }
    }

    PathBuf::from("/usr/share/vibe-wallet")
}

fn css_exists(dir: &std::path::Path) -> bool {
    ["css/vibe.css", "css/vibe-vault.css", "css/oldgrowth.css"]
        .iter()
        .any(|rel| dir.join(rel).exists())
}

pub fn css_path() -> PathBuf {
    let dir = data_dir();
    for name in ["css/vibe.css", "css/vibe-vault.css", "css/oldgrowth.css"] {
        let cand = dir.join(name);
        if cand.exists() {
            return cand;
        }
    }
    dir.join("css/vibe.css")
}

pub fn icon_search_path() -> PathBuf {
    data_dir().join("icons")
}

pub fn brand_asset(rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_start_matches('/');
    let mut cands = Vec::new();
    cands.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vibe_wallet/assets/brand").join(rel),
    );
    cands.push(data_dir().join("brand").join(rel));
    if let Ok(root) = std::env::var("VIBE_BRAND_DIR") {
        cands.push(PathBuf::from(root).join(rel));
    }
    cands.into_iter().find(|p| p.is_file())
}

pub fn coin_asset(rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_start_matches('/');
    let mut cands = Vec::new();
    cands.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vibe_wallet/assets/coins").join(rel),
    );
    cands.push(data_dir().join("coins").join(rel));
    cands.into_iter().find(|p| p.is_file())
}
