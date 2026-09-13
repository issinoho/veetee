//! Device control strings: DECUDK, DECDLD, DECRQSS, DECRSPS, DECAUPSS.

use vt_parser::Sequence;

use super::Emulator;
use crate::softfont::DldParams;

/// Longest DCS payload kept; longer strings are truncated (DEC terminals
/// have far smaller buffers, so real hosts never approach this).
const LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub(super) enum DcsState {
    None,
    Udk {
        clear_all: bool,
        lock: bool,
        data: Vec<u8>,
    },
    Dld {
        params: DldParams,
        data: Vec<u8>,
    },
    RequestSetting(Vec<u8>),
    RestorePresentation {
        kind: u16,
        data: Vec<u8>,
    },
    AssignSupplemental {
        size: u16,
        data: Vec<u8>,
    },
    DefineMacro {
        id: u16,
        delete: u16,
        hex: bool,
        data: Vec<u8>,
    },
    RestoreTerminalState {
        data: Vec<u8>,
    },
    RestoreColorTable {
        data: Vec<u8>,
    },
    LoadAnswerback {
        encoding: u16,
        data: Vec<u8>,
    },
    /// DECPFK (`x`), DECPAK (`y`) or DECCKD (`z`).
    ProgramKeys {
        kind: u8,
        data: Vec<u8>,
    },
    LoadBanner {
        encoding: u16,
        data: Vec<u8>,
    },
}

impl DcsState {
    pub(super) fn hook(level: u8, seq: &Sequence<'_>) -> DcsState {
        let p = seq.params;
        let data = Vec::new();
        match (seq.private, seq.intermediates, seq.final_byte) {
            (None, [], b'|') if level >= 2 => DcsState::Udk {
                clear_all: p.get_or(0, 0) == 0,
                lock: p.get_or(1, 0) == 0,
                data,
            },
            (None, [], b'{') if level >= 2 => DcsState::Dld {
                params: DldParams {
                    font_number: p.get_or(0, 0),
                    start: p.get_or(1, 0),
                    erase: p.get_or(2, 0),
                    width: p.get_or(3, 0),
                    font_set_size: p.get_or(4, 0),
                    text_or_full_cell: p.get_or(5, 0),
                    height: p.get_or(6, 0),
                    is_96: p.get_or(7, 0) == 1,
                },
                data,
            },
            (None, [b'$'], b'q') if level >= 3 => DcsState::RequestSetting(data),
            (None, [b'$'], b't') if level >= 3 => DcsState::RestorePresentation {
                kind: p.get_or(0, 0),
                data,
            },
            (None, [b'!'], b'u') if level >= 3 => DcsState::AssignSupplemental {
                size: p.get_or(0, 0),
                data,
            },
            (None, [b'!'], b'z') if level >= 4 => match (p.get_or(1, 0), p.get_or(2, 0)) {
                (delete @ 0..=1, pen @ 0..=1) => DcsState::DefineMacro {
                    id: p.get_or(0, 0),
                    delete,
                    hex: pen == 1,
                    data,
                },
                _ => DcsState::None,
            },
            (None, [b'$'], b'p') if level >= 4 && p.get_or(0, 0) == 1 => {
                DcsState::RestoreTerminalState { data }
            }
            (None, [b'$'], b'p') if level >= 5 && p.get_or(0, 0) == 2 => {
                DcsState::RestoreColorTable { data }
            }
            (None, [b'"'], kind @ (b'x' | b'y' | b'z')) if level >= 1 => {
                DcsState::ProgramKeys { kind, data }
            }
            (None, [], b'v') if level >= 5 => DcsState::LoadAnswerback {
                encoding: p.get_or(0, 0),
                data,
            },
            (None, [], b'r') if level >= 5 => DcsState::LoadBanner {
                encoding: p.get_or(0, 0),
                data,
            },
            _ => DcsState::None,
        }
    }

    pub(super) fn put(&mut self, byte: u8) {
        let data = match self {
            DcsState::None => return,
            DcsState::Udk { data, .. }
            | DcsState::Dld { data, .. }
            | DcsState::RequestSetting(data)
            | DcsState::RestorePresentation { data, .. }
            | DcsState::AssignSupplemental { data, .. }
            | DcsState::DefineMacro { data, .. }
            | DcsState::RestoreTerminalState { data }
            | DcsState::RestoreColorTable { data }
            | DcsState::LoadAnswerback { data, .. }
            | DcsState::ProgramKeys { data, .. }
            | DcsState::LoadBanner { data, .. } => data,
        };
        if data.len() < LIMIT {
            data.push(byte);
        }
    }
}

impl Emulator {
    pub(super) fn finish_dcs(&mut self, state: DcsState) {
        match state {
            DcsState::None => {}
            DcsState::Udk {
                clear_all,
                lock,
                data,
            } => {
                if self.udk.load(clear_all, &data) && lock {
                    self.udk.locked = true;
                }
            }
            DcsState::Dld { params, data } => {
                if self.soft.load(params, &data, self.level).is_ok() {
                    self.soft_generation += 1;
                }
            }
            DcsState::RequestSetting(data) => self.request_setting(&data),
            DcsState::RestorePresentation { kind, data } => {
                self.restore_presentation_state(kind, &data)
            }
            DcsState::AssignSupplemental { size, data } => self.assign_supplemental(size, &data),
            DcsState::DefineMacro {
                id,
                delete,
                hex,
                data,
            } => self.define_macro(id, delete, hex, &data),
            DcsState::RestoreTerminalState { data } => self.restore_terminal_state(&data),
            DcsState::RestoreColorTable { data } if self.color_terminal() => {
                self.restore_color_table(&data)
            }
            DcsState::RestoreColorTable { .. } => {}
            DcsState::LoadAnswerback { encoding, data } => self.load_answerback(encoding, &data),
            DcsState::ProgramKeys { kind, data } if self.config.model.max_level() >= 5 => {
                match kind {
                    b'x' => self.keyprog.program_function_keys(&data),
                    b'y' => self.keyprog.program_alphanumeric_keys(&data),
                    _ => self.keyprog.copy_key_defaults(&data),
                };
            }
            DcsState::ProgramKeys { .. } => {}
            DcsState::LoadBanner { encoding, data } => self.load_banner(encoding, &data),
        }
    }

    /// DECDMAC. Invalid definitions are ignored entirely.
    fn define_macro(&mut self, id: u16, delete: u16, hex: bool, data: &[u8]) {
        let Some(slot) = Some(usize::from(id)).filter(|i| *i < self.macros.len()) else {
            return;
        };
        let body = if hex {
            match decode_macro_hex(data) {
                Some(b) => b,
                None => return,
            }
        } else {
            // Format effectors (BS–CR) may lay out the string but are not part of it.
            data.iter().copied().filter(|b| *b >= 0x20).collect()
        };
        if delete == 1 {
            self.macros.iter_mut().for_each(Vec::clear);
        } else {
            self.macros[slot].clear();
        }
        let used: usize = self.macros.iter().map(Vec::len).sum();
        if used + body.len() <= MACRO_MEMORY {
            self.macros[slot] = body;
        }
    }

    /// DECINVM: the macro text is processed as though received from the host.
    pub(super) fn invoke_macro(&mut self, id: u16) {
        if let Some(body) = self.macros.get(usize::from(id)).filter(|m| !m.is_empty()) {
            self.pending_input.extend_from_slice(body);
            self.pause = true;
        }
    }

    pub(super) fn macro_space(&self) -> usize {
        MACRO_MEMORY - self.macros.iter().map(Vec::len).sum::<usize>()
    }

    pub(super) fn macro_checksum(&self) -> u32 {
        let sum: u32 = self.macros.iter().flatten().map(|&b| u32::from(b)).sum();
        sum.wrapping_neg() & 0xFFFF
    }
}

/// The VT420 has 6 KB for macro definitions.
pub(super) const MACRO_MEMORY: usize = 6 * 1024;

/// Hex pairs with optional repeat groups `!Pn;hex…;`.
fn decode_macro_hex(data: &[u8]) -> Option<Vec<u8>> {
    let digits: Vec<u8> = data
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    let pair =
        |s: &[u8]| -> Option<u8> { u8::from_str_radix(std::str::from_utf8(s).ok()?, 16).ok() };
    while i < digits.len() {
        if digits[i] == b'!' {
            let semi = digits[i + 1..].iter().position(|&b| b == b';')? + i + 1;
            let count_text = std::str::from_utf8(&digits[i + 1..semi]).ok()?;
            let count = if count_text.is_empty() {
                1
            } else {
                count_text.parse::<usize>().ok()?
            };
            let end = digits[semi + 1..]
                .iter()
                .position(|&b| b == b';')
                .map_or(digits.len(), |e| e + semi + 1);
            let group = &digits[semi + 1..end];
            if group.len() % 2 != 0 {
                return None;
            }
            let bytes: Vec<u8> = group.chunks(2).map(pair).collect::<Option<_>>()?;
            for _ in 0..count.min(MACRO_MEMORY) {
                out.extend_from_slice(&bytes);
            }
            i = end + 1;
        } else {
            out.push(pair(digits.get(i..i + 2)?)?);
            i += 2;
        }
    }
    Some(out)
}
