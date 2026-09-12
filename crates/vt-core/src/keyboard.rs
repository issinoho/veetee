//! Codes sent by DEC keyboard keys, per the VT100, VT220 and VT420
//! programmer references. Physical keyboard mapping (PC → LK401) lives in
//! the frontend; this module only knows DEC logical keys.

/// A DEC logical key (LK201/LK401 layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Return,
    /// The `<X]` key. Sends DEL unless DECBKM selects BS.
    Delete,
    Tab,
    /// VT100 Line Feed key (F13 in VT100 mode on later keyboards).
    LineFeed,
    /// VT100 Escape key (F11 in VT100 mode on later keyboards).
    Escape,
    /// VT100 Back Space key (F12 in VT100 mode on later keyboards).
    Backspace,
    Up,
    Down,
    Right,
    Left,
    // LK201 editing keypad (VT220 and later).
    Find,
    InsertHere,
    Remove,
    Select,
    PrevScreen,
    NextScreen,
    // Numeric keypad.
    Pf1,
    Pf2,
    Pf3,
    Pf4,
    /// Keypad digit 0–9.
    Keypad(u8),
    KeypadMinus,
    KeypadComma,
    KeypadPeriod,
    KeypadEnter,
    /// Top-row function key F6–F20 (F15 is Help, F16 is Do). F1–F5 are local
    /// functions (Hold Screen, Print Screen, Set-Up, Data/Talk, Break) and
    /// are handled by the frontend.
    Function(u8),
}

/// Terminal state that affects the codes a key sends.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyContext {
    pub ansi: bool,
    pub level: u8,
    pub eight_bit: bool,
    pub cursor_app: bool,
    pub keypad_app: bool,
    pub new_line: bool,
    pub backarrow_bs: bool,
}

impl KeyContext {
    fn csi(&self, out: &mut Vec<u8>) {
        if self.eight_bit {
            out.push(0x9B);
        } else {
            out.extend_from_slice(b"\x1b[");
        }
    }

    fn ss3(&self, out: &mut Vec<u8>) {
        if self.eight_bit {
            out.push(0x8F);
        } else {
            out.extend_from_slice(b"\x1bO");
        }
    }
}

/// Appends the codes for `key` to `out`. Keys that send nothing in the
/// current mode leave `out` unchanged.
pub(crate) fn encode(key: Key, cx: KeyContext, out: &mut Vec<u8>) {
    use Key::*;
    match key {
        Return => {
            out.push(b'\r');
            if cx.new_line {
                out.push(b'\n');
            }
        }
        Delete => out.push(if cx.backarrow_bs { 0x08 } else { 0x7F }),
        Tab => out.push(b'\t'),
        LineFeed => out.push(b'\n'),
        Escape => out.push(0x1B),
        Backspace => out.push(0x08),

        Up | Down | Right | Left => {
            let code = match key {
                Up => b'A',
                Down => b'B',
                Right => b'C',
                _ => b'D',
            };
            if !cx.ansi {
                out.extend_from_slice(&[0x1B, code]);
            } else if cx.cursor_app {
                cx.ss3(out);
                out.push(code);
            } else {
                cx.csi(out);
                out.push(code);
            }
        }

        Pf1 | Pf2 | Pf3 | Pf4 => {
            let code = match key {
                Pf1 => b'P',
                Pf2 => b'Q',
                Pf3 => b'R',
                _ => b'S',
            };
            if cx.ansi {
                cx.ss3(out);
            } else {
                out.push(0x1B);
            }
            out.push(code);
        }

        Keypad(_) | KeypadMinus | KeypadComma | KeypadPeriod | KeypadEnter => {
            if !cx.keypad_app {
                match key {
                    Keypad(d) => out.push(b'0' + d.min(9)),
                    KeypadMinus => out.push(b'-'),
                    KeypadComma => out.push(b','),
                    KeypadPeriod => out.push(b'.'),
                    _ => encode(Return, cx, out),
                }
                return;
            }
            let code = match key {
                Keypad(d) => b'p' + d.min(9),
                KeypadMinus => b'm',
                KeypadComma => b'l',
                KeypadPeriod => b'n',
                _ => b'M',
            };
            if cx.ansi {
                cx.ss3(out);
            } else {
                out.extend_from_slice(b"\x1b?");
            }
            out.push(code);
        }

        Find | InsertHere | Remove | Select | PrevScreen | NextScreen => {
            if cx.level < 2 || !cx.ansi {
                return;
            }
            let n = match key {
                Find => 1,
                InsertHere => 2,
                Remove => 3,
                Select => 4,
                PrevScreen => 5,
                _ => 6,
            };
            cx.csi(out);
            out.extend_from_slice(format!("{n}~").as_bytes());
        }

        Function(f) => {
            if cx.level < 2 || !cx.ansi {
                // VT100 mode: only F11–F13 send codes (ESC, BS, LF).
                match f {
                    11 => out.push(0x1B),
                    12 => out.push(0x08),
                    13 => out.push(b'\n'),
                    _ => {}
                }
                return;
            }
            let n = match f {
                6..=10 => f + 11,
                11..=14 => f + 12,
                15 | 16 => f + 13,
                17..=20 => f + 14,
                _ => return,
            };
            cx.csi(out);
            out.extend_from_slice(format!("{n}~").as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VT420: KeyContext = KeyContext {
        ansi: true,
        level: 4,
        eight_bit: false,
        cursor_app: false,
        keypad_app: false,
        new_line: false,
        backarrow_bs: false,
    };

    fn enc(key: Key, cx: KeyContext) -> Vec<u8> {
        let mut out = Vec::new();
        encode(key, cx, &mut out);
        out
    }

    #[test]
    fn cursor_keys_by_mode() {
        assert_eq!(enc(Key::Up, VT420), b"\x1b[A");
        assert_eq!(
            enc(
                Key::Up,
                KeyContext {
                    cursor_app: true,
                    ..VT420
                }
            ),
            b"\x1bOA"
        );
        assert_eq!(
            enc(
                Key::Up,
                KeyContext {
                    cursor_app: true,
                    eight_bit: true,
                    ..VT420
                }
            ),
            b"\x8fA"
        );
        assert_eq!(
            enc(
                Key::Left,
                KeyContext {
                    ansi: false,
                    ..VT420
                }
            ),
            b"\x1bD"
        );
    }

    #[test]
    fn keypad_by_mode() {
        assert_eq!(enc(Key::Keypad(7), VT420), b"7");
        assert_eq!(
            enc(
                Key::KeypadEnter,
                KeyContext {
                    new_line: true,
                    ..VT420
                }
            ),
            b"\r\n"
        );
        let app = KeyContext {
            keypad_app: true,
            ..VT420
        };
        assert_eq!(enc(Key::Keypad(7), app), b"\x1bOw");
        assert_eq!(enc(Key::KeypadComma, app), b"\x1bOl");
        assert_eq!(enc(Key::KeypadEnter, app), b"\x1bOM");
        assert_eq!(
            enc(Key::KeypadPeriod, KeyContext { ansi: false, ..app }),
            b"\x1b?n"
        );
        assert_eq!(enc(Key::Pf1, VT420), b"\x1bOP");
        assert_eq!(
            enc(
                Key::Pf4,
                KeyContext {
                    ansi: false,
                    ..VT420
                }
            ),
            b"\x1bS"
        );
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(enc(Key::Find, VT420), b"\x1b[1~");
        assert_eq!(enc(Key::NextScreen, VT420), b"\x1b[6~");
        assert_eq!(enc(Key::Function(6), VT420), b"\x1b[17~");
        assert_eq!(enc(Key::Function(11), VT420), b"\x1b[23~");
        assert_eq!(enc(Key::Function(15), VT420), b"\x1b[28~");
        assert_eq!(enc(Key::Function(16), VT420), b"\x1b[29~");
        assert_eq!(
            enc(
                Key::Function(20),
                KeyContext {
                    eight_bit: true,
                    ..VT420
                }
            ),
            b"\x9b34~"
        );
        let vt100 = KeyContext { level: 1, ..VT420 };
        assert_eq!(enc(Key::Function(11), vt100), b"\x1b");
        assert!(enc(Key::Function(6), vt100).is_empty());
        assert!(enc(Key::Find, vt100).is_empty());
    }

    #[test]
    fn delete_and_return() {
        assert_eq!(enc(Key::Delete, VT420), b"\x7f");
        assert_eq!(
            enc(
                Key::Delete,
                KeyContext {
                    backarrow_bs: true,
                    ..VT420
                }
            ),
            b"\x08"
        );
        assert_eq!(enc(Key::Return, VT420), b"\r");
        assert_eq!(
            enc(
                Key::Return,
                KeyContext {
                    new_line: true,
                    ..VT420
                }
            ),
            b"\r\n"
        );
    }
}
