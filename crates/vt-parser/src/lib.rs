//! Streaming parser for DEC VT and ECMA-48 control functions.
//!
//! This is the state machine from DEC STD 070 as described by Paul Williams
//! (<https://vt100.net/emu/dec_ansi_parser>), extended with:
//!
//! * selectable 7-bit, 8-bit and UTF-8 input (see [`InputMode`]),
//! * VT52 mode, including the two-byte `ESC Y` cursor address,
//! * ECMA-48 colon sub-parameters,
//! * string terminators reported as normal or aborted ([`StringEnd`]).
//!
//! The parser never allocates. String payloads (DCS, OSC, SOS/PM/APC) are
//! streamed byte-by-byte to the [`Perform`] implementation, which decides how
//! much to buffer.

#![no_std]
#![forbid(unsafe_code)]

mod params;

pub use params::{MAX_PARAMS, Param, Params};

/// Maximum number of intermediate bytes. Sequences with more are ignored.
pub const MAX_INTERMEDIATES: usize = 2;

/// How incoming bytes are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    /// 7-bit line: bit 8 is stripped from every byte.
    SevenBit,
    /// 8-bit line: 0x80–0x9F are C1 controls, 0xA0–0xFF are GR graphics.
    #[default]
    EightBit,
    /// 8-bit line where 0x80–0x9F are not recognised as controls and are
    /// discarded (GR graphics still print).
    EightBitNoC1,
    /// UTF-8: multi-byte sequences are decoded in the ground state. Encoded
    /// U+0080–U+009F act as C1 controls. Malformed input yields U+FFFD.
    Utf8,
}

/// How a DCS, OSC or SOS/PM/APC string ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringEnd {
    /// ST (`ESC \` or 0x9C), BEL for OSC, or any `ESC` that begins a new sequence.
    Terminated,
    /// CAN, SUB or an interrupting C1 control. DEC terminals discard the string.
    Aborted,
}

/// Which kind of opaque string is being received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringKind {
    Sos,
    Pm,
    Apc,
}

/// Header of a CSI or DCS sequence.
#[derive(Debug, Clone, Copy)]
pub struct Sequence<'a> {
    /// Private parameter marker (`<`, `=`, `>` or `?`) if the sequence began with one.
    pub private: Option<u8>,
    pub params: &'a Params,
    /// Intermediate bytes (0x20–0x2F), at most [`MAX_INTERMEDIATES`].
    pub intermediates: &'a [u8],
    pub final_byte: u8,
}

/// Receiver for parsed actions. Every method has a no-op default.
#[allow(unused_variables)]
pub trait Perform {
    /// A graphic byte from GL (0x20–0x7F) or GR (0xA0–0xFF). In UTF-8 mode
    /// only ASCII bytes arrive here, so charset designations still apply.
    fn print(&mut self, byte: u8) {}

    /// A run of consecutive graphic bytes. Override for speed; the default
    /// forwards each byte to [`Perform::print`].
    fn print_run(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.print(b);
        }
    }

    /// A non-ASCII character decoded in [`InputMode::Utf8`].
    fn print_char(&mut self, ch: char) {}

    /// A C0 control (0x00–0x1F) or an executable C1 control (0x80–0x9F).
    fn execute(&mut self, byte: u8) {}

    fn esc_dispatch(&mut self, intermediates: &[u8], final_byte: u8) {}

    fn csi_dispatch(&mut self, seq: Sequence<'_>) {}

    /// Start of a DCS string. Data follows through [`Perform::dcs_put`].
    fn dcs_hook(&mut self, seq: Sequence<'_>) {}
    fn dcs_put(&mut self, byte: u8) {}
    fn dcs_unhook(&mut self, end: StringEnd) {}

    fn osc_start(&mut self) {}
    fn osc_put(&mut self, byte: u8) {}
    fn osc_end(&mut self, end: StringEnd) {}

    fn string_start(&mut self, kind: StringKind) {}
    fn string_put(&mut self, byte: u8) {}
    fn string_end(&mut self, end: StringEnd) {}

    /// VT52 `ESC Y line column`, both zero-based.
    fn vt52_cursor(&mut self, line: u8, column: u8) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    OscString,
    SosPmApcString,
    Vt52Line,
    Vt52Column,
}

#[derive(Debug, Clone, Copy, Default)]
struct Utf8Decoder {
    needed: u8,
    codepoint: u32,
    lower: u8,
    upper: u8,
}

/// The control-function parser. See the crate documentation.
#[derive(Debug, Clone)]
pub struct Parser {
    state: State,
    mode: InputMode,
    vt52: bool,
    osc_bel_terminates: bool,
    params: Params,
    private: Option<u8>,
    intermediates: [u8; MAX_INTERMEDIATES],
    intermediate_len: u8,
    ignoring: bool,
    vt52_line: u8,
    utf8: Utf8Decoder,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub const fn new() -> Self {
        Parser {
            state: State::Ground,
            mode: InputMode::EightBit,
            vt52: false,
            osc_bel_terminates: true,
            params: Params::new(),
            private: None,
            intermediates: [0; MAX_INTERMEDIATES],
            intermediate_len: 0,
            ignoring: false,
            vt52_line: 0,
            utf8: Utf8Decoder {
                needed: 0,
                codepoint: 0,
                lower: 0,
                upper: 0,
            },
        }
    }

    pub fn input_mode(&self) -> InputMode {
        self.mode
    }

    pub fn set_input_mode(&mut self, mode: InputMode) {
        self.mode = mode;
        self.utf8 = Utf8Decoder::default();
    }

    pub fn vt52(&self) -> bool {
        self.vt52
    }

    /// Switches between ANSI (DECANM set) and VT52 escape parsing.
    pub fn set_vt52(&mut self, vt52: bool) {
        self.vt52 = vt52;
    }

    /// Whether BEL ends an OSC string (xterm convention). Default `true`.
    pub fn set_osc_bel_terminates(&mut self, yes: bool) {
        self.osc_bel_terminates = yes;
    }

    /// `true` when no sequence or string is in progress.
    pub fn is_ground(&self) -> bool {
        self.state == State::Ground && self.utf8.needed == 0
    }

    /// Returns to the ground state without emitting anything.
    pub fn reset(&mut self) {
        self.state = State::Ground;
        self.utf8 = Utf8Decoder::default();
        self.clear();
    }

    /// Feeds a buffer. Output is identical however the input is chunked.
    pub fn advance<P: Perform + ?Sized>(&mut self, performer: &mut P, bytes: &[u8]) {
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground && self.utf8.needed == 0 {
                let start = i;
                while i < bytes.len() && self.is_run_byte(bytes[i]) {
                    i += 1;
                }
                if i > start {
                    performer.print_run(&bytes[start..i]);
                    continue;
                }
            }
            self.advance_byte(performer, bytes[i]);
            i += 1;
        }
    }

    /// Feeds a single byte.
    pub fn advance_byte<P: Perform + ?Sized>(&mut self, p: &mut P, byte: u8) {
        match self.mode {
            InputMode::SevenBit => self.step(p, byte & 0x7F),
            InputMode::Utf8 if self.state == State::Ground => {
                if byte >= 0x80 || self.utf8.needed > 0 {
                    self.utf8_step(p, byte);
                } else {
                    self.step(p, byte);
                }
            }
            InputMode::Utf8 if byte >= 0x80 => self.high_non_control(p, byte),
            _ => self.step(p, byte),
        }
    }

    fn is_run_byte(&self, b: u8) -> bool {
        match self.mode {
            InputMode::SevenBit | InputMode::Utf8 => (0x20..=0x7F).contains(&b),
            InputMode::EightBit | InputMode::EightBitNoC1 => {
                (0x20..=0x7F).contains(&b) || b >= 0xA0
            }
        }
    }

    fn clear(&mut self) {
        self.params.clear();
        self.private = None;
        self.intermediate_len = 0;
        self.ignoring = false;
    }

    fn collect(&mut self, b: u8) {
        if usize::from(self.intermediate_len) < MAX_INTERMEDIATES {
            self.intermediates[usize::from(self.intermediate_len)] = b;
            self.intermediate_len += 1;
        } else {
            self.ignoring = true;
        }
    }

    fn param(&mut self, c: u8) {
        match c {
            b'0'..=b'9' => self.params.push_digit(c - b'0'),
            b';' => self.params.separator(false),
            _ => self.params.separator(true),
        }
    }

    fn sequence(&self, final_byte: u8) -> Sequence<'_> {
        Sequence {
            private: self.private,
            params: &self.params,
            intermediates: &self.intermediates[..usize::from(self.intermediate_len)],
            final_byte,
        }
    }

    /// Runs the exit action of the current state, then enters `next` and runs its entry action.
    fn transition<P: Perform + ?Sized>(&mut self, p: &mut P, next: State, end: StringEnd) {
        match self.state {
            State::OscString => p.osc_end(end),
            State::DcsPassthrough => p.dcs_unhook(end),
            State::SosPmApcString => p.string_end(end),
            _ => {}
        }
        self.state = next;
        match next {
            State::Escape | State::CsiEntry | State::DcsEntry => self.clear(),
            State::OscString => p.osc_start(),
            _ => {}
        }
    }

    fn start_string<P: Perform + ?Sized>(&mut self, p: &mut P, kind: StringKind, end: StringEnd) {
        self.transition(p, State::SosPmApcString, end);
        p.string_start(kind);
    }

    /// Handles an 8-bit C1 control, whether received raw or decoded from UTF-8.
    fn c1<P: Perform + ?Sized>(&mut self, p: &mut P, b: u8) {
        use StringEnd::Aborted;
        match b {
            0x90 => self.transition(p, State::DcsEntry, Aborted),
            0x9B => self.transition(p, State::CsiEntry, Aborted),
            0x9C => self.transition(p, State::Ground, StringEnd::Terminated),
            0x9D => self.transition(p, State::OscString, Aborted),
            0x98 => self.start_string(p, StringKind::Sos, Aborted),
            0x9E => self.start_string(p, StringKind::Pm, Aborted),
            0x9F => self.start_string(p, StringKind::Apc, Aborted),
            _ => {
                self.transition(p, State::Ground, Aborted);
                p.execute(b);
            }
        }
    }

    /// A byte ≥ 0x80 that is not a control in the current mode.
    fn high_non_control<P: Perform + ?Sized>(&mut self, p: &mut P, b: u8) {
        match self.state {
            State::DcsPassthrough => p.dcs_put(b),
            State::OscString => p.osc_put(b),
            State::SosPmApcString => p.string_put(b),
            _ => {}
        }
    }

    fn utf8_step<P: Perform + ?Sized>(&mut self, p: &mut P, b: u8) {
        let d = &mut self.utf8;
        if d.needed == 0 {
            let (needed, bits, lower, upper) = match b {
                0xC2..=0xDF => (1, b & 0x1F, 0x80, 0xBF),
                0xE0 => (2, 0, 0xA0, 0xBF),
                0xE1..=0xEC | 0xEE..=0xEF => (2, b & 0x0F, 0x80, 0xBF),
                0xED => (2, 0x0D, 0x80, 0x9F),
                0xF0 => (3, 0, 0x90, 0xBF),
                0xF1..=0xF3 => (3, b & 0x07, 0x80, 0xBF),
                0xF4 => (3, 0x04, 0x80, 0x8F),
                _ => return p.print_char(char::REPLACEMENT_CHARACTER),
            };
            *d = Utf8Decoder {
                needed,
                codepoint: u32::from(bits),
                lower,
                upper,
            };
        } else if (d.lower..=d.upper).contains(&b) {
            d.codepoint = (d.codepoint << 6) | u32::from(b & 0x3F);
            d.lower = 0x80;
            d.upper = 0xBF;
            d.needed -= 1;
            if d.needed == 0 {
                match d.codepoint {
                    cp @ 0x80..=0x9F => self.c1(p, cp as u8),
                    cp => p.print_char(char::from_u32(cp).unwrap_or(char::REPLACEMENT_CHARACTER)),
                }
            }
        } else {
            *d = Utf8Decoder::default();
            p.print_char(char::REPLACEMENT_CHARACTER);
            // Reprocess the offending byte from a clean state.
            self.advance_byte(p, b);
        }
    }

    fn step<P: Perform + ?Sized>(&mut self, p: &mut P, b: u8) {
        // "Anywhere" transitions.
        match b {
            0x18 | 0x1A => {
                self.transition(p, State::Ground, StringEnd::Aborted);
                p.execute(b);
                return;
            }
            0x1B => {
                self.transition(p, State::Escape, StringEnd::Terminated);
                return;
            }
            0x80..=0x9F => {
                if self.mode == InputMode::EightBit {
                    self.c1(p, b);
                } else {
                    self.high_non_control(p, b);
                }
                return;
            }
            _ => {}
        }

        // GR bytes are classified as their GL equivalents; payloads keep the raw byte.
        let c = if b >= 0xA0 { b & 0x7F } else { b };
        let is_c0 = c < 0x20;

        match self.state {
            State::Ground => {
                if is_c0 {
                    p.execute(c);
                } else {
                    p.print(b);
                }
            }

            State::Escape if self.vt52 => match c {
                _ if is_c0 => p.execute(c),
                0x7F => {}
                b'Y' => self.state = State::Vt52Line,
                _ => {
                    self.state = State::Ground;
                    p.esc_dispatch(&[], c);
                }
            },

            State::Escape => match c {
                _ if is_c0 => p.execute(c),
                0x20..=0x2F => {
                    self.collect(c);
                    self.state = State::EscapeIntermediate;
                }
                b'P' => self.transition(p, State::DcsEntry, StringEnd::Terminated),
                b'[' => self.transition(p, State::CsiEntry, StringEnd::Terminated),
                b']' => self.transition(p, State::OscString, StringEnd::Terminated),
                b'X' => self.start_string(p, StringKind::Sos, StringEnd::Terminated),
                b'^' => self.start_string(p, StringKind::Pm, StringEnd::Terminated),
                b'_' => self.start_string(p, StringKind::Apc, StringEnd::Terminated),
                0x7F => {}
                _ => {
                    self.state = State::Ground;
                    p.esc_dispatch(&[], c);
                }
            },

            State::EscapeIntermediate => match c {
                _ if is_c0 => p.execute(c),
                0x20..=0x2F => self.collect(c),
                0x7F => {}
                _ => {
                    self.state = State::Ground;
                    if !self.ignoring {
                        let n = usize::from(self.intermediate_len);
                        p.esc_dispatch(&self.intermediates[..n], c);
                    }
                }
            },

            State::CsiEntry => match c {
                _ if is_c0 => p.execute(c),
                0x20..=0x2F => {
                    self.collect(c);
                    self.state = State::CsiIntermediate;
                }
                0x30..=0x3B => {
                    self.param(c);
                    self.state = State::CsiParam;
                }
                0x3C..=0x3F => {
                    self.private = Some(c);
                    self.state = State::CsiParam;
                }
                0x7F => {}
                _ => self.csi_dispatch(p, c),
            },

            State::CsiParam => match c {
                _ if is_c0 => p.execute(c),
                0x20..=0x2F => {
                    self.collect(c);
                    self.state = State::CsiIntermediate;
                }
                0x30..=0x3B => self.param(c),
                0x3C..=0x3F => self.state = State::CsiIgnore,
                0x7F => {}
                _ => self.csi_dispatch(p, c),
            },

            State::CsiIntermediate => match c {
                _ if is_c0 => p.execute(c),
                0x20..=0x2F => self.collect(c),
                0x30..=0x3F => self.state = State::CsiIgnore,
                0x7F => {}
                _ => self.csi_dispatch(p, c),
            },

            State::CsiIgnore => match c {
                _ if is_c0 => p.execute(c),
                0x40..=0x7E => self.state = State::Ground,
                _ => {}
            },

            State::DcsEntry => match c {
                0x20..=0x2F => {
                    self.collect(c);
                    self.state = State::DcsIntermediate;
                }
                0x30..=0x3B => {
                    self.param(c);
                    self.state = State::DcsParam;
                }
                0x3C..=0x3F => {
                    self.private = Some(c);
                    self.state = State::DcsParam;
                }
                0x40..=0x7E => self.dcs_hook(p, c),
                _ => {}
            },

            State::DcsParam => match c {
                0x20..=0x2F => {
                    self.collect(c);
                    self.state = State::DcsIntermediate;
                }
                0x30..=0x3B => self.param(c),
                0x3C..=0x3F => self.state = State::DcsIgnore,
                0x40..=0x7E => self.dcs_hook(p, c),
                _ => {}
            },

            State::DcsIntermediate => match c {
                0x20..=0x2F => self.collect(c),
                0x30..=0x3F => self.state = State::DcsIgnore,
                0x40..=0x7E => self.dcs_hook(p, c),
                _ => {}
            },

            State::DcsPassthrough => {
                if c != 0x7F {
                    p.dcs_put(b);
                }
            }

            State::DcsIgnore => {}

            State::OscString => {
                if c == 0x07 && self.osc_bel_terminates {
                    self.transition(p, State::Ground, StringEnd::Terminated);
                } else if !is_c0 {
                    p.osc_put(b);
                }
            }

            State::SosPmApcString => {
                if !is_c0 {
                    p.string_put(b);
                }
            }

            State::Vt52Line => {
                if is_c0 {
                    p.execute(c);
                } else {
                    self.vt52_line = c - 0x20;
                    self.state = State::Vt52Column;
                }
            }

            State::Vt52Column => {
                if is_c0 {
                    p.execute(c);
                } else {
                    self.state = State::Ground;
                    p.vt52_cursor(self.vt52_line, c - 0x20);
                }
            }
        }
    }

    fn csi_dispatch<P: Perform + ?Sized>(&mut self, p: &mut P, final_byte: u8) {
        self.state = State::Ground;
        if !self.ignoring {
            p.csi_dispatch(self.sequence(final_byte));
        }
    }

    fn dcs_hook<P: Perform + ?Sized>(&mut self, p: &mut P, final_byte: u8) {
        if self.ignoring {
            self.state = State::DcsIgnore;
        } else {
            self.state = State::DcsPassthrough;
            p.dcs_hook(self.sequence(final_byte));
        }
    }
}
