//! What stands in for the printer on the terminal's printer port: each print
//! job becomes a PDF in a folder (docs/printing.md, P2).
//!
//! The terminal decides what is printed and when ([`vt_core::PrintJob`]);
//! this turns a job into pages and draws them. A job from the screen is
//! text already. One from the host in printer controller mode is a printer's
//! stream — carriage returns, line feeds, form feeds, and escape sequences
//! meant for a particular printer — and is read the way a plain printer
//! would read it, the escape sequences left out.

use std::io;
use std::path::{Path, PathBuf};

use gtk::glib;
use vt_core::PrintJob;

/// Where print jobs go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printer {
    /// The folder PDFs are written to; `None` is no printer at all, and the
    /// host is told so.
    pub folder: Option<PathBuf>,
}

impl Default for Printer {
    /// PDFs in the user's Documents folder: decided on 25 September 2026, so
    /// that Print Screen works from the first and the host is told a printer
    /// is ready — it cannot tell a PDF from paper.
    fn default() -> Printer {
        Printer {
            folder: Some(
                glib::user_special_dir(glib::UserDirectory::Documents)
                    .unwrap_or_else(glib::home_dir),
            ),
        }
    }
}

fn path() -> PathBuf {
    glib::user_config_dir().join("veetee").join("printer.conf")
}

pub fn load() -> Printer {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Printer::default();
    };
    let mut printer = Printer::default();
    for line in text.lines() {
        match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
            Some(("destination", "none")) => printer.folder = None,
            Some(("folder", folder)) if !folder.is_empty() && printer.folder.is_some() => {
                printer.folder = Some(PathBuf::from(folder));
            }
            _ => {}
        }
    }
    printer
}

pub fn save(printer: &Printer) {
    let text = match &printer.folder {
        Some(folder) => format!(
            "# veetee printer: where print jobs go\ndestination=pdf\nfolder={}\n",
            folder.display()
        ),
        None => "# veetee printer: where print jobs go\ndestination=none\n".to_string(),
    };
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, text) {
        eprintln!("veetee: cannot save {}: {e}", path.display());
    }
}

/// A job as the lines and form feeds a plain printer would print.
pub fn text_of(job: &PrintJob) -> String {
    match job {
        PrintJob::Text(text) => text.clone(),
        PrintJob::Controller(bytes) => stream_text(bytes),
    }
}

/// Reads a printer's stream as a plain printer does: a line feed ends a line,
/// a form feed a page, a carriage return goes back to the start of the line
/// so that what follows prints over it, a tab moves to the next multiple of
/// eight, and escape sequences — bold, a pitch, a form's layout, for the
/// printer the host had in mind — are passed over. Bytes above 0x9F are the
/// ISO Latin-1 or DEC Supplemental characters of the upper half.
fn stream_text(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut line: Vec<char> = Vec::new();
    let mut col: usize = 0;
    let mut i = 0;
    let finish = |line: &mut Vec<char>, out: &mut String| {
        let text: String = line.iter().collect();
        out.push_str(text.trim_end());
        line.clear();
    };
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        match byte {
            b'\n' | 0x0b => {
                finish(&mut line, &mut out);
                out.push('\n');
                col = 0;
            }
            0x0c => {
                finish(&mut line, &mut out);
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\x0c');
                col = 0;
            }
            b'\r' => col = 0,
            b'\t' => col = (col / 8 + 1) * 8,
            0x08 => col = col.saturating_sub(1),
            // An escape sequence, or its 8-bit introducer: passed over.
            0x1b | 0x9b => i = skip_sequence(bytes, i, byte == 0x9b),
            0x00..=0x1f | 0x7f..=0x9f => {}
            _ => {
                let ch = char::from(byte);
                if line.len() < col {
                    line.resize(col, ' ');
                }
                if col < line.len() {
                    line[col] = ch;
                } else {
                    line.push(ch);
                }
                col += 1;
            }
        }
    }
    finish(&mut line, &mut out);
    if !out.is_empty() && !out.ends_with(['\n', '\x0c']) {
        out.push('\n');
    }
    out
}

/// Where an escape sequence that starts before `i` ends: after the final
/// byte of a control sequence, or after the one character of a plain escape.
fn skip_sequence(bytes: &[u8], mut i: usize, csi: bool) -> usize {
    if !csi {
        match bytes.get(i) {
            Some(b'[') => i += 1,
            // ESC and one character, with any intermediates before it.
            Some(_) => {
                while bytes.get(i).is_some_and(|b| (0x20..=0x2f).contains(b)) {
                    i += 1;
                }
                return (i + 1).min(bytes.len());
            }
            None => return i,
        }
    }
    // Parameters and intermediates, then the final byte.
    while bytes.get(i).is_some_and(|b| (0x20..=0x3f).contains(b)) {
        i += 1;
    }
    (i + 1).min(bytes.len())
}

/// The text in pages: a form feed starts a new one, and so does running out
/// of lines. A trailing form feed does not make a blank last page.
pub fn paginate(text: &str, lines_per_page: usize) -> Vec<Vec<String>> {
    let lines_per_page = lines_per_page.max(1);
    let mut pages = Vec::new();
    for sheet in text.split('\x0c') {
        let lines: Vec<String> = sheet.lines().map(str::to_string).collect();
        if lines.is_empty() {
            continue;
        }
        for chunk in lines.chunks(lines_per_page) {
            pages.push(chunk.to_vec());
        }
    }
    if pages.is_empty() {
        pages.push(Vec::new());
    }
    pages
}

/// Six lines to the inch, as a printer of the time printed.
const LINE_HEIGHT: f64 = 12.0;
/// Half an inch all round.
const MARGIN: f64 = 36.0;
/// Ten points, shrunk where 132 columns would not otherwise fit.
const FONT_SIZE: f64 = 10.0;

#[cfg(windows)]
const FONT: &str = "Consolas";
#[cfg(not(windows))]
const FONT: &str = "monospace";

/// A4, in points, for where nothing better is known.
pub const A4: (f64, f64) = (595.28, 841.89);

/// The paper the user's locale prints on, in points: A4 or US Letter. Needs
/// GTK running, which the window has and a test does not.
pub fn paper() -> (f64, f64) {
    let size = gtk::PaperSize::new(Some(&gtk::PaperSize::default()));
    let (w, h) = (
        size.width(gtk::Unit::Points),
        size.height(gtk::Unit::Points),
    );
    if w > 0.0 && h > 0.0 { (w, h) } else { A4 }
}

/// Writes the text as a PDF on `paper` (width and height in points):
/// portrait for up to 80 columns, landscape past that, the font made smaller
/// if even landscape is too narrow. Returns how many pages it took.
pub fn write_pdf(text: &str, path: &Path, paper: (f64, f64)) -> io::Result<usize> {
    let widest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    let (short, long) = paper;
    let (width, height) = if widest > 80 {
        (long, short)
    } else {
        (short, long)
    };
    let surface = cairo::PdfSurface::new(width, height, path).map_err(io::Error::other)?;
    let cr = cairo::Context::new(&surface).map_err(io::Error::other)?;
    cr.select_font_face(FONT, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(FONT_SIZE);
    let advance = cr
        .text_extents("M")
        .map(|e| e.x_advance())
        .unwrap_or(FONT_SIZE * 0.6);
    let usable = width - 2.0 * MARGIN;
    if widest > 0 && advance * widest as f64 > usable {
        cr.set_font_size(FONT_SIZE * usable / (advance * widest as f64));
    }
    let lines_per_page = ((height - 2.0 * MARGIN) / LINE_HEIGHT).floor() as usize;
    let pages = paginate(text, lines_per_page);
    for page in &pages {
        cr.set_source_rgb(0.0, 0.0, 0.0);
        for (i, line) in page.iter().enumerate() {
            cr.move_to(MARGIN, MARGIN + LINE_HEIGHT * (i as f64 + 0.8));
            cr.show_text(line).map_err(io::Error::other)?;
        }
        cr.show_page().map_err(io::Error::other)?;
    }
    drop(cr);
    surface.finish();
    Ok(pages.len())
}

/// A name for a job's PDF in `folder` that nothing has yet: the session and
/// the time, and a number after it where that is taken.
pub fn new_path(folder: &Path, session: &str) -> PathBuf {
    let stamp = glib::DateTime::now_local()
        .and_then(|t| t.format("%Y%m%d-%H%M%S"))
        .map(|s| s.to_string())
        .unwrap_or_default();
    let safe: String = session
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = format!("{safe}-print-{stamp}");
    let mut path = folder.join(format!("{stem}.pdf"));
    let mut n = 1;
    while path.exists() {
        path = folder.join(format!("{stem}-{n}.pdf"));
        n += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller(bytes: &[u8]) -> String {
        text_of(&PrintJob::Controller(bytes.to_vec()))
    }

    #[test]
    fn a_printers_stream_is_read_as_a_plain_printer_reads_it() {
        assert_eq!(controller(b"one\r\ntwo\r\n"), "one\ntwo\n");
        assert_eq!(controller(b"no ending"), "no ending\n");
        assert_eq!(controller(b"a\tb"), "a       b\n", "tabs every eight");
        assert_eq!(
            controller(b"page one\r\n\x0cpage two\r\n"),
            "page one\n\x0cpage two\n"
        );
        // A carriage return alone overprints: what comes after replaces it.
        assert_eq!(controller(b"xxxxx\r12\r\n"), "12xxx\n");
        assert_eq!(
            controller(b"caf\xe9\r\n"),
            "café\n",
            "the upper half as Latin-1"
        );
    }

    #[test]
    fn escape_sequences_for_the_printer_are_passed_over() {
        assert_eq!(controller(b"\x1b[1mBold\x1b[0m text\r\n"), "Bold text\n");
        assert_eq!(
            controller(b"\x1b#6wide\r\n"),
            "wide\n",
            "ESC with an intermediate"
        );
        assert_eq!(controller(b"\x1bcreset\r\n"), "reset\n");
        assert_eq!(controller(b"\x9b4;1mcsi\r\n"), "csi\n", "8-bit CSI");
        assert_eq!(controller(b"end\x1b["), "end\n", "cut short at the end");
    }

    #[test]
    fn text_is_paged_by_form_feeds_and_by_length() {
        let text = "a\nb\nc\n\x0cd\n";
        assert_eq!(paginate(text, 60), [vec!["a", "b", "c"], vec!["d"]]);
        assert_eq!(paginate(text, 2), [vec!["a", "b"], vec!["c"], vec!["d"]]);
        assert_eq!(
            paginate("a\n\x0c", 60),
            [vec!["a"]],
            "no blank page after a form feed"
        );
        assert_eq!(paginate("", 60).len(), 1, "an empty job is one blank page");
    }

    #[test]
    fn a_job_becomes_a_pdf_with_its_pages() {
        let dir = std::env::temp_dir().join(format!("veetee-print-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text: String = (1..=100).map(|i| format!("line {i} ┌─┐ café\n")).collect();
        let path = new_path(&dir, "vms1");
        let pages = write_pdf(&text, &path, A4).unwrap();
        assert_eq!(pages, 2, "a hundred lines at six to the inch");
        let pdf = std::fs::read(&path).unwrap();
        assert!(pdf.starts_with(b"%PDF-"), "a PDF");
        // Another job never overwrites the first.
        let second = new_path(&dir, "vms1");
        std::fs::write(&second, b"").unwrap();
        assert_ne!(new_path(&dir, "vms1"), second);
        let wide: String = format!("{}\n", "x".repeat(132));
        write_pdf(&wide, &dir.join("wide.pdf"), A4).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
