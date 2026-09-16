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
//! | Ctrl+F6–F12, Ctrl+Shift+F1–F10 | User-defined keys (DEC Shift+F6–F20) |
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

pub mod keymap;
pub use keymap::{Keymap, Target};

/// Modifier state for a key press.
pub use vt_core::KeyMods as Mods;

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
    /// Adds a checkpoint to the session recording.
    MarkCheckpoint,
    /// Local panning through page memory (Ctrl with ⇑ ⇓ Prev Next).
    PanUp,
    PanDown,
    PanPrevPage,
    PanNextPage,
    /// Reviews the session's history: back or forward a screen.
    ReviewBack,
    ReviewForward,
    /// Opens the find bar to search the history.
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Key(Key),
    /// A DEC key held with DEC modifiers (VT500 DECFNK sequences).
    ModifiedKey(Key, Mods),
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
    pub const SCROLL_LOCK: u32 = 0xff14;
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

/// The LK411 key station (EK-VT520-RM figure 8-4) at a PC main-keypad
/// position, from the X11/GDK hardware keycode (Linux evdev code + 8).
#[cfg(not(windows))]
pub fn station_for_keycode(keycode: u32) -> Option<u8> {
    let evdev = keycode.checked_sub(8)?;
    Some(match evdev {
        41 => 1,                    // ` ~
        2..=11 => evdev as u8,      // 1 … 0
        12 => 12,                   // - _
        13 => 13,                   // = +
        16..=25 => evdev as u8 + 1, // Q … P
        26 => 27,                   // [ {
        27 => 28,                   // ] }
        30..=38 => evdev as u8 + 1, // A … L
        39 => 40,                   // ; :
        40 => 41,                   // ' "
        43 => 42,                   // \ |
        86 => 45,                   // < > (ISO key)
        44..=50 => evdev as u8 + 2, // Z … M
        51 => 53,                   // , <
        52 => 54,                   // . >
        53 => 55,                   // / ?
        57 => 61,                   // space
        _ => return None,
    })
}

/// On Windows GDK reports virtual-key codes, which follow the characters of
/// the keyboard layout; the table assumes the US layout's positions.
// 🔎 Non-US layouts move letter virtual keys (AZERTY swaps A and Q), so
// host-programmed keys (DECPAK) may land on other keys there.
#[cfg(windows)]
pub fn station_for_keycode(keycode: u32) -> Option<u8> {
    Some(match keycode {
        0xC0 => 1,                                 // VK_OEM_3 ` ~
        0x31..=0x39 => (keycode - 0x31) as u8 + 2, // 1 … 9
        0x30 => 11,                                // 0
        0xBD => 12,                                // VK_OEM_MINUS
        0xBB => 13,                                // VK_OEM_PLUS
        0xDB => 27,                                // VK_OEM_4 [
        0xDD => 28,                                // VK_OEM_6 ]
        0xBA => 40,                                // VK_OEM_1 ;
        0xDE => 41,                                // VK_OEM_7 '
        0xDC => 42,                                // VK_OEM_5 \
        0xE2 => 45,                                // VK_OEM_102 < >
        0xBC => 53,                                // VK_OEM_COMMA
        0xBE => 54,                                // VK_OEM_PERIOD
        0xBF => 55,                                // VK_OEM_2 /
        0x20 => 61,                                // space
        0x41..=0x5A => {
            let row = "QWERTYUIOP ASDFGHJKL ZXCVBNM";
            let letter = char::from_u32(keycode)?;
            let i = row.find(letter)? as u8;
            match i {
                0..=9 => 17 + i,
                11..=19 => 31 + i - 11,
                _ => 46 + i - 21,
            }
        }
        _ => return None,
    })
}

/// Maps a key press with the built-in keymap. `ch` is the character the
/// key produces (if any), used for Ctrl combinations. Returns `None` for
/// keys that should go through normal text input (input methods, compose,
/// dead keys).
pub fn map_key(sym: u32, ch: Option<char>, mods: Mods) -> Option<Action> {
    static DEFAULT: std::sync::OnceLock<Keymap> = std::sync::OnceLock::new();
    DEFAULT.get_or_init(Keymap::default).map(sym, ch, mods)
}

/// Ctrl combinations on the main keypad, following the LK201: Ctrl+Space or
/// Ctrl+2 is NUL, Ctrl+3–7 are ESC FS GS RS US, Ctrl+8 is DEL.
pub(crate) fn control_character(ch: char, mods: Mods) -> Option<Action> {
    if !mods.ctrl || mods.alt {
        return None;
    }
    if mods.shift {
        match ch.to_ascii_lowercase() {
            'c' => return Some(Action::Local(Local::Copy)),
            'v' => return Some(Action::Local(Local::Paste)),
            'm' => return Some(Action::Local(Local::MarkCheckpoint)),
            'f' => return Some(Action::Local(Local::Search)),
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
    fn scroll_lock_is_the_do_key() {
        // A PC keyboard has no Do key and Scroll Lock has no DEC meaning,
        // so it stands in for one. Shift+F6 still works.
        assert_eq!(key(SCROLL_LOCK, NONE), Some(Action::Key(Key::Function(16))));
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
        assert_eq!(key(F1 + 5, CTRL), Some(Action::Key(Key::UserDefined(6))));
        assert_eq!(
            key(
                F1 + 5,
                Mods {
                    shift: true,
                    ..CTRL
                }
            ),
            Some(Action::Key(Key::UserDefined(16))),
            "Ctrl+Shift+F6 is the Do key's UDK"
        );
    }

    #[test]
    fn modified_editing_cursor_and_function_keys() {
        const ALT: Mods = Mods {
            shift: false,
            ctrl: false,
            alt: true,
        };
        assert_eq!(
            key(INSERT, CTRL),
            Some(Action::ModifiedKey(Key::Find, CTRL))
        );
        assert_eq!(key(LEFT, ALT), Some(Action::ModifiedKey(Key::Left, ALT)));
        assert_eq!(key(UP, CTRL), Some(Action::Local(Local::PanUp)));
        assert_eq!(
            key(PAGE_DOWN, CTRL),
            Some(Action::Local(Local::PanNextPage))
        );
        assert_eq!(
            key(F1 + 5, ALT),
            Some(Action::ModifiedKey(Key::Function(6), ALT))
        );
        assert_eq!(
            key(F1 + 1, Mods { shift: true, ..ALT }),
            Some(Action::ModifiedKey(Key::Function(12), ALT)),
            "Alt+Shift+F2 is Alt+F12"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn main_keypad_stations() {
        // evdev KEY_A = 30, KEY_Z = 44, KEY_1 = 2, KEY_SPACE = 57.
        assert_eq!(station_for_keycode(30 + 8), Some(31));
        assert_eq!(station_for_keycode(44 + 8), Some(46));
        assert_eq!(station_for_keycode(2 + 8), Some(2));
        assert_eq!(station_for_keycode(57 + 8), Some(61));
        assert_eq!(
            station_for_keycode(1 + 8),
            None,
            "Escape is not a main keypad station"
        );
    }

    #[test]
    #[cfg(windows)]
    fn main_keypad_stations() {
        // Virtual keys: A, Z, 1, space, VK_OEM_2 (/).
        assert_eq!(station_for_keycode(0x41), Some(31));
        assert_eq!(station_for_keycode(0x5A), Some(46));
        assert_eq!(station_for_keycode(0x4D), Some(52));
        assert_eq!(station_for_keycode(0x50), Some(26));
        assert_eq!(station_for_keycode(0x31), Some(2));
        assert_eq!(station_for_keycode(0x20), Some(61));
        assert_eq!(station_for_keycode(0xBF), Some(55));
        assert_eq!(
            station_for_keycode(0x1B),
            None,
            "Escape is not a main keypad station"
        );
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
