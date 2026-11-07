//! The Connections window: saved connections (profiles) to open in a new
//! window, add, change and delete.

use std::cell::RefCell;
use std::rc::Rc;
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};

use adw::prelude::*;
use gtk::glib;
use vt_core::Model;
use vt_transport::serial::{FlowControl, Parity, SerialConfig};
use vt_transport::ssh::{SshConfig, X11};
use vt_transport::telnet::ComPort;

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
const KINDS: [&str; 6] = [
    "Telnet",
    "SSH",
    "Serial line",
    "LAT",
    "Local shell",
    "Command",
];
const X11_CHOICES: [(X11, &str); 3] = [
    (X11::AsConfigured, "As ~/.ssh/config says"),
    (X11::On, "On (ssh -X)"),
    (X11::Trusted, "Trusted (ssh -Y)"),
];
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

/// Where New Session is told to connect: see [`choose_session`].
type Chosen = Rc<dyn Fn(Option<(Connection, Option<String>)>)>;

/// What each of [`KINDS`] is for, on New Session's second page.
const KIND_HINTS: [&str; 6] = [
    "OpenVMS and terminal servers over TCP/IP",
    "Through OpenSSH, with ~/.ssh/config and the agent",
    "A port on this computer",
    "OpenVMS over Ethernet, without TCP/IP",
    "A shell on this computer",
    "A program on this computer",
];

/// New Session: where the new session connects. The first page has a new
/// connection, the window's own connection, then the saved connections;
/// choosing one calls `chosen` with the connection and the saved
/// connection's name, or with `None` for the window's own. A new connection
/// is made in two more pages, the kind and then its details, and can be saved
/// on the way, as `template` with the new connection and name. Only where it
/// connects is taken: the session belongs to this terminal.
pub fn choose_session(
    parent: &adw::ApplicationWindow,
    current: &str,
    template: Profile,
    chosen: impl Fn(Option<(Connection, Option<String>)>) + 'static,
) {
    let chosen: Chosen = Rc::new(chosen);
    let nav = adw::NavigationView::new();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&nav));
    let dialog = adw::Dialog::builder()
        .title("New Session")
        .content_width(460)
        .content_height(560)
        .child(&toasts)
        .build();
    let row = |title: &str, subtitle: &str| {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(title).as_str())
            .subtitle(glib::markup_escape_text(subtitle).as_str())
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row
    };

    // Page 1: where.
    let fresh = row(
        "New Connection…",
        if cfg!(target_os = "linux") {
            "Telnet, SSH, a serial line, LAT or a command"
        } else {
            "Telnet, SSH, a serial line or a command"
        },
    );
    fresh.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
    let new_group = adw::PreferencesGroup::new();
    new_group.add(&fresh);
    let open_group = adw::PreferencesGroup::new();
    let own = row("This window's connection", current);
    own.connect_activated({
        let (dialog, chosen) = (dialog.clone(), chosen.clone());
        move |_| {
            dialog.close();
            chosen(None);
        }
    });
    open_group.add(&own);
    // A file that cannot be read leaves the window's own connection, and a
    // new connection that is not saved.
    let saved = profiles::load();
    for profile in saved.clone().unwrap_or_default() {
        let entry = row(&profile.name, &profile.connection.label());
        entry.connect_activated({
            let (dialog, chosen) = (dialog.clone(), chosen.clone());
            move |_| {
                dialog.close();
                chosen(Some((
                    profile.connection.clone(),
                    Some(profile.name.clone()),
                )));
            }
        });
        open_group.add(&entry);
    }
    let where_page = adw::PreferencesPage::new();
    where_page.add(&new_group);
    where_page.add(&open_group);
    nav.add(&nav_page("New Session", &where_page, None));

    // Page 2: what kind.
    let kind_group = adw::PreferencesGroup::new();
    for (kind, (title, hint)) in KINDS.iter().zip(KIND_HINTS).enumerate() {
        // LAT needs a raw Ethernet socket, which only Linux gives.
        if kind == 3 && !cfg!(target_os = "linux") {
            continue;
        }
        let entry = row(title, hint);
        entry.connect_activated({
            let (dialog, nav, toasts, chosen) =
                (dialog.clone(), nav.clone(), toasts.clone(), chosen.clone());
            let (template, saved) = (template.clone(), saved.clone());
            move |_| {
                // A local shell has nothing to ask.
                if kind == 4 {
                    dialog.close();
                    chosen(Some((Connection::Shell, None)));
                    return;
                }
                let details = details_page(
                    kind as u32,
                    &dialog,
                    &toasts,
                    template.clone(),
                    saved.clone(),
                    chosen.clone(),
                );
                nav.push(&details);
            }
        });
        kind_group.add(&entry);
    }
    let kind_page = adw::PreferencesPage::new();
    kind_page.add(&kind_group);
    let kinds = nav_page("Connect With", &kind_page, None);
    kinds.set_tag(Some("kind"));
    nav.add(&kinds);
    fresh.connect_activated({
        let nav = nav.clone();
        move |_| nav.push_by_tag("kind")
    });

    dialog.present(Some(parent));
}

/// A page of New Session, with its header and `action` at its end.
fn nav_page(
    title: &str,
    content: &impl IsA<gtk::Widget>,
    action: Option<&gtk::Button>,
) -> adw::NavigationPage {
    let header = adw::HeaderBar::new();
    if let Some(action) = action {
        header.pack_end(action);
    }
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(content));
    adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build()
}

/// New Session's last page: the details of a new connection of `kind`, and
/// whether to save it. `saved` are the saved connections, or why they cannot
/// be read, which leaves saving off.
fn details_page(
    kind: u32,
    dialog: &adw::Dialog,
    toasts: &adw::ToastOverlay,
    template: Profile,
    saved: Result<Vec<Profile>, String>,
    chosen: Chosen,
) -> adw::NavigationPage {
    let blank = Connection::Telnet {
        host: String::new(),
        port: 23,
        binary: false,
        com_port: None,
    };
    let (fields, _) = ConnectionFields::new(&blank);
    fields.show(kind);
    let group = adw::PreferencesGroup::new();
    for row in fields.rows() {
        group.add(row);
    }
    let keep = adw::SwitchRow::builder()
        .title("Save as a connection")
        .subtitle(match &saved {
            Ok(_) => "To open again from New Session and Connections",
            Err(_) => "The saved connections cannot be read",
        })
        .sensitive(saved.is_ok())
        .build();
    let name = adw::EntryRow::builder()
        .title("Name")
        .visible(false)
        .build();
    let save_group = adw::PreferencesGroup::new();
    save_group.add(&keep);
    save_group.add(&name);
    let page = adw::PreferencesPage::new();
    page.add(&group);
    page.add(&fields.line_group);
    page.add(&save_group);

    keep.connect_active_notify({
        let (fields, name) = (fields.clone(), name.clone());
        move |keep| {
            name.set_visible(keep.is_active());
            // A name to start from: what it connects to.
            if keep.is_active()
                && name.text().trim().is_empty()
                && let Ok(connection) = fields.connection(kind)
            {
                name.set_text(&suggested_name(&connection));
            }
        }
    });

    let connect = gtk::Button::builder()
        .label("Connect")
        .css_classes(["suggested-action"])
        .build();
    connect.connect_clicked({
        let (dialog, toasts) = (dialog.clone(), toasts.clone());
        let (fields, name, keep) = (fields.clone(), name.clone(), keep.clone());
        move |_| {
            name.remove_css_class("error");
            let connection = match fields.connection(kind) {
                Ok(c) => c,
                Err((row, why)) => return show_problem(&toasts, &row, why),
            };
            let mut profile = None;
            if keep.is_active() {
                let mut all = saved.clone().unwrap_or_default();
                let n = name.text().trim().to_string();
                if n.is_empty() {
                    return show_problem(&toasts, name.upcast_ref(), "The connection needs a name");
                }
                if all.iter().any(|p| p.name == n) {
                    return show_problem(
                        &toasts,
                        name.upcast_ref(),
                        "Another connection has that name",
                    );
                }
                let new = Profile {
                    name: n.clone(),
                    connection: connection.clone(),
                    sessions: 1,
                    default: false,
                    ..template.clone()
                };
                profiles::put(&mut all, None, new);
                if let Err(e) = profiles::save(&all) {
                    toasts.add_toast(adw::Toast::new(&format!("Cannot save: {e}")));
                    return;
                }
                profile = Some(n);
            }
            dialog.close();
            chosen(Some((connection, profile)));
        }
    });
    // Enter in a field connects, as the button does.
    for entry in fields.entries().into_iter().chain([&name]) {
        entry.connect_entry_activated({
            let connect = connect.clone();
            move |_| connect.emit_clicked()
        });
    }
    let details = nav_page(KINDS[kind as usize], &page, Some(&connect));
    details.connect_shown({
        let first = [&fields.host, &fields.device, &fields.command]
            .into_iter()
            .find(|row| row.is_visible())
            .cloned();
        move |_| {
            if let Some(first) = &first {
                first.grab_focus();
            }
        }
    });
    details
}

/// A name for a new saved connection: the host, node, device or program it
/// connects to.
fn suggested_name(connection: &Connection) -> String {
    match connection {
        Connection::Telnet { host, .. } => host.clone(),
        Connection::Ssh(s) => s.destination.clone(),
        Connection::Serial(s) => s
            .device
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        Connection::Lat { node, service, .. } => service.clone().unwrap_or_else(|| node.clone()),
        Connection::Shell => "Local shell".into(),
        Connection::Command(c) => c.split_whitespace().next().unwrap_or("").into(),
    }
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
        profiles::put(&mut all, None, profile);
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
        // The star is the default connection: what veetee opens when it is
        // started without one. Filled where it is, and it takes it away again.
        let star = gtk::Button::builder()
            .icon_name(if p.default {
                "starred-symbolic"
            } else {
                "non-starred-symbolic"
            })
            .tooltip_text(if p.default {
                "The default connection: opened when veetee starts without one. Click to stop"
            } else {
                "Make this the default connection, opened when veetee starts without one"
            })
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        row.add_suffix(&star);
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
        star.connect_clicked({
            let ui = Rc::downgrade(self);
            move |_| {
                let Some(ui) = ui.upgrade() else { return };
                let mut all = ui.profiles.borrow().clone();
                profiles::toggle_default(&mut all, index);
                if let Err(e) = profiles::save(&all) {
                    eprintln!("veetee: cannot save connections: {e}");
                }
                ui.reload();
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
            profiles::put(&mut all, index, profile);
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

/// The rows that say where a connection goes, for the kinds in [`KINDS`]:
/// shared by the connection editor and New Session's wizard, so the two fill
/// in and check a connection alike.
#[derive(Clone)]
struct ConnectionFields {
    host: adw::EntryRow,
    port: adw::SpinRow,
    x11: adw::ComboRow,
    display: adw::EntryRow,
    command: adw::EntryRow,
    device: adw::EntryRow,
    interface: adw::ComboRow,
    service: adw::EntryRow,
    browse: gtk::Button,
    /// The serial line settings, a group of their own.
    line_group: adw::PreferencesGroup,
    speed: adw::ComboRow,
    data_bits: adw::ComboRow,
    parity: adw::ComboRow,
    stop_bits: adw::ComboRow,
    flow: adw::ComboRow,
    /// The interface each entry of `interface` names; `None` is Automatic.
    wires: Rc<Vec<Option<String>>>,
    // There are no controls for 8-bit Telnet or RFC 2217 line settings, so
    // an edited connection keeps what it was saved with.
    keep_binary: bool,
    keep_com_port: Option<ComPort>,
}

impl ConnectionFields {
    /// The rows filled in from `connection`, and its index in [`KINDS`].
    fn new(connection: &Connection) -> (ConnectionFields, u32) {
        let keep_binary = matches!(connection, Connection::Telnet { binary: true, .. });
        let keep_com_port = match connection {
            Connection::Telnet { com_port, .. } => *com_port,
            _ => None,
        };
        let host = adw::EntryRow::builder().title("Host").build();
        let port = adw::SpinRow::with_range(0.0, 65535.0, 1.0);
        port.set_title("Port");
        port.set_subtitle("0 uses the standard port");
        let x11 = combo("X11 forwarding", &X11_CHOICES.map(|x| x.1));
        x11.set_subtitle("Shows the host's X clients on a display here");
        let display = adw::EntryRow::builder()
            .title("X display (empty uses $DISPLAY)")
            .build();
        let command = adw::EntryRow::builder().title("Command").build();
        let device = adw::EntryRow::builder().title("Device").build();
        // What is up now, so nobody has to leave the window to run `ip addr`.
        // A name a saved connection carries is kept even when that interface
        // is not here, since connections travel between machines.
        let saved = match connection {
            Connection::Lat { interface, .. } => interface.clone(),
            _ => None,
        };
        let live = lat_interfaces();
        // Automatic first, then the saved one where there is one, then the
        // rest of what is up. The saved one keeps its place whether or not it
        // is here.
        let mut wires: Vec<Option<String>> = vec![None];
        let mut labels: Vec<String> = vec!["Automatic".into()];
        if let Some(name) = &saved {
            labels.push(live.iter().find(|(up, _)| up == name).map_or_else(
                || format!("{name} (not here now)"),
                |(_, label)| label.clone(),
            ));
            wires.push(Some(name.clone()));
        }
        for (name, label) in live {
            if wires
                .iter()
                .any(|wire| wire.as_deref() == Some(name.as_str()))
            {
                continue;
            }
            wires.push(Some(name));
            labels.push(label);
        }
        let interface = adw::ComboRow::builder()
            .title("Interface")
            .subtitle("Automatic uses the one Ethernet interface that is up")
            .model(&gtk::StringList::new(
                &labels.iter().map(String::as_str).collect::<Vec<_>>(),
            ))
            .build();
        let service = adw::EntryRow::builder().title("Service").build();
        // Listening for LAT services needs a socket, so it belongs to the one
        // platform that has one.
        let browse = gtk::Button::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("List the LAT services announcing themselves")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        host.add_suffix(&browse);
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
        let line_group = adw::PreferencesGroup::builder()
            .title("Serial Line")
            .description(
                "DEC factory settings are 9600 baud, 8 bits, no parity, 1 stop bit, XON/XOFF",
            )
            .build();
        for row in [&speed, &data_bits, &parity, &stop_bits, &flow] {
            line_group.add(row);
        }

        let mut serial = SerialConfig::new(if cfg!(windows) {
            "COM1"
        } else {
            "/dev/ttyUSB0"
        });
        let kind = match connection {
            Connection::Telnet {
                host: h, port: n, ..
            } => {
                host.set_text(h);
                port.set_value(if *n == 23 { 0.0 } else { f64::from(*n) });
                0
            }
            Connection::Ssh(s) => {
                host.set_text(&s.destination);
                port.set_value(s.port.map_or(0.0, f64::from));
                x11.set_selected(position(X11_CHOICES.map(|x| x.0), s.x11));
                display.set_text(s.display.as_deref().unwrap_or(""));
                1
            }
            Connection::Serial(s) => {
                serial = s.clone();
                2
            }
            Connection::Lat {
                interface: i,
                node,
                service: sv,
            } => {
                host.set_text(node);
                if i.is_some() {
                    interface.set_selected(1);
                }
                service.set_text(sv.as_deref().unwrap_or(""));
                3
            }
            Connection::Shell => 4,
            Connection::Command(c) => {
                command.set_text(c);
                5
            }
        };
        device.set_text(&serial.device.to_string_lossy());
        speed.set_selected(position(SPEEDS, serial.baud));
        data_bits.set_selected(position(DATA_BITS, serial.data_bits));
        parity.set_selected(position(PARITIES.map(|p| p.0), serial.parity));
        stop_bits.set_selected(u32::from(serial.stop_bits == 2));
        flow.set_selected(position(FLOWS.map(|f| f.0), serial.flow));

        let fields = ConnectionFields {
            host,
            port,
            x11,
            display,
            command,
            device,
            interface,
            service,
            browse,
            line_group,
            speed,
            data_bits,
            parity,
            stop_bits,
            flow,
            wires: Rc::new(wires),
            keep_binary,
            keep_com_port,
        };
        #[cfg(target_os = "linux")]
        fields.browse.connect_clicked({
            let fields = fields.clone();
            move |button| {
                let named = fields.wire().unwrap_or_default();
                let (host, service) = (fields.host.clone(), fields.service.clone());
                browse_lat(
                    button,
                    (!named.is_empty()).then_some(named),
                    move |node, name| {
                        host.set_text(&node);
                        // The node's own name is the usual service, and
                        // saying it twice is noise: the field is left empty
                        // for that.
                        service.set_text(if name == node { "" } else { &name });
                    },
                );
            }
        });
        (fields, kind)
    }

    /// The rows for a group, in order; [`ConnectionFields::show`] hides the
    /// ones a kind does not use. The serial line has its own group.
    fn rows(&self) -> [&gtk::Widget; 8] {
        [
            self.host.upcast_ref(),
            self.port.upcast_ref(),
            self.x11.upcast_ref(),
            self.display.upcast_ref(),
            self.command.upcast_ref(),
            self.device.upcast_ref(),
            self.interface.upcast_ref(),
            self.service.upcast_ref(),
        ]
    }

    /// Shows the rows `kind` uses.
    fn show(&self, kind: u32) {
        self.x11.set_visible(kind == 1);
        self.display.set_visible(kind == 1);
        self.browse
            .set_visible(kind == 3 && cfg!(target_os = "linux"));
        // LAT names a node rather than a host, and there is no port: it is
        // not IP at all.
        self.host.set_visible(kind <= 1 || kind == 3);
        self.host.set_title(match kind {
            1 => "Destination ([user@]host)",
            3 => "Node",
            _ => "Host",
        });
        self.port.set_visible(kind <= 1);
        self.device.set_visible(kind == 2);
        self.line_group.set_visible(kind == 2);
        self.interface.set_visible(kind == 3);
        self.service.set_visible(kind == 3);
        self.command.set_visible(kind == 5);
    }

    /// The entry rows, for pressing Enter in.
    fn entries(&self) -> [&adw::EntryRow; 5] {
        [
            &self.host,
            &self.display,
            &self.command,
            &self.device,
            &self.service,
        ]
    }

    fn wire(&self) -> Option<String> {
        self.wires
            .get(self.interface.selected() as usize)
            .cloned()
            .flatten()
    }

    /// The connection of `kind` the rows describe, or the row that is wrong
    /// and why. Rows marked wrong before are cleared first.
    fn connection(&self, kind: u32) -> Result<Connection, (gtk::Widget, &'static str)> {
        for row in [&self.host, &self.display, &self.command, &self.device] {
            row.remove_css_class("error");
        }
        let wrong = |row: &adw::EntryRow, why| Err((row.clone().upcast(), why));
        let port = self.port.value() as u16;
        Ok(match kind {
            0 | 1 => {
                let h = self.host.text().trim().to_string();
                if h.is_empty() || h.starts_with('-') {
                    return wrong(&self.host, "Enter a host name");
                }
                if kind == 0 {
                    Connection::Telnet {
                        host: h,
                        port: if port == 0 { 23 } else { port },
                        binary: self.keep_binary,
                        com_port: self.keep_com_port,
                    }
                } else {
                    let d = self.display.text().trim().to_string();
                    if !d.is_empty() && !vt_transport::ssh::is_x_display(&d) {
                        return wrong(
                            &self.display,
                            "An X display looks like :0 (the colon matters)",
                        );
                    }
                    Connection::Ssh(SshConfig {
                        destination: h,
                        port: (port != 0).then_some(port),
                        x11: X11_CHOICES[self.x11.selected() as usize % X11_CHOICES.len()].0,
                        display: (!d.is_empty()).then_some(d),
                    })
                }
            }
            2 => {
                let d = self.device.text().trim().to_string();
                if d.is_empty() {
                    return wrong(&self.device, "Enter the serial device");
                }
                let mut s = SerialConfig::new(d);
                s.baud = SPEEDS[self.speed.selected() as usize % SPEEDS.len()];
                s.data_bits = DATA_BITS[self.data_bits.selected() as usize % DATA_BITS.len()];
                s.parity = PARITIES[self.parity.selected() as usize % PARITIES.len()].0;
                s.stop_bits = if self.stop_bits.selected() == 1 { 2 } else { 1 };
                s.flow = FLOWS[self.flow.selected() as usize % FLOWS.len()].0;
                Connection::Serial(s)
            }
            3 => {
                let node = self.host.text().trim().to_string();
                if node.is_empty() {
                    return wrong(&self.host, "Enter the node to connect to");
                }
                let service = self.service.text().trim().to_string();
                Connection::Lat {
                    interface: self.wire(),
                    node,
                    service: (!service.is_empty()).then_some(service),
                }
            }
            4 => Connection::Shell,
            _ => {
                let c = self.command.text().trim().to_string();
                if c.is_empty() {
                    return wrong(&self.command, "Enter the command to run");
                }
                Connection::Command(c)
            }
        })
    }
}

/// Marks `row` wrong, moves to it and says why.
fn show_problem(toasts: &adw::ToastOverlay, row: &gtk::Widget, why: &str) {
    row.add_css_class("error");
    row.grab_focus();
    toasts.add_toast(adw::Toast::new(why));
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
            binary: false,
            com_port: None,
        },
        phosphor: "white".into(),
        sessions: 1,
        keymap: None,
        log: None,
        log_timestamps: false,
        log_raw: false,
        default: false,
    });

    let name = adw::EntryRow::builder().title("Name").text(&p.name).build();
    let kind = combo("Type", &KINDS);
    let (fields, selected) = ConnectionFields::new(&p.connection);
    kind.set_selected(selected);
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
    let default = adw::SwitchRow::builder()
        .title("Default connection")
        .subtitle("Opened when veetee starts without being told a connection")
        .active(p.default)
        .build();
    model.set_selected(position(MODELS, p.model));
    phosphor.set_selected(position(PHOSPHORS.map(|p| p.0), p.phosphor.as_str()));

    let connection_group = adw::PreferencesGroup::new();
    connection_group.add(&name);
    connection_group.add(&kind);
    for row in fields.rows() {
        connection_group.add(row);
    }
    connection_group.add(&default);
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
    page.add(&fields.line_group);
    page.add(&terminal_group);
    page.add(&log_group);

    fields.show(kind.selected());
    kind.connect_selected_notify({
        let fields = fields.clone();
        move |k| fields.show(k.selected())
    });

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
            name.remove_css_class("error");
            let n = name.text().trim().to_string();
            if n.is_empty() {
                return show_problem(&toasts, name.upcast_ref(), "The connection needs a name");
            }
            if taken.contains(&n) {
                return show_problem(
                    &toasts,
                    name.upcast_ref(),
                    "Another connection has that name",
                );
            }
            let connection = match fields.connection(kind.selected()) {
                Ok(c) => c,
                Err((row, why)) => return show_problem(&toasts, &row, why),
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
                default: default.is_active(),
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

/// The interfaces LAT could speak on: the name, and a label to show.
///
/// Empty where LAT cannot work at all, which leaves the picker with nothing
/// but Automatic and a saved name, and costs nothing: the connection can
/// still be edited on a machine that will never open it.
fn lat_interfaces() -> Vec<(String, String)> {
    #[cfg(target_os = "linux")]
    {
        vt_transport::lat::interfaces()
            .into_iter()
            .map(|found| {
                let label = if found.wireless {
                    format!("{} (wireless)", found.name)
                } else {
                    found.name.clone()
                };
                (found.name, label)
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    Vec::new()
}

/// Lists the LAT services announcing themselves, for one to be picked rather
/// than typed, and calls `chosen` with the node and service of the one that
/// is.
///
/// Nothing is asked for: a node announces itself about once a minute and a
/// solicit built by hand has never been answered, so the list fills in as
/// they arrive rather than all at once. The window says so, or it would look
/// broken for the first minute.
#[cfg(target_os = "linux")]
fn browse_lat(
    parent: &impl IsA<gtk::Widget>,
    interface: Option<String>,
    chosen: impl Fn(String, String) + 'static,
) {
    let group = adw::PreferencesGroup::builder()
        .title("Services")
        .description("A node announces itself about once a minute, so this fills in as they arrive")
        .build();
    let waiting = adw::ActionRow::builder().title("Listening…").build();
    let spinner = gtk::Spinner::builder().spinning(true).build();
    waiting.add_prefix(&spinner);
    group.add(&waiting);

    let page = adw::PreferencesPage::new();
    page.add(&group);
    let close = gtk::Button::with_label("Close");
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&close);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title("LAT Services")
        .content_width(460)
        .content_height(420)
        .child(&toolbar)
        .build();
    close.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });
    dialog.present(Some(parent));

    // Opening the socket can want the helper, and reading waits on the wire,
    // so both happen away from the interface thread.
    let (tx, rx) = async_channel::bounded(16);
    // The thread waits on the wire, which is quiet for a minute at a time, so
    // it cannot be left to notice the receiver going: it would go on polling
    // for the life of the program after the window had closed.
    let listening = Arc::new(AtomicBool::new(true));
    {
        let listening = listening.clone();
        dialog.connect_closed(move |_| listening.store(false, Ordering::Relaxed));
    }
    let stop = listening.clone();
    std::thread::spawn(move || {
        let named = cli::lat_interface(interface.as_deref());
        let mut browser = match named.and_then(|name| vt_transport::lat::Browser::open(&name)) {
            Ok(browser) => browser,
            Err(e) => {
                // A sandbox is the one refusal no capability answers.
                let grantable = e.kind() != std::io::ErrorKind::Unsupported;
                let _ = tx.send_blocking(Err((e.to_string(), grantable)));
                return;
            }
        };
        loop {
            match browser.next(std::time::Duration::from_millis(500)) {
                Ok(found) if found.is_empty() => {}
                Ok(found) => {
                    for service in found {
                        if tx.send_blocking(Ok(service)).is_err() {
                            return;
                        }
                    }
                }
                Err(e) => {
                    let grantable = e.kind() != std::io::ErrorKind::Unsupported;
                    let _ = tx.send_blocking(Err((e.to_string(), grantable)));
                    return;
                }
            }
            // The window being gone is the only reason to stop.
            if !stop.load(Ordering::Relaxed) || tx.is_closed() {
                return;
            }
        }
    });

    let chosen = Rc::new(chosen);
    glib::spawn_future_local(async move {
        let mut seen: Vec<(String, String)> = Vec::new();
        while let Ok(message) = rx.recv().await {
            if !listening.load(Ordering::Relaxed) {
                return;
            }
            match message {
                Ok(service) => {
                    // A node announces itself again and again; one row each.
                    let key = (service.node.clone(), service.service.clone());
                    if seen.contains(&key) {
                        continue;
                    }
                    if seen.is_empty() {
                        group.remove(&waiting);
                    }
                    seen.push(key);
                    let row = adw::ActionRow::builder()
                        .title(if service.service == service.node {
                            service.node.clone()
                        } else {
                            format!("{} — {}", service.node, service.service)
                        })
                        .subtitle(if service.identification.is_empty() {
                            format!("rating {}", service.rating)
                        } else {
                            format!("rating {} · {}", service.rating, service.identification)
                        })
                        .activatable(true)
                        .build();
                    row.connect_activated({
                        let (dialog, chosen) = (dialog.clone(), chosen.clone());
                        let (node, name) = (service.node.clone(), service.service.clone());
                        move |_| {
                            chosen(node.clone(), name.clone());
                            dialog.close();
                        }
                    });
                    group.add(&row);
                }
                Err((why, grantable)) => {
                    waiting.set_title("Cannot listen for LAT services");
                    waiting.set_subtitle(&why);
                    // The command is the whole use of the message, so it can
                    // be selected, and taken in one go where there is one.
                    waiting.set_subtitle_selectable(true);
                    spinner.set_visible(false);
                    if grantable {
                        let copy = gtk::Button::builder()
                            .icon_name("edit-copy-symbolic")
                            .tooltip_text("Copy the command that grants it")
                            .valign(gtk::Align::Center)
                            .css_classes(["flat"])
                            .build();
                        copy.connect_clicked(|button| {
                            button.clipboard().set_text(&vt_transport::lat::grant());
                            button.set_tooltip_text(Some("Copied"));
                        });
                        waiting.add_suffix(&copy);
                    }
                    return;
                }
            }
        }
    });
}
