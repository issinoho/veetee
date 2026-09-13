//! A host session: a transport plus the terminal it drives, run on an I/O thread.

use std::fs::File;
use std::io::{self, LineWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use vt_core::recording::Recorder;
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

/// Where and how to record a session.
#[derive(Debug, Clone)]
pub struct RecordOptions {
    pub path: PathBuf,
    /// Also record typed keys (which include passwords).
    pub keys: bool,
}

type SessionRecorder = Recorder<LineWriter<File>>;

struct Shared {
    term: Mutex<Terminal>,
    recorder: Mutex<Option<SessionRecorder>>,
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
    /// Starts driving `transport`. With `record`, the session is written as
    /// a `.vtrec` recording for `vt-headless replay`.
    pub fn start(
        config: Config,
        transport: Box<dyn Transport>,
        record: Option<&RecordOptions>,
    ) -> io::Result<(Session, async_channel::Receiver<Notice>)> {
        let recorder = match record {
            Some(r) => Some(Recorder::new(
                LineWriter::new(File::create(&r.path)?),
                &config,
                r.keys,
            )?),
            None => None,
        };
        let (tx, rx) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            writer: Mutex::new(transport.writer()?),
            term: Mutex::new(Terminal::new(config)),
            recorder: Mutex::new(recorder),
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
            .spawn(move || io_loop(transport, shared, tx))?;
        Ok((session, rx))
    }

    /// Locks the terminal for reading (rendering) or local changes.
    pub fn terminal(&self) -> MutexGuard<'_, Terminal> {
        self.shared.term.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn key(&self, key: Key) -> vt_core::KeyOutcome {
        self.key_with(key, vt_core::KeyMods::NONE)
    }

    pub fn key_with(&self, key: Key, mods: vt_core::KeyMods) -> vt_core::KeyOutcome {
        let (outcome, output) = {
            let mut term = self.terminal();
            let outcome = term.key_with(key, mods);
            (outcome, term.take_output())
        };
        self.send(&output);
        outcome
    }

    /// A main keypad key the host may have programmed (DECPAK).
    pub fn alphanumeric_key(
        &self,
        station: u8,
        mods: vt_core::KeyMods,
        alt_graph: bool,
    ) -> vt_core::KeyOutcome {
        let (outcome, output) = {
            let mut term = self.terminal();
            let outcome = term.alphanumeric_key(station, mods, alt_graph);
            (outcome, term.take_output())
        };
        self.send(&output);
        outcome
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

    /// Whether this session is being recorded.
    pub fn is_recording(&self) -> bool {
        self.shared
            .recorder
            .lock()
            .map(|r| r.is_some())
            .unwrap_or(false)
    }

    /// Adds a checkpoint to the recording and returns its name.
    pub fn mark_checkpoint(&self) -> Option<String> {
        let mut recorder = self
            .shared
            .recorder
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        recorder.as_mut().and_then(|r| r.mark(None).ok())
    }

    pub fn close(&self) {
        if let Some(r) = self
            .shared
            .recorder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = r.flush();
        }
        self.shared.closed.store(true, Ordering::Relaxed);
        self.set_held(false);
    }

    fn send(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        record(&self.shared, |r| r.keys(bytes));
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

/// Writes to the recording, dropping it if the file cannot be written.
fn record(shared: &Shared, f: impl FnOnce(&mut SessionRecorder) -> io::Result<()>) {
    let mut recorder = shared.recorder.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = recorder.as_mut() {
        if let Err(e) = f(r) {
            eprintln!("veetee: recording stopped: {e}");
            *recorder = None;
        }
    }
}

fn io_loop(
    mut transport: Box<dyn Transport>,
    shared: Arc<Shared>,
    tx: async_channel::Sender<Notice>,
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
        record(&shared, |r| r.host(&buf[..n]));
        let (reply, events, rows, cols) = {
            let mut term = shared.term.lock().unwrap_or_else(|e| e.into_inner());
            term.advance(&buf[..n]);
            let (rows, cols) = (term.grid().rows(), term.grid().cols());
            (term.take_output(), term.take_events(), rows, cols)
        };
        if !reply.is_empty() {
            record(&shared, |r| r.reply(&reply));
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
