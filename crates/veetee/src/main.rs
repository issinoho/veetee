//! veetee — a DEC VT terminal for the Linux (and Windows) desktop.
// Release builds on Windows start without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod cli;
mod gl_loader;
mod keymap_editor;
mod keymaps;
mod session;
mod view;
mod workspace;

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use vt_core::Config;
use vt_render::{Phosphor, Theme};

use cli::{Options, Parsed};

const APP_ID: &str = "com.issinoho.Veetee";

/// A Windows GUI program has no console; when started from one, write
/// messages (such as `--help`) to it.
#[cfg(windows)]
#[allow(unsafe_code)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: no arguments to validate; failure just means no parent console.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_console() {}

fn main() -> glib::ExitCode {
    attach_console();
    let (config, options) = match cli::parse_args(std::env::args().skip(1)) {
        Ok(Parsed::Run(config, options)) => (config, options),
        Ok(Parsed::Help) => {
            println!("{}", cli::USAGE);
            return glib::ExitCode::SUCCESS;
        }
        Err(msg) => {
            eprintln!("veetee: {msg}\n\n{}", cli::USAGE);
            return glib::ExitCode::FAILURE;
        }
    };

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| build_window(app, config.clone(), options.clone()));
    // Arguments are handled above; don't let GTK interpret them.
    app.run_with_args::<&str>(&[])
}

fn build_window(app: &adw::Application, config: Config, options: Options) {
    let connection = options.connection.label();
    let base_subtitle = format!("{} · {connection}", cli::model_name(config.model));
    let title = adw::WindowTitle::new("veetee", &base_subtitle);
    let header = adw::HeaderBar::builder().title_widget(&title).build();

    let menu = gio::Menu::new();
    let phosphor = gio::Menu::new();
    phosphor.append(Some("White (P4)"), Some("win.phosphor::white"));
    phosphor.append(Some("Green (P1)"), Some("win.phosphor::green"));
    phosphor.append(Some("Amber (P3)"), Some("win.phosphor::amber"));
    menu.append_section(Some("Phosphor"), &phosphor);
    let session_section = gio::Menu::new();
    session_section.append(Some("Open Second Session"), Some("win.new-session"));
    session_section.append(Some("Mark Checkpoint"), Some("win.mark-checkpoint"));
    session_section.append(Some("Keyboard Map…"), Some("win.keymap"));
    menu.append_section(None, &session_section);
    let window_section = gio::Menu::new();
    window_section.append(Some("Full Screen"), Some("win.fullscreen"));
    window_section.append(Some("About veetee"), Some("win.about"));
    menu.append_section(None, &window_section);
    header.pack_end(
        &gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .build(),
    );

    let toasts = adw::ToastOverlay::new();
    let connecting = adw::StatusPage::builder()
        .icon_name("network-transmit-receive-symbolic")
        .title("Connecting")
        .description(glib::markup_escape_text(&connection).as_str())
        .build();
    toasts.set_child(Some(&connecting));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toasts));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("veetee")
        .default_width(1024)
        .default_height(820)
        .content(&toolbar)
        .build();
    add_window_actions(&window);
    window.present();
    capture_window_later(&window);

    // Connecting can block on DNS or TCP; do it off the UI thread.
    let (tx, rx) = async_channel::bounded(1);
    {
        let (config, connection) = (config.clone(), options.connection.clone());
        std::thread::spawn(move || {
            let _ = tx.send_blocking(cli::open_transport(&config, &connection));
        });
    }
    glib::spawn_future_local(async move {
        let Ok(opened) = rx.recv().await else { return };
        let started = opened
            .and_then(|t| session::Session::start(config.clone(), t, options.record.as_ref()));
        let (session, notices) = match started {
            Ok(s) => s,
            Err(e) => {
                show_fatal_error(
                    &window,
                    &format!("Cannot connect to {connection}"),
                    &e.to_string(),
                );
                return;
            }
        };
        let phosphor = phosphor_named(&options.phosphor);
        let workspace =
            workspace::Workspace::new(&window, &title, &toasts, config, options, base_subtitle);
        workspace.set_theme(Theme::phosphor(phosphor));
        workspace.add_session(session, notices);
        if workspace.sessions_to_open() > 1 {
            workspace.open_session();
        }
        add_session_actions(&window, &workspace);
        // Developer hook for screenshots: VEETEE_STARTUP_ACTION=keymap.
        if let Ok(action) = std::env::var("VEETEE_STARTUP_ACTION") {
            gtk::prelude::ActionGroupExt::activate_action(&window, &action, None);
        }
    });
}

fn phosphor_named(name: &str) -> Phosphor {
    match name {
        "green" => Phosphor::Green,
        "amber" => Phosphor::Amber,
        _ => Phosphor::White,
    }
}

fn add_session_actions(window: &adw::ApplicationWindow, workspace: &Rc<workspace::Workspace>) {
    let initial = workspace.phosphor();
    let phosphor_action = gio::SimpleAction::new_stateful(
        "phosphor",
        Some(glib::VariantTy::STRING),
        &initial.to_variant(),
    );
    phosphor_action.connect_activate({
        let workspace = workspace.clone();
        move |action, param| {
            let Some(name) = param.and_then(|p| p.str().map(str::to_owned)) else {
                return;
            };
            action.set_state(&name.to_variant());
            workspace.set_theme(Theme::phosphor(phosphor_named(&name)));
        }
    });
    window.add_action(&phosphor_action);

    let new_session = gio::SimpleAction::new("new-session", None);
    new_session.connect_activate({
        let workspace = Rc::downgrade(workspace);
        move |_, _| {
            if let Some(ws) = workspace.upgrade() {
                ws.open_session();
            }
        }
    });
    window.add_action(&new_session);

    let mark = gio::SimpleAction::new("mark-checkpoint", None);
    mark.connect_activate({
        let workspace = Rc::downgrade(workspace);
        move |_, _| {
            if let Some(ws) = workspace.upgrade() {
                ws.mark_checkpoint();
            }
        }
    });
    window.add_action(&mark);

    let keymap = gio::SimpleAction::new("keymap", None);
    keymap.connect_activate({
        let (workspace, window) = (Rc::downgrade(workspace), window.downgrade());
        move |_, _| {
            if let (Some(ws), Some(w)) = (workspace.upgrade(), window.upgrade()) {
                keymap_editor::open(&w, ws.keymap());
            }
        }
    });
    window.add_action(&keymap);
}

fn add_window_actions(window: &adw::ApplicationWindow) {
    let fullscreen = gio::SimpleAction::new("fullscreen", None);
    fullscreen.connect_activate({
        let window = window.downgrade();
        move |_, _| {
            if let Some(w) = window.upgrade() {
                w.set_fullscreened(!w.is_fullscreen());
            }
        }
    });
    window.add_action(&fullscreen);

    let about = gio::SimpleAction::new("about", None);
    about.connect_activate({
        let window = window.downgrade();
        move |_, _| {
            let dialog = adw::AboutDialog::builder()
                .application_name("veetee")
                .developer_name("The veetee Authors")
                .version(env!("CARGO_PKG_VERSION"))
                .comments("A DEC VT terminal for the Linux desktop")
                .license_type(gtk::License::MitX11)
                .build();
            if let Some(w) = window.upgrade() {
                dialog.present(Some(&w));
            }
        }
    });
    window.add_action(&about);
}

/// Developer hook: `VEETEE_CAPTURE_WINDOW=file.png` saves the whole window
/// after `VEETEE_CAPTURE_DELAY_MS` (default 2000) and quits.
fn capture_window_later(window: &adw::ApplicationWindow) {
    let Some(path) = std::env::var_os("VEETEE_CAPTURE_WINDOW") else {
        return;
    };
    let delay = std::env::var("VEETEE_CAPTURE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let window = window.downgrade();
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        let Some(main) = window.upgrade() else {
            return;
        };
        // The active window: a dialog, if one is open.
        let window: gtk::Window = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .find(|w| w.is_active())
            .unwrap_or_else(|| main.clone().upcast());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let (w, h) = (window.width(), window.height());
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
        let saved = snapshot
            .to_node()
            .zip(window.native().and_then(|n| n.renderer()))
            .map(|(node, renderer)| renderer.render_texture(&node, None))
            .map(|texture| texture.save_to_png(&path));
        match saved {
            Some(Ok(())) => eprintln!("veetee: captured window to {}", path.to_string_lossy()),
            Some(Err(e)) => eprintln!("veetee: window capture failed: {e}"),
            None => eprintln!("veetee: window capture failed: nothing rendered"),
        }
        if let Some(app) = main.application() {
            app.quit();
        }
    });
}

/// Shows an error in place of a terminal and closes the window when dismissed.
fn show_fatal_error(window: &adw::ApplicationWindow, heading: &str, detail: &str) {
    eprintln!("veetee: {heading}: {detail}");
    window.present();
    let dialog = adw::AlertDialog::new(Some(heading), Some(detail));
    dialog.add_response("close", "Close");
    let weak = window.downgrade();
    dialog.connect_response(None, move |_, _| {
        if let Some(w) = weak.upgrade() {
            w.close();
        }
    });
    dialog.present(Some(window));
}
