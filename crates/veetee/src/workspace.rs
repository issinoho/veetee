//! The sessions in one window. Like a VT520 with sessions on separate comm
//! lines, each session has its own connection and terminal. A VT520 or VT525
//! has up to four (EK-VT520-RM 2.5), a VT420 two; the screen shows at most
//! two, split horizontally with a title bar each: the active session and the
//! one active before it (EK-VT520-IN 2.3). The Session key (F4) moves the
//! keyboard to the next session, and Alt+1 to Alt+4 to one directly.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib;
use vt_core::Config;
use vt_render::Theme;

use crate::cli::{self, Options};
use crate::session::{self, Notice, Session};
use crate::view::{Callbacks, SharedKeymap, Split, TerminalView};

/// The most sessions a model has: four on the VT520 and VT525 (EK-VT520-RM
/// 2.5), two on the VT420 (RM420 chapter 14) and the VT510, which had one —
/// kept at two so as not to take away what worked — and one before them.
pub fn max_sessions(model: vt_core::Model) -> usize {
    use vt_core::Model;
    match model {
        Model::Vt520 | Model::Vt525 => 4,
        Model::Vt420 | Model::Vt510 => 2,
        _ => 1,
    }
}

/// The lowest session number, 1 to 4, not in `taken`.
fn free_number(taken: impl IntoIterator<Item = u8>) -> u8 {
    let taken: Vec<u8> = taken.into_iter().collect();
    (1..=4).find(|n| !taken.contains(n)).unwrap_or(1)
}

/// The sessions to show: the active one and, with two windows, the one active
/// before it, in session order, of `open` (session numbers) given the order
/// they were last active in, most recent first.
fn shown(open: &[u8], recent: &[u8], two_windows: bool) -> Vec<u8> {
    let windows = if two_windows { 2 } else { 1 };
    let mut shown: Vec<u8> = recent
        .iter()
        .copied()
        .filter(|n| open.contains(n))
        .take(windows)
        .collect();
    // A session never active yet, where fewer than two have been.
    for &n in open {
        if shown.len() >= windows {
            break;
        }
        if !shown.contains(&n) {
            shown.push(n);
        }
    }
    shown.sort_unstable();
    shown
}

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
    /// The session number, S1 to S4: which saved Set-Up it has, and what its
    /// title bar and the subtitle call it.
    number: u8,
    /// Where this session is connected, as on a VT520 with its sessions on
    /// different comm ports: the window's own connection, or another chosen
    /// with New Session.
    connection: cli::Connection,
    /// The saved connection it came from, if one.
    profile: Option<String>,
    /// The connection as the title bar and subtitle show it; a serial line's
    /// settings follow the line.
    label: RefCell<String>,
    view: TerminalView,
    frame: gtk::Box,
    header: gtk::Label,
    /// Shown while a Kermit transfer has the session's line.
    transfer: TransferBar,
    /// Host-supplied session name (DECSWT).
    name: RefCell<String>,
    /// Host-supplied icon name (DECSIN).
    icon: RefCell<String>,
    /// Output has come while the session was off the screen, and its icon
    /// blinks until it is shown (EK-VT520-IN 2.3).
    unseen: Cell<bool>,
    /// Extra status such as "Hold Screen" or "Disconnected".
    status: RefCell<String>,
}

pub struct Workspace {
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    paned: gtk::Paned,
    /// The session icons above the windows, with framed windows.
    icons: gtk::Box,
    /// The icons and the windows under them.
    body: gtk::Box,
    config: Config,
    options: Options,
    panes: RefCell<Vec<Rc<Pane>>>,
    active: Cell<u64>,
    next_id: Cell<u64>,
    theme: RefCell<Theme>,
    phosphor: Cell<vt_render::Phosphor>,
    appearance: Cell<crate::appearance::Appearance>,
    opening: Cell<bool>,
    /// Sessions still to open, after this one, for `--sessions`.
    to_open: Cell<u8>,
    /// Session ids in the order they were last active, most recent first.
    recent: RefCell<Vec<u64>>,
    /// Two windows, split horizontally, where more than one session is
    /// open; Ctrl+F4 toggles it, as Ctrl+Session on a VT520.
    two_windows: Cell<bool>,
    /// Where the line between two windows is, as a share of the height.
    split_at: Rc<Cell<f64>>,
    /// The sessions on the screen now.
    on_screen: RefCell<Vec<u64>>,
    /// The blink timer is running, and whether unseen icons are lit.
    blinking: Cell<bool>,
    blink_lit: Cell<bool>,
    keymap: SharedKeymap,
}

impl Workspace {
    pub fn new(
        window: &adw::ApplicationWindow,
        title: &adw::WindowTitle,
        toasts: &adw::ToastOverlay,
        config: Config,
        options: Options,
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
        // Where the user drags the line between two windows is kept, so
        // switching sessions does not move it back.
        let split_at = Rc::new(Cell::new(0.5f64));
        paned.connect_position_notify({
            let split_at = split_at.clone();
            move |paned| {
                let height = paned.height();
                if height > 0 && paned.end_child().is_some() {
                    split_at.set(f64::from(paned.position()) / f64::from(height));
                }
            }
        });
        let icons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .margin_start(6)
            .margin_end(6)
            .margin_top(2)
            .margin_bottom(2)
            .visible(false)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&icons);
        body.append(&paned);
        paned.set_vexpand(true);
        let workspace = Rc::new(Workspace {
            window: window.clone(),
            title: title.clone(),
            toasts: toasts.clone(),
            paned,
            icons,
            body,
            config,
            options,
            panes: RefCell::new(Vec::new()),
            active: Cell::new(0),
            next_id: Cell::new(1),
            theme: RefCell::new(Theme::default()),
            phosphor: Cell::new(vt_render::Phosphor::default()),
            appearance: Cell::new(crate::appearance::load()),
            opening: Cell::new(false),
            to_open: Cell::new(0),
            recent: RefCell::new(Vec::new()),
            two_windows: Cell::new(true),
            split_at,
            on_screen: RefCell::new(Vec::new()),
            blinking: Cell::new(false),
            blink_lit: Cell::new(true),
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
        let connection = self
            .active_pane()
            .map_or_else(|| self.options.connection.clone(), |p| p.connection.clone());
        let name = match &connection {
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
        let options = Options {
            connection,
            ..self.options.clone()
        };
        crate::profiles::Profile::from_options(&name, &self.config, &options)
    }

    fn notify(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::new(msg));
    }

    /// The line was set anew from Set-Up or by the host: a serial line's
    /// settings are in the subtitle, and either way a message says so.
    fn line_set(&self, id: u64, line: vt_transport::serial::Line, error: Option<String>) {
        match error {
            Some(e) => self.notify(&format!("The line stays at {line}: {e}")),
            None => self.notify(&format!("Line set to {line}")),
        }
        let pane = self.panes.borrow().iter().find(|p| p.id == id).cloned();
        if let Some(pane) = pane
            && let cli::Connection::Serial(serial) = &pane.connection
        {
            *pane.label.borrow_mut() = format!("{} {line}", serial.device.display());
            self.refresh();
        }
    }

    /// Adds a connected session to the window.
    pub fn add_session(
        self: &Rc<Self>,
        session: Session,
        notices: async_channel::Receiver<Notice>,
        connection: cli::Connection,
        profile: Option<String>,
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
        let session_number = free_number(self.panes.borrow().iter().map(|p| p.number));
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
            icon: Box::new({
                let weak = weak.clone();
                move |name: &str| {
                    if let Some(ws) = weak.upgrade() {
                        ws.set_icon(id, name);
                    }
                }
            }),
            output: Box::new(on(|ws, id| ws.output(id))),
            switch_session: Box::new(on(|ws, _| ws.switch_session())),
            split: Box::new({
                let weak = weak.clone();
                move |op: Split| {
                    if let Some(ws) = weak.upgrade() {
                        ws.split(op);
                    }
                }
            }),
            go_to_session: Box::new({
                let weak = weak.clone();
                move |n: u8| {
                    if let Some(ws) = weak.upgrade() {
                        ws.go_to_session(n);
                    }
                }
            }),
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
            print: Box::new({
                let weak = weak.clone();
                move |job: vt_core::PrintJob| {
                    if let Some(ws) = weak.upgrade() {
                        ws.print(id, job);
                    }
                }
            }),
            line: Box::new({
                let weak = weak.clone();
                move |line, error| {
                    if let Some(ws) = weak.upgrade() {
                        ws.line_set(id, line, error);
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
        let label = RefCell::new(connection.label());
        let pane = Rc::new(Pane {
            id,
            number: session_number,
            connection,
            profile,
            label,
            view,
            frame,
            header,
            transfer,
            name: RefCell::new(String::new()),
            icon: RefCell::new(String::new()),
            unseen: Cell::new(false),
            status: RefCell::new(String::new()),
        });
        {
            let mut panes = self.panes.borrow_mut();
            panes.push(pane.clone());
            panes.sort_by_key(|p| p.number);
        }
        self.activate_session(id);
        if self.to_open.get() > 0 {
            self.to_open.set(self.to_open.get() - 1);
            self.open_session(None);
        }
    }

    /// Opens `n` more sessions, one after another, for `--sessions`.
    pub fn open_more(self: &Rc<Self>, n: u8) {
        if n > 0 {
            self.to_open.set(n - 1);
            self.open_session(None);
        }
    }

    /// New Session: asks where the session is to connect — the window's own
    /// connection or a saved one — and opens it there.
    pub fn new_session(self: &Rc<Self>) {
        let most = max_sessions(self.config.model);
        if self.panes.borrow().len() >= most {
            self.open_session(None);
            return;
        }
        let weak = Rc::downgrade(self);
        crate::connections::choose_session(
            &self.window,
            &self.options.connection.label(),
            move |chosen| {
                if let Some(ws) = weak.upgrade() {
                    ws.open_session(chosen.map(|p| (p.connection, Some(p.name))));
                }
            },
        );
    }

    /// Opens another session, to `to` — a connection and the saved connection
    /// it came from — or else to the window's own connection. The session is
    /// part of this terminal, so it has the window's model whatever the saved
    /// connection says.
    pub fn open_session(self: &Rc<Self>, to: Option<(cli::Connection, Option<String>)>) {
        let most = max_sessions(self.config.model);
        if self.panes.borrow().len() >= most {
            self.to_open.set(0);
            let model = cli::model_name(self.config.model);
            self.notify(&match most {
                1 => format!("A {model} has one session"),
                n => format!("This window has {n} sessions, the most a {model} has"),
            });
            return;
        }
        if self.opening.replace(true) {
            return;
        }
        let number = free_number(self.panes.borrow().iter().map(|p| p.number));
        let (tx, rx) = async_channel::bounded(1);
        let mut config = self.config.clone();
        crate::setup_store::load_into(&mut config, number);
        let (connection, profile) = to.unwrap_or_else(|| {
            (
                self.options.connection.clone(),
                self.options.profile.clone(),
            )
        });
        let session_config = config.clone();
        let for_pane = connection.clone();
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
                Ok((session, notices)) => ws.add_session(session, notices, for_pane, profile),
                Err(e) => ws.notify(&format!("Cannot open a session: {e}")),
            }
        });
    }

    fn layout(&self) {
        let panes = self.panes.borrow();
        let number_of = |id: u64| panes.iter().find(|p| p.id == id).map(|p| p.number);
        let open: Vec<u8> = panes.iter().map(|p| p.number).collect();
        let recent: Vec<u8> = self
            .recent
            .borrow()
            .iter()
            .filter_map(|&id| number_of(id))
            .collect();
        let shown = shown(&open, &recent, self.two_windows.get());
        let pane = |n: u8| panes.iter().find(|p| p.number == n);
        self.paned.set_start_child(None::<&gtk::Widget>);
        self.paned.set_end_child(None::<&gtk::Widget>);
        if let Some(first) = shown.first().and_then(|&n| pane(n)) {
            self.paned.set_start_child(Some(&first.frame));
        }
        if let Some(second) = shown.get(1).and_then(|&n| pane(n)) {
            self.paned.set_end_child(Some(&second.frame));
            self.place_split();
        }
        let framed = self.framed();
        for pane in panes.iter() {
            pane.header.set_visible(framed && panes.len() > 1);
        }
        let on_screen: Vec<u64> = shown
            .iter()
            .filter_map(|&n| pane(n).map(|p| p.id))
            .collect();
        for pane in panes.iter() {
            if on_screen.contains(&pane.id) {
                pane.unseen.set(false);
            }
        }
        *self.on_screen.borrow_mut() = on_screen;
        if self.toasts.child().as_ref() != Some(self.body.upcast_ref::<gtk::Widget>()) {
            self.toasts.set_child(Some(&self.body));
        }
        let count = panes.len() as u8;
        for pane in panes.iter() {
            pane.view.session().terminal().set_sessions(count);
        }
    }

    /// Framed windows (DECFWM), as the active session's Set-Up has it.
    fn framed(&self) -> bool {
        let panes = self.panes.borrow();
        panes
            .iter()
            .find(|p| p.id == self.active.get())
            .or(panes.first())
            .is_none_or(|p| p.view.session().terminal().framed_windows())
    }

    /// The host sent a session something: one off the screen has output
    /// unseen, and its icon blinks.
    fn output(self: &Rc<Self>, id: u64) {
        if self.on_screen.borrow().contains(&id) {
            return;
        }
        let pane = self.panes.borrow().iter().find(|p| p.id == id).cloned();
        let Some(pane) = pane else { return };
        if pane.unseen.replace(true) {
            return;
        }
        self.update_icons();
        if !self.blinking.replace(true) {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                let Some(ws) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let any = ws.panes.borrow().iter().any(|p| p.unseen.get());
                if !any {
                    ws.blinking.set(false);
                    ws.blink_lit.set(true);
                    ws.update_icons();
                    return glib::ControlFlow::Break;
                }
                ws.blink_lit.set(!ws.blink_lit.get());
                ws.update_icons();
                glib::ControlFlow::Continue
            });
        }
    }

    fn set_icon(&self, id: u64, name: &str) {
        if let Some(pane) = self.panes.borrow().iter().find(|p| p.id == id) {
            *pane.icon.borrow_mut() = name.to_string();
        }
        self.refresh();
    }

    /// The session icons, with framed windows and more than one session:
    /// `S1 name` for each, the name the host's icon name (DECSIN) or the first
    /// 12 characters of the session's name (EK-VT520-RM 2.6.2). The active one
    /// is marked, one off the screen is dimmed, and one with output unseen
    /// blinks. Clicking one goes to that session.
    fn update_icons(&self) {
        while let Some(child) = self.icons.first_child() {
            self.icons.remove(&child);
        }
        let panes = self.panes.borrow();
        let visible = self.framed() && panes.len() > 1;
        self.icons.set_visible(visible);
        if !visible {
            return;
        }
        let on_screen = self.on_screen.borrow();
        for pane in panes.iter() {
            let icon = pane.icon.borrow();
            let name = if !icon.is_empty() {
                icon.clone()
            } else {
                let name = pane.name.borrow();
                let full = if name.is_empty() {
                    pane.label.borrow().clone()
                } else {
                    name.clone()
                };
                full.chars().take(12).collect()
            };
            let button = gtk::Button::builder()
                .label(format!("S{} {name}", pane.number))
                .tooltip_text(format!("Session {} (Alt+{})", pane.number, pane.number))
                .build();
            // The active session's icon is raised; the others are flat, and
            // dimmed off the screen. Output unseen shows in the warning colour,
            // blinking.
            if pane.id != self.active.get() {
                button.add_css_class("flat");
            }
            if pane.unseen.get() {
                button.add_css_class("warning");
                if !self.blink_lit.get() {
                    button.set_opacity(0.25);
                }
            } else if !on_screen.contains(&pane.id) {
                button.add_css_class("dim-label");
            }
            let workspace = self.window.clone();
            let n = pane.number;
            button.connect_clicked(move |_| {
                gtk::gio::prelude::ActionGroupExt::activate_action(
                    &workspace,
                    "session-go",
                    Some(&n.to_variant()),
                );
            });
            self.icons.append(&button);
        }
    }

    /// Puts the line between two windows where it was last left, once the
    /// window has a size.
    fn place_split(&self) {
        let paned = self.paned.clone();
        let at = self.split_at.get();
        glib::idle_add_local_once(move || {
            let height = paned.height();
            if height > 0 {
                paned.set_position((f64::from(height) * at).round() as i32);
            }
        });
    }

    /// Ctrl+F4 and Ctrl+Shift+Up/Down: one window or two, and the line
    /// between them (EK-VT520-RM 3.7).
    fn split(&self, op: Split) {
        if self.panes.borrow().len() < 2 {
            self.notify("Only one session is open (window menu: New Session)");
            return;
        }
        match op {
            Split::Toggle => {
                self.two_windows.set(!self.two_windows.get());
                self.activate_session(self.active.get());
            }
            Split::Up | Split::Down if !self.two_windows.get() => {
                self.notify("One window: Ctrl+F4 splits it in two");
            }
            Split::Up | Split::Down => {
                let height = f64::from(self.paned.height().max(1));
                let now = f64::from(self.paned.position()) / height;
                // A step of about a line of a 24-line screen, between a
                // quarter and three quarters of the window.
                let step = 1.0 / 24.0;
                let at = if op == Split::Up {
                    now - step
                } else {
                    now + step
                };
                let at = at.clamp(0.25, 0.75);
                self.split_at.set(at);
                self.paned.set_position((height * at).round() as i32);
            }
        }
    }

    /// Makes a session active: the keyboard goes to it, and it is put on the
    /// screen beside the one active before it.
    fn activate_session(&self, id: u64) {
        let Some(pane) = self.panes.borrow().iter().find(|p| p.id == id).cloned() else {
            return;
        };
        {
            let mut recent = self.recent.borrow_mut();
            recent.retain(|&r| r != id);
            recent.insert(0, id);
        }
        self.active.set(id);
        self.layout();
        pane.view.widget().grab_focus();
        self.refresh();
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.panes.borrow().iter().position(|p| p.id == id)
    }

    /// F4: move the keyboard to the next session, in session order.
    fn switch_session(&self) {
        let next = {
            let panes = self.panes.borrow();
            if panes.len() < 2 {
                None
            } else {
                let current = panes
                    .iter()
                    .position(|p| p.id == self.active.get())
                    .unwrap_or(0);
                Some(panes[(current + 1) % panes.len()].id)
            }
        };
        match next {
            Some(id) => self.activate_session(id),
            None => self.notify("Only one session is open (window menu: New Session)"),
        }
    }

    /// Alt+1 to Alt+4: to a session directly, as Caps Lock with keypad 1–4
    /// on a VT520 (EK-VT520-RM 2.5.4).
    pub fn go_to_session(&self, n: u8) {
        let id = self
            .panes
            .borrow()
            .iter()
            .find(|p| p.number == n)
            .map(|p| p.id);
        match id {
            Some(id) => self.activate_session(id),
            None => self.notify(&format!("Session {n} is not open")),
        }
    }

    /// DECES: the host made a session active.
    fn activate(&self, id: u64) {
        self.window.present();
        self.activate_session(id);
    }

    /// A session's screen took the keyboard, as by a click: it is active, and
    /// already on the screen.
    fn focused(&self, id: u64) {
        {
            let mut recent = self.recent.borrow_mut();
            recent.retain(|&r| r != id);
            recent.insert(0, id);
        }
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
        let label = pane.label.borrow().clone();
        let msg = match reason {
            Some(r) => format!("Connection closed: {r}"),
            None => format!("Connection to {label} closed"),
        };
        eprintln!("veetee: {msg}");
        let split = self.panes.borrow().len() > 1;
        if !split && pane.connection.keep_open_on_close() {
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
        let gone = self.panes.borrow_mut().remove(index);
        self.recent.borrow_mut().retain(|&r| r != gone.id);
        if self.panes.borrow().is_empty() {
            self.window.close();
            return;
        }
        // The session active before this one, or else the first.
        let next = {
            let panes = self.panes.borrow();
            let recent = self.recent.borrow();
            recent
                .iter()
                .copied()
                .find(|&r| panes.iter().any(|p| p.id == r))
                .unwrap_or(panes[0].id)
        };
        self.activate_session(next);
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
        self.update_icons();
        let panes = self.panes.borrow();
        let split = panes.len() > 1;
        for pane in panes.iter() {
            let name = pane.name.borrow();
            let label = if name.is_empty() {
                pane.label.borrow().clone()
            } else {
                name.clone()
            };
            let status = pane.status.borrow();
            let mut text = format!("S{}  {label}", pane.number);
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
            active.profile.as_deref().unwrap_or("veetee")
        };
        self.title.set_title(window_title);
        self.window.set_title(Some(window_title));
        let status = active.status.borrow();
        let mut subtitle = format!(
            "{} · {}",
            cli::model_name(self.config.model),
            active.label.borrow()
        );
        if split {
            subtitle.push_str(&format!(" · Session {}", active.number));
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
            .active_pane()
            .and_then(|p| p.profile.clone().or_else(|| Some(p.label.borrow().clone())))
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

    /// A print job, from the host or the screen: to the chosen printer, or
    /// written as a PDF into the printer folder off the main thread; either
    /// way a message says where it went.
    fn print(self: &Rc<Self>, id: u64, job: vt_core::PrintJob) {
        let folder = match crate::printing::load() {
            crate::printing::Printer::Folder(folder) => folder,
            crate::printing::Printer::System => {
                let text = crate::printing::text_of(&job);
                let name = self
                    .options
                    .profile
                    .clone()
                    .unwrap_or_else(|| "veetee".into());
                self.print_on_printer(&text, &name, false);
                return;
            }
            crate::printing::Printer::None => return,
        };
        let name = self
            .panes
            .borrow()
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| {
                let name = p.name.borrow().clone();
                if name.is_empty() {
                    p.profile.clone()
                } else {
                    Some(name)
                }
            })
            .unwrap_or_else(|| "veetee".into());
        let paper = crate::printing::paper();
        let text = crate::printing::text_of(&job);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&folder);
            let path = crate::printing::new_path(&folder, &name);
            let written =
                crate::printing::write_pdf(&text, &path, paper).map(|pages| (path, pages));
            let _ = tx.send_blocking(written);
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let (Ok(written), Some(ws)) = (rx.recv().await, weak.upgrade()) else {
                return;
            };
            match written {
                Ok((path, pages)) => {
                    let shown = path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    let toast = adw::Toast::builder()
                        .title(glib::markup_escape_text(&format!(
                            "Printed {shown}{}",
                            if pages == 1 {
                                String::new()
                            } else {
                                format!(", {pages} pages")
                            }
                        )))
                        .button_label("Open")
                        .build();
                    let window = ws.window.clone();
                    toast.connect_button_clicked(move |_| {
                        gtk::FileLauncher::new(Some(&gtk::gio::File::for_path(&path))).launch(
                            Some(&window),
                            gtk::gio::Cancellable::NONE,
                            |_| {},
                        );
                    });
                    ws.toasts.add_toast(toast);
                }
                Err(e) => ws.notify(&format!("Cannot print: {e}")),
            }
        });
    }

    /// Print Screen from the window menu, for the active session.
    pub fn print_screen(&self) {
        let Some(pane) = self.active_pane() else {
            return;
        };
        if !crate::printing::load().attached() {
            self.notify(
                "No printer: choose Print to Folder or Print to Printer in the window menu",
            );
            return;
        }
        pane.view.session().print_screen();
    }

    /// Sends text to the chosen printer with GTK's print dialog, which works
    /// through the desktop's print portal: the first job of a session, or
    /// `choosing`, shows the dialog; every job after goes straight to the
    /// printer chosen there. A job is written as a PDF to a temporary file and
    /// that is printed, so paper and PDF print alike.
    ///
    /// The desktop portal would not print silently from saved settings, and
    /// with it bypassed GTK printed to whatever printer it found when the one
    /// named was missing — the wrong one, for a host that prints unasked. So
    /// nothing is printed until a printer has been chosen in this session.
    #[cfg(not(windows))]
    fn print_on_printer(self: &Rc<Self>, text: &str, job_name: &str, choosing: bool) {
        static JOBS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        // Once a printer has been chosen, jobs go straight to it through
        // CUPS: the portal would ask again for every one, and printed the
        // host's job as a blank page when handed a setup it had already used.
        let chosen = gtk::PrintSettings::from_file(crate::printing::settings_path())
            .ok()
            .and_then(|s| s.printer())
            .map(|p| p.to_string());
        if let (false, Some(printer)) = (choosing, chosen) {
            let text = text.to_string();
            let title = format!("veetee: {job_name}");
            let paper = crate::printing::paper();
            let media = crate::printing::paper_name();
            let path = crate::printing::spool_dir().join(format!(
                "veetee-print-{}-{}.pdf",
                std::process::id(),
                JOBS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn({
                let printer = printer.clone();
                move || {
                    let sent = crate::printing::write_pdf(&text, &path, paper).and_then(|pages| {
                        crate::printing::send_to_cups(&printer, &title, &path, &media)
                            .map(|()| pages)
                    });
                    let _ = std::fs::remove_file(&path);
                    let _ = tx.send_blocking(sent);
                }
            });
            let weak = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                let (Ok(sent), Some(ws)) = (rx.recv().await, weak.upgrade()) else {
                    return;
                };
                match sent {
                    Ok(pages) => ws.notify(&format!(
                        "Sent to {printer}{}",
                        if pages == 1 {
                            String::new()
                        } else {
                            format!(", {pages} pages")
                        }
                    )),
                    Err(e) => ws.notify(&format!("Cannot print to {printer}: {e}")),
                }
            });
            return;
        }
        let text = text.to_string();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Some(ws) = weak.upgrade() else { return };
            let dialog = gtk::PrintDialog::builder().modal(true).build();
            if let Ok(saved) = gtk::PrintSettings::from_file(crate::printing::settings_path()) {
                dialog.set_print_settings(&saved);
            }
            let known = if choosing {
                None
            } else {
                crate::printing::SETUP.with(|s| s.borrow().clone())
            };
            let setup = match known {
                Some(setup) => setup,
                None => {
                    let Ok(setup) = dialog.setup_future(Some(&ws.window)).await else {
                        return; // the dialog was cancelled
                    };
                    let path = crate::printing::settings_path();
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = setup.print_settings().to_file(&path);
                    crate::printing::SETUP.with(|s| *s.borrow_mut() = Some(setup.clone()));
                    if choosing {
                        crate::printing::save(&crate::printing::Printer::System);
                        for pane in ws.panes.borrow().iter() {
                            pane.view.session().terminal().set_printer(true);
                        }
                    }
                    setup
                }
            };
            let size = setup.page_setup().paper_size();
            let paper = (
                size.width(gtk::Unit::Points),
                size.height(gtk::Unit::Points),
            );
            let paper = if paper.0 > 0.0 && paper.1 > 0.0 {
                paper
            } else {
                crate::printing::paper()
            };
            let path = glib::tmp_dir().join(format!(
                "veetee-print-{}-{}.pdf",
                std::process::id(),
                JOBS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let (tx, rx) = async_channel::bounded(1);
            {
                let path = path.clone();
                std::thread::spawn(move || {
                    let _ = tx.send_blocking(crate::printing::write_pdf(&text, &path, paper));
                });
            }
            let pages = match rx.recv().await {
                Ok(Ok(pages)) => pages,
                Ok(Err(e)) => return ws.notify(&format!("Cannot print: {e}")),
                Err(_) => return,
            };
            let printed = dialog
                .print_file_future(
                    Some(&ws.window),
                    Some(&setup),
                    &gtk::gio::File::for_path(&path),
                )
                .await;
            let _ = std::fs::remove_file(&path);
            let printer = setup
                .print_settings()
                .printer()
                .map(|p| p.to_string())
                .unwrap_or_else(|| "the printer".into());
            match printed {
                Ok(()) if choosing => {
                    ws.notify(&format!("Printing to {printer}; a test page is on its way"))
                }
                Ok(()) => ws.notify(&format!(
                    "Sent to {printer}{}",
                    if pages == 1 {
                        String::new()
                    } else {
                        format!(", {pages} pages")
                    }
                )),
                Err(e) => ws.notify(&format!("Cannot print: {e}")),
            }
        });
    }

    /// Sends text to the chosen printer through GTK's print operation — CUPS
    /// on Linux, the Windows print system on Windows, the print portal in the
    /// Flatpak — with no dialog, as to a printer on the terminal's port.
    /// `choosing` shows the print dialog instead, to pick the printer, and
    /// keeps what was chosen for every job after. The Windows path: there is
    /// no portal there, and GTK's print dialog is not known to work.
    #[cfg(windows)]
    fn print_on_printer(self: &Rc<Self>, text: &str, job_name: &str, choosing: bool) {
        let op = gtk::PrintOperation::new();
        op.set_unit(gtk::Unit::Points);
        op.set_job_name(job_name);
        let settings = gtk::PrintSettings::from_file(crate::printing::settings_path()).ok();
        let page_setup = gtk::PageSetup::new();
        if let Some(paper) = settings.as_ref().and_then(gtk::PrintSettings::paper_size) {
            page_setup.set_paper_size(&paper);
        }
        if crate::printing::landscape(text) {
            page_setup.set_orientation(gtk::PageOrientation::Landscape);
        }
        op.set_default_page_setup(Some(&page_setup));
        if let Some(settings) = &settings {
            op.set_print_settings(Some(settings));
        }
        let text = text.to_string();
        let widest = crate::printing::widest(&text);
        let pages = Rc::new(RefCell::new(Vec::new()));
        op.connect_begin_print({
            let (pages, text) = (pages.clone(), text.clone());
            move |op, context| {
                let lines = crate::printing::lines_per_page(
                    context.height(),
                    crate::printing::PRINTER_MARGIN,
                );
                let paged = crate::printing::paginate(&text, lines);
                op.set_n_pages(paged.len() as i32);
                *pages.borrow_mut() = paged;
            }
        });
        op.connect_draw_page({
            let pages = pages.clone();
            move |_, context, n| {
                let cr = context.cairo_context();
                if let Some(page) = pages.borrow().get(n as usize) {
                    let _ = crate::printing::draw_page(
                        &cr,
                        page,
                        widest,
                        context.width(),
                        crate::printing::PRINTER_MARGIN,
                    );
                }
            }
        });
        let action = if choosing || settings.is_none() {
            gtk::PrintOperationAction::PrintDialog
        } else {
            gtk::PrintOperationAction::Print
        };
        match op.run(action, Some(&self.window)) {
            Ok(gtk::PrintOperationResult::Apply) => {
                if let Some(chosen) = op.print_settings() {
                    let _ = std::fs::create_dir_all(
                        crate::printing::settings_path()
                            .parent()
                            .unwrap_or(std::path::Path::new(".")),
                    );
                    if let Err(e) = chosen.to_file(crate::printing::settings_path()) {
                        self.notify(&format!("Cannot keep the printer's settings: {e}"));
                    }
                }
                let printer = op
                    .print_settings()
                    .and_then(|s| s.printer())
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "the printer".into());
                if choosing {
                    crate::printing::save(&crate::printing::Printer::System);
                    for pane in self.panes.borrow().iter() {
                        pane.view.session().terminal().set_printer(true);
                    }
                    self.notify(&format!("Printing to {printer}; a test page is on its way"));
                } else {
                    let count = pages.borrow().len();
                    self.notify(&format!(
                        "Sent to {printer}{}",
                        if count == 1 {
                            String::new()
                        } else {
                            format!(", {count} pages")
                        }
                    ));
                }
            }
            Ok(_) => {}
            Err(e) => self.notify(&format!("Cannot print: {e}")),
        }
    }

    /// Print to Printer: the print dialog, with a test page, to choose the
    /// printer every job goes to after it.
    pub fn choose_printer(self: &Rc<Self>) {
        self.print_on_printer(&crate::printing::test_page(), "veetee printer test", true);
    }

    /// Print to Folder: where print jobs go from now on. Choosing one also
    /// tells every session's host that a printer is ready.
    pub fn choose_print_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Print to Folder")
            .accept_label("Print Here")
            .build();
        if let crate::printing::Printer::Folder(folder) = crate::printing::load() {
            dialog.set_initial_folder(Some(&gtk::gio::File::for_path(folder)));
        }
        let weak = Rc::downgrade(self);
        dialog.select_folder(
            Some(&self.window),
            gtk::gio::Cancellable::NONE,
            move |result| {
                let (Ok(chosen), Some(ws)) = (result, weak.upgrade()) else {
                    return;
                };
                let Some(folder) = chosen.path() else { return };
                crate::printing::save(&crate::printing::Printer::Folder(folder.clone()));
                for pane in ws.panes.borrow().iter() {
                    pane.view.session().terminal().set_printer(true);
                }
                ws.notify(&format!("Printing to {}", folder.display()));
            },
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use vt_core::Model;

    #[test]
    fn a_vt520_has_four_sessions_and_a_vt420_two() {
        assert_eq!(max_sessions(Model::Vt520), 4);
        assert_eq!(max_sessions(Model::Vt525), 4);
        assert_eq!(max_sessions(Model::Vt420), 2);
        assert_eq!(max_sessions(Model::Vt510), 2);
        assert_eq!(max_sessions(Model::Vt320), 1);
        assert_eq!(max_sessions(Model::Vt100), 1);
    }

    #[test]
    fn a_new_session_takes_the_lowest_free_number() {
        assert_eq!(free_number([]), 1);
        assert_eq!(free_number([1, 2]), 3);
        // S2 closed, so the next session is S2 again, with S2's Set-Up.
        assert_eq!(free_number([1, 3, 4]), 2);
    }

    #[test]
    fn the_screen_shows_the_active_session_and_the_one_before() {
        // One session: the whole window.
        assert_eq!(shown(&[1], &[1], true), [1]);
        // Four open, S3 active and S1 before it: those two, S1 on top.
        assert_eq!(shown(&[1, 2, 3, 4], &[3, 1, 4, 2], true), [1, 3]);
        // Going to S4 puts it beside S3, the one it came from.
        assert_eq!(shown(&[1, 2, 3, 4], &[4, 3, 1, 2], true), [3, 4]);
        // A session opened but never active still shows beside the active one.
        assert_eq!(shown(&[1, 2], &[1], true), [1, 2]);
        // A closed session in the history is passed over.
        assert_eq!(shown(&[1, 4], &[2, 4, 1], true), [1, 4]);
    }

    #[test]
    fn one_window_shows_only_the_active_session() {
        assert_eq!(shown(&[1, 2, 3, 4], &[3, 1, 4, 2], false), [3]);
        assert_eq!(shown(&[1, 2], &[], false), [1]);
    }
}
