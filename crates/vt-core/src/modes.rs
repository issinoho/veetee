/// Terminal modes set by SM/RM (ANSI) and DECSET/DECRST (DEC private).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    // ANSI modes
    /// KAM (2): keyboard locked.
    pub keyboard_locked: bool,
    /// IRM (4): insert rather than replace.
    pub insert: bool,
    /// SRM (12): set = no local echo (DEC default).
    pub send_receive: bool,
    /// LNM (20): LF/VT/FF also return; Return sends CR LF.
    pub new_line: bool,

    // DEC private modes
    /// DECCKM (1): cursor keys send application sequences.
    pub cursor_keys_application: bool,
    /// DECANM (2): ANSI mode; reset = VT52 mode.
    pub ansi: bool,
    /// DECCOLM (3): 132 columns.
    pub columns_132: bool,
    /// DECSCLM (4): smooth scroll.
    pub smooth_scroll: bool,
    /// DECSCNM (5): reverse (light background) screen.
    pub reverse_screen: bool,
    /// DECOM (6): cursor addressing relative to the margins.
    pub origin: bool,
    /// DECAWM (7): autowrap.
    pub autowrap: bool,
    /// DECARM (8): auto-repeat.
    pub auto_repeat: bool,
    /// DECPFF (18): form feed after print screen.
    pub print_form_feed: bool,
    /// DECPEX (19): print screen prints the full page rather than the scroll region.
    pub print_extent_full: bool,
    /// DECTCEM (25): cursor visible.
    pub cursor_visible: bool,
    /// DECKPAM / DECKPNM, DECNKM (66): keypad sends application sequences.
    pub keypad_application: bool,
    /// DECBKM (67): set = backarrow key sends BS; reset (DEC default) = DEL.
    pub backarrow_sends_bs: bool,
}

impl Modes {
    /// Power-up modes. `autowrap` and `new_line` come from Set-Up.
    pub const fn power_up(autowrap: bool, new_line: bool) -> Modes {
        Modes {
            keyboard_locked: false,
            insert: false,
            send_receive: true,
            new_line,
            cursor_keys_application: false,
            ansi: true,
            columns_132: false,
            smooth_scroll: false,
            reverse_screen: false,
            origin: false,
            autowrap,
            auto_repeat: true,
            print_form_feed: false,
            print_extent_full: false,
            cursor_visible: true,
            keypad_application: false,
            backarrow_sends_bs: false,
        }
    }
}
