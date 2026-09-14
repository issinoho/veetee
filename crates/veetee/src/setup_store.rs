//! Saved Set-Up settings: the terminal's "nonvolatile memory", one file per
//! model and session in the user's configuration directory.

use std::io;
use std::path::PathBuf;

use gtk::glib;
use vt_core::setup::Features;
use vt_core::{Config, Model};

/// `$XDG_CONFIG_HOME/veetee/setup-vt420-session1.conf`, or the Windows
/// equivalent.
pub fn path(model: Model, session: u8) -> PathBuf {
    glib::user_config_dir().join("veetee").join(format!(
        "setup-{}-session{session}.conf",
        model.term_name_exact()
    ))
}

/// Applies the saved settings for `session`, if any, as power-up settings.
pub fn load_into(config: &mut Config, session: u8) {
    let Ok(text) = std::fs::read_to_string(path(config.model, session)) else {
        return;
    };
    let features = Features::from_text(config.model, &text);
    // Open the connection at the saved page size.
    config.cols = if features.columns_132 { 132 } else { 80 };
    config.rows = features.page_length;
    config.set_saved_features(features);
}

pub fn save(model: Model, session: u8, features: &Features) -> io::Result<PathBuf> {
    let path = path(model, session);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = format!(
        "# veetee saved Set-Up for the {} model, session {session}.\n# Written by Save in the Set-Up Directory (F3).\n{}",
        model.setup_name(),
        features.to_text()
    );
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Forgets the saved settings (Set-Up Directory "Default").
pub fn remove(model: Model, session: u8) -> io::Result<()> {
    match std::fs::remove_file(path(model, session)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
