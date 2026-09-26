//! The printer port: what the host prints on the terminal's printer, and what
//! the user prints from the screen.
//!
//! A DEC terminal from the VT102 on has a printer port, and OpenVMS knows it —
//! `SHOW TERMINAL` lists *Printer port* for veetee. What arrives for the
//! printer leaves here as a [`PrintJob`] in [`Event::Print`], for whatever is
//! standing in for paper to take; the terminal itself only decides what is
//! printed, and when.
//!
//! - **Print screen** (`CSI i`, `CSI 0 i`, VT52 `ESC ]`, the Print Screen key):
//!   the scrolling region, or the whole page with DECPEX, then a form feed
//!   with DECPFF.
//! - **Print cursor line** (`CSI ? 1 i`, VT52 `ESC V`).
//! - **Auto print** (`CSI ? 5 i` on, `CSI ? 4 i` off; VT52 `ESC ^`, `ESC _`):
//!   each line as the cursor leaves it by a line feed, vertical tab, form
//!   feed or wrap.
//! - **Printer controller** (`CSI 5 i` on, `CSI 4 i` off; VT52 `ESC W`,
//!   `ESC X`): everything the host sends goes to the printer and none of it
//!   to the screen, until the terminator.
//!
//! 🔎 Written from DEC STD 070's Media Copy and the VT420's printing
//! functions as understood; the full list of `CSI ? … i` functions (printing
//! the composed display, all pages, the VT500 printer-to-host session) and
//! exactly what controller mode passes through are to be checked against the
//! printing chapters of EK-VT420-RM and EK-VT520-RM. Those not listed above
//! are ignored.

use super::{Emulator, Event};
use crate::setup::PrintMode;

/// Something to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintJob {
    /// Text from the screen: lines ending in `\n`, a form feed where DECPFF
    /// asks for one, DEC's graphic sets already translated to Unicode.
    Text(String),
    /// What the host sent in printer controller mode, byte for byte: often
    /// meant for a particular printer, escape sequences and all.
    Controller(Vec<u8>),
}

/// The printer port's state.
#[derive(Debug, Clone, Default)]
pub(super) struct Printer {
    /// Printer controller mode is on.
    pub(super) controller: bool,
    /// What the host has sent for the printer so far.
    job: Vec<u8>,
    /// The part of a possible terminator seen so far, held back from the job
    /// until it is known to be one or not.
    pending: Vec<u8>,
    /// Auto print mode is on, and the lines it has printed since.
    pub(super) auto: bool,
    auto_lines: String,
}

/// What ends printer controller mode: `CSI 4 i` in either form, or VT52
/// `ESC X`.
const TERMINATORS: [&[u8]; 3] = [b"\x1b[4i", b"\x9b4i", b"\x1bX"];

impl Emulator {
    /// Takes bytes in printer controller mode: everything to the printer,
    /// nothing to the screen, until the terminator, which may arrive split
    /// across reads. Returns how many bytes it used; after the terminator the
    /// rest are the parser's again.
    pub(super) fn printer_controller(&mut self, bytes: &[u8]) -> usize {
        for (i, &byte) in bytes.iter().enumerate() {
            let printer = &mut self.printer;
            printer.pending.push(byte);
            loop {
                let pending = printer.pending.as_slice();
                if TERMINATORS.contains(&pending) {
                    printer.pending.clear();
                    printer.controller = false;
                    let job = std::mem::take(&mut printer.job);
                    self.events.push(Event::Print(PrintJob::Controller(job)));
                    return i + 1;
                }
                if TERMINATORS.iter().any(|t| t.starts_with(pending)) {
                    break;
                }
                // Not a terminator after all: its first byte is the
                // printer's, and the rest may still begin one.
                let first = printer.pending.remove(0);
                printer.job.push(first);
                if printer.pending.is_empty() {
                    break;
                }
            }
        }
        bytes.len()
    }

    pub(super) fn controller_on(&mut self) {
        self.printer.controller = true;
        self.printer.job.clear();
        self.printer.pending.clear();
        // Stop the parser here, so the very next byte is the printer's.
        self.pause = true;
    }

    /// Ends printer controller mode other than by the host's terminator —
    /// from Printer Set-Up — printing what it had.
    fn controller_off(&mut self) {
        if !self.printer.controller {
            return;
        }
        self.printer.controller = false;
        let mut job = std::mem::take(&mut self.printer.job);
        job.append(&mut self.printer.pending);
        if !job.is_empty() {
            self.events.push(Event::Print(PrintJob::Controller(job)));
        }
    }

    /// Printer Set-Up's print mode, which the host also sets with MC.
    pub(super) fn print_mode(&self) -> PrintMode {
        if self.printer.controller {
            PrintMode::Controller
        } else if self.printer.auto {
            PrintMode::Auto
        } else {
            PrintMode::Normal
        }
    }

    /// Sets the print mode from Printer Set-Up. Leaving auto print or
    /// controller mode prints what each had gathered.
    pub(super) fn set_print_mode(&mut self, mode: PrintMode) {
        match mode {
            PrintMode::Normal => {
                self.auto_print(false);
                self.controller_off();
            }
            PrintMode::Auto => {
                self.controller_off();
                self.auto_print(true);
            }
            PrintMode::Controller => {
                self.auto_print(false);
                if !self.printer.controller {
                    self.controller_on();
                }
            }
        }
    }

    /// Print screen: the scrolling region, or the whole page with DECPEX.
    pub(super) fn print_screen(&mut self) {
        let rows = if self.modes.print_extent_full {
            0..self.rows()
        } else {
            self.top..self.bottom + 1
        };
        let mut text = String::new();
        for row in rows {
            text.push_str(&self.print_line(row));
            text.push('\n');
        }
        if self.modes.print_form_feed {
            text.push('\x0c');
        }
        self.events.push(Event::Print(PrintJob::Text(text)));
    }

    pub(super) fn print_cursor_line(&mut self) {
        let mut text = self.print_line(self.cursor.row);
        text.push('\n');
        self.events.push(Event::Print(PrintJob::Text(text)));
    }

    pub(super) fn auto_print(&mut self, on: bool) {
        if !on && self.printer.auto {
            let lines = std::mem::take(&mut self.printer.auto_lines);
            if !lines.is_empty() {
                self.events.push(Event::Print(PrintJob::Text(lines)));
            }
        }
        self.printer.auto = on;
    }

    /// The line the cursor is leaving, in auto print mode.
    pub(super) fn auto_print_line(&mut self) {
        if self.printer.auto {
            let line = self.print_line(self.cursor.row);
            self.printer.auto_lines.push_str(&line);
            self.printer.auto_lines.push('\n');
        }
    }

    /// A line of the page as a printer gets it: what it shows, trailing
    /// blanks dropped.
    fn print_line(&self, row: usize) -> String {
        let text: String = self.grid.line(row).cells().iter().map(|c| c.ch).collect();
        text.trim_end_matches(' ').to_string()
    }

    /// DSR for the printer: ready where one is configured, none where not.
    pub(super) fn printer_status(&self) -> &'static str {
        if self.config.printer { "?10n" } else { "?13n" }
    }
}
