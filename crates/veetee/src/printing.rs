//! What stands in for the printer on the terminal's printer port: each print
//! job becomes a PDF in a folder, or goes to a real printer through GTK
//! (docs/printing.md, P2 and P3).
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
pub enum Printer {
    /// A PDF for each job, in this folder.
    Folder(PathBuf),
    /// The printer chosen in the print dialog, whose settings are kept in
    /// `print-settings.ini`.
    System,
    /// Nowhere, and the host is told there is no printer.
    None,
}

impl Default for Printer {
    /// PDFs in the user's Documents folder: decided on 25 September 2026, so
    /// that Print Screen works from the first and the host is told a printer
    /// is ready — it cannot tell a PDF from paper.
    fn default() -> Printer {
        Printer::Folder(
            glib::user_special_dir(glib::UserDirectory::Documents).unwrap_or_else(glib::home_dir),
        )
    }
}

impl Printer {
    /// Whether anything stands in for a printer.
    pub fn attached(&self) -> bool {
        *self != Printer::None
    }
}

fn dir() -> PathBuf {
    glib::user_config_dir().join("veetee")
}

fn path() -> PathBuf {
    dir().join("printer.conf")
}

thread_local! {
    /// The printer chosen in the print dialog this session, which every job
    /// after goes to without asking. The desktop's print portal will not
    /// print without a dialog on the strength of saved settings alone, so a
    /// session asks once, at its first job or at Print to Printer.
    pub static SETUP: std::cell::RefCell<Option<gtk::PrintSetup>> =
        const { std::cell::RefCell::new(None) };
}

/// The command line that sends a PDF to a CUPS printer: `lp`, or through
/// `flatpak-spawn --host` where veetee is a Flatpak and CUPS is the host's.
///
/// The paper is always named — `media` is the size the PDF was drawn at, as
/// CUPS knows it (`A4`, `Letter`), and the type is plain paper. Left to the
/// printer's defaults a job took them: a Canon set to 4×6 photo paper stopped
/// with a paper size error on every page veetee sent it.
#[cfg(not(windows))]
pub fn lp_command(
    printer: &str,
    title: &str,
    pdf: &Path,
    media: &str,
    flatpak: bool,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    if flatpak {
        args.extend(["flatpak-spawn".into(), "--host".into()]);
    }
    args.extend([
        "lp".into(),
        "-d".into(),
        printer.into(),
        "-t".into(),
        title.into(),
        "-o".into(),
        format!("media={media}").into(),
        "-o".into(),
        "media-type=stationery".into(),
        "--".into(),
        pdf.into(),
    ]);
    args
}

/// Sends a PDF to the CUPS printer chosen in the print dialog, with no
/// dialog: the desktop's print portal asks again for every job, which will
/// not do for a host that prints unasked, and `lp` is how a job reaches a
/// named CUPS printer directly. Blocks until CUPS has the job.
#[cfg(not(windows))]
pub fn send_to_cups(printer: &str, title: &str, pdf: &Path, media: &str) -> io::Result<()> {
    let args = lp_command(printer, title, pdf, media, vt_transport::pty::in_flatpak());
    let out = std::process::Command::new(&args[0])
        .args(&args[1..])
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// Where a PDF waits to be handed to CUPS: somewhere the host can read it,
/// which in a Flatpak the private `/tmp` is not.
#[cfg(not(windows))]
pub fn spool_dir() -> PathBuf {
    if vt_transport::pty::in_flatpak() {
        glib::user_cache_dir()
    } else {
        glib::tmp_dir()
    }
}

/// Where the chosen printer's settings are kept, as GTK writes them.
pub fn settings_path() -> PathBuf {
    dir().join("print-settings.ini")
}

pub fn load() -> Printer {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Printer::default();
    };
    let mut destination = "pdf";
    let mut folder = None;
    for line in text.lines() {
        match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
            Some(("destination", d)) => {
                destination = if d == "none" {
                    "none"
                } else if d == "printer" {
                    "printer"
                } else {
                    "pdf"
                }
            }
            Some(("folder", f)) if !f.is_empty() => folder = Some(PathBuf::from(f)),
            _ => {}
        }
    }
    match (destination, folder) {
        ("none", _) => Printer::None,
        ("printer", _) if settings_path().exists() => Printer::System,
        (_, Some(folder)) => Printer::Folder(folder),
        _ => Printer::default(),
    }
}

pub fn save(printer: &Printer) {
    let header = "# veetee printer: where print jobs go\n";
    let text = match printer {
        Printer::Folder(folder) => {
            format!("{header}destination=pdf\nfolder={}\n", folder.display())
        }
        Printer::System => format!("{header}destination=printer\n"),
        Printer::None => format!("{header}destination=none\n"),
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

/// The locale's paper as CUPS names it (`A4`, `Letter`), to go with a PDF
/// drawn on [`paper`]. Needs GTK running.
#[cfg(not(windows))]
pub fn paper_name() -> String {
    let name = gtk::PaperSize::new(Some(&gtk::PaperSize::default()))
        .ppd_name()
        .to_string();
    if name.is_empty() { "A4".into() } else { name }
}

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

/// The longest line, in characters.
pub fn widest(text: &str) -> usize {
    text.lines().map(|l| l.chars().count()).max().unwrap_or(0)
}

/// Whether a job wants the paper turned: past 80 columns it does.
pub fn landscape(text: &str) -> bool {
    widest(text) > 80
}

/// How many lines fit between the margins of a page this tall.
pub fn lines_per_page(height: f64, margin: f64) -> usize {
    ((height - 2.0 * margin) / LINE_HEIGHT).floor().max(1.0) as usize
}

/// Draws one page's lines: the monospaced font at ten points, made smaller
/// where the widest line of the job would not otherwise fit `width`.
/// Shared by the PDF and the printer, so the two print alike.
pub fn draw_page(
    cr: &cairo::Context,
    lines: &[String],
    widest: usize,
    width: f64,
    margin: f64,
) -> Result<(), cairo::Error> {
    cr.select_font_face(FONT, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(FONT_SIZE);
    let advance = cr
        .text_extents("M")
        .map(|e| e.x_advance())
        .unwrap_or(FONT_SIZE * 0.6);
    let usable = width - 2.0 * margin;
    if widest > 0 && advance * widest as f64 > usable {
        cr.set_font_size(FONT_SIZE * usable / (advance * widest as f64));
    }
    cr.set_source_rgb(0.0, 0.0, 0.0);
    for (i, line) in lines.iter().enumerate() {
        cr.move_to(margin, margin + LINE_HEIGHT * (i as f64 + 0.8));
        cr.show_text(line)?;
    }
    Ok(())
}

/// Writes the text as a PDF on `paper` (width and height in points):
/// portrait for up to 80 columns, landscape past that, the font made smaller
/// if even landscape is too narrow. Returns how many pages it took.
pub fn write_pdf(text: &str, path: &Path, paper: (f64, f64)) -> io::Result<usize> {
    let widest = widest(text);
    let (short, long) = paper;
    let (width, height) = if landscape(text) {
        (long, short)
    } else {
        (short, long)
    };
    let surface = cairo::PdfSurface::new(width, height, path).map_err(io::Error::other)?;
    let cr = cairo::Context::new(&surface).map_err(io::Error::other)?;
    let pages = paginate(text, lines_per_page(height, MARGIN));
    for page in &pages {
        draw_page(&cr, page, widest, width, MARGIN).map_err(io::Error::other)?;
        cr.show_page().map_err(io::Error::other)?;
    }
    drop(cr);
    surface.finish();
    Ok(pages.len())
}

/// A quarter of an inch inside what the printer can print, which is already
/// inside the paper's edge.
#[cfg(windows)]
pub const PRINTER_MARGIN: f64 = 18.0;

/// A page to print when choosing a printer, as a DEC terminal's Set-Up could
/// print a test page: a ruler, and the character sets a host might send.
pub fn test_page() -> String {
    let mut text = String::from("veetee printer test\n\n");
    text.push_str(
        "         1         2         3         4         5         6         7         8\n",
    );
    text.push_str(&"1234567890".repeat(8));
    text.push_str("\n\n");
    text.push_str(" !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_\n");
    text.push_str("`abcdefghijklmnopqrstuvwxyz{|}~\n");
    text.push_str("¡¢£¤¥¦§¨©ª«¬\u{AD}®¯°±²³´µ¶·¸¹º»¼½¾¿ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖ×ØÙÚÛÜÝÞß\n");
    text.push_str("àáâãäåæçèéêëìíîïðñòóôõö÷øùúûüýþÿ\n\n");
    text.push_str("┌──┬──┐  ◆▒␉␌␍␊°±␤␋┘┐┌└┼⎺⎻─⎼⎽├┤┴┬│≤≥π≠£·\n");
    text.push_str("│  │  │\n├──┼──┤\n└──┴──┘\n");
    text
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

    #[cfg(not(windows))]
    #[test]
    fn a_job_goes_to_the_chosen_cups_printer_by_name() {
        let pdf = Path::new("/tmp/job.pdf");
        let args = lp_command("Canon_MX470_series", "veetee: vms1", pdf, "A4", false);
        assert_eq!(
            args,
            [
                "lp",
                "-d",
                "Canon_MX470_series",
                "-t",
                "veetee: vms1",
                "-o",
                "media=A4",
                "-o",
                "media-type=stationery",
                "--",
                "/tmp/job.pdf"
            ],
            "the paper named, whatever the printer's defaults"
        );
        let args = lp_command("Canon_MX470_series", "veetee", pdf, "A4", true);
        assert_eq!(
            &args[..3],
            ["flatpak-spawn", "--host", "lp"],
            "the host's CUPS"
        );
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
