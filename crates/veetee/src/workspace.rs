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

struct Pane {
    id: u64,
    view: TerminalView,
    frame: gtk::Box,
    header: gtk::Label,
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
        let callbacks = Callbacks {
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
        let header = gtk::Label::builder()
            .xalign(0.0)
            .margin_start(8)
            .margin_top(2)
            .margin_bottom(2)
            .css_classes(["caption-heading"])
            .build();
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.append(&header);
        frame.append(view.widget());
        let pane = Rc::new(Pane {
            id,
            view,
            frame,
            header,
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
        let (config, connection) = (self.config.clone(), self.options.connection.clone());
        std::thread::spawn(move || {
            let _ = tx.send_blocking(cli::open_transport(&config, &connection));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(opened) = rx.recv().await else { return };
            let Some(ws) = weak.upgrade() else { return };
            ws.opening.set(false);
            let started = opened.and_then(|t| session::Session::start(ws.config.clone(), t, None));
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
        if self.options.connection.keep_open_on_close() {
            // Keep the screen readable (and copyable) after the line drops.
            let label = self.options.connection.label();
            let msg = match reason {
                Some(r) => format!("Connection closed: {r}"),
                None => format!("Connection to {label} closed"),
            };
            eprintln!("veetee: {msg}");
            *pane.status.borrow_mut() = "Disconnected".into();
            self.toasts
                .add_toast(adw::Toast::builder().title(msg).timeout(0).build());
            self.refresh();
            return;
        }
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
        let window_title = if name.is_empty() {
            "veetee"
        } else {
            name.as_str()
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
        if !status.is_empty() {
            subtitle.push_str(&format!(" · {status}"));
        }
        self.title.set_subtitle(&subtitle);
    }

    pub fn set_theme(&self, theme: Theme) {
        *self.theme.borrow_mut() = theme.clone();
        for pane in self.panes.borrow().iter() {
            pane.view.set_theme(theme.clone());
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
