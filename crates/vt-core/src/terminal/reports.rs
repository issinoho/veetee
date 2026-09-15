//! Replies to the host: device attributes, status reports, mode and setting
//! requests, presentation state reports.

use super::{Emulator, Flags, charset::Charset};
use crate::config::StatusDisplay;

impl Emulator {
    pub(super) fn reply_csi(&mut self, body: &str) {
        if self.c1_8bit {
            self.output.push(0x9B);
        } else {
            self.output.extend_from_slice(b"\x1b[");
        }
        self.output.extend_from_slice(body.as_bytes());
    }

    pub(super) fn reply_dcs(&mut self, body: &str) {
        if self.c1_8bit {
            self.output.push(0x90);
            self.output.extend_from_slice(body.as_bytes());
            self.output.push(0x9C);
        } else {
            self.output.extend_from_slice(b"\x1bP");
            self.output.extend_from_slice(body.as_bytes());
            self.output.extend_from_slice(b"\x1b\\");
        }
    }

    // -------------------------------------------------------- attributes

    pub(super) fn device_attributes(&mut self) {
        // DA1 reports the identity Set-Up's Terminal ID selects (DECTID),
        // whose default is the terminal's own. Only VT52 mode ignores it, and
        // answers ESC / Z instead; VT100 mode is not an exception, so a VT220
        // answers as a VT100, VT101 or VT102 there only when that ID is
        // selected (EK-VT510-RM 2.6.2, EK-VT220-RM 4.17.1.1).
        let da = self
            .terminal_id_attributes()
            .unwrap_or_else(|| self.config.model.primary_da());
        self.reply_csi(&format!("?{da}c"));
    }

    pub(super) fn secondary_attributes(&mut self) {
        if let Some(da) = self.config.model.secondary_da() {
            self.reply_csi(&format!(">{da}c"));
        }
    }

    /// DA3: the unit identification (DECRPTUI).
    pub(super) fn tertiary_attributes(&mut self) {
        self.reply_dcs("!|00000000");
    }

    // ------------------------------------------------------------ status

    pub(super) fn device_status(&mut self, private: Option<u8>, code: u16) {
        let level = self.level;
        match (private, code) {
            (None, 5) => self.reply_csi("0n"),
            (None, 6) => {
                let (row, col) = self.report_position();
                self.reply_csi(&format!("{row};{col}R"));
            }
            // DECXCPR: extended cursor position with page number.
            (Some(b'?'), 6) if level >= 4 => {
                let (row, col) = self.report_position();
                let page = self.page + 1;
                self.reply_csi(&format!("?{row};{col};{page}R"));
            }
            // Printer: no printer attached.
            (Some(b'?'), 15) if level >= 2 => self.reply_csi("?13n"),
            (Some(b'?'), 25) if level >= 2 => {
                let locked = if self.udk.locked { 21 } else { 20 };
                self.reply_csi(&format!("?{locked}n"));
            }
            (Some(b'?'), 26) if level >= 2 => {
                let language = match self.setup.keyboard {
                    Some((_, language)) if level >= 5 => language,
                    _ => u16::from(keyboard_language_code(self.config.keyboard_language)),
                };
                if level >= 4 {
                    // Keyboard ready; type 1 is the VT420's LK401 and 4 the
                    // VT500s' LK411/LK450 (EK-VT420-RM p. 277, EK-VT510-RM 5-166).
                    let kind = match (level, self.setup.keyboard) {
                        (5.., Some((2, _))) => 5,
                        (5.., _) => 4,
                        _ => 1,
                    };
                    self.reply_csi(&format!("?27;{language};0;{kind}n"));
                } else {
                    self.reply_csi(&format!("?27;{language}n"));
                }
            }
            // Macro space report (DECMSR): available bytes / 16.
            (Some(b'?'), 62) if level >= 4 => {
                let free = self.macro_space() / 16;
                self.reply_csi(&format!("{free}*{{"));
            }
            // Data integrity: no communication errors.
            (Some(b'?'), 75) if level >= 4 => self.reply_csi("?70n"),
            // Multiple sessions: each veetee session has its own connection, like
            // the VT420/VT520 "sessions on separate lines"; one session reports
            // sessions not ready.
            (Some(b'?'), 85) if level >= 4 => {
                self.reply_csi(if self.sessions > 1 { "?87n" } else { "?83n" })
            }
            _ => {}
        }
    }

    /// DSR ?63: memory checksum (DECCKSR) of the macro definitions.
    pub(super) fn memory_checksum(&mut self, pid: u16) {
        let sum = self.macro_checksum();
        self.reply_dcs(&format!("{pid}!~{sum:04X}"));
    }

    /// Cursor position for CPR, relative to the margins in origin mode.
    fn report_position(&self) -> (usize, usize) {
        let (row, col) = if self.status.active {
            let (r, c, _) = self.status.main_cursor;
            (r, c)
        } else {
            (self.cursor.row, self.cursor.col)
        };
        let (oy, ox) = if self.modes.origin {
            (self.top, self.left)
        } else {
            (0, 0)
        };
        (row + 1 - oy.min(row), col + 1 - ox.min(col))
    }

    // ----------------------------------------------------- mode requests

    /// DECRQM, answered with DECRPM: 0 unknown, 1 set, 2 reset, 3/4 permanent.
    pub(super) fn request_mode(&mut self, private: Option<u8>, mode: u16) {
        let on = |b: bool| if b { 1 } else { 2 };
        let m = &self.modes;
        let state = match private {
            None => match mode {
                2 => on(m.keyboard_locked),
                4 => on(m.insert),
                12 => on(m.send_receive),
                20 => on(m.new_line),
                1 | 3 | 5 | 7 | 10 | 11 | 13..=19 => 4,
                _ => 0,
            },
            Some(_) => match mode {
                1 => on(m.cursor_keys_application),
                2 => on(m.ansi),
                3 => on(m.columns_132),
                4 => on(m.smooth_scroll),
                5 => on(m.reverse_screen),
                6 => on(m.origin),
                7 => on(m.autowrap),
                8 => on(m.auto_repeat),
                18 => on(m.print_form_feed),
                19 => on(m.print_extent_full),
                25 => on(m.cursor_visible),
                42 => on(m.national),
                66 => on(m.keypad_application),
                67 => on(m.backarrow_sends_bs),
                60 if self.level >= 4 => 4,
                61 if self.level >= 4 => on(m.vertical_coupling),
                64 if self.level >= 4 => on(m.page_coupling),
                68 if self.level >= 4 => on(m.data_processing_keys),
                69 if self.level >= 4 => on(m.lr_margins),
                _ if self.level >= 5 => self.vt520_mode(mode).map_or(0, on),
                _ => 0,
            },
        };
        let marker = if private.is_some() { "?" } else { "" };
        self.reply_csi(&format!("{marker}{mode};{state}$y"));
    }

    /// DECRQSS, answered with DECRPSS. Real VT420/VT520 terminals send 1 for a
    /// valid request (the VT510 manual has the digits reversed).
    pub(super) fn request_setting(&mut self, data: &[u8]) {
        let level = self.level;
        let body = match data {
            b"m" => Some(format!("{}m", self.sgr_report())),
            b"r" => Some(format!("{};{}r", self.top + 1, self.bottom + 1)),
            b"\"p" => Some(if level == 1 {
                "61\"p".to_string()
            } else {
                format!("6{level};{}\"p", if self.c1_8bit { 0 } else { 1 })
            }),
            b"\"q" => Some(format!(
                "{}\"q",
                u8::from(self.cursor.attrs.flags.contains(Flags::PROTECTED))
            )),
            b"$}" => Some(format!("{}$}}", u8::from(self.status.active))),
            b"$~" => Some(format!(
                "{}$~",
                match self.status.kind {
                    StatusDisplay::None => 0,
                    StatusDisplay::Indicator => 1,
                    StatusDisplay::HostWritable => 2,
                }
            )),
            b"$|" if level >= 4 => Some(format!("{}$|", self.cols())),
            b"t" if level >= 4 => Some(format!("{}t", self.rows())),
            b"*|" if level >= 4 => Some(format!("{}*|", self.screen_lines)),
            b"s" if level >= 4 => Some(format!("{};{}s", self.left + 1, self.right + 1)),
            b"*x" if level >= 4 => Some(format!("{}*x", if self.sace_rectangle { 2 } else { 0 })),
            _ if level >= 5 => self.vt520_setting(data),
            _ => None,
        };
        match body {
            Some(body) => self.reply_dcs(&format!("1$r{body}")),
            None => self.reply_dcs("0$r"),
        }
    }

    fn sgr_report(&self) -> String {
        use crate::cell::Color;
        let a = self.cursor.attrs;
        let mut s = String::from("0");
        for (flag, code) in [
            (Flags::BOLD, "1"),
            (Flags::UNDERLINE, "4"),
            (Flags::BLINK, "5"),
            (Flags::REVERSE, "7"),
            (Flags::INVISIBLE, "8"),
        ] {
            if a.flags.contains(flag) {
                s.push(';');
                s.push_str(code);
            }
        }
        if let Color::Indexed(i @ 0..=7) = a.fg {
            s.push_str(&format!(";3{i}"));
        }
        if let Color::Indexed(i @ 0..=7) = a.bg {
            s.push_str(&format!(";4{i}"));
        }
        s
    }

    // ----------------------------------------------- presentation state

    /// DECRQPSR: 1 = cursor information (DECCIR), 2 = tab stops (DECTABSR).
    pub(super) fn presentation_state_report(&mut self, ps: u16) {
        match ps {
            1 => {
                let body = self.cursor_information();
                self.reply_dcs(&body);
            }
            2 => {
                let stops: Vec<String> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, set)| **set)
                    .map(|(c, _)| (c + 1).to_string())
                    .collect();
                self.reply_dcs(&format!("2$u{}", stops.join("/")));
            }
            _ => {}
        }
    }

    fn cursor_information(&self) -> String {
        let a = self.cursor.attrs.flags;
        let bits = |pairs: &[(bool, u8)]| {
            char::from(
                0x40 | pairs
                    .iter()
                    .filter(|(on, _)| *on)
                    .fold(0, |acc, (_, bit)| acc | bit),
            )
        };
        let srend = bits(&[
            (a.contains(Flags::BOLD), 1),
            (a.contains(Flags::UNDERLINE), 2),
            (a.contains(Flags::BLINK), 4),
            (a.contains(Flags::REVERSE), 8),
        ]);
        let satt = bits(&[(a.contains(Flags::PROTECTED), 1)]);
        let ss = self.charsets.single_shift;
        let sflag = bits(&[
            (self.modes.origin, 1),
            (ss == Some(2), 2),
            (ss == Some(3), 4),
            (self.cursor.pending_wrap, 8),
        ]);
        let g = self.charsets.g;
        let scss = bits(&[
            (g[0].is_96(), 1),
            (g[1].is_96(), 2),
            (g[2].is_96(), 4),
            (g[3].is_96(), 8),
        ]);
        let designations: String = g.iter().map(|set| self.designator(*set)).collect();
        format!(
            "1$u{};{};1;{srend};{satt};{sflag};{};{};{scss};{designations}",
            self.cursor.row + 1,
            self.cursor.col + 1,
            self.charsets.gl,
            self.charsets.gr,
        )
    }

    fn designator(&self, set: Charset) -> String {
        match set {
            Charset::Soft { slot, .. } => self
                .soft
                .designation(slot)
                .map(|d| String::from_utf8_lossy(d).into_owned())
                .unwrap_or_else(|| " @".into()),
            other => other.designator().into(),
        }
    }

    /// DECRSPS: restores a DECCIR or DECTABSR report. Invalid values stop the
    /// restore, possibly leaving it partial (as on DEC terminals).
    pub(super) fn restore_presentation_state(&mut self, kind: u16, data: &[u8]) {
        let Ok(text) = std::str::from_utf8(data) else {
            return;
        };
        match kind {
            1 => {
                let _ = self.restore_cursor_information(text);
            }
            2 => {
                self.tabs.fill(false);
                for stop in text.split('/') {
                    let Ok(col) = stop.trim().parse::<usize>() else {
                        break;
                    };
                    if (1..=self.tabs.len()).contains(&col) {
                        self.tabs[col - 1] = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn restore_cursor_information(&mut self, text: &str) -> Option<()> {
        let f: Vec<&str> = text.splitn(10, ';').collect();
        let num = |i: usize| f.get(i)?.parse::<usize>().ok();
        let bits = |i: usize| {
            let b = *f.get(i)?.as_bytes().first()?;
            (b & 0xC0 == 0x40).then_some(b & 0x3F)
        };
        let (row, col) = (num(0)?, num(1)?);
        let srend = bits(3)?;
        let satt = bits(4)?;
        let sflag = bits(5)?;
        let flags = &mut self.cursor.attrs.flags;
        flags.set(Flags::BOLD, srend & 1 != 0);
        flags.set(Flags::UNDERLINE, srend & 2 != 0);
        flags.set(Flags::BLINK, srend & 4 != 0);
        flags.set(Flags::REVERSE, srend & 8 != 0);
        flags.set(Flags::PROTECTED, satt & 1 != 0);
        self.modes.origin = sflag & 1 != 0;
        self.goto(row.max(1) - 1, col.max(1) - 1);
        self.cursor.pending_wrap = sflag & 8 != 0 && self.cursor.col == self.last_col();
        self.charsets.single_shift = match (sflag & 2 != 0, sflag & 4 != 0) {
            (true, _) => Some(2),
            (_, true) => Some(3),
            _ => None,
        };
        let gl = num(6).filter(|g| *g < 4)?;
        let gr = num(7).filter(|g| *g < 4)?;
        self.charsets.gl = gl as u8;
        self.charsets.gr = gr as u8;
        let scss = bits(8)?;
        let mut desig = f.get(9)?.bytes();
        for g in 0..4 {
            let mut intermediate = None;
            let final_byte = loop {
                let b = desig.next()?;
                if (0x20..=0x2F).contains(&b) {
                    intermediate = Some(b);
                } else {
                    break b;
                }
            };
            self.designate(g, scss & (1 << g) != 0, intermediate, final_byte);
        }
        Some(())
    }

    // ----------------------------------------------------- programmed keys

    /// DECRQKD, answered with DECRPFK or DECRPAK.
    pub(super) fn key_definition_report(&mut self, station: u16, modifier: u16) {
        let Ok(station) = u8::try_from(station) else {
            return;
        };
        if crate::keyprog::is_alphanumeric(station) {
            let body = self.keyprog.report_alphanumeric(station);
            self.reply_dcs(&format!("\"~{body}"));
            return;
        }
        let (Some(key), modifier @ 0..=8) = (crate::keyprog::key_at(station), modifier) else {
            return;
        };
        let mods = crate::keyboard::KeyMods {
            shift: matches!(modifier, 2 | 4 | 6 | 8),
            alt: matches!(modifier, 3 | 4 | 7 | 8),
            ctrl: matches!(modifier, 5..=8),
        };
        let cx = crate::keyboard::KeyContext {
            ansi: self.modes.ansi,
            level: self.level,
            eight_bit: self.c1_8bit,
            cursor_app: self.modes.cursor_keys_application,
            keypad_app: self.modes.keypad_application,
            new_line: self.modes.new_line,
            backarrow_bs: self.modes.backarrow_sends_bs,
        };
        let mut default = Vec::new();
        crate::keyboard::encode_with(key, mods, cx, &mut default);
        let body = self
            .keyprog
            .report_function(station, modifier as u8, &default);
        self.reply_dcs(&format!("\"}}{body}"));
    }

    // -------------------------------------------------- terminal state

    /// DECTSR. DEC documents the data string as model-specific; veetee's is
    /// `VT1` followed by `;`-separated settings that DECRSTS restores.
    pub(super) fn terminal_state_report(&mut self) {
        let status = match self.status.kind {
            StatusDisplay::None => 0,
            StatusDisplay::Indicator => 1,
            StatusDisplay::HostWritable => 2,
        };
        let tabs: String = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| **t)
            .map(|(c, _)| format!("{}/", c + 1))
            .collect();
        let body = format!(
            "1$sVT1;{:08X};{};{};{};{};{};{};{};{};{};{};{}",
            self.modes.to_bits(),
            self.top + 1,
            self.bottom + 1,
            self.left + 1,
            self.right + 1,
            self.rows(),
            self.cols(),
            self.screen_lines,
            status,
            u8::from(self.sace_rectangle),
            u8::from(self.c1_8bit),
            tabs.trim_end_matches('/'),
        );
        self.reply_dcs(&body);
    }

    /// DECRSTS with a DECTSR data string produced by [`Self::terminal_state_report`].
    pub(super) fn restore_terminal_state(&mut self, data: &[u8]) {
        let Ok(text) = std::str::from_utf8(data) else {
            return;
        };
        let f: Vec<&str> = text.split(';').collect();
        if f.len() < 13 || f[0] != "VT1" {
            return;
        }
        let num = |i: usize| f[i].parse::<usize>().ok();
        let (Ok(bits), Some(rows), Some(cols)) = (u32::from_str_radix(f[1], 16), num(6), num(7))
        else {
            return;
        };
        self.set_page_length(rows);
        self.set_page_width(cols);
        let columns_132 = self.modes.columns_132;
        self.modes = crate::modes::Modes::from_bits(bits, self.modes);
        self.modes.columns_132 = columns_132;
        self.pause = true;
        let clamp = |v: Option<usize>, max: usize| v.map(|v| v.clamp(1, max) - 1);
        let (rows, cols) = (self.rows(), self.cols());
        if let (Some(t), Some(b), Some(l), Some(r)) = (
            clamp(num(2), rows),
            clamp(num(3), rows),
            clamp(num(4), cols),
            clamp(num(5), cols),
        ) {
            if t < b && l < r {
                (self.top, self.bottom, self.left, self.right) = (t, b, l, r);
            }
        }
        if let Some(lines) = num(8) {
            self.set_screen_lines(lines);
        }
        if let Some(kind) = num(9) {
            self.set_status_type(kind as u16);
        }
        self.sace_rectangle = f[10] == "1";
        self.c1_8bit = f[11] == "1" && self.level >= 2;
        self.tabs.fill(false);
        for stop in f[12].split('/').filter_map(|c| c.parse::<usize>().ok()) {
            if (1..=self.tabs.len()).contains(&stop) {
                self.tabs[stop - 1] = true;
            }
        }
    }

    // ---------------------------------------------- supplemental sets

    /// DECRQUPSS, answered with DECAUPSS.
    pub(super) fn user_preferred_supplemental_report(&mut self) {
        let size = u8::from(self.upss.is_96());
        let body = format!("{size}!u{}", self.upss.designator());
        self.reply_dcs(&body);
    }

    /// DECAUPSS: size 0 designates a 94-character set, 1 a 96-character set.
    pub(super) fn assign_supplemental(&mut self, size: u16, data: &[u8]) {
        match (size, data) {
            (0, b"%5") => self.upss = Charset::DecSupplemental,
            (1, b"A") => self.upss = Charset::IsoLatin1,
            // VT520 supplemental sets; the size parameter is not reliable in
            // the VT520 manual, so the designator decides.
            (_, [f]) | (_, [_, f]) if self.level >= 5 => {
                let (intermediate, is_96) = match data {
                    [i, _] => (Some(*i), false),
                    _ => (None, true),
                };
                if let Some(set) =
                    crate::charset::Vt500Set::from_designator(is_96, intermediate, *f)
                {
                    if !set.is_national() {
                        self.upss = Charset::Vt500(set);
                    }
                }
            }
            _ => {}
        }
    }
}

/// DSR keyboard language codes (VT510 RM, DSR-KBD).
fn keyboard_language_code(language: Option<crate::charset::Nrc>) -> u8 {
    use crate::charset::Nrc::*;
    match language {
        None => 1,
        Some(British) => 2,
        Some(FrenchCanadian) => 4,
        Some(NorwegianDanish) => 13,
        Some(Finnish) => 6,
        Some(German) => 7,
        Some(Dutch) => 8,
        Some(Italian) => 9,
        Some(Swiss) => 10,
        Some(Swedish) => 12,
        Some(French) => 14,
        Some(Spanish) => 15,
        Some(Portuguese) => 16,
    }
}
