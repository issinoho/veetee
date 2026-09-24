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
use vt_kermit::files::Transfer;
use vt_kermit::{Progress, Status};
use vt_transport::{Transport, TransportWriter};

/// Notifications from the I/O thread to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Redraw,
    /// A bell or DECPS note, at the volume the terminal selected.
    Sound(crate::sound::Sound),
    /// The host named the session (DECSWT).
    Title(String),
    /// The host made this session active (DECES).
    Activate,
    /// A line started scrolling smoothly; keep redrawing until it settles.
    SmoothScroll,
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

/// A smooth scroll in progress.
#[derive(Debug, Clone)]
pub struct ScrollAnimation {
    pub scroll: vt_core::SmoothScroll,
    pub start: std::time::Instant,
    pub duration: Duration,
}

impl ScrollAnimation {
    /// How far the line has moved, 0 to 1.
    pub fn progress(&self) -> f32 {
        (self.start.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.0)
    }
}

struct Shared {
    term: Mutex<Terminal>,
    recorder: Mutex<Option<SessionRecorder>>,
    logger: Mutex<Option<crate::log::Logger>>,
    writer: Mutex<Box<dyn TransportWriter>>,
    redraw_pending: AtomicBool,
    /// The line scrolling smoothly now, set together with the scroll itself.
    scroll: Mutex<Option<ScrollAnimation>>,
    /// Hold Screen: the I/O thread stops reading, so the host is flow-controlled.
    held: Mutex<bool>,
    resume: Condvar,
    closed: AtomicBool,
    /// A Kermit transfer using the line. While it wants the line, what the
    /// host sends goes to it rather than the terminal, and typed keys are
    /// dropped. It stays here once finished until the UI takes it.
    transfer: Mutex<Option<Transfer>>,
}

/// Where a Kermit transfer has got to, for the UI.
#[derive(Debug, Clone)]
pub struct TransferState {
    pub status: Status,
    pub progress: Progress,
    /// Where received files were written.
    pub saved: Vec<PathBuf>,
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
        log: Option<&crate::log::LogOptions>,
    ) -> io::Result<(Session, async_channel::Receiver<Notice>)> {
        let recorder = match record {
            Some(r) => Some(Recorder::new(
                LineWriter::new(File::create(&r.path)?),
                &config,
                r.keys,
            )?),
            None => None,
        };
        let logger = log.map(crate::log::Logger::open).transpose()?;
        let mut term = Terminal::new(config);
        term.set_capture(logger.as_ref().is_some_and(|l| !l.is_raw()));
        let (tx, rx) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            writer: Mutex::new(transport.writer()?),
            term: Mutex::new(term),
            recorder: Mutex::new(recorder),
            logger: Mutex::new(logger),
            redraw_pending: AtomicBool::new(false),
            scroll: Mutex::new(None),
            held: Mutex::new(false),
            resume: Condvar::new(),
            closed: AtomicBool::new(false),
            transfer: Mutex::new(None),
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

    /// The smooth scroll being shown, if it has not finished. Read it while
    /// holding [`Session::terminal`] so it matches the page.
    pub fn scroll_animation(&self) -> Option<ScrollAnimation> {
        let anim = self.shared.scroll.lock().unwrap_or_else(|e| e.into_inner());
        anim.clone().filter(|a| a.progress() < 1.0)
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

    /// Pasted text, translated for the host's character sets.
    pub fn paste(&self, text: &str) {
        let output = {
            let mut term = self.terminal();
            term.paste(text);
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

    /// The file this session is logged to, if any.
    pub fn log_path(&self) -> Option<PathBuf> {
        let logger = self.shared.logger.lock().unwrap_or_else(|e| e.into_inner());
        logger.as_ref().map(|l| l.path().to_path_buf())
    }

    /// Starts logging to a file, replacing any log in progress.
    pub fn start_log(&self, options: &crate::log::LogOptions) -> io::Result<()> {
        let logger = crate::log::Logger::open(options)?;
        let mut term = self.terminal();
        let mut current = self.shared.logger.lock().unwrap_or_else(|e| e.into_inner());
        term.set_capture(!logger.is_raw());
        *current = Some(logger);
        Ok(())
    }

    /// Stops logging, writing out what the terminal has shown so far.
    pub fn stop_log(&self) {
        let mut term = self.terminal();
        let text = term.take_captured_text();
        term.set_capture(false);
        let mut current = self.shared.logger.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut logger) = current.take() {
            let _ = logger.text(&text);
        }
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

    /// Starts a Kermit transfer on this session's line, sending `first` —
    /// a sender's send-init — to begin it.
    pub fn start_transfer(&self, transfer: Transfer, first: &[u8]) -> Result<(), String> {
        let mut slot = self
            .shared
            .transfer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(Transfer::is_running) {
            return Err("a transfer is already running in this session".into());
        }
        *slot = Some(transfer);
        drop(slot);
        write_line(&self.shared, first);
        Ok(())
    }

    /// Asks the transfer to stop: tidily the first time, at once the second.
    pub fn cancel_transfer(&self) {
        let out = {
            let mut slot = self
                .shared
                .transfer
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            slot.as_mut().map(Transfer::cancel).unwrap_or_default()
        };
        write_line(&self.shared, &out);
    }

    /// Where the transfer has got to, if there is one.
    pub fn transfer(&self) -> Option<TransferState> {
        let slot = self
            .shared
            .transfer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        slot.as_ref().map(|t| TransferState {
            status: t.status().clone(),
            progress: t.progress().clone(),
            saved: t.saved().to_vec(),
        })
    }

    /// Removes a transfer that has finished with the line, which a receiver
    /// has done a couple of seconds after it finishes.
    pub fn clear_transfer(&self) {
        let mut slot = self
            .shared
            .transfer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if slot
            .as_ref()
            .is_some_and(|t| !t.wants_line(std::time::Instant::now()))
        {
            *slot = None;
        }
    }

    fn send(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // Keys typed during a transfer would land in the middle of its
        // packets, where they are noise at best.
        let transferring = {
            let slot = self
                .shared
                .transfer
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            slot.as_ref().is_some_and(Transfer::is_running)
        };
        if transferring {
            return;
        }
        {
            // Local (Global Set-Up): typed characters go to the screen.
            let mut term = self.terminal();
            if !term.on_line() {
                term.advance(bytes);
                drop(term);
                request_redraw(&self.shared, &self.notices);
                return;
            }
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

/// Writes a transfer's packets to the line, and to any recording, which is a
/// record of the line.
fn write_line(shared: &Shared, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    record(shared, |r| r.reply(bytes));
    let mut writer = shared.writer.lock().unwrap_or_else(|e| e.into_inner());
    let _ = writer.write_all(bytes).and_then(|()| writer.flush());
}

fn request_redraw(shared: &Shared, tx: &async_channel::Sender<Notice>) {
    if !shared.redraw_pending.swap(true, Ordering::AcqRel) {
        let _ = tx.try_send(Notice::Redraw);
    }
}

/// Called by the UI when it handles a redraw notice, so the next change sends another.
pub fn frame_drawn(session: &Session) {
    session
        .shared
        .redraw_pending
        .store(false, Ordering::Release);
}

/// Writes to the recording, dropping it if the file cannot be written.
fn record(shared: &Shared, f: impl FnOnce(&mut SessionRecorder) -> io::Result<()>) {
    let mut recorder = shared.recorder.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = recorder.as_mut()
        && let Err(e) = f(r)
    {
        eprintln!("veetee: recording stopped: {e}");
        *recorder = None;
    }
}

/// Writes to the log, stopping it if the file cannot be written.
fn log(shared: &Shared, f: impl FnOnce(&mut crate::log::Logger) -> io::Result<()>) {
    let mut logger = shared.logger.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(l) = logger.as_mut()
        && let Err(e) = f(l)
    {
        eprintln!("veetee: log {} stopped: {e}", l.path().display());
        *logger = None;
    }
}

/// Waits while the session is held; returns false once it is closed.
fn wait_while_held(shared: &Shared) -> bool {
    let mut held = shared.held.lock().unwrap_or_else(|e| e.into_inner());
    while *held && !shared.closed.load(Ordering::Relaxed) {
        held = shared.resume.wait(held).unwrap_or_else(|e| e.into_inner());
    }
    !shared.closed.load(Ordering::Relaxed)
}

fn io_loop(
    mut transport: Box<dyn Transport>,
    shared: Arc<Shared>,
    tx: async_channel::Sender<Notice>,
) {
    let mut buf = vec![0u8; 64 * 1024];
    let reason = loop {
        if !wait_while_held(&shared) {
            break None;
        }
        let n = match transport.read_timeout(&mut buf, Duration::from_millis(250)) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                break e.get_ref().map(|inner| inner.to_string());
            }
            Err(e) => break Some(e.to_string()),
        };
        if n > 0 {
            record(&shared, |r| r.host(&buf[..n]));
        }
        // A transfer that wants the line has it: its packets are not for the
        // screen, and not text for the log. It is fed on every pass, whether
        // anything arrived or not, since that is also how its time passes.
        let routed = {
            let mut slot = shared.transfer.lock().unwrap_or_else(|e| e.into_inner());
            let now = std::time::Instant::now();
            match slot.as_mut() {
                Some(t) if t.claims(&buf[..n], now) => Some(t.feed(&buf[..n], now)),
                _ => None,
            }
        };
        if let Some(out) = routed {
            write_line(&shared, &out);
            continue;
        }
        if n > 0 {
            log(&shared, |l| l.host(&buf[..n]));
        }
        let mut rest = &buf[..n];
        loop {
            let step = {
                let mut term = shared.term.lock().unwrap_or_else(|e| e.into_inner());
                let used = if rest.is_empty() {
                    0
                } else {
                    term.advance_paced(rest)
                };
                rest = &rest[used..];
                let scroll = term.take_smooth_scroll();
                let rate = term.smooth_scroll_rate();
                if let (Some(scroll), Some(rate)) = (&scroll, rate) {
                    *shared.scroll.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(ScrollAnimation {
                            scroll: scroll.clone(),
                            start: std::time::Instant::now(),
                            duration: Duration::from_millis(1000 / u64::from(rate)),
                        });
                }
                Step {
                    text: term.take_captured_text(),
                    reply: term.take_output(),
                    events: term.take_events(),
                    rows: term.grid().rows(),
                    cols: term.grid().cols(),
                    volumes: term.sound_volumes(),
                    scroll,
                    rate,
                }
            };
            let busy = n > 0 || !step.reply.is_empty() || !step.events.is_empty();
            handle_step(&step, &shared, transport.as_mut(), &tx);
            if busy {
                request_redraw(&shared, &tx);
            }
            if let (Some(scroll), Some(rate)) = (step.scroll, step.rate) {
                // Smooth scroll: show this line moving before processing more,
                // which holds the host back as a DEC terminal does.
                let _ = scroll;
                let duration = Duration::from_millis(1000 / u64::from(rate));
                let _ = tx.try_send(Notice::SmoothScroll);
                thread::sleep(duration);
                if !wait_while_held(&shared) {
                    break;
                }
            }
            if rest.is_empty() {
                break;
            }
        }
    };
    let _ = tx.try_send(Notice::Exited(reason));
}

/// What one step of processing produced.
struct Step {
    text: String,
    reply: Vec<u8>,
    events: Vec<Event>,
    rows: usize,
    cols: usize,
    volumes: vt_core::SoundVolumes,
    scroll: Option<vt_core::SmoothScroll>,
    rate: Option<u32>,
}

fn handle_step(
    step: &Step,
    shared: &Shared,
    transport: &mut dyn Transport,
    tx: &async_channel::Sender<Notice>,
) {
    if !step.text.is_empty() {
        log(shared, |l| l.text(&step.text));
    }
    if !step.reply.is_empty() {
        record(shared, |r| r.reply(&step.reply));
        let mut writer = shared.writer.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writer.write_all(&step.reply);
    }
    let volumes = step.volumes;
    for event in &step.events {
        match event {
            Event::Bell => {
                let _ = tx.try_send(Notice::Sound(crate::sound::Sound::Bell(
                    volumes.warning_bell,
                )));
            }
            Event::MarginBell => {
                let _ = tx.try_send(Notice::Sound(crate::sound::Sound::Bell(
                    volumes.margin_bell,
                )));
            }
            Event::PlaySound {
                volume,
                duration_ms,
                note,
            } => {
                let _ = tx.try_send(Notice::Sound(crate::sound::Sound::Note {
                    volume: *volume,
                    duration_ms: *duration_ms,
                    note: *note,
                }));
            }
            // The host addresses the whole page, so that is its size.
            Event::ColumnsChanged(_) | Event::LinesChanged(_) => {
                let _ = transport.resize(step.rows as u16, step.cols as u16);
            }
            Event::TitleChanged(title) => {
                let _ = tx.try_send(Notice::Title(title.clone()));
            }
            Event::SessionActivated => {
                let _ = tx.try_send(Notice::Activate);
            }
            Event::ScreenLinesChanged(_) | Event::IconNameChanged(_) => {}
            Event::LedsChanged(_) => {}
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use vt_transport::pty::Pty;

    #[test]
    fn a_log_started_later_gets_the_text_from_then_on() {
        let dir = std::env::temp_dir().join(format!("veetee-session-log-{}", std::process::id()));
        let path = dir.join("later.log");
        let pty = Pty::spawn(
            "/bin/sh",
            &[
                "-c",
                "printf 'before\\r\\n'; sleep 1; printf 'after \\033[1mbold\\033[m\\r\\n'; sleep 1",
            ],
            24,
            80,
            "vt420",
        )
        .unwrap();
        let (session, notices) =
            Session::start(Config::default(), Box::new(pty), None, None).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        session
            .start_log(&crate::log::LogOptions {
                path: path.clone(),
                raw: false,
                timestamps: false,
                append: false,
            })
            .unwrap();
        assert_eq!(session.log_path().as_deref(), Some(path.as_path()));
        // Wait for the program to finish.
        while let Ok(notice) = notices.recv_blocking() {
            if matches!(notice, Notice::Exited(_)) {
                break;
            }
        }
        session.stop_log();
        assert_eq!(session.log_path(), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after bold\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A whole receive through a session, against G-Kermit on a pty: the
    /// file arrives, the screen shows what the host printed either side of
    /// the transfer and none of its packets, and keys typed meanwhile go
    /// nowhere.
    #[test]
    fn a_kermit_transfer_takes_the_line_and_gives_it_back() {
        let have_gkermit = std::process::Command::new("sh")
            .args(["-c", "command -v gkermit"])
            .output()
            .is_ok_and(|o| o.status.success());
        if !have_gkermit {
            assert!(
                std::env::var_os("VEETEE_REQUIRE_KERMITS").is_none(),
                "gkermit is not installed"
            );
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("veetee-session-kermit-{}", std::process::id()));
        let (from, into) = (dir.join("from"), dir.join("into"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&into).unwrap();
        let data: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        std::fs::write(from.join("every.dat"), &data).unwrap();

        let script = format!(
            "printf 'before\\r\\n'; sleep 1; cd '{}' && gkermit -q -s every.dat; printf 'after\\r\\n'; sleep 1",
            from.display()
        );
        let pty = Pty::spawn("/bin/sh", &["-c", script.as_str()], 24, 80, "vt420").unwrap();
        let (session, notices) =
            Session::start(Config::default(), Box::new(pty), None, None).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let settings = vt_kermit::Settings::default();
        let transfer = Transfer::receive(
            vt_kermit::files::Folder::new(into.clone()),
            settings,
            std::time::Instant::now(),
        );
        session.start_transfer(transfer, &[]).unwrap();
        session.type_text("typed during the transfer");

        while let Ok(notice) = notices.recv_blocking() {
            if matches!(notice, Notice::Exited(_)) {
                break;
            }
        }
        let state = session
            .transfer()
            .expect("the finished transfer stays to be read");
        assert_eq!(state.status, Status::Done);
        assert_eq!(state.saved, [into.join("every.dat")]);
        assert_eq!(std::fs::read(into.join("every.dat")).unwrap(), data);
        std::thread::sleep(Duration::from_secs(2));
        session.clear_transfer();
        assert!(session.transfer().is_none());

        let term = session.terminal();
        let screen: Vec<String> = (0..term.grid().rows())
            .map(|row| vt_core::dump::row_text(&term, row))
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(screen, ["before", "after"], "and no packets");
        drop(term);
        let _ = std::fs::remove_dir_all(dir);
    }
}
