//! veetee — a DEC VT terminal for the Linux desktop.

mod cli;
mod gl_loader;
mod session;
mod view;

use adw::prelude::*;
use gtk::{gio, glib};
use vt_core::Config;
use vt_render::{Phosphor, Theme};

use cli::{Options, Parsed};

const APP_ID: &str = "com.issinoho.Veetee";

fn main() -> glib::ExitCode {
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
        let started =
            opened.and_then(|t| session::Session::start(config, t, options.record.as_deref()));
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
        attach_terminal(
            &window,
            &title,
            &toasts,
            base_subtitle,
            &options,
            session,
            notices,
        );
    });
}

fn attach_terminal(
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    toasts: &adw::ToastOverlay,
    base_subtitle: String,
    options: &Options,
    session: session::Session,
    notices: async_channel::Receiver<session::Notice>,
) {
    let notify = {
        let toasts = toasts.clone();
        move |msg: &str| toasts.add_toast(adw::Toast::new(msg))
    };
    let status = {
        let title = title.clone();
        move |extra: &str| {
            if extra.is_empty() {
                title.set_subtitle(&base_subtitle);
            } else {
                title.set_subtitle(&format!("{base_subtitle} · {extra}"));
            }
        }
    };
    let keep_open = options.connection.keep_open_on_close();
    let label = options.connection.label();
    let on_exit = {
        let (window, session, toasts, title) = (
            window.downgrade(),
            session.clone(),
            toasts.clone(),
            title.clone(),
        );
        move |reason: Option<String>| {
            session.close();
            if keep_open {
                // Keep the screen readable (and copyable) after the line drops.
                let msg = match reason {
                    Some(r) => format!("Connection closed: {r}"),
                    None => format!("Connection to {label} closed"),
                };
                eprintln!("veetee: {msg}");
                title.set_subtitle(&format!("{label} · Disconnected"));
                toasts.add_toast(adw::Toast::builder().title(msg).timeout(0).build());
            } else if let Some(w) = window.upgrade() {
                w.close();
            }
        }
    };
    let view = view::TerminalView::new(session.clone(), notices, notify, status, on_exit);
    toasts.set_child(Some(view.widget()));

    let phosphor_action = gio::SimpleAction::new_stateful(
        "phosphor",
        Some(glib::VariantTy::STRING),
        &"white".to_variant(),
    );
    phosphor_action.connect_activate({
        let view = view.clone();
        move |action, param| {
            let Some(name) = param.and_then(|p| p.str().map(str::to_owned)) else {
                return;
            };
            let p = match name.as_str() {
                "green" => Phosphor::Green,
                "amber" => Phosphor::Amber,
                _ => Phosphor::White,
            };
            action.set_state(&name.to_variant());
            view.set_theme(Theme::phosphor(p));
        }
    });
    window.add_action(&phosphor_action);

    window.connect_close_request(move |_| {
        session.close();
        glib::Propagation::Proceed
    });
    view.widget().grab_focus();
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
