//! User-defined keys (DECUDK): strings the host assigns to shifted F6–F20.

/// Total definition memory of a VT510; earlier terminals had less.
pub const CAPACITY: usize = 804;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserKeys {
    /// Definitions for F6–F20, indexed by function key number − 6.
    keys: [Vec<u8>; 15],
    /// Locked keys cannot be redefined by the host until unlocked in Set-Up.
    pub locked: bool,
}

/// Maps a DECUDK key selector to a function key number (6–20).
fn key_number(selector: u16) -> Option<u8> {
    Some(match selector {
        17..=21 => selector - 11,
        23..=26 => selector - 12,
        28..=29 => selector - 13,
        31..=34 => selector - 14,
        _ => return None,
    } as u8)
}

impl UserKeys {
    pub fn new(locked: bool) -> UserKeys {
        UserKeys {
            locked,
            ..UserKeys::default()
        }
    }

    pub fn get(&self, function_key: u8) -> Option<&[u8]> {
        let index = usize::from(function_key.checked_sub(6)?);
        self.keys
            .get(index)
            .map(Vec::as_slice)
            .filter(|k| !k.is_empty())
    }

    pub fn used(&self) -> usize {
        self.keys.iter().map(Vec::len).sum()
    }

    pub fn clear(&mut self) {
        self.keys = Default::default();
    }

    /// Applies a DECUDK string (`Kyn/hex;Kyn/hex…`). `clear_all` is Pc = 0.
    /// Loading stops at the first malformed definition; earlier ones stay.
    /// Returns `false` if the keys are locked and nothing was changed.
    pub fn load(&mut self, clear_all: bool, data: &[u8]) -> bool {
        if self.locked {
            return false;
        }
        if clear_all {
            self.clear();
        }
        for definition in data.split(|&b| b == b';') {
            let Some(slash) = definition.iter().position(|&b| b == b'/') else {
                if definition.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                break;
            };
            let selector = std::str::from_utf8(&definition[..slash])
                .ok()
                .and_then(|s| s.trim().parse::<u16>().ok());
            let Some(index) = selector.and_then(key_number).map(|k| usize::from(k - 6)) else {
                break;
            };
            let hex = &definition[slash + 1..];
            let Some(bytes) = decode_hex(hex) else {
                break;
            };
            let others = self.used() - self.keys[index].len();
            if others + bytes.len() > CAPACITY {
                break;
            }
            self.keys[index] = bytes;
        }
        true
    }
}

fn decode_hex(hex: &[u8]) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    hex.chunks(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(s, 16).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_clears() {
        let mut k = UserKeys::default();
        assert!(k.load(true, b"17/4449520D;34/5052494E54"));
        assert_eq!(k.get(6), Some(&b"DIR\r"[..]));
        assert_eq!(k.get(20), Some(&b"PRINT"[..]));
        k.load(false, b"18/41");
        assert_eq!(k.get(6), Some(&b"DIR\r"[..]), "Pc=1 keeps other keys");
        k.load(true, b"18/42");
        assert_eq!(k.get(6), None, "Pc=0 clears all keys first");
        assert_eq!(k.get(7), Some(&b"B"[..]));
    }

    #[test]
    fn malformed_definition_stops_loading() {
        let mut k = UserKeys::default();
        k.load(true, b"17/41;18/4G;19/43");
        assert_eq!(k.get(6), Some(&b"A"[..]));
        assert_eq!(k.get(7), None);
        assert_eq!(k.get(8), None);
    }

    #[test]
    fn locked_keys_ignore_host() {
        let mut k = UserKeys::new(true);
        assert!(!k.load(true, b"17/41"));
        assert_eq!(k.get(6), None);
    }

    #[test]
    fn selector_numbers() {
        assert_eq!(key_number(17), Some(6));
        assert_eq!(key_number(24), Some(12));
        assert_eq!(key_number(28), Some(15));
        assert_eq!(key_number(29), Some(16));
        assert_eq!(key_number(34), Some(20));
        assert_eq!(key_number(22), None);
    }
}
