//! Loading and saving the user's keymap.

use std::path::{Path, PathBuf};

use gtk::glib;
use vt_keyboard::Keymap;

/// Where the keymap editor saves: `$XDG_CONFIG_HOME/veetee/keymap.toml`.
pub fn user_path() -> PathBuf {
    glib::user_config_dir().join("veetee").join("keymap.toml")
}

/// The keymap from `path`, else the saved keymap, else the built-in one.
/// A file that cannot be read is reported and the built-in map is used.
pub fn load(path: Option<&Path>) -> Keymap {
    let path = path.map(Path::to_path_buf).or_else(|| {
        let saved = user_path();
        saved.exists().then_some(saved)
    });
    let Some(path) = path else {
        return Keymap::default();
    };
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|text| Keymap::from_toml(&text))
    {
        Ok(map) => map,
        Err(e) => {
            eprintln!(
                "veetee: keymap {}: {e}; using the built-in keymap",
                path.display()
            );
            Keymap::default()
        }
    }
}

/// Saves `keymap` as the user's keymap.
pub fn save(keymap: &Keymap) -> std::io::Result<PathBuf> {
    let path = user_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, keymap.to_toml())?;
    Ok(path)
}
