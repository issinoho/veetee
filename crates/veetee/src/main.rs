//! veetee — a DEC VT terminal for the Linux (and Windows) desktop.
// Release builds on Windows start without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod appearance;
mod cli;
mod connections;
mod gl_loader;
mod keymap_editor;
mod keymaps;
mod log;
mod profiles;
mod session;
mod setup_store;
mod sound;
mod view;
mod workspace;

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use vt_core::Config;
use vt_render::Phosphor;

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
        Ok(Parsed::ListProfiles) => {
            return match profiles::load() {
                Ok(list) => {
                    for p in &list {
                        println!("{:<20} {}", p.name, p.summary());
                    }
                    if list.is_empty() {
                        println!("No saved connections ({}).", profiles::path().display());
                    }
                    glib::ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("veetee: {e}");
                    glib::ExitCode::FAILURE
                }
            };
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
    let name = options.profile.clone().unwrap_or_else(|| "veetee".into());
    let title = adw::WindowTitle::new(&name, &base_subtitle);
    let header = adw::HeaderBar::builder().title_widget(&title).build();

    let menu = gio::Menu::new();
    let phosphor = gio::Menu::new();
    phosphor.append(Some("White (P4)"), Some("win.phosphor::white"));
    phosphor.append(Some("Green (P1)"), Some("win.phosphor::green"));
    phosphor.append(Some("Amber (P3)"), Some("win.phosphor::amber"));
    menu.append_section(Some("Phosphor"), &phosphor);
    let effects = gio::Menu::new();
    effects.append(Some("Glow"), Some("win.glow"));
    effects.append(Some("Afterglow"), Some("win.afterglow"));
    effects.append(Some("Curved Screen"), Some("win.curvature"));
    effects.append(Some("Visible Bell"), Some("win.visible-bell"));
    menu.append_section(Some("Screen"), &effects);
    let connection_section = gio::Menu::new();
    connection_section.append(Some("Connections…"), Some("win.connections"));
    connection_section.append(Some("Save as Connection…"), Some("win.save-connection"));
    menu.append_section(None, &connection_section);
    let session_section = gio::Menu::new();
    session_section.append(Some("Set-Up"), Some("win.setup"));
    session_section.append(Some("Open Second Session"), Some("win.new-session"));
    session_section.append(Some("Find…"), Some("win.search"));
    session_section.append(Some("Mark Checkpoint"), Some("win.mark-checkpoint"));
    session_section.append(Some("Log to File…"), Some("win.log"));
    session_section.append(Some("Timestamp Log Lines"), Some("win.log-timestamps"));
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
        .title(name.as_str())
        .default_width(1024)
        .default_height(820)
        .content(&toolbar)
        .build();
    add_window_actions(&window);
    window.present();
    capture_window_later(&window);

    // Open the audio output now rather than on the first keyclick.
    std::thread::spawn(sound::warm_up);

    // The saved Set-Up settings are the terminal's power-up settings.
    let mut first = config.clone();
    setup_store::load_into(&mut first, 1);

    // Connecting can block on DNS or TCP; do it off the UI thread.
    let (tx, rx) = async_channel::bounded(1);
    {
        let (config, connection) = (first.clone(), options.connection.clone());
        std::thread::spawn(move || {
            let _ = tx.send_blocking(cli::open_transport(&config, &connection));
        });
    }
    glib::spawn_future_local(async move {
        let Ok(opened) = rx.recv().await else { return };
        let started = opened.and_then(|t| {
            session::Session::start(first, t, options.record.as_ref(), options.log.as_ref())
        });
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
        workspace.set_phosphor(phosphor);
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

/// Opens a saved connection in a new window.
pub fn open_profile(app: &adw::Application, profile: &profiles::Profile) {
    let mut config = Config::default();
    let mut options = Options {
        sessions: 1,
        phosphor: "white".into(),
        ..Options::default()
    };
    profile.apply(&mut config, &mut options);
    build_window(app, config, options);
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
            workspace.set_phosphor(phosphor_named(&name));
        }
    });
    window.add_action(&phosphor_action);

    type Toggle = fn(&mut appearance::Appearance) -> &mut bool;
    let toggles: [(&str, Toggle); 4] = [
        ("glow", |a| &mut a.effects.glow),
        ("afterglow", |a| &mut a.effects.afterglow),
        ("curvature", |a| &mut a.effects.curvature),
        ("visible-bell", |a| &mut a.visible_bell),
    ];
    for (name, field) in toggles {
        let mut current = workspace.appearance();
        let action =
            gio::SimpleAction::new_stateful(name, None, &(*field(&mut current)).to_variant());
        action.connect_activate({
            let workspace = Rc::downgrade(workspace);
            move |action, _| {
                let Some(ws) = workspace.upgrade() else {
                    return;
                };
                let mut appearance = ws.appearance();
                let on = field(&mut appearance);
                *on = !*on;
                action.set_state(&(*on).to_variant());
                ws.set_appearance(appearance);
            }
        });
        window.add_action(&action);
    }

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

    let setup = gio::SimpleAction::new("setup", None);
    setup.connect_activate({
        let workspace = Rc::downgrade(workspace);
        move |_, _| {
            if let Some(ws) = workspace.upgrade() {
                ws.open_setup();
            }
        }
    });
    window.add_action(&setup);

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

    let log = gio::SimpleAction::new_stateful("log", None, &workspace.is_logging().to_variant());
    log.connect_activate({
        let workspace = Rc::downgrade(workspace);
        move |_, _| {
            if let Some(ws) = workspace.upgrade() {
                ws.toggle_log();
            }
        }
    });
    window.add_action(&log);

    let stamps = gio::SimpleAction::new_stateful(
        "log-timestamps",
        None,
        &workspace.appearance().log_timestamps.to_variant(),
    );
    stamps.connect_activate({
        let workspace = Rc::downgrade(workspace);
        move |action, _| {
            if let Some(ws) = workspace.upgrade() {
                let mut appearance = ws.appearance();
                appearance.log_timestamps = !appearance.log_timestamps;
                action.set_state(&appearance.log_timestamps.to_variant());
                ws.set_appearance(appearance);
            }
        }
    });
    window.add_action(&stamps);

    let search = gio::SimpleAction::new("search", None);
    search.connect_activate({
        let workspace = Rc::downgrade(workspace);
        move |_, _| {
            if let Some(ws) = workspace.upgrade() {
                ws.search();
            }
        }
    });
    window.add_action(&search);

    let save_connection = gio::SimpleAction::new("save-connection", None);
    save_connection.connect_activate({
        let (workspace, window) = (Rc::downgrade(workspace), window.downgrade());
        move |_, _| {
            if let (Some(ws), Some(w)) = (workspace.upgrade(), window.upgrade()) {
                connections::save_as(&w, ws.profile());
            }
        }
    });
    window.add_action(&save_connection);

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

    let connections = gio::SimpleAction::new("connections", None);
    connections.connect_activate({
        let window = window.downgrade();
        move |_, _| {
            if let Some(w) = window.upgrade() {
                connections::open(&w);
            }
        }
    });
    window.add_action(&connections);

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
