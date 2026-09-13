//! Replies to the host: device attributes, status reports, mode and setting
//! requests, presentation state reports.

use super::{Emulator, Flags, charset::Charset};
use crate::config::{Model, StatusDisplay};

impl Emulator {
    pub(super) fn reply_csi(&mut self, body: &str) {
        if self.c1_8bit {
            self.output.push(0x9B);
        } else {
            self.output.extend_from_slice(b"\x1b[");
        }
        self.output.extend_from_slice(body.as_bytes());
    }

    fn reply_dcs(&mut self, body: &str) {
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
        let da = self.config.model.primary_da();
        // In VT100 mode a VT220+ reports its VT100-compatible identity.
        let da = if self.level == 1 && self.config.model.max_level() > 1 {
            Model::Vt102.primary_da()
        } else {
            da
        };
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
                self.reply_csi(&format!("?{row};{col};1R"));
            }
            // Printer: no printer attached.
            (Some(b'?'), 15) if level >= 2 => self.reply_csi("?13n"),
            (Some(b'?'), 25) if level >= 2 => {
                let locked = if self.udk.locked { 21 } else { 20 };
                self.reply_csi(&format!("?{locked}n"));
            }
            (Some(b'?'), 26) if level >= 2 => {
                let language = keyboard_language_code(self.config.keyboard_language);
                if level >= 4 {
                    // Keyboard ready; type 1 is the VT420's LK401 and 4 the
                    // VT500s' LK411/LK450 (EK-VT420-RM p. 277, EK-VT510-RM 5-166).
                    let kind = if level >= 5 { 4 } else { 1 };
                    self.reply_csi(&format!("?27;{language};0;{kind}n"));
                } else {
                    self.reply_csi(&format!("?27;{language}n"));
                }
            }
            // Data integrity: no communication errors.
            (Some(b'?'), 75) if level >= 4 => self.reply_csi("?70n"),
            // Multiple sessions: not configured.
            (Some(b'?'), 85) if level >= 4 => self.reply_csi("?83n"),
            _ => {}
        }
    }

    /// Cursor position for CPR, relative to the margins in origin mode.
    fn report_position(&self) -> (usize, usize) {
        let (row, col) = if self.status.active {
            let (r, c, _) = self.status.main_cursor;
            (r, c)
        } else {
            (self.cursor.row, self.cursor.col)
        };
        let origin = if self.modes.origin { self.top } else { 0 };
        (row + 1 - origin.min(row), col + 1)
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
                61 | 64 | 68 | 69 if self.level >= 4 => 2,
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
            b"*|" if level >= 4 => Some(format!("{}*|", self.rows())),
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

    // ---------------------------------------------- supplemental sets

    /// DECRQUPSS, answered with DECAUPSS.
    pub(super) fn user_preferred_supplemental_report(&mut self) {
        let body = match self.upss {
            Charset::IsoLatin1 => "1!uA",
            _ => "0!u%5",
        };
        self.reply_dcs(body);
    }

    /// DECAUPSS: size 0 designates a 94-character set, 1 a 96-character set.
    pub(super) fn assign_supplemental(&mut self, size: u16, data: &[u8]) {
        match (size, data) {
            (0, b"%5") => self.upss = Charset::DecSupplemental,
            (1, b"A") => self.upss = Charset::IsoLatin1,
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
