//! The Connections window: saved connections (profiles) to open in a new
//! window, add, change and delete.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use vt_core::Model;
use vt_transport::serial::{FlowControl, Parity, SerialConfig};
use vt_transport::ssh::SshConfig;

use crate::cli::{self, Connection};
use crate::profiles::{self, Profile};

const MODELS: [Model; 8] = [
    Model::Vt100,
    Model::Vt102,
    Model::Vt220,
    Model::Vt320,
    Model::Vt420,
    Model::Vt510,
    Model::Vt520,
    Model::Vt525,
];
const KINDS: [&str; 5] = ["Telnet", "SSH", "Serial line", "Local shell", "Command"];
const PHOSPHORS: [(&str, &str); 3] = [
    ("white", "White (P4)"),
    ("green", "Green (P1)"),
    ("amber", "Amber (P3)"),
];
const SPEEDS: [u32; 11] = [
    300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 76800, 115200,
];
const DATA_BITS: [u8; 4] = [5, 6, 7, 8];
const PARITIES: [(Parity, &str); 5] = [
    (Parity::None, "None"),
    (Parity::Even, "Even"),
    (Parity::Odd, "Odd"),
    (Parity::Mark, "Mark"),
    (Parity::Space, "Space"),
];
const FLOWS: [(FlowControl, &str); 3] = [
    (FlowControl::XonXoff, "XON/XOFF"),
    (FlowControl::RtsCts, "RTS/CTS"),
    (FlowControl::None, "None"),
];

/// Opens the Connections window over `parent`.
pub fn open(parent: &adw::ApplicationWindow) {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let empty = adw::StatusPage::builder()
        .icon_name("network-server-symbolic")
        .title("No Saved Connections")
        .description("Add a connection to open it here or with veetee --profile NAME.")
        .vexpand(true)
        .build();
    let problem = adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title("Connections Cannot Be Read")
        .vexpand(true)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let clamp = adw::Clamp::builder()
        .maximum_size(560)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .child(&list)
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();
    content.append(&scrolled);
    content.append(&empty);
    content.append(&problem);

    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add Connection")
        .build();
    let header = adw::HeaderBar::new();
    header.pack_start(&add);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    let dialog = adw::Dialog::builder()
        .title("Connections")
        .content_width(520)
        .content_height(480)
        .child(&toolbar)
        .build();

    let ui = Rc::new(ListUi {
        parent: parent.clone(),
        dialog: dialog.clone(),
        list,
        scrolled,
        empty,
        problem,
        add: add.clone(),
        profiles: RefCell::new(Vec::new()),
    });
    ui.reload();
    add.connect_clicked({
        let ui = Rc::downgrade(&ui);
        move |_| {
            if let Some(ui) = ui.upgrade() {
                ui.edit(None, None);
            }
        }
    });
    // The window keeps its state until it closes.
    let keep = RefCell::new(Some(ui));
    dialog.connect_closed(move |_| {
        keep.take();
    });
    dialog.present(Some(parent));
}

/// Opens the editor for a new connection filled in from `profile`, such as
/// the connection of the window it was opened from.
pub fn save_as(parent: &adw::ApplicationWindow, profile: Profile) {
    let existing = match profiles::load() {
        Ok(p) => p,
        Err(e) => {
            let alert = adw::AlertDialog::new(Some("Connections Cannot Be Read"), Some(&e));
            alert.add_response("close", "Close");
            alert.present(Some(parent));
            return;
        }
    };
    let names: Vec<String> = existing.iter().map(|p| p.name.clone()).collect();
    editor(parent, Some(profile), None, names, move |profile| {
        let mut all = existing.clone();
        all.push(profile);
        profiles::save(&all).map_err(|e| e.to_string())
    });
}

struct ListUi {
    parent: adw::ApplicationWindow,
    dialog: adw::Dialog,
    list: gtk::ListBox,
    scrolled: gtk::ScrolledWindow,
    empty: adw::StatusPage,
    problem: adw::StatusPage,
    add: gtk::Button,
    profiles: RefCell<Vec<Profile>>,
}

impl ListUi {
    fn reload(self: &Rc<Self>) {
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        let loaded = profiles::load();
        let ok = loaded.is_ok();
        // A file with mistakes is left alone rather than overwritten.
        self.add.set_sensitive(ok);
        self.problem.set_visible(!ok);
        match loaded {
            Ok(list) => {
                self.empty.set_visible(list.is_empty());
                self.scrolled.set_visible(!list.is_empty());
                for (i, p) in list.iter().enumerate() {
                    self.list.append(&self.row(i, p));
                }
                *self.profiles.borrow_mut() = list;
            }
            Err(e) => {
                self.empty.set_visible(false);
                self.scrolled.set_visible(false);
                self.problem
                    .set_description(Some(&glib::markup_escape_text(&e)));
            }
        }
    }

    fn row(self: &Rc<Self>, index: usize, p: &Profile) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&p.name).as_str())
            .subtitle(glib::markup_escape_text(&p.summary()).as_str())
            .activatable(true)
            .build();
        let edit = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("Edit")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let delete = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        row.add_suffix(&edit);
        row.add_suffix(&delete);
        let connect = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .tooltip_text("Connect")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        row.add_suffix(&connect);
        row.set_activatable_widget(Some(&connect));
        // Rows belong to the list, so they hold it weakly.
        connect.connect_clicked({
            let ui = Rc::downgrade(self);
            move |_| {
                let Some(ui) = ui.upgrade() else { return };
                let profile = ui.profiles.borrow().get(index).cloned();
                if let (Some(profile), Some(app)) = (profile, ui.parent.application()) {
                    ui.dialog.close();
                    crate::open_profile(&app.downcast().expect("an adw application"), &profile);
                }
            }
        });
        edit.connect_clicked({
            let ui = Rc::downgrade(self);
            move |_| {
                let Some(ui) = ui.upgrade() else { return };
                let profile = ui.profiles.borrow().get(index).cloned();
                ui.edit(profile, Some(index));
            }
        });
        delete.connect_clicked({
            let ui = Rc::downgrade(self);
            move |_| {
                if let Some(ui) = ui.upgrade() {
                    ui.confirm_delete(index);
                }
            }
        });
        row
    }

    fn edit(self: &Rc<Self>, profile: Option<Profile>, index: Option<usize>) {
        let names: Vec<String> = self
            .profiles
            .borrow()
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != index)
            .map(|(_, p)| p.name.clone())
            .collect();
        let ui = self.clone();
        editor(&self.dialog, profile, index, names, move |profile| {
            let mut all = ui.profiles.borrow().clone();
            match index {
                Some(i) if i < all.len() => all[i] = profile,
                _ => all.push(profile),
            }
            profiles::save(&all).map_err(|e| e.to_string())?;
            ui.reload();
            Ok(())
        });
    }

    fn confirm_delete(self: &Rc<Self>, index: usize) {
        let Some(name) = self.profiles.borrow().get(index).map(|p| p.name.clone()) else {
            return;
        };
        let alert = adw::AlertDialog::new(
            Some("Delete Connection?"),
            Some(&format!(
                "“{name}” will be removed from the saved connections."
            )),
        );
        alert.add_response("cancel", "Cancel");
        alert.add_response("delete", "Delete");
        alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        let ui = self.clone();
        alert.connect_response(Some("delete"), move |_, _| {
            let mut all = ui.profiles.borrow().clone();
            if index < all.len() {
                all.remove(index);
            }
            if let Err(e) = profiles::save(&all) {
                eprintln!("veetee: cannot save connections: {e}");
            }
            ui.reload();
        });
        alert.present(Some(&self.dialog));
    }
}

fn combo(title: &str, items: &[&str]) -> adw::ComboRow {
    adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(items))
        .build()
}

fn position<T: PartialEq>(items: impl IntoIterator<Item = T>, value: T) -> u32 {
    items.into_iter().position(|v| v == value).unwrap_or(0) as u32
}

/// The connection editor. `taken` are the names other connections use;
/// `save` stores the result and may refuse it.
fn editor(
    parent: &impl IsA<gtk::Widget>,
    profile: Option<Profile>,
    index: Option<usize>,
    taken: Vec<String>,
    save: impl Fn(Profile) -> Result<(), String> + 'static,
) {
    let editing = index.is_some();
    let p = profile.unwrap_or_else(|| Profile {
        name: String::new(),
        model: vt_core::Config::default().model,
        connection: Connection::Telnet {
            host: String::new(),
            port: 23,
        },
        phosphor: "white".into(),
        sessions: 1,
        keymap: None,
        log: None,
        log_timestamps: false,
        log_raw: false,
    });

    let name = adw::EntryRow::builder().title("Name").text(&p.name).build();
    let kind = combo("Type", &KINDS);
    let host = adw::EntryRow::builder().title("Host").build();
    let port = adw::SpinRow::with_range(0.0, 65535.0, 1.0);
    port.set_title("Port");
    port.set_subtitle("0 uses the standard port");
    let command = adw::EntryRow::builder().title("Command").build();
    let device = adw::EntryRow::builder().title("Device").build();
    let speed = combo(
        "Speed",
        &SPEEDS
            .map(|s| s.to_string())
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let data_bits = combo("Data bits", &["5", "6", "7", "8"]);
    let parity = combo("Parity", &PARITIES.map(|p| p.1));
    let stop_bits = combo("Stop bits", &["1", "2"]);
    let flow = combo("Flow control", &FLOWS.map(|f| f.1));
    let names: Vec<&str> = MODELS.iter().map(|m| cli::model_name(*m)).collect();
    let model = combo("Terminal", &names);
    let phosphor = combo("Phosphor", &PHOSPHORS.map(|p| p.1));
    let log_file = adw::EntryRow::builder()
        .title("Log file (~ and %Y %m %d %H %M %S expand)")
        .text(p.log.as_deref().unwrap_or(""))
        .build();
    let log_stamps = adw::SwitchRow::builder()
        .title("Timestamp log lines")
        .active(p.log_timestamps)
        .build();
    let two = adw::SwitchRow::builder()
        .title("Two sessions")
        .subtitle("Split the window; F4 switches")
        .active(p.sessions == 2)
        .build();

    // Fill in the connection.
    let mut serial = SerialConfig::new(if cfg!(windows) {
        "COM1"
    } else {
        "/dev/ttyUSB0"
    });
    match &p.connection {
        Connection::Telnet { host: h, port: n } => {
            kind.set_selected(0);
            host.set_text(h);
            port.set_value(if *n == 23 { 0.0 } else { f64::from(*n) });
        }
        Connection::Ssh(s) => {
            kind.set_selected(1);
            host.set_text(&s.destination);
            port.set_value(s.port.map_or(0.0, f64::from));
        }
        Connection::Serial(s) => {
            kind.set_selected(2);
            serial = s.clone();
        }
        Connection::Shell => kind.set_selected(3),
        Connection::Command(c) => {
            kind.set_selected(4);
            command.set_text(c);
        }
    }
    device.set_text(&serial.device.to_string_lossy());
    speed.set_selected(position(SPEEDS, serial.baud));
    data_bits.set_selected(position(DATA_BITS, serial.data_bits));
    parity.set_selected(position(PARITIES.map(|p| p.0), serial.parity));
    stop_bits.set_selected(u32::from(serial.stop_bits == 2));
    flow.set_selected(position(FLOWS.map(|f| f.0), serial.flow));
    model.set_selected(position(MODELS, p.model));
    phosphor.set_selected(position(PHOSPHORS.map(|p| p.0), p.phosphor.as_str()));

    let connection_group = adw::PreferencesGroup::new();
    for row in [
        name.upcast_ref::<gtk::Widget>(),
        kind.upcast_ref(),
        host.upcast_ref(),
        port.upcast_ref(),
        command.upcast_ref(),
        device.upcast_ref(),
    ] {
        connection_group.add(row);
    }
    let line_group = adw::PreferencesGroup::builder()
        .title("Serial Line")
        .description("DEC factory settings are 9600 baud, 8 bits, no parity, 1 stop bit, XON/XOFF")
        .build();
    for row in [&speed, &data_bits, &parity, &stop_bits, &flow] {
        line_group.add(row);
    }
    let terminal_group = adw::PreferencesGroup::builder().title("Terminal").build();
    terminal_group.add(&model);
    terminal_group.add(&phosphor);
    terminal_group.add(&two);
    let log_group = adw::PreferencesGroup::builder()
        .title("Log")
        .description("The session's text is added to the file each time it opens")
        .build();
    log_group.add(&log_file);
    log_group.add(&log_stamps);
    let page = adw::PreferencesPage::new();
    page.add(&connection_group);
    page.add(&line_group);
    page.add(&terminal_group);
    page.add(&log_group);

    let show_fields = {
        let (host, port, command, device, line_group) = (
            host.clone(),
            port.clone(),
            command.clone(),
            device.clone(),
            line_group.clone(),
        );
        move |kind: u32| {
            host.set_visible(kind <= 1);
            host.set_title(if kind == 1 {
                "Destination ([user@]host)"
            } else {
                "Host"
            });
            port.set_visible(kind <= 1);
            device.set_visible(kind == 2);
            line_group.set_visible(kind == 2);
            command.set_visible(kind == 4);
        }
    };
    show_fields(kind.selected());
    kind.connect_selected_notify(move |k| show_fields(k.selected()));

    let cancel = gtk::Button::with_label("Cancel");
    let save_button = gtk::Button::builder()
        .label(if editing { "Save" } else { "Add" })
        .css_classes(["suggested-action"])
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save_button);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&page));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toasts));
    let dialog = adw::Dialog::builder()
        .title(if editing {
            "Edit Connection"
        } else {
            "Add Connection"
        })
        .content_width(460)
        .content_height(640)
        .child(&toolbar)
        .build();

    cancel.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });
    save_button.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            let problem = |row: &gtk::Widget, msg: &str| {
                row.add_css_class("error");
                row.grab_focus();
                toasts.add_toast(adw::Toast::new(msg));
            };
            for row in [&name, &host, &command, &device] {
                row.remove_css_class("error");
            }
            let n = name.text().trim().to_string();
            if n.is_empty() {
                return problem(name.upcast_ref(), "The connection needs a name");
            }
            if taken.contains(&n) {
                return problem(name.upcast_ref(), "Another connection has that name");
            }
            let port_value = port.value() as u16;
            let connection = match kind.selected() {
                0 | 1 => {
                    let h = host.text().trim().to_string();
                    if h.is_empty() || h.starts_with('-') {
                        return problem(host.upcast_ref(), "Enter a host name");
                    }
                    if kind.selected() == 0 {
                        Connection::Telnet {
                            host: h,
                            port: if port_value == 0 { 23 } else { port_value },
                        }
                    } else {
                        Connection::Ssh(SshConfig {
                            destination: h,
                            port: (port_value != 0).then_some(port_value),
                        })
                    }
                }
                2 => {
                    let d = device.text().trim().to_string();
                    if d.is_empty() {
                        return problem(device.upcast_ref(), "Enter the serial device");
                    }
                    let mut s = SerialConfig::new(d);
                    s.baud = SPEEDS[speed.selected() as usize % SPEEDS.len()];
                    s.data_bits = DATA_BITS[data_bits.selected() as usize % DATA_BITS.len()];
                    s.parity = PARITIES[parity.selected() as usize % PARITIES.len()].0;
                    s.stop_bits = if stop_bits.selected() == 1 { 2 } else { 1 };
                    s.flow = FLOWS[flow.selected() as usize % FLOWS.len()].0;
                    Connection::Serial(s)
                }
                3 => Connection::Shell,
                _ => {
                    let c = command.text().trim().to_string();
                    if c.is_empty() {
                        return problem(command.upcast_ref(), "Enter the command to run");
                    }
                    Connection::Command(c)
                }
            };
            let profile = Profile {
                name: n,
                model: MODELS[model.selected() as usize % MODELS.len()],
                connection,
                phosphor: PHOSPHORS[phosphor.selected() as usize % PHOSPHORS.len()]
                    .0
                    .into(),
                sessions: if two.is_active() { 2 } else { 1 },
                keymap: p.keymap.clone(),
                log: Some(log_file.text().trim().to_string()).filter(|l| !l.is_empty()),
                log_timestamps: log_stamps.is_active(),
                log_raw: p.log_raw,
            };
            match save(profile) {
                Ok(()) => {
                    dialog.close();
                }
                Err(e) => toasts.add_toast(adw::Toast::new(&format!("Cannot save: {e}"))),
            }
        }
    });
    dialog.present(Some(parent));
}
