//! Session logs: the text a session shows, or the raw bytes the host sends,
//! written to a file as they arrive.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use gtk::glib;

/// Where and how to log a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogOptions {
    pub path: PathBuf,
    /// The host's bytes exactly as received, instead of the text shown.
    pub raw: bool,
    /// Start each line of text with the date and time.
    pub timestamps: bool,
    /// Add to an existing file rather than replacing it.
    pub append: bool,
}

pub struct Logger {
    out: BufWriter<File>,
    raw: bool,
    timestamps: bool,
    at_line_start: bool,
    path: PathBuf,
}

impl Logger {
    pub fn open(options: &LogOptions) -> io::Result<Logger> {
        if let Some(dir) = options.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(options.append)
            .truncate(!options.append)
            .open(&options.path)?;
        // An appended text log continues on a line of its own.
        let at_line_start = true;
        let mut logger = Logger {
            out: BufWriter::new(file),
            raw: options.raw,
            timestamps: options.timestamps,
            at_line_start,
            path: options.path.clone(),
        };
        if options.append && !options.raw && std::fs::metadata(&options.path)?.len() > 0 {
            logger.out.write_all(b"\n")?;
        }
        Ok(logger)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_raw(&self) -> bool {
        self.raw
    }

    /// Bytes from the host (raw logs only).
    pub fn host(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.raw {
            self.out.write_all(bytes)?;
            self.out.flush()?;
        }
        Ok(())
    }

    /// Text the session showed (text logs only).
    pub fn text(&mut self, text: &str) -> io::Result<()> {
        if self.raw || text.is_empty() {
            return Ok(());
        }
        for line in text.split_inclusive('\n') {
            if self.at_line_start && self.timestamps {
                write!(self.out, "[{}] ", now("%Y-%m-%d %H:%M:%S"))?;
            }
            self.out.write_all(line.as_bytes())?;
            self.at_line_start = line.ends_with('\n');
        }
        self.out.flush()
    }
}

fn now(format: &str) -> String {
    glib::DateTime::now_local()
        .and_then(|t| t.format(format))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// A log path from a template: a leading `~` is the home directory, and
/// `%Y %m %d %H %M %S` are replaced with the date and time.
pub fn expand_path(template: &str) -> PathBuf {
    let mut text = template.to_string();
    if let Some(rest) = text
        .strip_prefix("~/")
        .or(if text == "~" { Some("") } else { None })
    {
        text = glib::home_dir().join(rest).to_string_lossy().into_owned();
    }
    if text.contains('%') {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, chars.peek()) {
                ('%', Some(&f @ ('Y' | 'm' | 'd' | 'H' | 'M' | 'S'))) => {
                    chars.next();
                    out.push_str(&now(&format!("%{f}")));
                }
                ('%', Some('%')) => {
                    chars.next();
                    out.push('%');
                }
                _ => out.push(c),
            }
        }
        text = out;
    }
    PathBuf::from(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("veetee-log-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn text_logs_can_stamp_each_line() {
        let path = temp("stamped");
        let mut log = Logger::open(&LogOptions {
            path: path.clone(),
            raw: false,
            timestamps: true,
            append: false,
        })
        .unwrap();
        log.text("$ SHOW TIME\n  14-SEP").unwrap();
        log.text("-2026 13:05\n").unwrap();
        log.host(b"ignored").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with('[') && lines[0].ends_with("] $ SHOW TIME"),
            "{text}"
        );
        assert!(lines[1].ends_with("]   14-SEP-2026 13:05"), "{text}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn raw_logs_append() {
        let path = temp("raw");
        let _ = std::fs::remove_file(&path);
        for part in [&b"\x1b[H"[..], b"more"] {
            let mut log = Logger::open(&LogOptions {
                path: path.clone(),
                raw: true,
                timestamps: true,
                append: true,
            })
            .unwrap();
            log.host(part).unwrap();
            log.text("ignored").unwrap();
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"\x1b[Hmore");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn paths_expand_home_and_dates() {
        let p = expand_path("~/logs/vms1-%Y%m%d-100%%.log");
        let s = p.to_string_lossy();
        assert!(s.starts_with(&*glib::home_dir().to_string_lossy()));
        assert!(!s.contains("%Y") && s.ends_with("-100%.log"), "{s}");
        assert_eq!(expand_path("plain.log"), PathBuf::from("plain.log"));
    }
}
