//! Maps a PC keyboard onto the DEC LK401 layout.
//!
//! The default map is *positional*: keys keep the physical positions a DEC
//! user expects, as on SmarTerm and Reflection VT keyboard maps.
//!
//! | PC key                         | DEC key                              |
//! |--------------------------------|--------------------------------------|
//! | F1 F2 F3 F4 F5                 | Hold Screen, Print Screen, Set-Up, Data/Talk, Break |
//! | F6–F12                         | F6–F12                               |
//! | Shift+F1–F10                   | F11–F20 (Shift+F5 = Help, Shift+F6 = Do) |
//! | Ctrl+F5, Ctrl+Break            | Answerback                           |
//! | Insert Home PgUp               | Find, Insert Here, Remove            |
//! | Delete End PgDn                | Select, Prev Screen, Next Screen     |
//! | NumLock / * −                  | PF1 PF2 PF3 PF4                      |
//! | Keypad + (Shift: minus)        | Keypad , (−)                         |
//! | Keypad 0–9 . Enter             | Keypad 0–9 . Enter                   |
//! | Backspace                      | `<X]` (Delete)                       |
//! | Ctrl+Shift+C / Ctrl+Shift+V    | Copy / Paste                         |
//!
//! Keys are identified by X11/GDK keysym values, so this crate has no
//! toolkit dependency.

use vt_core::Key;

/// Modifier state for a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// DEC local functions: handled by the terminal itself, not sent as key codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Local {
    HoldScreen,
    PrintScreen,
    SetUp,
    /// Data/Talk on the VT420: switch sessions.
    SwitchSession,
    Break,
    Answerback,
    Copy,
    Paste,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Key(Key),
    /// Characters to type, e.g. a control character from Ctrl+letter.
    Text(String),
    Local(Local),
}

/// X11 keysym values used by the default map.
pub mod keysym {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const PAUSE: u32 = 0xff13;
    pub const ESCAPE: u32 = 0xff1b;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const PRINT: u32 = 0xff61;
    pub const INSERT: u32 = 0xff63;
    pub const BREAK: u32 = 0xff6b;
    pub const NUM_LOCK: u32 = 0xff7f;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const KP_HOME: u32 = 0xff95;
    pub const KP_LEFT: u32 = 0xff96;
    pub const KP_UP: u32 = 0xff97;
    pub const KP_RIGHT: u32 = 0xff98;
    pub const KP_DOWN: u32 = 0xff99;
    pub const KP_PAGE_UP: u32 = 0xff9a;
    pub const KP_PAGE_DOWN: u32 = 0xff9b;
    pub const KP_END: u32 = 0xff9c;
    pub const KP_BEGIN: u32 = 0xff9d;
    pub const KP_INSERT: u32 = 0xff9e;
    pub const KP_DELETE: u32 = 0xff9f;
    pub const KP_MULTIPLY: u32 = 0xffaa;
    pub const KP_ADD: u32 = 0xffab;
    pub const KP_SEPARATOR: u32 = 0xffac;
    pub const KP_SUBTRACT: u32 = 0xffad;
    pub const KP_DECIMAL: u32 = 0xffae;
    pub const KP_DIVIDE: u32 = 0xffaf;
    pub const KP_0: u32 = 0xffb0;
    pub const KP_9: u32 = 0xffb9;
    pub const F1: u32 = 0xffbe;
    pub const F20: u32 = 0xffd1;
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
    pub const DELETE: u32 = 0xffff;
}

/// Maps a key press. `ch` is the character the key produces (if any), used
/// for Ctrl combinations. Returns `None` for keys that should go through
/// normal text input (input methods, compose, dead keys).
pub fn map_key(sym: u32, ch: Option<char>, mods: Mods) -> Option<Action> {
    use Action::{Key as K, Local as L};
    use keysym::*;

    if let Some(action) = function_key(sym, mods) {
        return Some(action);
    }

    Some(match sym {
        UP => K(Key::Up),
        DOWN => K(Key::Down),
        LEFT => K(Key::Left),
        RIGHT => K(Key::Right),

        INSERT => K(Key::Find),
        HOME => K(Key::InsertHere),
        PAGE_UP => K(Key::Remove),
        DELETE => K(Key::Select),
        END => K(Key::PrevScreen),
        PAGE_DOWN => K(Key::NextScreen),

        NUM_LOCK => K(Key::Pf1),
        KP_DIVIDE => K(Key::Pf2),
        KP_MULTIPLY => K(Key::Pf3),
        KP_SUBTRACT => K(Key::Pf4),
        KP_ADD if mods.shift => K(Key::KeypadMinus),
        KP_ADD | KP_SEPARATOR => K(Key::KeypadComma),
        KP_DECIMAL | KP_DELETE => K(Key::KeypadPeriod),
        KP_ENTER => K(Key::KeypadEnter),
        KP_0..=KP_9 => K(Key::Keypad((sym - KP_0) as u8)),
        // Keypad with NumLock off reports navigation keysyms; keep them digits.
        KP_INSERT => K(Key::Keypad(0)),
        KP_END => K(Key::Keypad(1)),
        KP_DOWN => K(Key::Keypad(2)),
        KP_PAGE_DOWN => K(Key::Keypad(3)),
        KP_LEFT => K(Key::Keypad(4)),
        KP_BEGIN => K(Key::Keypad(5)),
        KP_RIGHT => K(Key::Keypad(6)),
        KP_HOME => K(Key::Keypad(7)),
        KP_UP => K(Key::Keypad(8)),
        KP_PAGE_UP => K(Key::Keypad(9)),

        RETURN => K(Key::Return),
        BACKSPACE => K(Key::Delete),
        TAB | ISO_LEFT_TAB => K(Key::Tab),
        ESCAPE => K(Key::Escape),

        PAUSE => L(Local::HoldScreen),
        PRINT => L(Local::PrintScreen),
        BREAK if mods.ctrl => L(Local::Answerback),
        BREAK => L(Local::Break),

        _ => return control_character(ch?, mods),
    })
}

fn function_key(sym: u32, mods: Mods) -> Option<Action> {
    use keysym::{F1, F20};
    if !(F1..=F20).contains(&sym) {
        return None;
    }
    let n = (sym - F1 + 1) as u8;
    Some(match (n, mods.shift, mods.ctrl) {
        (5, false, true) => Action::Local(Local::Answerback),
        (1..=10, true, _) => Action::Key(Key::Function(n + 10)),
        (1, ..) => Action::Local(Local::HoldScreen),
        (2, ..) => Action::Local(Local::PrintScreen),
        (3, ..) => Action::Local(Local::SetUp),
        (4, ..) => Action::Local(Local::SwitchSession),
        (5, ..) => Action::Local(Local::Break),
        _ => Action::Key(Key::Function(n)),
    })
}

/// Ctrl combinations on the main keypad, following the LK201: Ctrl+Space or
/// Ctrl+2 is NUL, Ctrl+3–7 are ESC FS GS RS US, Ctrl+8 is DEL.
fn control_character(ch: char, mods: Mods) -> Option<Action> {
    if !mods.ctrl || mods.alt {
        return None;
    }
    if mods.shift {
        match ch.to_ascii_lowercase() {
            'c' => return Some(Action::Local(Local::Copy)),
            'v' => return Some(Action::Local(Local::Paste)),
            _ => {}
        }
    }
    let code = match ch {
        ' ' | '2' | '@' => 0x00,
        'a'..='z' => ch as u8 - b'a' + 1,
        'A'..='Z' => ch as u8 - b'A' + 1,
        '[' | '3' => 0x1B,
        '\\' | '4' => 0x1C,
        ']' | '5' => 0x1D,
        '^' | '~' | '6' => 0x1E,
        '_' | '/' | '7' => 0x1F,
        '8' | '?' => 0x7F,
        _ => return None,
    };
    Some(Action::Text(char::from(code).to_string()))
}

#[cfg(test)]
mod tests {
    use super::keysym::*;
    use super::*;

    const NONE: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: false,
    };
    const SHIFT: Mods = Mods {
        shift: true,
        ctrl: false,
        alt: false,
    };
    const CTRL: Mods = Mods {
        shift: false,
        ctrl: true,
        alt: false,
    };

    fn key(sym: u32, mods: Mods) -> Option<Action> {
        map_key(sym, None, mods)
    }

    #[test]
    fn editing_keypad_is_positional() {
        let map: Vec<_> = [INSERT, HOME, PAGE_UP, DELETE, END, PAGE_DOWN]
            .iter()
            .map(|&s| key(s, NONE))
            .collect();
        let dec = [
            Key::Find,
            Key::InsertHere,
            Key::Remove,
            Key::Select,
            Key::PrevScreen,
            Key::NextScreen,
        ];
        assert_eq!(
            map,
            dec.iter()
                .map(|&k| Some(Action::Key(k)))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn keypad_gold_keys_and_digits() {
        assert_eq!(key(NUM_LOCK, NONE), Some(Action::Key(Key::Pf1)));
        assert_eq!(key(KP_SUBTRACT, NONE), Some(Action::Key(Key::Pf4)));
        assert_eq!(key(KP_ADD, NONE), Some(Action::Key(Key::KeypadComma)));
        assert_eq!(key(KP_ADD, SHIFT), Some(Action::Key(Key::KeypadMinus)));
        assert_eq!(key(KP_0 + 7, NONE), Some(Action::Key(Key::Keypad(7))));
        assert_eq!(
            key(KP_HOME, NONE),
            Some(Action::Key(Key::Keypad(7))),
            "NumLock off"
        );
        assert_eq!(key(KP_UP, NONE), Some(Action::Key(Key::Keypad(8))));
        assert_eq!(key(UP, NONE), Some(Action::Key(Key::Up)));
        assert_eq!(key(KP_DELETE, NONE), Some(Action::Key(Key::KeypadPeriod)));
    }

    #[test]
    fn function_keys_and_local_functions() {
        assert_eq!(key(F1 + 2, NONE), Some(Action::Local(Local::SetUp)));
        assert_eq!(key(F1 + 5, NONE), Some(Action::Key(Key::Function(6))));
        assert_eq!(
            key(F1 + 4, SHIFT),
            Some(Action::Key(Key::Function(15))),
            "Help"
        );
        assert_eq!(
            key(F1 + 5, SHIFT),
            Some(Action::Key(Key::Function(16))),
            "Do"
        );
        assert_eq!(key(F1 + 4, CTRL), Some(Action::Local(Local::Answerback)));
        assert_eq!(key(F20, NONE), Some(Action::Key(Key::Function(20))));
    }

    #[test]
    fn backspace_is_the_delete_key() {
        assert_eq!(key(BACKSPACE, NONE), Some(Action::Key(Key::Delete)));
    }

    #[test]
    fn control_characters() {
        let text = |c, m| map_key(0x61, Some(c), m);
        assert_eq!(text('a', CTRL), Some(Action::Text("\u{1}".into())));
        assert_eq!(text(' ', CTRL), Some(Action::Text("\u{0}".into())));
        assert_eq!(text('3', CTRL), Some(Action::Text("\u{1b}".into())));
        assert_eq!(text('8', CTRL), Some(Action::Text("\u{7f}".into())));
        assert_eq!(
            text(
                'c',
                Mods {
                    shift: true,
                    ..CTRL
                }
            ),
            Some(Action::Local(Local::Copy))
        );
        assert_eq!(
            text('a', NONE),
            None,
            "plain text goes through the input method"
        );
    }
}
