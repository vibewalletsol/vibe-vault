//! Prefer system Adwaita Sans 12. Register bundled Inter as a fallback.
//! GTK4 CSS does not support `@font-face`.

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};

#[link(name = "fontconfig")]
extern "C" {
    fn FcConfigGetCurrent() -> *mut c_void;
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const c_char) -> c_int;
}

const FALLBACK: &[&str] = &[
    "Inter-Regular.ttf",
    "Inter-Medium.ttf",
    "Inter-SemiBold.ttf",
    "Inter-Bold.ttf",
];

const ADWAITA_DIRS: &[&str] = &[
    "/usr/share/fonts/adwaita",
    "/usr/share/fonts/opentype/adwaita",
    "/usr/share/fonts/truetype/adwaita",
    "/usr/share/fonts/Adwaita",
];

pub fn register_bundled_fonts() {
    unsafe {
        let config = FcConfigGetCurrent();
        if config.is_null() {
            eprintln!("vibe-vault: fontconfig has no current config");
            return;
        }
        for dir in ADWAITA_DIRS {
            add_dir(config, Path::new(dir));
        }
        let bundled = crate::paths::data_dir().join("fonts");
        for file in FALLBACK {
            add_file(config, &bundled.join(file));
        }
    }
}

unsafe fn add_dir(config: *mut c_void, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("AdwaitaSans") || name.starts_with("AdwaitaMono")
        })
        .collect();
    paths.sort();
    for path in paths {
        add_file(config, &path);
    }
}

unsafe fn add_file(config: *mut c_void, path: &Path) {
    if !path.exists() {
        return;
    }
    let Some(s) = path.to_str() else { return };
    let Ok(c) = CString::new(s) else { return };
    let ok = FcConfigAppFontAddFile(config, c.as_ptr());
    if ok == 0 {
        eprintln!("vibe-vault: could not register font {}", path.display());
    }
}
