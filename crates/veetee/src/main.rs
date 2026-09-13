//! veetee — a DEC VT terminal for the Linux desktop.

mod gl_loader;
mod session;
mod view;

use adw::prelude::*;
use gtk::{gio, glib};
use vt_core::{Config, Model};
use vt_render::{Phosphor, Theme};
use vt_transport::Transport;
use vt_transport::pty::Pty;
use vt_transport::serial::{Serial, SerialConfig};

const APP_ID: &str = "com.issinoho.Veetee";

const USAGE: &str = "\
usage: veetee [--model MODEL] [--record FILE] [--command COMMAND | --serial DEVICE [LINE OPTIONS]]

  --model MODEL        vt100 vt102 vt220 vt320 vt420 (default) vt510 vt520 vt525
  --record FILE        append everything the host sends to FILE
  --command COMMAND    run COMMAND (via /bin/sh -c) instead of your shell,
                       e.g. --command \"telnet vms1\"
  --serial DEVICE      connect to a serial line, e.g. --serial /dev/ttyUSB0

serial line options (picocom style; defaults are DEC factory Set-Up):
  -b, --baud RATE      bits per second (9600)
  -d, --databits N     5, 6, 7 or 8 (8)
  -p, --parity P       n none, e even, o odd, m mark, s space (n)
  -s, --stopbits N     1 or 2 (1)
  -f, --flow F         x XON/XOFF, h RTS/CTS, n none (x)";

fn main() -> glib::ExitCode {
    let (config, options) = match parse_args(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(msg) if msg == "help" => {
            println!("{USAGE}");
            return glib::ExitCode::SUCCESS;
        }
        Err(msg) => {
            eprintln!("veetee: {msg}\n\n{USAGE}");
            return glib::ExitCode::FAILURE;
        }
    };

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
    serial: Option<SerialConfig>,
    record: Option<std::path::PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<(Config, Options), String> {
    let mut config = Config::default();
    let mut options = Options::default();
    let mut line: Vec<(String, String)> = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--model" => {
                let v = value()?;
                config.model = parse_model(&v).ok_or_else(|| format!("unknown model {v:?}"))?;
            }
            "--command" => options.command = Some(value()?),
            "--record" => options.record = Some(value()?.into()),
            "--serial" => options.serial = Some(SerialConfig::new(value()?)),
            "-b" | "--baud" | "-d" | "--databits" | "-p" | "--parity" | "-s" | "--stopbits"
            | "-f" | "--flow" => {
                let v = value()?;
                line.push((arg, v));
            }
            "-h" | "--help" => return Err("help".into()),
            _ => return Err(format!("unexpected argument {arg:?}")),
        }
    }
    if options.command.is_some() && options.serial.is_some() {
        return Err("use either --command or --serial, not both".into());
    }
    match options.serial.as_mut() {
        Some(serial) => {
            for (flag, v) in line {
                let number = |v: &str| {
                    v.parse::<u32>()
                        .map_err(|_| format!("{flag}: not a number: {v:?}"))
                };
                match flag.as_str() {
                    "-b" | "--baud" => serial.baud = number(&v)?,
                    "-d" | "--databits" => serial.data_bits = number(&v)? as u8,
                    "-p" | "--parity" => serial.parity = v.parse()?,
                    "-s" | "--stopbits" => serial.stop_bits = number(&v)? as u8,
                    _ => serial.flow = v.parse()?,
                }
            }
        }
        None if !line.is_empty() => return Err("line options need --serial DEVICE".into()),
        None => {}
    }
    Ok((config, options))
}

/// Opens the connection the options ask for.
fn open_transport(config: &Config, options: &Options) -> std::io::Result<Box<dyn Transport>> {
    let (rows, cols, term) = (
        config.rows as u16,
        config.cols as u16,
        config.model.term_name(),
    );
    Ok(match (&options.serial, &options.command) {
        (Some(serial), _) => Box::new(Serial::open(serial.clone())?),
        (None, Some(cmd)) => Box::new(Pty::spawn("/bin/sh", &["-c", cmd], rows, cols, term)?),
        (None, None) => {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            Box::new(Pty::spawn::<&str>(&shell, &[], rows, cols, term)?)
        }
    })
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
    let transport = open_transport(&config, options);
    let connection = match (&transport, &options.command) {
        (Ok(t), _) if options.serial.is_some() => t.description(),
        (_, Some(cmd)) => cmd.clone(),
        _ => "Local shell".to_string(),
    };
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

    let started =
        transport.and_then(|t| session::Session::start(config, t, options.record.as_deref()));
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
    let keep_open = options.serial.is_some();
    let on_exit = {
        let (window, session, toasts) = (window.downgrade(), session.clone(), toasts.clone());
        let connection = connection.clone();
        move |reason: Option<String>| {
            session.close();
            if keep_open {
                // A serial line going away (e.g. an unplugged adapter) should not
                // throw away the screen.
                let msg = match reason {
                    Some(r) => format!("Connection closed: {r}"),
                    None => format!("Connection to {connection} closed"),
                };
                eprintln!("veetee: {msg}");
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

#[cfg(test)]
mod tests {
    use super::*;
    use vt_transport::serial::{FlowControl, Parity};

    fn parse(args: &[&str]) -> Result<(Config, Options), String> {
        parse_args(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn picocom_style_serial_options() {
        let (_, o) = parse(&[
            "--serial",
            "/dev/ttyUSB0",
            "-b",
            "19200",
            "-d",
            "7",
            "-p",
            "e",
            "-s",
            "2",
            "-f",
            "n",
        ])
        .unwrap();
        let s = o.serial.unwrap();
        assert_eq!(
            (s.baud, s.data_bits, s.parity, s.stop_bits, s.flow),
            (19200, 7, Parity::Even, 2, FlowControl::None)
        );
    }

    #[test]
    fn serial_defaults_are_dec_factory_settings() {
        let (_, o) = parse(&["--serial", "/dev/ttyS0"]).unwrap();
        assert_eq!(o.serial.unwrap().to_string(), "/dev/ttyS0 9600 8N1");
    }

    #[test]
    fn rejects_conflicting_or_orphaned_options() {
        assert!(parse(&["--serial", "/dev/ttyS0", "--command", "sh"]).is_err());
        assert!(parse(&["-b", "9600"]).is_err());
        assert!(parse(&["--serial", "/dev/ttyS0", "-p", "q"]).is_err());
    }
}
