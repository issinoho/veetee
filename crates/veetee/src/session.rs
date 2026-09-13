//! A host session: a transport plus the terminal it drives, run on an I/O thread.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use vt_core::{Config, Event, Key, Terminal};
use vt_transport::{Transport, TransportWriter};

/// Notifications from the I/O thread to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Redraw,
    Bell,
    /// The host named the session (DECSWT).
    Title(String),
    /// The host made this session active (DECES).
    Activate,
    /// The connection closed; the text says why when known.
    Exited(Option<String>),
}

struct Shared {
    term: Mutex<Terminal>,
    writer: Mutex<Box<dyn TransportWriter>>,
    redraw_pending: AtomicBool,
    /// Hold Screen: the I/O thread stops reading, so the host is flow-controlled.
    held: Mutex<bool>,
    resume: Condvar,
    closed: AtomicBool,
}

/// Handle used by the UI thread.
#[derive(Clone)]
pub struct Session {
    shared: Arc<Shared>,
    notices: async_channel::Sender<Notice>,
}

impl Session {
    /// Starts driving `transport`. With `record`, every byte received from
    /// the host is appended to that file for replay with `vt-headless trace`.
    pub fn start(
        config: Config,
        transport: Box<dyn Transport>,
        record: Option<&Path>,
    ) -> io::Result<(Session, async_channel::Receiver<Notice>)> {
        let recorder = match record {
            Some(path) => Some(OpenOptions::new().create(true).append(true).open(path)?),
            None => None,
        };
        let (tx, rx) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            writer: Mutex::new(transport.writer()?),
            term: Mutex::new(Terminal::new(config)),
            redraw_pending: AtomicBool::new(false),
            held: Mutex::new(false),
            resume: Condvar::new(),
            closed: AtomicBool::new(false),
        });
        let session = Session {
            shared: shared.clone(),
            notices: tx.clone(),
        };
        thread::Builder::new()
            .name("veetee-io".into())
            .spawn(move || io_loop(transport, shared, tx, recorder))?;
        Ok((session, rx))
    }

    /// Locks the terminal for reading (rendering) or local changes.
    pub fn terminal(&self) -> MutexGuard<'_, Terminal> {
        self.shared.term.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn key(&self, key: Key) {
        let output = {
            let mut term = self.terminal();
            term.key(key);
            term.take_output()
        };
        self.send(&output);
    }

    pub fn type_text(&self, text: &str) {
        let output = {
            let mut term = self.terminal();
            term.type_text(text);
            term.take_output()
        };
        self.send(&output);
    }

    pub fn send_answerback(&self) {
        let output = {
            let mut term = self.terminal();
            term.send_answerback();
            term.take_output()
        };
        self.send(&output);
    }

    /// The DEC Break key.
    pub fn send_break(&self) -> io::Result<()> {
        let mut writer = self.shared.writer.lock().unwrap_or_else(|e| e.into_inner());
        writer.send_break()
    }

    pub fn is_held(&self) -> bool {
        *self.shared.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_held(&self, held: bool) {
        *self.shared.held.lock().unwrap_or_else(|e| e.into_inner()) = held;
        self.shared.resume.notify_all();
    }

    pub fn close(&self) {
        self.shared.closed.store(true, Ordering::Relaxed);
        self.set_held(false);
    }

    fn send(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut writer = self.shared.writer.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = writer.write_all(bytes) {
            let _ = self.notices.try_send(Notice::Exited(Some(e.to_string())));
        }
        // Local echo may have changed the screen.
        request_redraw(&self.shared, &self.notices);
    }
}

fn request_redraw(shared: &Shared, tx: &async_channel::Sender<Notice>) {
    if !shared.redraw_pending.swap(true, Ordering::AcqRel) {
        let _ = tx.try_send(Notice::Redraw);
    }
}

/// Called by the UI after it has drawn, so the next change schedules a new frame.
pub fn frame_drawn(session: &Session) {
    session
        .shared
        .redraw_pending
        .store(false, Ordering::Release);
}

fn io_loop(
    mut transport: Box<dyn Transport>,
    shared: Arc<Shared>,
    tx: async_channel::Sender<Notice>,
    mut recorder: Option<File>,
) {
    let mut buf = vec![0u8; 64 * 1024];
    let reason = loop {
        {
            let mut held = shared.held.lock().unwrap_or_else(|e| e.into_inner());
            while *held && !shared.closed.load(Ordering::Relaxed) {
                held = shared.resume.wait(held).unwrap_or_else(|e| e.into_inner());
            }
        }
        if shared.closed.load(Ordering::Relaxed) {
            break None;
        }
        let n = match transport.read_timeout(&mut buf, Duration::from_millis(250)) {
            Ok(0) => continue,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                break e.get_ref().map(|inner| inner.to_string());
            }
            Err(e) => break Some(e.to_string()),
        };
        if let Some(file) = recorder.as_mut() {
            if file.write_all(&buf[..n]).is_err() {
                recorder = None;
            }
        }
        let (reply, events, rows, cols) = {
            let mut term = shared.term.lock().unwrap_or_else(|e| e.into_inner());
            term.advance(&buf[..n]);
            let (rows, cols) = (term.grid().rows(), term.grid().cols());
            (term.take_output(), term.take_events(), rows, cols)
        };
        if !reply.is_empty() {
            let mut writer = shared.writer.lock().unwrap_or_else(|e| e.into_inner());
            let _ = writer.write_all(&reply);
        }
        for event in events {
            match event {
                Event::Bell => {
                    let _ = tx.try_send(Notice::Bell);
                }
                // The host addresses the whole page, so that is its size.
                Event::ColumnsChanged(_) | Event::LinesChanged(_) => {
                    let _ = transport.resize(rows as u16, cols as u16);
                }
                Event::TitleChanged(title) => {
                    let _ = tx.try_send(Notice::Title(title));
                }
                Event::SessionActivated => {
                    let _ = tx.try_send(Notice::Activate);
                }
                // Tones (DECPS) arrive with bell and keyclick sounds (M7).
                Event::ScreenLinesChanged(_)
                | Event::IconNameChanged(_)
                | Event::PlaySound { .. } => {}
                Event::LedsChanged(_) => {}
            }
        }
        request_redraw(&shared, &tx);
    };
    let _ = tx.try_send(Notice::Exited(reason));
}
