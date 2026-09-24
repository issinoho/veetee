//! The sessions in one window. Like a VT520 with sessions on separate comm
//! lines, each session has its own connection and terminal; with two sessions
//! the window is split horizontally, each session under a title bar, and the
//! Session key (F4) moves the keyboard between them.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib;
use vt_core::Config;
use vt_render::Theme;

use crate::cli::{self, Options};
use crate::session::{self, Notice, Session};
use crate::view::{Callbacks, SharedKeymap, TerminalView};

/// The most sessions a window holds.
pub const MAX_SESSIONS: usize = 2;

/// A bar above a session's screen while a file transfer has its line: what
/// is going, how far it has got, and a way to stop it.
struct TransferBar {
    bar: gtk::Box,
    label: gtk::Label,
    progress: gtk::ProgressBar,
    cancel: gtk::Button,
}

impl TransferBar {
    fn new() -> TransferBar {
        let label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        let progress = gtk::ProgressBar::builder()
            .valign(gtk::Align::Center)
            .width_request(160)
            .build();
        let cancel = gtk::Button::builder().label("Cancel").build();
        let bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .margin_bottom(4)
            .visible(false)
            .build();
        bar.append(&label);
        bar.append(&progress);
        bar.append(&cancel);
        TransferBar {
            bar,
            label,
            progress,
            cancel,
        }
    }
}

/// A size for people: bytes, then KB and MB.
fn size_text(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{} KB", bytes / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

struct Pane {
    id: u64,
    view: TerminalView,
    frame: gtk::Box,
    header: gtk::Label,
    /// Shown while a Kermit transfer has the session's line.
    transfer: TransferBar,
    /// Host-supplied session name (DECSWT).
    name: RefCell<String>,
    /// Extra status such as "Hold Screen" or "Disconnected".
    status: RefCell<String>,
}

pub struct Workspace {
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    paned: gtk::Paned,
    config: Config,
    options: Options,
    base_subtitle: String,
    panes: RefCell<Vec<Rc<Pane>>>,
    active: Cell<u64>,
    next_id: Cell<u64>,
    theme: RefCell<Theme>,
    phosphor: Cell<vt_render::Phosphor>,
    appearance: Cell<crate::appearance::Appearance>,
    opening: Cell<bool>,
    keymap: SharedKeymap,
}

impl Workspace {
    pub fn new(
        window: &adw::ApplicationWindow,
        title: &adw::WindowTitle,
        toasts: &adw::ToastOverlay,
        config: Config,
        options: Options,
        base_subtitle: String,
    ) -> Rc<Workspace> {
        let options_keymap = options.keymap.clone();
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .wide_handle(true)
            .resize_start_child(true)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .build();
        let workspace = Rc::new(Workspace {
            window: window.clone(),
            title: title.clone(),
            toasts: toasts.clone(),
            paned,
            config,
            options,
            base_subtitle,
            panes: RefCell::new(Vec::new()),
            active: Cell::new(0),
            next_id: Cell::new(1),
            theme: RefCell::new(Theme::default()),
            phosphor: Cell::new(vt_render::Phosphor::default()),
            appearance: Cell::new(crate::appearance::load()),
            opening: Cell::new(false),
            keymap: Rc::new(RefCell::new(crate::keymaps::load(
                options_keymap.as_deref(),
            ))),
        });
        let weak = Rc::downgrade(&workspace);
        window.connect_close_request(move |_| {
            if let Some(ws) = weak.upgrade() {
                for pane in ws.panes.borrow().iter() {
                    pane.view.session().close();
                }
            }
            glib::Propagation::Proceed
        });
        workspace
    }

    /// The window's connection and options as a profile to save, named
    /// after the connection.
    pub fn profile(&self) -> crate::profiles::Profile {
        let name = match &self.options.connection {
            cli::Connection::Telnet { host, .. } => host.clone(),
            cli::Connection::Ssh(s) => s.destination.clone(),
            cli::Connection::Serial(s) => s
                .device
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            cli::Connection::Lat { node, .. } => node.clone(),
            cli::Connection::Shell => "Local shell".into(),
            cli::Connection::Command(c) => c.split_whitespace().next().unwrap_or("").into(),
        };
        crate::profiles::Profile::from_options(&name, &self.config, &self.options)
    }

    fn notify(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::new(msg));
    }

    /// Adds a connected session to the window.
    pub fn add_session(
        self: &Rc<Self>,
        session: Session,
        notices: async_channel::Receiver<Notice>,
    ) {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let weak = Rc::downgrade(self);
        let on = |f: fn(&Rc<Workspace>, u64)| {
            let weak: Weak<Workspace> = weak.clone();
            move || {
                if let Some(ws) = weak.upgrade() {
                    f(&ws, id);
                }
            }
        };
        let session_number = (self.panes.borrow().len() + 1) as u8;
        let callbacks = Callbacks {
            session_number,
            notify: Box::new({
                let weak = weak.clone();
                move |msg: &str| {
                    if let Some(ws) = weak.upgrade() {
                        ws.notify(msg);
                    }
                }
            }),
            status: Box::new({
                let weak = weak.clone();
                move |text: &str| {
                    if let Some(ws) = weak.upgrade() {
                        ws.set_status(id, text);
                    }
                }
            }),
            title: Box::new({
                let weak = weak.clone();
                move |name: &str| {
                    if let Some(ws) = weak.upgrade() {
                        ws.set_name(id, name);
                    }
                }
            }),
            switch_session: Box::new(on(|ws, _| ws.switch_session())),
            activate: Box::new(on(|ws, id| ws.activate(id))),
            focused: Box::new(on(|ws, id| ws.focused(id))),
            exited: Box::new({
                let weak = weak.clone();
                move |reason: Option<String>| {
                    if let Some(ws) = weak.upgrade() {
                        ws.exited(id, reason);
                    }
                }
            }),
        };
        let view = TerminalView::new(session, notices, callbacks, self.keymap.clone());
        view.set_theme(self.theme.borrow().clone());
        view.set_visible_bell(self.appearance.get().visible_bell);
        let header = gtk::Label::builder()
            .xalign(0.0)
            .margin_start(8)
            .margin_top(2)
            .margin_bottom(2)
            .css_classes(["caption-heading"])
            .build();
        let transfer = TransferBar::new();
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.append(&header);
        frame.append(&transfer.bar);
        frame.append(view.container());
        let pane = Rc::new(Pane {
            id,
            view,
            frame,
            header,
            transfer,
            name: RefCell::new(String::new()),
            status: RefCell::new(String::new()),
        });
        self.panes.borrow_mut().push(pane.clone());
        self.layout();
        self.active.set(id);
        pane.view.widget().grab_focus();
        self.refresh();
    }

    /// Opens another session to the same destination as the first.
    pub fn open_session(self: &Rc<Self>) {
        if self.panes.borrow().len() >= MAX_SESSIONS {
            self.notify("This window already has two sessions");
            return;
        }
        if self.opening.replace(true) {
            return;
        }
        let (tx, rx) = async_channel::bounded(1);
        let mut config = self.config.clone();
        crate::setup_store::load_into(&mut config, 2);
        let connection = self.options.connection.clone();
        let session_config = config.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(cli::open_transport(&config, &connection));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(opened) = rx.recv().await else { return };
            let Some(ws) = weak.upgrade() else { return };
            ws.opening.set(false);
            let started =
                opened.and_then(|t| session::Session::start(session_config, t, None, None));
            match started {
                Ok((session, notices)) => ws.add_session(session, notices),
                Err(e) => ws.notify(&format!("Cannot open a session: {e}")),
            }
        });
    }

    fn layout(&self) {
        let panes = self.panes.borrow();
        self.paned.set_start_child(None::<&gtk::Widget>);
        self.paned.set_end_child(None::<&gtk::Widget>);
        if let Some(first) = panes.first() {
            self.paned.set_start_child(Some(&first.frame));
        }
        if let Some(second) = panes.get(1) {
            self.paned.set_end_child(Some(&second.frame));
            // Split the screen evenly once the window has a size.
            let paned = self.paned.clone();
            glib::idle_add_local_once(move || {
                let height = paned.height();
                if height > 0 {
                    paned.set_position(height / 2);
                }
            });
        }
        for pane in panes.iter() {
            pane.header.set_visible(panes.len() > 1);
        }
        if self.toasts.child().as_ref() != Some(self.paned.upcast_ref::<gtk::Widget>()) {
            self.toasts.set_child(Some(&self.paned));
        }
        let count = panes.len() as u8;
        for pane in panes.iter() {
            pane.view.session().terminal().set_sessions(count);
        }
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.panes.borrow().iter().position(|p| p.id == id)
    }

    /// F4: move the keyboard to the next session.
    fn switch_session(&self) {
        let panes = self.panes.borrow();
        if panes.len() < 2 {
            drop(panes);
            self.notify("Only one session is open (window menu: Open Second Session)");
            return;
        }
        let current = panes
            .iter()
            .position(|p| p.id == self.active.get())
            .unwrap_or(0);
        let next = panes[(current + 1) % panes.len()].clone();
        drop(panes);
        next.view.widget().grab_focus();
    }

    /// DECES: the host made a session active.
    fn activate(&self, id: u64) {
        let pane = self.panes.borrow().iter().find(|p| p.id == id).cloned();
        if let Some(pane) = pane {
            self.window.present();
            pane.view.widget().grab_focus();
        }
    }

    fn focused(&self, id: u64) {
        self.active.set(id);
        self.refresh();
    }

    fn set_name(&self, id: u64, name: &str) {
        if let Some(pane) = self.panes.borrow().iter().find(|p| p.id == id) {
            *pane.name.borrow_mut() = name.to_string();
        }
        self.refresh();
    }

    fn set_status(&self, id: u64, text: &str) {
        if let Some(pane) = self.panes.borrow().iter().find(|p| p.id == id) {
            *pane.status.borrow_mut() = text.to_string();
        }
        self.refresh();
    }

    fn exited(&self, id: u64, reason: Option<String>) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let pane = self.panes.borrow()[index].clone();
        pane.view.session().close();
        let label = self.options.connection.label();
        let msg = match reason {
            Some(r) => format!("Connection closed: {r}"),
            None => format!("Connection to {label} closed"),
        };
        eprintln!("veetee: {msg}");
        let split = self.panes.borrow().len() > 1;
        if !split && self.options.connection.keep_open_on_close() {
            // Keep the screen readable (and copyable) after the line drops.
            // Only worth it for the last session: it is the only thing left
            // to look at, and closing the window is the only alternative.
            *pane.status.borrow_mut() = "Disconnected".into();
            self.toasts
                .add_toast(adw::Toast::builder().title(msg).timeout(0).build());
            self.refresh();
            return;
        }
        // With the window split, a session that has ended is half a window
        // doing nothing: the other session gets it back, and the reason for
        // the ending is in the toast rather than on the dead screen.
        if split {
            self.notify(&msg);
        }
        self.remove_pane(index);
    }

    /// Takes a pane out and gives the window to whatever is left of it.
    fn remove_pane(&self, index: usize) {
        self.panes.borrow_mut().remove(index);
        if self.panes.borrow().is_empty() {
            self.window.close();
            return;
        }
        self.layout();
        let first = self.panes.borrow()[0].clone();
        first.view.widget().grab_focus();
        self.active.set(first.id);
        self.refresh();
    }

    /// Closes the session the keyboard is in, leaving the other one the whole
    /// window.
    ///
    /// Nothing to do with one session: that session *is* the window, and the
    /// window has its own close button. Said rather than done quietly, as F4
    /// says it when there is nothing to switch to.
    pub fn close_active_session(&self) {
        if self.panes.borrow().len() < 2 {
            self.notify("Only one session is open; close the window instead");
            return;
        }
        let Some(index) = self.index_of(self.active.get()) else {
            return;
        };
        let pane = self.panes.borrow()[index].clone();
        pane.view.session().close();
        self.remove_pane(index);
    }

    /// Updates session headers and the window title for the active session.
    fn refresh(&self) {
        let panes = self.panes.borrow();
        let split = panes.len() > 1;
        for (i, pane) in panes.iter().enumerate() {
            let name = pane.name.borrow();
            let label = if name.is_empty() {
                self.options.connection.label()
            } else {
                name.clone()
            };
            let status = pane.status.borrow();
            let mut text = format!("S{}  {label}", i + 1);
            if !status.is_empty() {
                text.push_str(&format!("  ·  {status}"));
            }
            pane.header.set_text(&text);
            let active = pane.id == self.active.get();
            if active {
                pane.header.remove_css_class("dim-label");
            } else {
                pane.header.add_css_class("dim-label");
            }
        }
        let Some(active) = panes.iter().find(|p| p.id == self.active.get()) else {
            return;
        };
        let name = active.name.borrow();
        // A host-supplied session name, else the saved connection's name.
        let window_title = if !name.is_empty() {
            name.as_str()
        } else {
            self.options.profile.as_deref().unwrap_or("veetee")
        };
        self.title.set_title(window_title);
        self.window.set_title(Some(window_title));
        let status = active.status.borrow();
        let mut subtitle = self.base_subtitle.clone();
        if split {
            let n = panes.iter().position(|p| p.id == active.id).unwrap_or(0) + 1;
            subtitle.push_str(&format!(" · Session {n}"));
        }
        if active.view.session().is_recording() {
            subtitle.push_str(" · Recording");
        }
        let logging = active.view.session().log_path().is_some();
        if logging {
            subtitle.push_str(" · Logging");
        }
        if let Some(action) = self.window.lookup_action("log") {
            action.change_state(&logging.to_variant());
        }
        if !status.is_empty() {
            subtitle.push_str(&format!(" · {status}"));
        }
        self.title.set_subtitle(&subtitle);
    }

    fn set_theme(&self, theme: Theme) {
        *self.theme.borrow_mut() = theme.clone();
        let visible_bell = self.appearance.get().visible_bell;
        for pane in self.panes.borrow().iter() {
            pane.view.set_theme(theme.clone());
            pane.view.set_visible_bell(visible_bell);
        }
    }

    pub fn set_phosphor(&self, phosphor: vt_render::Phosphor) {
        self.phosphor.set(phosphor);
        self.set_theme(Theme::with_effects(phosphor, self.appearance.get().effects));
    }

    pub fn appearance(&self) -> crate::appearance::Appearance {
        self.appearance.get()
    }

    /// Changes the display preferences and remembers them.
    pub fn set_appearance(&self, appearance: crate::appearance::Appearance) {
        self.appearance.set(appearance);
        crate::appearance::save(&appearance);
        self.set_phosphor(self.phosphor.get());
    }

    fn active_pane(&self) -> Option<Rc<Pane>> {
        let panes = self.panes.borrow();
        panes.iter().find(|p| p.id == self.active.get()).cloned()
    }

    /// Find: opens the find bar of the active session.
    pub fn search(&self) {
        if let Some(pane) = self.active_pane() {
            pane.view.open_search();
        }
    }

    pub fn is_logging(&self) -> bool {
        self.active_pane()
            .is_some_and(|p| p.view.session().log_path().is_some())
    }

    /// Log to File: stops the active session's log, or asks for a file and
    /// starts one.
    pub fn toggle_log(self: &Rc<Self>) {
        let Some(pane) = self.active_pane() else {
            return;
        };
        let session = pane.view.session().clone();
        if let Some(path) = session.log_path() {
            session.stop_log();
            self.notify(&format!("Log saved to {}", path.display()));
            self.refresh();
            return;
        }
        let stamp = glib::DateTime::now_local()
            .and_then(|t| t.format("%Y%m%d-%H%M"))
            .map(|s| s.to_string())
            .unwrap_or_default();
        let label = self
            .options
            .profile
            .clone()
            .unwrap_or_else(|| self.options.connection.label());
        let safe: String = label
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let dialog = gtk::FileDialog::builder()
            .title("Log to File")
            .accept_label("Log")
            .initial_name(format!("{safe}-{stamp}.log"))
            .build();
        let weak = Rc::downgrade(self);
        dialog.save(
            Some(&self.window),
            gtk::gio::Cancellable::NONE,
            move |result| {
                let (Ok(file), Some(ws)) = (result, weak.upgrade()) else {
                    return;
                };
                let Some(path) = file.path() else { return };
                let options = crate::log::LogOptions {
                    path: path.clone(),
                    raw: false,
                    timestamps: ws.appearance.get().log_timestamps,
                    append: false,
                };
                match session.start_log(&options) {
                    Ok(()) => ws.notify(&format!("Logging to {}", path.display())),
                    Err(e) => ws.notify(&format!("Cannot log to {}: {e}", path.display())),
                }
                ws.refresh();
            },
        );
    }

    /// Opens Set-Up for the active session, as F3 does.
    pub fn open_setup(&self) {
        let view = self
            .panes
            .borrow()
            .iter()
            .find(|p| p.id == self.active.get())
            .map(|p| p.view.clone());
        if let Some(view) = view {
            view.widget().grab_focus();
            view.open_setup();
        }
    }

    /// Adds a checkpoint to the active session's recording.
    pub fn mark_checkpoint(&self) {
        let session = self
            .panes
            .borrow()
            .iter()
            .find(|p| p.id == self.active.get())
            .map(|p| p.view.session());
        let message = match session.and_then(|s| s.mark_checkpoint()) {
            Some(name) => format!("Recorded checkpoint {name}"),
            None => "This session is not being recorded (--record FILE)".into(),
        };
        self.notify(&message);
    }

    /// Receive File: asks where to put them, then waits for the host's
    /// Kermit to send.
    pub fn receive_file(self: &Rc<Self>) {
        let Some(pane) = self.ready_for_transfer() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Receive Files Into")
            .accept_label("Receive")
            .build();
        if let Some(downloads) = glib::user_special_dir(glib::UserDirectory::Downloads) {
            dialog.set_initial_folder(Some(&gtk::gio::File::for_path(downloads)));
        }
        let weak = Rc::downgrade(self);
        dialog.select_folder(
            Some(&self.window),
            gtk::gio::Cancellable::NONE,
            move |result| {
                let (Ok(folder), Some(ws)) = (result, weak.upgrade()) else {
                    return;
                };
                let Some(dir) = folder.path() else { return };
                let transfer = vt_kermit::files::Transfer::receive(
                    vt_kermit::files::Folder::new(dir),
                    vt_kermit::Settings::default(),
                    std::time::Instant::now(),
                );
                ws.run_transfer(&pane, transfer, &[]);
            },
        );
    }

    /// Send File: asks which, then sends them to the host's Kermit, each as
    /// text or binary by what is in it.
    pub fn send_file(self: &Rc<Self>) {
        let Some(pane) = self.ready_for_transfer() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Send Files")
            .accept_label("Send")
            .build();
        let weak = Rc::downgrade(self);
        dialog.open_multiple(
            Some(&self.window),
            gtk::gio::Cancellable::NONE,
            move |result| {
                let (Ok(chosen), Some(ws)) = (result, weak.upgrade()) else {
                    return;
                };
                let paths: Vec<std::path::PathBuf> = (0..chosen.n_items())
                    .filter_map(|i| chosen.item(i))
                    .filter_map(|item| item.downcast::<gtk::gio::File>().ok())
                    .filter_map(|file| file.path())
                    .collect();
                if paths.is_empty() {
                    return;
                }
                let (transfer, first) = vt_kermit::files::Transfer::send(
                    vt_kermit::files::Paths::deciding_each(paths),
                    vt_kermit::Settings::default(),
                    std::time::Instant::now(),
                );
                ws.run_transfer(&pane, transfer, &first);
            },
        );
    }

    /// The active session, if it can start a transfer now.
    fn ready_for_transfer(&self) -> Option<Rc<Pane>> {
        let pane = self.active_pane()?;
        if pane
            .view
            .session()
            .transfer()
            .is_some_and(|t| t.status == vt_kermit::Status::Running)
        {
            self.notify("A transfer is already running in this session");
            return None;
        }
        Some(pane)
    }

    /// Starts a transfer on the pane's session and shows it until it ends.
    fn run_transfer(
        self: &Rc<Self>,
        pane: &Rc<Pane>,
        transfer: vt_kermit::files::Transfer,
        first: &[u8],
    ) {
        let session = pane.view.session().clone();
        let receiving = transfer.is_receiving();
        if let Err(why) = session.start_transfer(transfer, first) {
            self.notify(&why);
            return;
        }
        let bar = &pane.transfer;
        bar.cancel.set_label("Cancel");
        bar.cancel.set_sensitive(true);
        bar.progress.set_fraction(0.0);
        bar.bar.set_visible(true);
        // The first click stops tidily, which a far end may take a moment
        // to act on; the second stops at once.
        let pressed = Rc::new(Cell::new(0u8));
        let handler = bar.cancel.connect_clicked({
            let (session, pressed, label) = (session.clone(), pressed.clone(), bar.label.clone());
            move |button| {
                session.cancel_transfer();
                match pressed.replace(pressed.get() + 1) {
                    0 => {
                        label.set_text("Cancelling…");
                        button.set_label("Stop Now");
                    }
                    _ => button.set_sensitive(false),
                }
            }
        });
        let handler = RefCell::new(Some(handler));
        let (weak, pane) = (Rc::downgrade(self), pane.clone());
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let Some(ws) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(state) = session.transfer() else {
                pane.transfer.bar.set_visible(false);
                return glib::ControlFlow::Break;
            };
            let bar = &pane.transfer;
            let p = &state.progress;
            if state.status == vt_kermit::Status::Running {
                if pressed.get() == 0 {
                    bar.label.set_text(&match (&p.file, receiving) {
                        (None, true) => {
                            "Waiting for the host to send: SEND at its Kermit prompt".to_string()
                        }
                        (None, false) => {
                            "Waiting for the host: RECEIVE at its Kermit prompt".to_string()
                        }
                        (Some(name), true) => format!("Receiving {name}"),
                        (Some(name), false) => format!("Sending {name}"),
                    });
                }
                match p.size {
                    Some(size) if size > 0 => {
                        bar.progress
                            .set_fraction((p.bytes as f64 / size as f64).min(1.0));
                        bar.progress.set_text(Some(&format!(
                            "{} of {}",
                            size_text(p.bytes),
                            size_text(size)
                        )));
                    }
                    _ if p.file.is_some() => {
                        bar.progress.pulse();
                        bar.progress.set_text(Some(&size_text(p.bytes)));
                    }
                    _ => bar.progress.pulse(),
                }
                bar.progress.set_show_text(p.file.is_some());
                return glib::ControlFlow::Continue;
            }

            bar.bar.set_visible(false);
            if let Some(handler) = handler.take() {
                bar.cancel.disconnect(handler);
            }
            let files = |n: u32| {
                if n == 1 {
                    "1 file".to_string()
                } else {
                    format!("{n} files")
                }
            };
            let message = match &state.status {
                vt_kermit::Status::Done if receiving => match state.saved.first() {
                    Some(first) => {
                        let place = first
                            .parent()
                            .map_or_else(String::new, |d| format!(" into {}", d.display()));
                        format!("Received {}{place}", files(p.files))
                    }
                    None => "The host sent no files".to_string(),
                },
                vt_kermit::Status::Done => format!("Sent {}", files(p.files)),
                vt_kermit::Status::Cancelled => "Transfer cancelled".to_string(),
                vt_kermit::Status::Failed(why) => format!("Transfer failed: {why}"),
                vt_kermit::Status::Running => unreachable!(),
            };
            ws.notify(&message);
            // A receiver keeps the line a moment for a repeated packet;
            // clear it away once that has passed.
            let session = session.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                session.clear_transfer();
            });
            glib::ControlFlow::Break
        });
    }

    pub fn keymap(&self) -> SharedKeymap {
        self.keymap.clone()
    }

    pub fn phosphor(&self) -> String {
        self.options.phosphor.clone()
    }

    pub fn sessions_to_open(&self) -> u8 {
        self.options.sessions
    }
}
