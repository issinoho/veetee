use crate::charset::{Charset, Nrc};

/// The DEC terminal being emulated. Ordering follows capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Model {
    Vt100,
    Vt102,
    Vt220,
    Vt320,
    Vt420,
    Vt510,
    Vt520,
    Vt525,
}

impl Model {
    /// Highest conformance level (DECSCL): 1 = VT100, 2 = VT200, 3 = VT300, 4 = VT400, 5 = VT500.
    pub const fn max_level(self) -> u8 {
        match self {
            Model::Vt100 | Model::Vt102 => 1,
            Model::Vt220 => 2,
            Model::Vt320 => 3,
            Model::Vt420 => 4,
            Model::Vt510 | Model::Vt520 | Model::Vt525 => 5,
        }
    }

    /// Parameters of the Primary DA response (`CSI ? … c`).
    ///
    /// Extension codes: 1 132 columns, 2 printer port, 6 selective erase,
    /// 7 DRCS, 8 UDK, 9 NRCS, 15 technical set, 18 windowing,
    /// 21 horizontal scrolling, 22 colour.
    // TODO(M4): confirm VT5xx attribute lists against EK-VT510-RM / EK-VT520-RM.
    pub const fn primary_da(self) -> &'static str {
        match self {
            Model::Vt100 => "1;2",
            Model::Vt102 => "6",
            Model::Vt220 => "62;1;2;6;7;8;9",
            Model::Vt320 => "63;1;2;6;7;8;9",
            Model::Vt420 => "64;1;2;6;7;8;9;15;18;21",
            // The VT510 reports class 64 (EK-VT510-RM DA1). The VT500 factory
            // lists (international models) also include 19 sessions, 44 PCTerm,
            // 45 soft key mapping and 46 ASCII emulation, which veetee does not
            // provide yet; 6, 8 and 15 are implied at level 5 (EK-VT520-RM DA1).
            Model::Vt510 => "64;1;2;7;8;9;12;15;18;21;23;24;42",
            Model::Vt520 => "65;1;2;7;9;12;18;21;23;24;42",
            Model::Vt525 => "65;1;2;7;9;12;18;21;22;23;24;42",
        }
    }

    /// Parameters of the Secondary DA response (`CSI > … c`); VT100-class terminals have none.
    pub const fn secondary_da(self) -> Option<&'static str> {
        match self {
            Model::Vt100 | Model::Vt102 => None,
            Model::Vt220 => Some("1;10;0"),
            Model::Vt320 => Some("24;10;0"),
            Model::Vt420 => Some("41;10;0"),
            Model::Vt510 => Some("61;10;0"),
            Model::Vt520 => Some("64;10;0"),
            Model::Vt525 => Some("65;10;0"),
        }
    }

    /// The conventional `TERM` name for hosts that use terminfo.
    pub const fn term_name(self) -> &'static str {
        match self {
            Model::Vt100 => "vt100",
            Model::Vt102 => "vt102",
            Model::Vt220 => "vt220",
            Model::Vt320 => "vt320",
            Model::Vt420 => "vt420",
            Model::Vt510 | Model::Vt520 | Model::Vt525 => "vt520",
        }
    }

    /// The model's own name in lower case (`vt510`), unlike [`Model::term_name`].
    pub const fn term_name_exact(self) -> &'static str {
        match self {
            Model::Vt100 => "vt100",
            Model::Vt102 => "vt102",
            Model::Vt220 => "vt220",
            Model::Vt320 => "vt320",
            Model::Vt420 => "vt420",
            Model::Vt510 => "vt510",
            Model::Vt520 => "vt520",
            Model::Vt525 => "vt525",
        }
    }

    /// Parses a model name such as `vt420` (any case).
    pub fn from_name(name: &str) -> Option<Model> {
        [
            Model::Vt100,
            Model::Vt102,
            Model::Vt220,
            Model::Vt320,
            Model::Vt420,
            Model::Vt510,
            Model::Vt520,
            Model::Vt525,
        ]
        .into_iter()
        .find(|m| m.term_name_exact().eq_ignore_ascii_case(name))
    }

    /// VT320 and later have a status line.
    pub const fn has_status_line(self) -> bool {
        self.max_level() >= 3
    }

    /// DECSCLM at power-up: smooth scroll on the VT420 and VT510 (their
    /// programmer references), jump on the VT520 and VT525 (EK-VT520-RM
    /// table 2-10).
    // 🔎 The VT100, VT102, VT220 and VT320 defaults are not in the manuals
    // at hand; they power up in jump scroll.
    pub const fn smooth_scroll_default(self) -> bool {
        matches!(self, Model::Vt420 | Model::Vt510)
    }

    pub const fn has_color(self) -> bool {
        matches!(self, Model::Vt525)
    }
}

/// Set-Up "Status Display".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusDisplay {
    None,
    /// Terminal indicators (the factory setting).
    #[default]
    Indicator,
    HostWritable,
}

/// Set-Up "User-Preferred Supplemental Set".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Supplemental {
    #[default]
    DecSupplemental,
    IsoLatin1,
}

impl Supplemental {
    pub const fn charset(self) -> Charset {
        match self {
            Supplemental::DecSupplemental => Charset::DecSupplemental,
            Supplemental::IsoLatin1 => Charset::IsoLatin1,
        }
    }
}

/// Non-DEC behaviour. Everything here is off by default: veetee emulates
/// DEC terminals first and only adopts xterm conventions when a profile asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Extensions {
    /// Decode UTF-8 from the host instead of DEC 8-bit graphic sets.
    pub utf8: bool,
    /// Accept ECMA-48 colon sub-parameters and xterm SGR 38/48/90–107.
    pub xterm_sgr: bool,
    /// Use xterm's reading where DEC documentation says otherwise: SU/SD
    /// scroll the scrolling region instead of panning the user window, and
    /// DECRQCRA with page 0 and a rectangle checksums the current page
    /// instead of all pages. Needed by esctest.
    pub xterm_compat: bool,
}

/// Power-up / Set-Up configuration. Defaults are DEC factory Set-Up values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub model: Model,
    pub rows: usize,
    pub cols: usize,
    /// Set-Up "Auto Wrap". DEC factory default: off.
    pub autowrap: bool,
    /// Set-Up "New Line" (LNM). DEC factory default: off.
    pub new_line: bool,
    /// Set-Up "Answerback" message, sent in reply to ENQ.
    pub answerback: Vec<u8>,
    /// Lines kept after scrolling off the top of the page.
    pub scrollback_lines: usize,
    /// Set-Up "Status Display" (VT320 and later).
    pub status_display: StatusDisplay,
    /// Set-Up "Keyboard Language"; `None` is North American.
    pub keyboard_language: Option<Nrc>,
    /// Set-Up "Character Mode": 7-bit national (DECNRCM set) rather than 8-bit multinational.
    pub national_mode: bool,
    pub supplemental: Supplemental,
    /// Set-Up "User Defined Keys: Locked".
    pub udk_locked: bool,
    pub extensions: Extensions,
    /// Saved Set-Up features applied at power-up and by RIS; `None` for the
    /// factory settings (see [`crate::setup`]).
    pub setup: Option<crate::setup::Features>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            model: Model::Vt420,
            rows: 24,
            cols: 80,
            autowrap: false,
            new_line: false,
            answerback: Vec::new(),
            scrollback_lines: 10_000,
            status_display: StatusDisplay::default(),
            keyboard_language: None,
            national_mode: false,
            supplemental: Supplemental::default(),
            udk_locked: false,
            extensions: Extensions::default(),
            setup: None,
        }
    }
}
