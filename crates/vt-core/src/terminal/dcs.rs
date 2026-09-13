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
            | DcsState::AssignSupplemental { data, .. } => data,
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
        }
    }
}
