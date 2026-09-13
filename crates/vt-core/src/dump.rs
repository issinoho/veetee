//! Plain-text screen dumps for golden-file conformance tests.
//!
//! ```text
//! page 24x80 cursor 3,17 visible
//! modes decawm decom decscnm
//!  1  |Test of autowrap
//!  2 W|DOUBLE WIDE
//!     ~bb..88
//! ```
//!
//! Each row is `NN S|text` with trailing blanks trimmed, where `S` is the
//! line size (` ` single, `W` double width, `T`/`B` double height top/bottom).
//! A `~` line follows any row with renditions: one hex digit per cell,
//! bold 1, underline 2, blink 4, reverse 8 (`.` for none), trimmed.
//! Protected (DECSCA) cells get a `p` line in the same style.
//! Soft-font characters are shown as `▯`. When a status line is in use,
//! `status <type>` follows the page with an `S` row for a host-writable line.

use std::fmt::Write as _;

use crate::StatusDisplay;
use crate::cell::Flags;
use crate::charset::SOFT_BASE;
use crate::grid::{Line, LineSize};
use crate::terminal::Terminal;

pub fn dump(term: &Terminal) -> String {
    let grid = term.grid();
    let cursor = term.cursor();
    let modes = term.modes();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "page {}x{} cursor {},{} {}",
        grid.rows(),
        grid.cols(),
        cursor.row + 1,
        cursor.col + 1,
        if modes.cursor_visible {
            "visible"
        } else {
            "hidden"
        }
    );
    let flags = [
        (!modes.ansi, "vt52"),
        (modes.autowrap, "decawm"),
        (modes.origin, "decom"),
        (modes.reverse_screen, "decscnm"),
        (modes.insert, "irm"),
        (modes.new_line, "lnm"),
        (modes.cursor_keys_application, "decckm"),
        (modes.keypad_application, "deckpam"),
    ];
    out.push_str("modes");
    for (_, name) in flags.iter().filter(|(on, _)| *on) {
        out.push(' ');
        out.push_str(name);
    }
    if term.leds() != 0 {
        let lit: Vec<String> = (0..4)
            .filter(|i| term.leds() & 1 << i != 0)
            .map(|i| format!("L{}", i + 1))
            .collect();
        let _ = write!(out, " leds={}", lit.join(","));
    }
    out.push('\n');

    for (i, line) in grid.lines().iter().enumerate() {
        let size = match line.size {
            LineSize::Single => ' ',
            LineSize::DoubleWidth => 'W',
            LineSize::DoubleHeightTop => 'T',
            LineSize::DoubleHeightBottom => 'B',
        };
        dump_line(&mut out, &format!("{:2} {size}", i + 1), line);
    }
    match term.status_display() {
        StatusDisplay::None => {}
        StatusDisplay::Indicator => out.push_str("status indicator\n"),
        StatusDisplay::HostWritable => {
            let active = if term.status_active() { " active" } else { "" };
            let _ = writeln!(out, "status host-writable{active}");
            dump_line(&mut out, " S  ", term.status_line());
        }
    }
    out
}

fn dump_line(out: &mut String, label: &str, line: &Line) {
    let text: String = line
        .cells()
        .iter()
        .map(|c| {
            if u32::from(c.ch) >= SOFT_BASE {
                '▯'
            } else {
                c.ch
            }
        })
        .collect();
    let _ = writeln!(out, "{label}|{}", text.trim_end_matches(' '));

    let attrs: String = line
        .cells()
        .iter()
        .map(|c| {
            let f = c.attrs.flags;
            let bits = [Flags::BOLD, Flags::UNDERLINE, Flags::BLINK, Flags::REVERSE]
                .iter()
                .enumerate()
                .filter(|(_, flag)| f.contains(**flag))
                .fold(0u32, |acc, (bit, _)| acc | 1 << bit);
            if bits == 0 {
                '.'
            } else {
                char::from_digit(bits, 16).unwrap_or('?')
            }
        })
        .collect();
    let attrs = attrs.trim_end_matches('.');
    if !attrs.is_empty() {
        let _ = writeln!(out, "   ~{attrs}");
    }
    let protected: String = line
        .cells()
        .iter()
        .map(|c| {
            if c.attrs.flags.contains(Flags::PROTECTED) {
                'p'
            } else {
                '.'
            }
        })
        .collect();
    let protected = protected.trim_end_matches('.');
    if !protected.is_empty() {
        let _ = writeln!(out, "   p{protected}");
    }
}

/// Text of one row with trailing blanks removed.
pub fn row_text(term: &Terminal, row: usize) -> String {
    let text: String = term.grid().line(row).cells().iter().map(|c| c.ch).collect();
    text.trim_end_matches(' ').to_string()
}
