//! The user's display preferences that are not terminal Set-Up: CRT effects
//! and the visible bell, kept in `$XDG_CONFIG_HOME/veetee/appearance.conf`.

use std::path::PathBuf;

use gtk::glib;
use vt_render::Effects;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Appearance {
    pub effects: Effects,
    /// Flash "Bell" on the status line when the bell sounds.
    pub visible_bell: bool,
    /// Logs started from the window menu stamp each line.
    pub log_timestamps: bool,
}

fn path() -> PathBuf {
    glib::user_config_dir()
        .join("veetee")
        .join("appearance.conf")
}

pub fn load() -> Appearance {
    let mut a = Appearance::default();
    let Ok(text) = std::fs::read_to_string(path()) else {
        return a;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let on = value.trim() == "1";
        match key.trim() {
            "glow" => a.effects.glow = on,
            "afterglow" => a.effects.afterglow = on,
            "curvature" => a.effects.curvature = on,
            "visible-bell" => a.visible_bell = on,
            "log-timestamps" => a.log_timestamps = on,
            _ => {}
        }
    }
    a
}

pub fn save(a: &Appearance) {
    let b = |v: bool| u8::from(v);
    let text = format!(
        "# veetee display preferences\nglow={}\nafterglow={}\ncurvature={}\nvisible-bell={}\nlog-timestamps={}\n",
        b(a.effects.glow),
        b(a.effects.afterglow),
        b(a.effects.curvature),
        b(a.visible_bell),
        b(a.log_timestamps)
    );
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, text) {
        eprintln!("veetee: cannot save {}: {e}", path.display());
    }
}
