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
    /// DECNRCM (42): 7-bit national replacement character sets.
    pub national: bool,
    /// DECLRMM / DECVSSM (69): left and right margins can be set.
    pub lr_margins: bool,
    /// DECVCCM (61): the user window pans to follow the cursor.
    pub vertical_coupling: bool,
    /// DECPCCM (64): moving to another page displays that page.
    pub page_coupling: bool,
    /// DECKBUM (68): data processing keys rather than typewriter keys.
    pub data_processing_keys: bool,
}

impl Modes {
    /// All modes as a bit set, for terminal state reports.
    pub fn to_bits(&self) -> u32 {
        self.flags()
            .iter()
            .enumerate()
            .fold(0, |acc, (i, on)| acc | (u32::from(*on) << i))
    }

    /// Restores modes from [`Modes::to_bits`].
    pub fn from_bits(bits: u32, mut base: Modes) -> Modes {
        let mut i = 0;
        base.for_each_mut(|m| {
            *m = bits & (1 << i) != 0;
            i += 1;
        });
        base
    }

    fn flags(&self) -> [bool; 23] {
        let mut copy = *self;
        let mut out = [false; 23];
        let mut i = 0;
        copy.for_each_mut(|m| {
            out[i] = *m;
            i += 1;
        });
        out
    }

    fn for_each_mut(&mut self, mut f: impl FnMut(&mut bool)) {
        for m in [
            &mut self.keyboard_locked,
            &mut self.insert,
            &mut self.send_receive,
            &mut self.new_line,
            &mut self.cursor_keys_application,
            &mut self.ansi,
            &mut self.columns_132,
            &mut self.smooth_scroll,
            &mut self.reverse_screen,
            &mut self.origin,
            &mut self.autowrap,
            &mut self.auto_repeat,
            &mut self.print_form_feed,
            &mut self.print_extent_full,
            &mut self.cursor_visible,
            &mut self.keypad_application,
            &mut self.backarrow_sends_bs,
            &mut self.national,
            &mut self.lr_margins,
            &mut self.vertical_coupling,
            &mut self.page_coupling,
            &mut self.data_processing_keys,
        ] {
            f(m);
        }
    }

    /// Power-up modes. `autowrap` and `new_line` come from Set-Up.
    pub const fn power_up(autowrap: bool, new_line: bool, national: bool) -> Modes {
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
            national,
            lr_margins: false,
            vertical_coupling: true,
            page_coupling: true,
            data_processing_keys: false,
        }
    }
}
