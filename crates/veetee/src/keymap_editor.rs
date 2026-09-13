//! The keymap editor: an LK401 keyboard whose keys show the PC keys bound
//! to them. Select a DEC key, then add or remove PC key combinations.
//! Changes apply at once to the window's sessions; Save keeps them.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::translate::IntoGlib;
use gtk::{gdk, glib};
use vt_core::Key;
use vt_keyboard::keymap::{self, PcKey, Target};
use vt_keyboard::{Keymap, Local, Mods};

use crate::keymaps;
use crate::view::SharedKeymap;

/// A key on the drawn keyboard: legend, what it is, width in key units.
type KeyCap = (&'static str, Target, f32);

const fn k(legend: &'static str, key: Key) -> KeyCap {
    (legend, Target::Key(key), 1.0)
}

const fn l(legend: &'static str, local: Local) -> KeyCap {
    (legend, Target::Local(local), 1.0)
}

/// An empty place in a row.
const SPACER: KeyCap = ("", Target::Local(Local::Paste), 1.0);

const fn wide(cap: KeyCap, width: f32) -> KeyCap {
    (cap.0, cap.1, width)
}

fn top_row() -> Vec<Vec<KeyCap>> {
    vec![
        vec![
            l("Hold\nScreen", Local::HoldScreen),
            l("Print\nScreen", Local::PrintScreen),
            l("Set-Up", Local::SetUp),
            l("Session", Local::SwitchSession),
            l("Break", Local::Break),
        ],
        (6..=10)
            .map(|n| k(FUNCTION_LEGENDS[n - 1], Key::Function(n as u8)))
            .collect(),
        (11..=14)
            .map(|n| k(FUNCTION_LEGENDS[n - 1], Key::Function(n as u8)))
            .collect(),
        vec![
            k("Help", Key::Function(15)),
            wide(k("Do", Key::Function(16)), 1.6),
        ],
        (17..=20)
            .map(|n| k(FUNCTION_LEGENDS[n - 1], Key::Function(n as u8)))
            .collect(),
    ]
}

const FUNCTION_LEGENDS: [&str; 20] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14",
    "Help", "Do", "F17", "F18", "F19", "F20",
];

const UDK_LEGENDS: [&str; 15] = [
    "⇧F6", "⇧F7", "⇧F8", "⇧F9", "⇧F10", "⇧F11", "⇧F12", "⇧F13", "⇧F14", "⇧Help", "⇧Do", "⇧F17",
    "⇧F18", "⇧F19", "⇧F20",
];

fn editing_keypad() -> Vec<Vec<KeyCap>> {
    vec![
        vec![
            k("Find", Key::Find),
            k("Insert\nHere", Key::InsertHere),
            k("Remove", Key::Remove),
        ],
        vec![
            k("Select", Key::Select),
            k("Prev\nScreen", Key::PrevScreen),
            k("Next\nScreen", Key::NextScreen),
        ],
    ]
}

fn numeric_keypad() -> Vec<Vec<KeyCap>> {
    vec![
        vec![
            k("PF1", Key::Pf1),
            k("PF2", Key::Pf2),
            k("PF3", Key::Pf3),
            k("PF4", Key::Pf4),
        ],
        vec![
            k("7", Key::Keypad(7)),
            k("8", Key::Keypad(8)),
            k("9", Key::Keypad(9)),
            k("−", Key::KeypadMinus),
        ],
        vec![
            k("4", Key::Keypad(4)),
            k("5", Key::Keypad(5)),
            k("6", Key::Keypad(6)),
            k(",", Key::KeypadComma),
        ],
        vec![
            k("1", Key::Keypad(1)),
            k("2", Key::Keypad(2)),
            k("3", Key::Keypad(3)),
            k("Enter", Key::KeypadEnter),
        ],
        vec![wide(k("0", Key::Keypad(0)), 2.1), k(".", Key::KeypadPeriod)],
    ]
}

fn other_keys() -> Vec<Vec<KeyCap>> {
    vec![
        vec![
            wide(k("Tab", Key::Tab), 1.5),
            wide(k("Return", Key::Return), 1.8),
            wide(k("<X]", Key::Delete), 1.5),
            k("Esc", Key::Escape),
            k("Line\nFeed", Key::LineFeed),
            k("Back\nSpace", Key::Backspace),
        ],
        vec![
            wide(l("Answer-\nback", Local::Answerback), 1.3),
            l("Copy", Local::Copy),
            l("Paste", Local::Paste),
            wide(l("Mark\nCheckpoint", Local::MarkCheckpoint), 1.5),
            l("Pan ↑", Local::PanUp),
            l("Pan ↓", Local::PanDown),
            wide(l("Prev\nPage", Local::PanPrevPage), 1.2),
            wide(l("Next\nPage", Local::PanNextPage), 1.2),
        ],
    ]
}

const CSS: &str = "
.lk-board { background: #d8cfbb; border-radius: 14px; padding: 16px; }
.lk-group-label { color: #6b6452; font-size: 0.8em; }
.lk-key {
  background: linear-gradient(#45494a, #2f3231);
  color: #eceae2; border-radius: 6px; padding: 2px 4px;
  box-shadow: 0 3px 0 #1a1b1b; min-height: 46px; font-size: 0.8em;
}
.lk-key.local { background: linear-gradient(#f0eadb, #d3cab5); color: #2a2721; box-shadow: 0 3px 0 #8f8671; }
.lk-key.selected { outline: 3px solid @accent_color; }
.lk-key .binding { font-size: 0.8em; opacity: 0.7; }
";

struct Editor {
    keymap: SharedKeymap,
    selected: RefCell<Option<(Target, String)>>,
    caps: RefCell<Vec<(Target, gtk::Button, gtk::Label)>>,
    detail_title: gtk::Label,
    detail_list: gtk::ListBox,
    add_button: gtk::Button,
    toasts: adw::ToastOverlay,
    window: adw::Window,
}

/// Opens the editor for `keymap`.
pub fn open(parent: &impl IsA<gtk::Window>, keymap: SharedKeymap) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(
        &gtk::prelude::WidgetExt::display(parent.as_ref()),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let window = adw::Window::builder()
        .title("Keyboard Map")
        .transient_for(parent)
        .modal(true)
        .default_width(1320)
        .default_height(760)
        .build();
    let header = adw::HeaderBar::new();
    let restore = gtk::Button::with_label("Restore Defaults");
    let save = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&restore);
    header.pack_end(&save);

    let detail_title = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["title-3"])
        .label("Select a DEC key")
        .build();
    let detail_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let add_button = gtk::Button::builder()
        .label("Add PC Key…")
        .sensitive(false)
        .halign(gtk::Align::Start)
        .build();

    let toasts = adw::ToastOverlay::new();
    let editor = Rc::new(Editor {
        keymap,
        selected: RefCell::new(None),
        caps: RefCell::new(Vec::new()),
        detail_title: detail_title.clone(),
        detail_list: detail_list.clone(),
        add_button: add_button.clone(),
        toasts: toasts.clone(),
        window: window.clone(),
    });

    let board = gtk::Box::new(gtk::Orientation::Vertical, 14);
    board.add_css_class("lk-board");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    for group in top_row() {
        top.append(&editor.rows(&[group]));
    }
    board.append(&top);
    let udks = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    udks.append(&group_label("User-defined keys"));
    let udk_caps: Vec<KeyCap> = (6..=20u8)
        .map(|n| {
            (
                UDK_LEGENDS[usize::from(n) - 6],
                Target::Key(Key::UserDefined(n)),
                1.0,
            )
        })
        .collect();
    udks.append(&editor.rows(&[udk_caps]));
    board.append(&udks);
    let lower = gtk::Box::new(gtk::Orientation::Horizontal, 28);
    let left = gtk::Box::new(gtk::Orientation::Vertical, 10);
    left.append(&group_label("Main keypad and veetee functions"));
    left.append(&editor.rows(&other_keys()));
    lower.append(&left);
    let editing = gtk::Box::new(gtk::Orientation::Vertical, 10);
    editing.append(&group_label("Editing keypad"));
    editing.append(&editor.rows(&editing_keypad()));
    editing.append(&group_label("Cursor keys"));
    editing.append(&editor.rows(&[
        vec![SPACER, k("↑", Key::Up)],
        vec![k("←", Key::Left), k("↓", Key::Down), k("→", Key::Right)],
    ]));
    lower.append(&editing);
    let keypad = gtk::Box::new(gtk::Orientation::Vertical, 10);
    keypad.append(&group_label("Numeric keypad"));
    keypad.append(&editor.rows(&numeric_keypad()));
    lower.append(&keypad);
    board.append(&lower);

    let detail = gtk::Box::new(gtk::Orientation::Vertical, 10);
    detail.append(&detail_title);
    detail.append(&gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label"])
        .label("PC keys bound to this DEC key. Extra modifiers pass on to DEC keys, so Ctrl+Insert is Ctrl+Find; veetee functions need their exact keys.")
        .build());
    detail.append(&detail_list);
    detail.append(&add_button);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .margin_top(20)
        .margin_bottom(20)
        .margin_start(20)
        .margin_end(20)
        .build();
    content.append(&board);
    content.append(&detail);
    let scroller = gtk::ScrolledWindow::builder().child(&content).build();
    toasts.set_child(Some(&scroller));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toasts));
    window.set_content(Some(&toolbar));

    add_button.connect_clicked({
        let editor = editor.clone();
        move |_| editor.capture()
    });
    save.connect_clicked({
        let editor = editor.clone();
        move |_| {
            let result = keymaps::save(&editor.keymap.borrow());
            match result {
                Ok(path) => editor.toast(&format!("Saved to {}", path.display())),
                Err(e) => editor.toast(&format!("Cannot save the keymap: {e}")),
            }
        }
    });
    restore.connect_clicked({
        let editor = editor.clone();
        move |_| {
            *editor.keymap.borrow_mut() = Keymap::default();
            editor.refresh();
            editor.toast("Restored the built-in keymap (Save to keep it)");
        }
    });

    editor.refresh();
    window.present();
}

fn group_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["lk-group-label"])
        .build()
}

impl Editor {
    fn rows(self: &Rc<Self>, rows: &[Vec<KeyCap>]) -> gtk::Box {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for row in rows {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            for &(legend, target, width) in row {
                line.append(&self.cap(legend, target, width));
            }
            column.append(&line);
        }
        column
    }

    fn cap(self: &Rc<Self>, legend: &'static str, target: Target, width: f32) -> gtk::Widget {
        let size = (52.0 * width) as i32;
        if legend.is_empty() {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_size_request(size, 46);
            return spacer.upcast();
        }
        let name = gtk::Label::new(Some(legend));
        name.set_justify(gtk::Justification::Center);
        let binding = gtk::Label::builder()
            .css_classes(["binding"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
        inner.append(&name);
        inner.append(&binding);
        let button = gtk::Button::builder()
            .child(&inner)
            .css_classes(["lk-key"])
            .build();
        button.set_size_request(size, 46);
        if matches!(target, Target::Local(_)) {
            button.add_css_class("local");
        }
        button.connect_clicked({
            let editor = self.clone();
            let legend = legend.replace('\n', " ");
            move |_| editor.select(target, &legend)
        });
        self.caps
            .borrow_mut()
            .push((target, button.clone(), binding));
        button.upcast()
    }

    fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(message));
    }

    fn select(self: &Rc<Self>, target: Target, legend: &str) {
        *self.selected.borrow_mut() = Some((target, legend.to_string()));
        self.refresh();
    }

    /// Redraws key legends and the detail list.
    fn refresh(self: &Rc<Self>) {
        let keymap = self.keymap.borrow();
        let selected = self.selected.borrow().clone();
        for (target, button, label) in self.caps.borrow().iter() {
            let names: Vec<String> = keymap
                .bindings_for(*target)
                .map(|b| keymap::pc_key_name(b.pc))
                .collect();
            label.set_text(names.first().map_or("—", String::as_str));
            button.set_tooltip_text(Some(&if names.is_empty() {
                "Not bound".to_string()
            } else {
                names.join(", ")
            }));
            if selected.as_ref().is_some_and(|(t, _)| t == target) {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
        while let Some(row) = self.detail_list.first_child() {
            self.detail_list.remove(&row);
        }
        let Some((target, legend)) = selected else {
            return;
        };
        self.detail_title
            .set_text(&format!("{legend}  ({})", keymap::target_name(target)));
        self.add_button.set_sensitive(true);
        let bindings: Vec<PcKey> = keymap.bindings_for(target).map(|b| b.pc).collect();
        if bindings.is_empty() {
            self.detail_list
                .append(&adw::ActionRow::builder().title("No PC key").build());
        }
        for pc in bindings {
            let row = adw::ActionRow::builder()
                .title(keymap::pc_key_name(pc))
                .build();
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .tooltip_text("Remove")
                .build();
            remove.connect_clicked({
                let editor = self.clone();
                move |_| {
                    editor.keymap.borrow_mut().unbind(pc);
                    editor.refresh();
                }
            });
            row.add_suffix(&remove);
            self.detail_list.append(&row);
        }
    }

    /// Waits for a PC key combination and binds it to the selected key.
    fn capture(self: &Rc<Self>) {
        let Some((target, legend)) = self.selected.borrow().clone() else {
            return;
        };
        let dialog = adw::AlertDialog::new(
            Some(&format!("Press a PC key for {legend}")),
            Some("Hold Ctrl, Shift or Alt as needed. Escape cancels."),
        );
        dialog.add_response("cancel", "Cancel");
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let (editor, dialog) = (self.clone(), dialog.clone());
            move |_, keyval, _, state| {
                if is_modifier(keyval) {
                    return glib::Propagation::Stop;
                }
                if keyval == gdk::Key::Escape && state.is_empty() {
                    dialog.close();
                    return glib::Propagation::Stop;
                }
                let mods = Mods {
                    shift: state.contains(gdk::ModifierType::SHIFT_MASK),
                    ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
                    alt: state.contains(gdk::ModifierType::ALT_MASK),
                };
                let sym = keyval.to_lower().into_glib();
                if keymap::key_name(sym).is_none() {
                    editor.toast("veetee cannot bind that key");
                    dialog.close();
                    return glib::Propagation::Stop;
                }
                let pc = PcKey { sym, mods };
                let previous = editor
                    .keymap
                    .borrow()
                    .bindings
                    .iter()
                    .find(|b| b.pc == pc)
                    .map(|b| b.target);
                editor.keymap.borrow_mut().bind(pc, target);
                if let Some(old) = previous.filter(|old| *old != target) {
                    editor.toast(&format!(
                        "{} was {}; it is now {}",
                        keymap::pc_key_name(pc),
                        keymap::target_name(old),
                        keymap::target_name(target)
                    ));
                }
                editor.refresh();
                dialog.close();
                glib::Propagation::Stop
            }
        });
        dialog.add_controller(keys);
        dialog.present(Some(&self.window));
    }
}

fn is_modifier(key: gdk::Key) -> bool {
    matches!(
        key,
        gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::ISO_Level3_Shift
            | gdk::Key::Caps_Lock
    )
}
