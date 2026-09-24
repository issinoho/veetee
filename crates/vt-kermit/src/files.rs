//! Files on disk for a transfer, and a transfer to drive.
//!
//! The one part of this crate that touches the filesystem, kept apart so that
//! the protocol itself stays free of I/O and testable anywhere. It is what
//! both ends of veetee share: `vt-headless kermit` and the window.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::{Mode, Progress, Receiver, Sender, Settings, Source, Status, Store};

/// Received files, written into one directory.
///
/// A file is created under its own name only if nothing has that name: an
/// existing file is never overwritten, and the next free of `login.1.com`,
/// `login.2.com` and so on is used instead. A file that does not arrive
/// complete is removed.
#[derive(Debug)]
pub struct Folder {
    dir: PathBuf,
    open: Option<(File, PathBuf)>,
    saved: Vec<PathBuf>,
}

impl Folder {
    #[must_use]
    pub fn new(dir: PathBuf) -> Folder {
        Folder {
            dir,
            open: None,
            saved: Vec::new(),
        }
    }

    /// Where the files that arrived complete were written.
    #[must_use]
    pub fn saved(&self) -> &[PathBuf] {
        &self.saved
    }

    /// The directory files are received into.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// `login.com` as `login.N.com`, or `readme` as `readme.N`.
fn numbered(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}.{n}.{ext}"),
        _ => format!("{name}.{n}"),
    }
}

fn create_new(dir: &Path, name: &str) -> io::Result<(File, PathBuf)> {
    for n in 0..1000 {
        let candidate = if n == 0 {
            name.to_string()
        } else {
            numbered(name, n)
        };
        let path = dir.join(&candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        format!("{name} and a thousand numbered copies of it already exist"),
    ))
}

impl Store for Folder {
    fn create(&mut self, name: &str) -> Result<(), String> {
        let (file, path) = create_new(&self.dir, name).map_err(|e| format!("{name}: {e}"))?;
        self.open = Some((file, path));
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        let (file, path) = self.open.as_mut().ok_or("no file is open")?;
        file.write_all(data)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    fn finish(&mut self, complete: bool) -> Result<(), String> {
        let Some((file, path)) = self.open.take() else {
            return Ok(());
        };
        if complete {
            file.sync_all()
                .map_err(|e| format!("{}: {e}", path.display()))?;
            self.saved.push(path);
            Ok(())
        } else {
            drop(file);
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
        }
    }
}

/// Files to send, one after another, each under its own name without the
/// directory.
#[derive(Debug)]
pub struct Paths {
    paths: VecDeque<PathBuf>,
    open: Option<File>,
    size: Option<u64>,
    /// Look at each file to decide text or binary, as C-Kermit does.
    detect: bool,
    mode: Option<Mode>,
}

impl Paths {
    /// Files sent in the transfer's own mode.
    #[must_use]
    pub fn new(paths: Vec<PathBuf>) -> Paths {
        Paths {
            paths: paths.into(),
            open: None,
            size: None,
            detect: false,
            mode: None,
        }
    }

    /// Files each sent as text or binary by what is in them (see
    /// [`looks_like_text`]), and the far end told which in an attribute
    /// packet.
    #[must_use]
    pub fn deciding_each(paths: Vec<PathBuf>) -> Paths {
        Paths {
            detect: true,
            ..Paths::new(paths)
        }
    }
}

/// How much of a file is looked at to decide whether it is text.
const SAMPLE: usize = 8192;

/// Whether a file that starts with `sample` is text.
///
/// Text has no nulls, and is either UTF-8 or nearly all characters a DEC
/// terminal would show: printable ASCII, the Latin-1 and DEC Supplemental
/// upper half, and the few controls text uses — tab, line feed, form feed,
/// carriage return, backspace and escape, the last for files of VT
/// sequences. More than one byte in a hundred outside that, and it is not.
#[must_use]
pub fn looks_like_text(sample: &[u8]) -> bool {
    if sample.contains(&0) {
        return false;
    }
    // In UTF-8, bytes from 0x80 to 0x9F are parts of characters; in a
    // Latin-1 or DEC Supplemental file they are C1 controls, which text does
    // not have. A sample cut off part way through a character is still UTF-8.
    let utf8 = match std::str::from_utf8(sample) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    let odd = sample
        .iter()
        .filter(|&&b| match b {
            0x08 | 0x09 | 0x0a | 0x0c | 0x0d | 0x1b => false,
            0x00..=0x1f | 0x7f => true,
            0x80..=0x9f => !utf8,
            _ => false,
        })
        .count();
    odd * 100 <= sample.len()
}

impl Source for Paths {
    fn next_file(&mut self) -> Result<Option<String>, String> {
        let Some(path) = self.paths.pop_front() else {
            self.open = None;
            return Ok(None);
        };
        let shown = |e: io::Error| format!("{}: {e}", path.display());
        let mut file = File::open(&path).map_err(shown)?;
        self.size = file.metadata().ok().map(|m| m.len());
        self.mode = if self.detect {
            let mut sample = Vec::with_capacity(SAMPLE);
            (&mut file)
                .take(SAMPLE as u64)
                .read_to_end(&mut sample)
                .map_err(shown)?;
            file.seek(SeekFrom::Start(0)).map_err(shown)?;
            Some(if looks_like_text(&sample) {
                Mode::Text
            } else {
                Mode::Binary
            })
        } else {
            None
        };
        self.open = Some(file);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{}: no file name", path.display()))?;
        Ok(Some(name))
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let file = self.open.as_mut().ok_or("no file is open")?;
        file.read(buf).map_err(|e| e.to_string())
    }

    fn size(&self) -> Option<u64> {
        self.size
    }

    fn mode(&self) -> Option<Mode> {
        self.mode
    }
}

/// How long a finished receiver keeps the line: long enough to answer the
/// sender asking again for the end of the batch, if its answer was lost,
/// rather than letting that packet land on the screen.
const LINGER: Duration = Duration::from_secs(2);

/// A transfer in either direction, with its files: what a front end drives.
#[derive(Debug)]
pub struct Transfer {
    end: End,
    finished: Option<Instant>,
}

#[derive(Debug)]
enum End {
    Receiving(Receiver, Folder),
    Sending(Sender, Paths),
}

impl Transfer {
    /// A transfer that waits for the far end's send-init.
    #[must_use]
    pub fn receive(into: Folder, settings: Settings, now: Instant) -> Transfer {
        Transfer {
            end: End::Receiving(Receiver::new(settings, now), into),
            finished: None,
        }
    }

    /// A transfer that sends, and the send-init that begins it.
    #[must_use]
    pub fn send(files: Paths, settings: Settings, now: Instant) -> (Transfer, Vec<u8>) {
        let mut sender = Sender::new(settings);
        let first = sender.start(now);
        (
            Transfer {
                end: End::Sending(sender, files),
                finished: None,
            },
            first,
        )
    }

    /// Bytes from the line. Returns what to send, the tick included, so a
    /// caller that reads with a timeout need call nothing else.
    pub fn feed(&mut self, bytes: &[u8], now: Instant) -> Vec<u8> {
        let mut out = match &mut self.end {
            End::Receiving(r, folder) => r.feed(bytes, now, folder),
            End::Sending(s, paths) => s.feed(bytes, now, paths),
        };
        out.extend(match &mut self.end {
            End::Receiving(r, folder) => r.tick(now, folder),
            End::Sending(s, _) => s.tick(now),
        });
        if self.finished.is_none() && *self.status() != Status::Running {
            self.finished = Some(now);
        }
        out
    }

    /// The user wants to stop: tidily the first time, at once the second.
    pub fn cancel(&mut self) -> Vec<u8> {
        match &mut self.end {
            End::Receiving(r, folder) => r.cancel(folder),
            End::Sending(s, _) => s.cancel(),
        }
    }

    #[must_use]
    pub fn status(&self) -> &Status {
        match &self.end {
            End::Receiving(r, _) => r.status(),
            End::Sending(s, _) => s.status(),
        }
    }

    #[must_use]
    pub fn progress(&self) -> &Progress {
        match &self.end {
            End::Receiving(r, _) => r.progress(),
            End::Sending(s, _) => s.progress(),
        }
    }

    #[must_use]
    pub fn is_receiving(&self) -> bool {
        matches!(self.end, End::Receiving(..))
    }

    /// Where received files were written; empty when sending.
    #[must_use]
    pub fn saved(&self) -> &[PathBuf] {
        match &self.end {
            End::Receiving(_, folder) => folder.saved(),
            End::Sending(..) => &[],
        }
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        *self.status() == Status::Running
    }

    /// Whether what arrives on the line is still the transfer's: while it
    /// runs, and for a moment after a receiver finishes.
    #[must_use]
    pub fn wants_line(&self, now: Instant) -> bool {
        match (self.status(), self.finished) {
            (Status::Running, _) => true,
            (Status::Done, Some(at)) if self.is_receiving() => now < at + LINGER,
            _ => false,
        }
    }

    /// Whether `bytes`, just arrived, are the transfer's rather than the
    /// terminal's. While it runs, everything is. In the moment a receiver
    /// lingers, only what starts with a packet mark is: the host's Kermit
    /// prints its prompt as soon as it has finished, and that belongs on the
    /// screen, where a repeated packet does not.
    #[must_use]
    pub fn claims(&self, bytes: &[u8], now: Instant) -> bool {
        if self.is_running() {
            return true;
        }
        self.wants_line(now) && bytes.first() == Some(&crate::MARK)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vt-kermit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_taken_name_gets_a_number_and_keeps_its_type() {
        assert_eq!(numbered("login.com", 1), "login.1.com");
        assert_eq!(numbered("readme", 2), "readme.2");
        assert_eq!(numbered(".profile", 1), ".profile.1");
        assert_eq!(numbered("a.tar.gz", 3), "a.tar.3.gz");
    }

    #[test]
    fn nothing_is_overwritten_and_nothing_half_done_is_kept() {
        let dir = scratch("folder");
        std::fs::write(dir.join("login.com"), b"keep me").unwrap();
        let mut folder = Folder::new(dir.clone());
        folder.create("login.com").unwrap();
        folder.write(b"new").unwrap();
        folder.finish(true).unwrap();
        assert_eq!(std::fs::read(dir.join("login.com")).unwrap(), b"keep me");
        assert_eq!(std::fs::read(dir.join("login.1.com")).unwrap(), b"new");
        assert_eq!(folder.saved(), [dir.join("login.1.com")]);

        folder.create("partial.dat").unwrap();
        folder.write(b"half").unwrap();
        folder.finish(false).unwrap();
        assert!(!dir.join("partial.dat").exists());
        assert_eq!(folder.saved().len(), 1, "and it is not counted");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn text_is_told_from_binary_by_looking() {
        assert!(looks_like_text(b"$ SET DEFAULT SYS$LOGIN\r\n$ EXIT\n"));
        assert!(looks_like_text("naïve café\n".as_bytes()), "UTF-8");
        assert!(
            looks_like_text(b"na\xefve caf\xe9\n"),
            "Latin-1 or DEC Supplemental"
        );
        assert!(
            looks_like_text(b"\x1b[1mbold\x1b[m\tand a tab\x0c"),
            "VT sequences"
        );
        assert!(looks_like_text(
            "ends mid-character \u{e9}"
                .as_bytes()
                .split_last()
                .unwrap()
                .1
        ));
        assert!(looks_like_text(b""), "an empty file is text");
        assert!(!looks_like_text(b"ELF\x00\x01\x02"), "a null");
        assert!(
            !looks_like_text(&(1..=255).collect::<Vec<u8>>()),
            "every byte"
        );
        let mostly = [b"text ".repeat(40).as_slice(), b"\x01\x02\x03"].concat();
        assert!(!looks_like_text(&mostly), "three odd bytes in two hundred");
    }

    #[test]
    fn deciding_each_file_reads_it_all_the_same() {
        let dir = scratch("paths");
        std::fs::write(dir.join("notes.txt"), b"one\ntwo\n").unwrap();
        let binary: Vec<u8> = (0..=255).collect();
        std::fs::write(dir.join("every.dat"), &binary).unwrap();
        let mut paths = Paths::deciding_each(vec![dir.join("notes.txt"), dir.join("every.dat")]);
        let read_all = |paths: &mut Paths| {
            let mut all = Vec::new();
            let mut buf = [0u8; 7];
            loop {
                let n = paths.read(&mut buf).unwrap();
                if n == 0 {
                    return all;
                }
                all.extend_from_slice(&buf[..n]);
            }
        };
        assert_eq!(paths.next_file().unwrap().as_deref(), Some("notes.txt"));
        assert_eq!((paths.mode(), paths.size()), (Some(Mode::Text), Some(8)));
        assert_eq!(
            read_all(&mut paths),
            b"one\ntwo\n",
            "from the start, after looking"
        );
        assert_eq!(paths.next_file().unwrap().as_deref(), Some("every.dat"));
        assert_eq!(paths.mode(), Some(Mode::Binary));
        assert_eq!(read_all(&mut paths), binary);
        assert_eq!(paths.next_file().unwrap(), None);
        assert_eq!(Paths::new(vec![dir.join("notes.txt")]).mode(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_whole_transfer_between_two_folders() {
        let dir = scratch("transfer");
        let (from, to) = (dir.join("from"), dir.join("to"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(from.join("notes.txt"), b"one\ntwo\n").unwrap();
        std::fs::write(from.join("every.dat"), (0..=255).collect::<Vec<u8>>()).unwrap();

        let now = Instant::now();
        let files = Paths::deciding_each(vec![from.join("notes.txt"), from.join("every.dat")]);
        let (mut sending, mut out) = Transfer::send(files, Settings::default(), now);
        // The receiver is set to binary: the sender's word has to win for
        // the text file to arrive as text.
        let receiving_settings = Settings {
            mode: Mode::Binary,
            local: crate::LineEnding::Lf,
            ..Settings::default()
        };
        let mut receiving = Transfer::receive(Folder::new(to.clone()), receiving_settings, now);
        for _ in 0..100 {
            let back = receiving.feed(&out, now);
            out = sending.feed(&back, now);
        }
        assert_eq!(*sending.status(), Status::Done);
        assert_eq!(*receiving.status(), Status::Done);
        assert_eq!(std::fs::read(to.join("notes.txt")).unwrap(), b"one\ntwo\n");
        assert_eq!(
            std::fs::read(to.join("every.dat")).unwrap(),
            (0..=255).collect::<Vec<u8>>()
        );
        assert_eq!(receiving.saved().len(), 2);
        assert!(receiving.wants_line(now), "a finished receiver lingers");
        assert!(
            receiving.claims(b"\x01#$B+\r", now),
            "for a packet sent again"
        );
        assert!(
            !receiving.claims(b"Kermit-32>", now),
            "and not the host's prompt"
        );
        assert!(!receiving.claims(b"", now));
        assert!(!receiving.is_running());
        assert!(!receiving.wants_line(now + LINGER));
        assert!(!sending.wants_line(now), "a finished sender does not");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
