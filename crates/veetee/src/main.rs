//! veetee — a DEC VT terminal for the Linux desktop.

mod gl_loader;
mod session;
mod view;

use adw::prelude::*;
use gtk::{gio, glib};
use vt_core::{Config, Model};
use vt_render::{Phosphor, Theme};

const APP_ID: &str = "com.issinoho.Veetee";

const USAGE: &str = "\
usage: veetee [--model MODEL] [--command COMMAND] [--record FILE]

  --model MODEL      vt100 vt102 vt220 vt320 vt420 (default) vt510 vt520 vt525
  --command COMMAND  run COMMAND (via /bin/sh -c) instead of your shell,
                     e.g. --command \"telnet vms1\"
  --record FILE      append everything the host sends to FILE";

fn main() -> glib::ExitCode {
    let mut config = Config::default();
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--command" | "--record" => {
                let Some(value) = args.next() else {
                    eprintln!("{USAGE}");
                    return glib::ExitCode::FAILURE;
                };
                if arg == "--command" {
                    options.command = Some(value);
                } else {
                    options.record = Some(value.into());
                }
            }
            "--model" => match args.next().as_deref().and_then(parse_model) {
                Some(model) => config.model = model,
                None => {
                    eprintln!("{USAGE}");
                    return glib::ExitCode::FAILURE;
                }
            },
            _ => {
                eprintln!("{USAGE}");
                return glib::ExitCode::FAILURE;
            }
        }
    }

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| build_window(app, config.clone(), &options));
    // Arguments are handled above; don't let GTK interpret them.
    app.run_with_args::<&str>(&[])
}

#[derive(Debug, Clone, Default)]
struct Options {
    command: Option<String>,
    record: Option<std::path::PathBuf>,
}

fn parse_model(name: &str) -> Option<Model> {
    Some(match name.to_ascii_lowercase().as_str() {
        "vt100" => Model::Vt100,
        "vt102" => Model::Vt102,
        "vt220" => Model::Vt220,
        "vt320" => Model::Vt320,
        "vt420" => Model::Vt420,
        "vt510" => Model::Vt510,
        "vt520" => Model::Vt520,
        "vt525" => Model::Vt525,
        _ => return None,
    })
}

fn model_name(model: Model) -> &'static str {
    match model {
        Model::Vt100 => "VT100",
        Model::Vt102 => "VT102",
        Model::Vt220 => "VT220",
        Model::Vt320 => "VT320",
        Model::Vt420 => "VT420",
        Model::Vt510 => "VT510",
        Model::Vt520 => "VT520",
        Model::Vt525 => "VT525",
    }
}

fn build_window(app: &adw::Application, config: Config, options: &Options) {
    let connection = options.command.as_deref().unwrap_or("Local shell");
    let base_subtitle = format!("{} · {connection}", model_name(config.model));
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

    let (session, notices) = match session::Session::local(
        config,
        options.command.as_deref(),
        options.record.as_deref(),
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("veetee: cannot start shell: {e}");
            app.quit();
            return;
        }
    };

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
    let on_exit = {
        let (window, session) = (window.downgrade(), session.clone());
        move || {
            session.close();
            if let Some(w) = window.upgrade() {
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

    window.connect_close_request(move |_| {
        session.close();
        glib::Propagation::Proceed
    });
    window.present();
    view.widget().grab_focus();
}
