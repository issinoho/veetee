//! Keymaps: PC key combinations bound to DEC keys and local functions,
//! loaded from and saved to TOML.

use serde::{Deserialize, Serialize};
use vt_core::Key;

use crate::keysym;
use crate::{Action, Local, Mods};

/// The built-in PC-to-LK401 keymap.
pub const DEFAULT_TOML: &str = include_str!("default.toml");

/// What a PC key produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Key(Key),
    Local(Local),
}

/// A PC key with the modifiers that must be held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcKey {
    pub sym: u32,
    pub mods: Mods,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub pc: PcKey,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub name: String,
    pub bindings: Vec<Binding>,
}

#[derive(Serialize, Deserialize)]
struct KeymapFile {
    name: String,
    #[serde(default)]
    bind: Vec<BindingFile>,
}

#[derive(Serialize, Deserialize)]
struct BindingFile {
    pc: String,
    dec: String,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::from_toml(DEFAULT_TOML).expect("built-in keymap is valid")
    }
}

impl Keymap {
    pub fn from_toml(text: &str) -> Result<Keymap, String> {
        let file: KeymapFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let bindings = file
            .bind
            .iter()
            .map(|b| {
                let pc = parse_pc_key(&b.pc).ok_or_else(|| format!("unknown PC key {:?}", b.pc))?;
                let target =
                    parse_target(&b.dec).ok_or_else(|| format!("unknown DEC key {:?}", b.dec))?;
                Ok(Binding { pc, target })
            })
            .collect::<Result<_, String>>()?;
        Ok(Keymap {
            name: file.name,
            bindings,
        })
    }

    pub fn to_toml(&self) -> String {
        let file = KeymapFile {
            name: self.name.clone(),
            bind: self
                .bindings
                .iter()
                .map(|b| BindingFile {
                    pc: pc_key_name(b.pc),
                    dec: target_name(b.target).to_string(),
                })
                .collect(),
        };
        let body = toml::to_string_pretty(&file).unwrap_or_default();
        format!("# veetee keymap: PC keys to DEC LK401 keys.\n\n{body}")
    }

    /// The binding a key press selects: DEC keys match when their modifiers
    /// are held (extra modifiers pass on), local functions only on an exact
    /// match. The binding requiring the most modifiers wins.
    pub fn lookup(&self, sym: u32, mods: Mods) -> Option<(Binding, Mods)> {
        let sym = normalize(sym);
        self.bindings
            .iter()
            .filter(|b| b.pc.sym == sym)
            .filter(|b| match b.target {
                Target::Local(_) => b.pc.mods == mods,
                Target::Key(_) => contains(mods, b.pc.mods),
            })
            .max_by_key(|b| count(b.pc.mods))
            .map(|b| (*b, subtract(mods, b.pc.mods)))
    }

    /// Maps a key press to an action.
    pub fn map(&self, sym: u32, ch: Option<char>, mods: Mods) -> Option<Action> {
        match self.lookup(sym, mods) {
            Some((binding, extra)) => Some(match binding.target {
                Target::Local(local) => Action::Local(local),
                Target::Key(key) if extra == Mods::NONE => Action::Key(key),
                Target::Key(key) => Action::ModifiedKey(key, extra),
            }),
            None => crate::control_character(ch?, mods),
        }
    }

    /// The bindings that produce `target`.
    pub fn bindings_for(&self, target: Target) -> impl Iterator<Item = &Binding> {
        self.bindings.iter().filter(move |b| b.target == target)
    }

    /// Binds `pc` to `target`, replacing whatever `pc` did before.
    pub fn bind(&mut self, pc: PcKey, target: Target) {
        let pc = PcKey {
            sym: normalize(pc.sym),
            ..pc
        };
        self.bindings.retain(|b| b.pc != pc);
        self.bindings.push(Binding { pc, target });
    }

    /// Removes a PC key's binding.
    pub fn unbind(&mut self, pc: PcKey) {
        self.bindings.retain(|b| b.pc != pc);
    }
}

fn normalize(sym: u32) -> u32 {
    // Letters are bound in lower case; Shift is a modifier.
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&sym) {
        sym + 32
    } else {
        sym
    }
}

fn contains(held: Mods, required: Mods) -> bool {
    (!required.shift || held.shift) && (!required.ctrl || held.ctrl) && (!required.alt || held.alt)
}

fn subtract(held: Mods, required: Mods) -> Mods {
    Mods {
        shift: held.shift && !required.shift,
        ctrl: held.ctrl && !required.ctrl,
        alt: held.alt && !required.alt,
    }
}

fn count(m: Mods) -> u8 {
    u8::from(m.shift) + u8::from(m.ctrl) + u8::from(m.alt)
}

/// GDK key names veetee understands, beyond single characters.
const KEY_NAMES: &[(&str, u32)] = &[
    ("BackSpace", keysym::BACKSPACE),
    ("Tab", keysym::TAB),
    ("ISO_Left_Tab", keysym::ISO_LEFT_TAB),
    ("Return", keysym::RETURN),
    ("Pause", keysym::PAUSE),
    ("Scroll_Lock", keysym::SCROLL_LOCK),
    ("Escape", keysym::ESCAPE),
    ("Home", keysym::HOME),
    ("Left", keysym::LEFT),
    ("Up", keysym::UP),
    ("Right", keysym::RIGHT),
    ("Down", keysym::DOWN),
    ("Page_Up", keysym::PAGE_UP),
    ("Page_Down", keysym::PAGE_DOWN),
    ("End", keysym::END),
    ("Print", keysym::PRINT),
    ("Insert", keysym::INSERT),
    ("Break", keysym::BREAK),
    ("Num_Lock", keysym::NUM_LOCK),
    ("KP_Enter", keysym::KP_ENTER),
    ("KP_Home", keysym::KP_HOME),
    ("KP_Left", keysym::KP_LEFT),
    ("KP_Up", keysym::KP_UP),
    ("KP_Right", keysym::KP_RIGHT),
    ("KP_Down", keysym::KP_DOWN),
    ("KP_Page_Up", keysym::KP_PAGE_UP),
    ("KP_Page_Down", keysym::KP_PAGE_DOWN),
    ("KP_End", keysym::KP_END),
    ("KP_Begin", keysym::KP_BEGIN),
    ("KP_Insert", keysym::KP_INSERT),
    ("KP_Delete", keysym::KP_DELETE),
    ("KP_Multiply", keysym::KP_MULTIPLY),
    ("KP_Add", keysym::KP_ADD),
    ("KP_Separator", keysym::KP_SEPARATOR),
    ("KP_Subtract", keysym::KP_SUBTRACT),
    ("KP_Decimal", keysym::KP_DECIMAL),
    ("KP_Divide", keysym::KP_DIVIDE),
    ("Delete", keysym::DELETE),
    ("space", 0x20),
];

/// The GDK name of a key symbol, if veetee knows it.
pub fn key_name(sym: u32) -> Option<String> {
    if let Some((name, _)) = KEY_NAMES.iter().find(|(_, s)| *s == sym) {
        return Some((*name).to_string());
    }
    match sym {
        keysym::KP_0..=keysym::KP_9 => Some(format!("KP_{}", sym - keysym::KP_0)),
        keysym::F1..=keysym::F20 => Some(format!("F{}", sym - keysym::F1 + 1)),
        0x21..=0x7E => char::from_u32(sym).map(|c| c.to_string()),
        _ => None,
    }
}

fn key_sym(name: &str) -> Option<u32> {
    if let Some((_, sym)) = KEY_NAMES.iter().find(|(n, _)| *n == name) {
        return Some(*sym);
    }
    if let Some(n) = name.strip_prefix("KP_").and_then(|d| d.parse::<u32>().ok()) {
        return (n <= 9).then_some(keysym::KP_0 + n);
    }
    if let Some(n) = name.strip_prefix('F').and_then(|d| d.parse::<u32>().ok()) {
        return (1..=20).contains(&n).then_some(keysym::F1 + n - 1);
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_graphic() => Some(normalize(u32::from(c))),
        _ => None,
    }
}

pub fn parse_pc_key(text: &str) -> Option<PcKey> {
    let mut mods = Mods::NONE;
    let mut rest = text;
    loop {
        if let Some(r) = rest.strip_prefix("Ctrl+") {
            mods.ctrl = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("Shift+") {
            mods.shift = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("Alt+") {
            mods.alt = true;
            rest = r;
        } else {
            break;
        }
    }
    Some(PcKey {
        sym: key_sym(rest)?,
        mods,
    })
}

pub fn pc_key_name(pc: PcKey) -> String {
    let mut s = String::new();
    if pc.mods.ctrl {
        s.push_str("Ctrl+");
    }
    if pc.mods.shift {
        s.push_str("Shift+");
    }
    if pc.mods.alt {
        s.push_str("Alt+");
    }
    s.push_str(&key_name(pc.sym).unwrap_or_else(|| format!("0x{:x}", pc.sym)));
    s
}

const LOCAL_NAMES: &[(&str, Local)] = &[
    ("hold-screen", Local::HoldScreen),
    ("print-screen", Local::PrintScreen),
    ("set-up", Local::SetUp),
    ("session", Local::SwitchSession),
    ("break", Local::Break),
    ("answerback", Local::Answerback),
    ("copy", Local::Copy),
    ("paste", Local::Paste),
    ("mark-checkpoint", Local::MarkCheckpoint),
    ("pan-up", Local::PanUp),
    ("pan-down", Local::PanDown),
    ("pan-prev-page", Local::PanPrevPage),
    ("pan-next-page", Local::PanNextPage),
    ("review-back", Local::ReviewBack),
    ("review-forward", Local::ReviewForward),
    ("search", Local::Search),
];

const KEY_TARGETS: &[(&str, Key)] = &[
    ("return", Key::Return),
    ("delete", Key::Delete),
    ("tab", Key::Tab),
    ("linefeed", Key::LineFeed),
    ("escape", Key::Escape),
    ("backspace", Key::Backspace),
    ("up", Key::Up),
    ("down", Key::Down),
    ("left", Key::Left),
    ("right", Key::Right),
    ("find", Key::Find),
    ("insert-here", Key::InsertHere),
    ("remove", Key::Remove),
    ("select", Key::Select),
    ("prev-screen", Key::PrevScreen),
    ("next-screen", Key::NextScreen),
    ("pf1", Key::Pf1),
    ("pf2", Key::Pf2),
    ("pf3", Key::Pf3),
    ("pf4", Key::Pf4),
    ("kp-minus", Key::KeypadMinus),
    ("kp-comma", Key::KeypadComma),
    ("kp-period", Key::KeypadPeriod),
    ("kp-enter", Key::KeypadEnter),
    ("help", Key::Function(15)),
    ("do", Key::Function(16)),
];

pub fn parse_target(name: &str) -> Option<Target> {
    if let Some((_, local)) = LOCAL_NAMES.iter().find(|(n, _)| *n == name) {
        return Some(Target::Local(*local));
    }
    if let Some((_, key)) = KEY_TARGETS.iter().find(|(n, _)| *n == name) {
        return Some(Target::Key(*key));
    }
    let number = |prefix: &str, range: std::ops::RangeInclusive<u8>| {
        name.strip_prefix(prefix)
            .and_then(|d| d.parse::<u8>().ok())
            .filter(|n| range.contains(n))
    };
    if let Some(d) = number("kp", 0..=9) {
        return Some(Target::Key(Key::Keypad(d)));
    }
    if let Some(n) = number("udk", 6..=20) {
        return Some(Target::Key(Key::UserDefined(n)));
    }
    if let Some(n) = number("f", 1..=20) {
        return Some(Target::Key(Key::Function(n)));
    }
    None
}

pub fn target_name(target: Target) -> String {
    match target {
        Target::Local(local) => LOCAL_NAMES
            .iter()
            .find(|(_, l)| *l == local)
            .map_or("", |(n, _)| n)
            .to_string(),
        Target::Key(key) => {
            if let Some((name, _)) = KEY_TARGETS.iter().find(|(_, k)| *k == key) {
                return (*name).to_string();
            }
            match key {
                Key::Keypad(d) => format!("kp{d}"),
                Key::UserDefined(n) => format!("udk{n}"),
                Key::Function(n) => format!("f{n}"),
                _ => String::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keymap_round_trips() {
        let map = Keymap::default();
        assert!(map.bindings.len() > 90);
        let again = Keymap::from_toml(&map.to_toml()).unwrap();
        assert_eq!(again, map);
    }

    #[test]
    fn pc_key_names() {
        let pc = parse_pc_key("Ctrl+Shift+F6").unwrap();
        assert_eq!(
            (pc.sym, pc.mods.ctrl, pc.mods.shift),
            (keysym::F1 + 5, true, true)
        );
        assert_eq!(pc_key_name(pc), "Ctrl+Shift+F6");
        assert_eq!(parse_pc_key("KP_7").unwrap().sym, keysym::KP_0 + 7);
        assert_eq!(parse_pc_key("Ctrl+Shift+C").unwrap().sym, u32::from(b'c'));
        assert!(parse_pc_key("Hyper+F1").is_none());
    }

    #[test]
    fn rebinding() {
        let mut map = Keymap::default();
        let f12 = parse_pc_key("F12").unwrap();
        map.bind(f12, Target::Key(Key::Function(16)));
        assert_eq!(
            map.map(keysym::F1 + 11, None, Mods::NONE),
            Some(Action::Key(Key::Function(16)))
        );
        map.unbind(f12);
        assert_eq!(map.map(keysym::F1 + 11, None, Mods::NONE), None);
        let toml = map.to_toml();
        assert!(!toml.contains("pc = \"F12\""));
    }

    #[test]
    fn bad_files_are_reported() {
        assert!(
            Keymap::from_toml("name = \"x\"\n[[bind]]\npc = \"F1\"\ndec = \"warp\"\n")
                .unwrap_err()
                .contains("warp")
        );
    }
}
