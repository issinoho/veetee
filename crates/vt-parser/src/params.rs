use core::fmt;

/// Maximum number of parameters retained for one control sequence.
///
/// DEC STD 070 requires at least 16; extra parameters are discarded (the
/// sequence is still dispatched) and [`Params::truncated`] is set.
pub const MAX_PARAMS: usize = 32;

/// Numeric parameters of a CSI or DCS sequence.
///
/// Each parameter is either *omitted* (no digits, e.g. both positions in
/// `CSI ; H`) or a value saturated to `u16::MAX`. Parameters introduced by a
/// colon (ECMA-48 sub-parameters, e.g. `CSI 38:2:1:2:3 m`) are flagged so the
/// emulator can treat them differently from `;`-separated ones.
#[derive(Clone, PartialEq, Eq)]
pub struct Params {
    values: [u16; MAX_PARAMS],
    present: u32,
    sub: u32,
    len: u8,
    truncated: bool,
}

/// One parameter as yielded by [`Params::iter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    /// `None` when the parameter was omitted.
    pub value: Option<u16>,
    /// `true` when this parameter was preceded by `:` rather than `;`.
    pub is_subparam: bool,
}

impl Params {
    pub(crate) const fn new() -> Self {
        Params {
            values: [0; MAX_PARAMS],
            present: 0,
            sub: 0,
            len: 0,
            truncated: false,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.present = 0;
        self.sub = 0;
        self.len = 0;
        self.truncated = false;
    }

    pub(crate) fn push_digit(&mut self, digit: u8) {
        if self.len == 0 {
            self.len = 1;
            self.values[0] = 0;
        }
        if self.truncated {
            return;
        }
        let i = usize::from(self.len - 1);
        self.values[i] = self.values[i]
            .saturating_mul(10)
            .saturating_add(u16::from(digit));
        self.present |= 1 << i;
    }

    pub(crate) fn separator(&mut self, is_subparam: bool) {
        if self.len == 0 {
            self.len = 1;
            self.values[0] = 0;
        }
        if usize::from(self.len) == MAX_PARAMS {
            self.truncated = true;
            return;
        }
        let i = usize::from(self.len);
        self.values[i] = 0;
        if is_subparam {
            self.sub |= 1 << i;
        }
        self.len += 1;
    }

    /// Number of parameter positions, including omitted ones.
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `true` if more than [`MAX_PARAMS`] parameters were received.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Value at `index`, or `None` if omitted or out of range.
    pub fn get(&self, index: usize) -> Option<u16> {
        (index < self.len() && self.present & (1 << index) != 0).then(|| self.values[index])
    }

    /// Value at `index`, or `default` if omitted or out of range.
    pub fn get_or(&self, index: usize, default: u16) -> u16 {
        self.get(index).unwrap_or(default)
    }

    /// DEC "selective parameter" default: omitted *or zero* yields `default`.
    /// This is the rule for cursor movement counts, CUP coordinates, etc.
    pub fn get_nonzero_or(&self, index: usize, default: u16) -> u16 {
        match self.get(index) {
            None | Some(0) => default,
            Some(v) => v,
        }
    }

    /// `true` if the parameter at `index` was introduced by `:`.
    pub fn is_subparam(&self, index: usize) -> bool {
        index < self.len() && self.sub & (1 << index) != 0
    }

    /// `true` if any parameter was introduced by `:`.
    pub fn has_subparams(&self) -> bool {
        self.sub != 0
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = Param> + '_ {
        (0..self.len()).map(|i| Param {
            value: self.get(i),
            is_subparam: self.is_subparam(i),
        })
    }
}

impl Default for Params {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Params {
    /// Formats as the wire syntax, e.g. `5;;38:2`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, p) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(if p.is_subparam { ":" } else { ";" })?;
            }
            if let Some(v) = p.value {
                write!(f, "{v}")?;
            }
        }
        if self.truncated {
            f.write_str("…")?;
        }
        Ok(())
    }
}
