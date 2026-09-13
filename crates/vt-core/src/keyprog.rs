//! VT520 key programming (EK-VT520-RM chapter 5 and 8): keys identified by
//! LK411 key-station number can be programmed by the host to perform a
//! local function or send a user-defined sequence (DECPFK), alphanumeric
//! keys to send other codes (DECPAK), copied from another key's default
//! (DECCKD), locked or restored (DECPKA), and reported (DECRQKD).

use std::collections::BTreeMap;

use crate::keyboard::{Key, KeyMods};

/// Memory for programmed keys (DECPKFMR reports it as 768 bytes).
pub const MEMORY: usize = 768;

/// LK411 key-station numbers of the DEC keys veetee models
/// (EK-VT520-RM figure 8-4, VT layout).
pub const STATIONS: &[(u8, Key)] = &[
    (15, Key::Delete),
    (16, Key::Tab),
    (43, Key::Return),
    (75, Key::Find),
    (80, Key::InsertHere),
    (85, Key::Remove),
    (76, Key::Select),
    (81, Key::PrevScreen),
    (86, Key::NextScreen),
    (83, Key::Up),
    (79, Key::Left),
    (84, Key::Down),
    (89, Key::Right),
    (90, Key::Pf1),
    (95, Key::Pf2),
    (100, Key::Pf3),
    (105, Key::Pf4),
    (91, Key::Keypad(7)),
    (96, Key::Keypad(8)),
    (101, Key::Keypad(9)),
    (106, Key::KeypadMinus),
    (92, Key::Keypad(4)),
    (97, Key::Keypad(5)),
    (102, Key::Keypad(6)),
    (107, Key::KeypadComma),
    (93, Key::Keypad(1)),
    (98, Key::Keypad(2)),
    (103, Key::Keypad(3)),
    (108, Key::KeypadEnter),
    (99, Key::Keypad(0)),
    (104, Key::KeypadPeriod),
    (112, Key::Function(1)),
    (113, Key::Function(2)),
    (114, Key::Function(3)),
    (115, Key::Function(4)),
    (116, Key::Function(5)),
    (117, Key::Function(6)),
    (118, Key::Function(7)),
    (119, Key::Function(8)),
    (120, Key::Function(9)),
    (121, Key::Function(10)),
    (122, Key::Function(11)),
    (123, Key::Function(12)),
    (124, Key::Function(13)),
    (125, Key::Function(14)),
    (126, Key::Function(15)),
    (127, Key::Function(16)),
    (130, Key::Function(17)),
    (131, Key::Function(18)),
    (132, Key::Function(19)),
    (133, Key::Function(20)),
];

/// The station number of a DEC key; shifted F6–F20 are their function key.
pub fn station_of(key: Key) -> Option<u8> {
    let key = match key {
        Key::UserDefined(n) => Key::Function(n),
        k => k,
    };
    STATIONS.iter().find(|(_, k)| *k == key).map(|(s, _)| *s)
}

pub fn key_at(station: u8) -> Option<Key> {
    STATIONS
        .iter()
        .find(|(s, _)| *s == station)
        .map(|(_, k)| *k)
}

/// Main key array stations (typewriter keys), which DECPAK programs.
pub fn is_alphanumeric(station: u8) -> bool {
    matches!(station, 1..=13 | 17..=28 | 31..=42 | 45..=55 | 61)
}

/// The DECPFK/DECRQKD modifier number: 1 none, 2 Shift, 3 Alt, 4 Alt+Shift,
/// 5 Ctrl, 6 Ctrl+Shift, 7 Alt+Ctrl, 8 all.
pub fn modifier_number(mods: KeyMods) -> u8 {
    mods.decfnk()
}

/// Where a user-defined sequence goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// The host in full duplex, the screen in local mode.
    Normal,
    /// The screen only.
    Local,
    /// The host only.
    Remote,
}

impl Direction {
    fn code(self) -> u8 {
        match self {
            Direction::Normal => 0,
            Direction::Local => 1,
            Direction::Remote => 2,
        }
    }
}

/// A programmed action: a local function (0 = none, 100 = send `uds`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub function: u16,
    pub uds: Vec<u8>,
    pub direction: Direction,
}

/// Local function number 100: send a user-defined sequence.
pub const SEND_SEQUENCE: u16 = 100;

/// An alphanumeric key's DECPAK definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alphanumeric {
    /// Codes for the seven modifier states: unshifted, shifted, alternate
    /// shifted, group 2 unshifted, shifted, alternate shifted, Control.
    pub codes: [Option<Vec<u8>>; 7],
    /// What Alt with the key does.
    pub alt: Option<Program>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyPrograms {
    functions: BTreeMap<(u8, u8), Program>,
    alphanumeric: BTreeMap<u8, Alphanumeric>,
    /// DECCKD: destination station → source station.
    copies: BTreeMap<u8, u8>,
    pub locked: bool,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn unhex(text: &[u8]) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    text.chunks(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
        .collect()
}

fn number(field: &[u8]) -> Option<u16> {
    if field.is_empty() {
        return Some(0);
    }
    std::str::from_utf8(field).ok()?.parse().ok()
}

impl KeyPrograms {
    pub fn used(&self) -> usize {
        let program = |p: &Program| 4 + p.uds.len();
        self.functions.values().map(program).sum::<usize>()
            + self
                .alphanumeric
                .values()
                .map(|a| {
                    2 + a.codes.iter().flatten().map(Vec::len).sum::<usize>()
                        + a.alt.as_ref().map_or(0, program)
                })
                .sum::<usize>()
            + 2 * self.copies.len()
    }

    pub fn free(&self) -> usize {
        MEMORY.saturating_sub(self.used())
    }

    fn fits(&self) -> bool {
        self.used() <= MEMORY
    }

    /// Removes every non-default definition of `station`.
    fn clear_station(&mut self, station: u8) {
        self.functions.retain(|(s, _), _| *s != station);
        self.alphanumeric.remove(&station);
        self.copies.remove(&station);
    }

    /// DECPFK: `Key/Mod/Function/UDS/Direction;…`. A malformed definition
    /// ends processing; earlier ones stay. Returns false when locked.
    pub fn program_function_keys(&mut self, data: &[u8]) -> bool {
        if self.locked {
            return false;
        }
        for definition in data.split(|&b| b == b';').filter(|d| !d.is_empty()) {
            let fields: Vec<&[u8]> = definition.split(|&b| b == b'/').collect();
            let field = |i: usize| fields.get(i).copied().unwrap_or(b"");
            let (Some(station), Some(modifier), Some(function)) =
                (number(field(0)), number(field(1)), number(field(2)))
            else {
                return true;
            };
            let Ok(station) = u8::try_from(station) else {
                return true;
            };
            // Break (F5) cannot be programmed from the host.
            if key_at(station).is_none() || station == 116 || modifier > 8 {
                return true;
            }
            let Some(uds) = unhex(field(3)) else {
                return true;
            };
            let direction = match number(field(4)) {
                Some(0) => Direction::Normal,
                Some(1) => Direction::Local,
                Some(2) => Direction::Remote,
                _ => return true,
            };
            if uds.len() > 255 {
                return true;
            }
            let modifier = modifier.max(1) as u8;
            let previous = self.functions.insert(
                (station, modifier),
                Program {
                    function,
                    uds,
                    direction,
                },
            );
            if !self.fits() {
                match previous {
                    Some(p) => self.functions.insert((station, modifier), p),
                    None => self.functions.remove(&(station, modifier)),
                };
                return true;
            }
        }
        true
    }

    /// DECPAK: `Key/HexCodes/Function/UDS/Direction;…`. HexCodes gives the
    /// codes for the seven modifier states, `.` for a state left undefined;
    /// the function, sequence and direction apply to Alt with the key.
    pub fn program_alphanumeric_keys(&mut self, data: &[u8]) -> bool {
        if self.locked {
            return false;
        }
        for definition in data.split(|&b| b == b';').filter(|d| !d.is_empty()) {
            let fields: Vec<&[u8]> = definition.split(|&b| b == b'/').collect();
            let field = |i: usize| fields.get(i).copied().unwrap_or(b"");
            let Some(station) = number(field(0)).and_then(|s| u8::try_from(s).ok()) else {
                return true;
            };
            if !is_alphanumeric(station) {
                return true;
            }
            let Some(codes) = parse_codes(field(1)) else {
                return true;
            };
            let alt = if fields.len() > 2 {
                let (Some(function), Some(uds)) = (number(field(2)), unhex(field(3))) else {
                    return true;
                };
                let direction = match number(field(4)) {
                    Some(1) => Direction::Local,
                    Some(2) => Direction::Remote,
                    _ => Direction::Normal,
                };
                Some(Program {
                    function,
                    uds,
                    direction,
                })
            } else {
                None
            };
            let previous = self
                .alphanumeric
                .insert(station, Alphanumeric { codes, alt });
            if !self.fits() {
                match previous {
                    Some(p) => self.alphanumeric.insert(station, p),
                    None => self.alphanumeric.remove(&station),
                };
                return true;
            }
        }
        true
    }

    /// DECCKD: `Ks/Kd;…` copies key Ks's default to Kd; Ks = Kd restores Kd.
    pub fn copy_key_defaults(&mut self, data: &[u8]) -> bool {
        if self.locked {
            return false;
        }
        for pair in data.split(|&b| b == b';').filter(|d| !d.is_empty()) {
            let fields: Vec<&[u8]> = pair.split(|&b| b == b'/').collect();
            let (Some(source), Some(dest)) = (
                fields
                    .first()
                    .and_then(|f| number(f))
                    .and_then(|s| u8::try_from(s).ok()),
                fields
                    .get(1)
                    .and_then(|f| number(f))
                    .and_then(|s| u8::try_from(s).ok()),
            ) else {
                return true;
            };
            if dest == 116 {
                continue;
            }
            self.clear_station(dest);
            if source != dest {
                self.copies.insert(dest, source);
                if !self.fits() {
                    self.copies.remove(&dest);
                    return true;
                }
            }
        }
        true
    }

    /// DECPKA: 1 locks, 2 restores factory defaults, 3 recalls saved
    /// definitions (the same as 2: veetee has no NVR).
    pub fn key_action(&mut self, action: u16) {
        match action {
            1 => self.locked = true,
            2 | 3 if !self.locked => {
                self.functions.clear();
                self.alphanumeric.clear();
                self.copies.clear();
            }
            _ => {}
        }
    }

    /// What a DEC key does: a program, if the host set one.
    pub fn function(&self, station: u8, mods: KeyMods) -> Option<&Program> {
        self.functions.get(&(station, modifier_number(mods)))
    }

    /// The key whose default a station uses (DECCKD), or the station itself.
    pub fn default_source(&self, station: u8) -> u8 {
        self.copies.get(&station).copied().unwrap_or(station)
    }

    pub fn alphanumeric(&self, station: u8) -> Option<&Alphanumeric> {
        self.alphanumeric.get(&station)
    }

    /// DECRPFK data for a function key with a modifier. `default` is the
    /// sequence the key sends when not programmed.
    pub fn report_function(&self, station: u8, modifier: u8, default: &[u8]) -> String {
        match self.functions.get(&(station, modifier.max(1))) {
            Some(p) => format!(
                "{station}/{}/{}/{}/{}",
                modifier.max(1),
                p.function,
                hex(&p.uds),
                p.direction.code()
            ),
            None => format!("{station}/{}//{}/0", modifier.max(1), hex(default)),
        }
    }

    /// DECRPAK data for an alphanumeric key.
    pub fn report_alphanumeric(&self, station: u8) -> String {
        match self.alphanumeric.get(&station) {
            Some(a) => {
                let codes: String = a
                    .codes
                    .iter()
                    .map(|c| c.as_ref().map_or(".".to_string(), |c| hex(c)))
                    .collect::<Vec<_>>()
                    .join(" ");
                match &a.alt {
                    Some(p) => format!(
                        "{station}/{codes}/{}/{}/{}",
                        p.function,
                        hex(&p.uds),
                        p.direction.code()
                    ),
                    None => format!("{station}/{codes}"),
                }
            }
            None => format!("{station}/. . . . . . ."),
        }
    }
}

/// Seven modifier-state codes: hex pairs separated by spaces, `.` for a
/// state left undefined.
// 🔎 RM520 describes the fields but gives no example of their separators.
fn parse_codes(field: &[u8]) -> Option<[Option<Vec<u8>>; 7]> {
    let mut codes: [Option<Vec<u8>>; 7] = Default::default();
    let text = std::str::from_utf8(field).ok()?;
    let items: Vec<&str> = if text.contains(' ') || text.contains(',') {
        text.split([' ', ',']).filter(|s| !s.is_empty()).collect()
    } else {
        // Undelimited: one hex pair or `.` per state.
        let mut out = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(r) = rest.strip_prefix('.') {
                out.push(".");
                rest = r;
            } else if rest.len() >= 2 {
                out.push(&rest[..2]);
                rest = &rest[2..];
            } else {
                return None;
            }
        }
        out
    };
    if items.len() > 7 {
        return None;
    }
    for (slot, item) in codes.iter_mut().zip(items) {
        let item = item.trim_start_matches('-');
        if item != "." {
            *slot = Some(unhex(item.as_bytes())?);
        }
    }
    Some(codes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stations_round_trip() {
        assert_eq!(station_of(Key::Find), Some(75));
        assert_eq!(station_of(Key::UserDefined(6)), Some(117));
        assert_eq!(key_at(126), Some(Key::Function(15)));
        assert!(is_alphanumeric(31) && !is_alphanumeric(75));
    }

    #[test]
    fn decpfk_programs_and_reports() {
        let mut p = KeyPrograms::default();
        p.program_function_keys(b"117/1/100/48656C6C6F/2;75/5/1//0");
        let f6 = p.function(117, KeyMods::NONE).unwrap();
        assert_eq!(
            (f6.function, f6.uds.as_slice(), f6.direction),
            (100, &b"Hello"[..], Direction::Remote)
        );
        let ctrl = KeyMods {
            ctrl: true,
            ..KeyMods::NONE
        };
        assert_eq!(p.function(75, ctrl).unwrap().function, 1);
        assert_eq!(p.report_function(117, 1, b""), "117/1/100/48656C6C6F/2");
        assert_eq!(
            p.report_function(118, 0, b"\x1b[18~"),
            "118/1//1B5B31387E/0"
        );
        assert_eq!(p.free(), MEMORY - 9 - 4);
    }

    #[test]
    fn break_cannot_be_programmed_and_bad_data_stops() {
        let mut p = KeyPrograms::default();
        p.program_function_keys(b"116/1/0//0;117/1/100/GG/0;118/1/0//0");
        assert!(p.function(116, KeyMods::NONE).is_none());
        assert!(
            p.function(118, KeyMods::NONE).is_none(),
            "processing stopped at the bad hex"
        );
    }

    #[test]
    fn decpak_codes_and_alt_function() {
        let mut p = KeyPrograms::default();
        p.program_alphanumeric_keys(b"31/61 41 . . . . 01/100/1B4F50/0");
        let a = p.alphanumeric(31).unwrap();
        assert_eq!(a.codes[0].as_deref(), Some(&b"a"[..]));
        assert_eq!(a.codes[2], None);
        assert_eq!(a.codes[6].as_deref(), Some(&b"\x01"[..]));
        assert_eq!(a.alt.as_ref().unwrap().uds, b"\x1bOP");
        assert_eq!(
            p.report_alphanumeric(31),
            "31/61 41 . . . . 01/100/1B4F50/0"
        );
        assert_eq!(p.report_alphanumeric(32), "32/. . . . . . .");
    }

    #[test]
    fn deckd_and_decpka() {
        let mut p = KeyPrograms::default();
        p.program_function_keys(b"117/1/0//0");
        p.copy_key_defaults(b"118/117");
        assert!(
            p.function(117, KeyMods::NONE).is_none(),
            "copy replaces the program"
        );
        assert_eq!(p.default_source(117), 118);
        p.copy_key_defaults(b"117/117");
        assert_eq!(p.default_source(117), 117);
        p.program_function_keys(b"117/1/0//0");
        p.key_action(1);
        assert!(!p.program_function_keys(b"118/1/0//0"));
        p.key_action(2);
        assert!(
            p.function(117, KeyMods::NONE).is_some(),
            "restore is refused while locked"
        );
    }

    #[test]
    fn memory_is_limited() {
        let mut p = KeyPrograms::default();
        let long = "41".repeat(255);
        for station in 117..=121 {
            p.program_function_keys(format!("{station}/1/100/{long}/0").as_bytes());
        }
        assert!(p.used() <= MEMORY);
        assert!(p.function(121, KeyMods::NONE).is_none());
    }
}
