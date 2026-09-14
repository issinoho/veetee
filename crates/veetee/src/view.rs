//! The terminal widget: a GL area showing one session, with DEC keyboard input.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use vt_core::setup::{self, SetupMenu};
use vt_core::{Point, Selection};
use vt_keyboard::{Action, Local, Mods};
use vt_render::{FrameState, Renderer, Theme};

use crate::session::{self, Notice, Session};

/// Cursor blink half-period.
const CURSOR_BLINK: Duration = Duration::from_millis(530);
/// Character blink half-period.
const TEXT_BLINK: Duration = Duration::from_millis(670);

/// How a view reports to the window that holds it.
pub struct Callbacks {
    /// 1 or 2: which session of the window this is (for saved Set-Up).
    pub session_number: u8,
    /// Shows a transient message.
    pub notify: Box<dyn Fn(&str)>,
    /// Extra text for the window subtitle ("Hold Screen").
    pub status: Box<dyn Fn(&str)>,
    /// The host named the session (DECSWT).
    pub title: Box<dyn Fn(&str)>,
    /// The Session key (F4).
    pub switch_session: Box<dyn Fn()>,
    /// The host made this session active (DECES).
    pub activate: Box<dyn Fn()>,
    /// The view received keyboard focus.
    pub focused: Box<dyn Fn()>,
    /// The connection closed; the text says why when known.
    pub exited: Box<dyn Fn(Option<String>)>,
}

/// The keymap shared by the views in a window.
pub type SharedKeymap = Rc<RefCell<vt_keyboard::Keymap>>;

struct State {
    session: Session,
    gl: Option<glow::Context>,
    renderer: Option<Renderer>,
    theme: Theme,
    epoch: Instant,
    focused: bool,
    last_phases: (bool, bool),
    callbacks: Rc<Callbacks>,
    /// Developer hook: save a frame to this PPM file after a delay, then exit.
    capture: Option<(std::path::PathBuf, Duration)>,
    selection: Option<Selection>,
    /// Page position where the current mouse drag began.
    drag_anchor: Option<Point>,
    /// Set-Up, while it is open.
    setup: Option<SetupView>,
    /// The picture is still changing by itself (afterglow, visible bell).
    animating: bool,
    /// A tick callback is redrawing every frame.
    ticking: bool,
    /// The last key press or host output, for the CRT saver.
    last_activity: Instant,
    /// The CRT saver has blanked the screen.
    saver: bool,
    visible_bell: bool,
    /// When the visible bell started flashing.
    bell_flash: Option<Instant>,
}

/// An open Set-Up: the menu and the screen it draws on.
struct SetupView {
    menu: SetupMenu,
    screen: vt_core::Terminal,
    /// Hold Screen was on before Set-Up held the host.
    was_held: bool,
}

/// Set-Up shows veetee's major and minor version after the model name.
fn short_version() -> &'static str {
    let v = env!("CARGO_PKG_VERSION");
    v.rsplit_once('.').map_or(v, |(major_minor, _)| major_minor)
}

/// A 24-line terminal showing the Set-Up screen.
fn setup_screen(model: vt_core::Model, menu: &SetupMenu) -> vt_core::Terminal {
    let features = menu.features();
    let mut screen = vt_core::Terminal::new(vt_core::Config {
        model,
        cols: if features.columns_132 { 132 } else { 80 },
        ..vt_core::Config::default()
    });
    // Light or dark screen takes effect in Set-Up.
    if features.light_screen {
        screen.advance(b"\x1b[?5h");
    }
    screen.advance(&menu.render());
    screen
}

/// The visible bell flashes six times in two seconds (EK-VT520-RM 2.12.7.1).
const BELL_FLASH: Duration = Duration::from_secs(2);

impl State {
    /// Whether the visible bell is lit now.
    fn bell_lit(&self) -> bool {
        self.bell_flash.is_some_and(|start| {
            let ms = start.elapsed().as_millis();
            ms < BELL_FLASH.as_millis() && (ms * 6 / BELL_FLASH.as_millis()) % 2 == 0
        })
    }

    fn phases(&self) -> (bool, bool) {
        let ms = self.epoch.elapsed().as_millis();
        (
            ms / CURSOR_BLINK.as_millis() % 2 == 0,
            ms / TEXT_BLINK.as_millis() % 2 == 0,
        )
    }
}

#[derive(Clone)]
pub struct TerminalView {
    area: gtk::GLArea,
    state: Rc<RefCell<State>>,
}

impl TerminalView {
    pub fn new(
        session: Session,
        notices: async_channel::Receiver<Notice>,
        callbacks: Callbacks,
        keymap: SharedKeymap,
    ) -> TerminalView {
        let area = gtk::GLArea::builder()
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .has_depth_buffer(false)
            .build();
        area.set_required_version(3, 3);

        let state = Rc::new(RefCell::new(State {
            session,
            gl: None,
            renderer: None,
            theme: Theme::default(),
            epoch: Instant::now(),
            focused: false,
            last_phases: (true, true),
            callbacks: Rc::new(callbacks),
            capture: std::env::var_os("VEETEE_CAPTURE").map(|path| {
                let delay = std::env::var("VEETEE_CAPTURE_DELAY_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1500);
                (path.into(), Duration::from_millis(delay))
            }),
            selection: None,
            drag_anchor: None,
            setup: None,
            animating: false,
            ticking: false,
            last_activity: Instant::now(),
            saver: false,
            visible_bell: false,
            bell_flash: None,
        }));
        let view = TerminalView { area, state };
        view.connect_gl();
        view.connect_input(keymap);
        view.connect_mouse();
        view.connect_notices(notices);
        view.start_blink_timer();
        view
    }

    pub fn widget(&self) -> &gtk::GLArea {
        &self.area
    }

    pub fn session(&self) -> Session {
        self.state.borrow().session.clone()
    }

    pub fn set_theme(&self, theme: Theme) {
        self.state.borrow_mut().theme = theme;
        self.area.queue_render();
    }

    pub fn set_visible_bell(&self, on: bool) {
        self.state.borrow_mut().visible_bell = on;
    }

    /// Records keyboard or host activity. Returns true if this woke the
    /// screen from the CRT saver.
    fn wake(&self) -> bool {
        let mut st = self.state.borrow_mut();
        st.last_activity = Instant::now();
        let was_blank = std::mem::take(&mut st.saver);
        drop(st);
        if was_blank {
            self.area.queue_render();
        }
        was_blank
    }

    fn connect_gl(&self) {
        let state = self.state.clone();
        self.area.connect_realize(move |area| {
            area.make_current();
            if let Some(err) = area.error() {
                eprintln!("veetee: OpenGL unavailable: {err}");
                return;
            }
            let model = state.borrow().session.terminal().config().model;
            let result = crate::gl_loader::glow_context().and_then(|gl| {
                let fonts = vt_fonts::FontSet::new(vt_render::family(model));
                // SAFETY: GTK made this area's context current above.
                #[allow(unsafe_code)]
                let renderer = unsafe { Renderer::new(&gl, fonts)? };
                Ok((gl, renderer))
            });
            match result {
                Ok((gl, renderer)) => {
                    let mut st = state.borrow_mut();
                    st.gl = Some(gl);
                    st.renderer = Some(renderer);
                }
                Err(e) => eprintln!("veetee: renderer setup failed: {e}"),
            }
        });

        let state = self.state.clone();
        self.area.connect_unrealize(move |area| {
            area.make_current();
            let mut st = state.borrow_mut();
            if let (Some(gl), Some(renderer)) = (st.gl.take(), st.renderer.take()) {
                // SAFETY: context made current above.
                #[allow(unsafe_code)]
                unsafe {
                    renderer.destroy(&gl)
                };
            }
        });

        let state = self.state.clone();
        self.area.connect_render(move |area, _ctx| {
            let st = &mut *state.borrow_mut();
            let (cursor_on, blink_on) = st.phases();
            st.last_phases = (cursor_on, blink_on);
            let frame = FrameState {
                cursor_on,
                blink_on,
                focused: st.focused,
                selection: st.selection,
            };
            let scale = area.scale_factor();
            let (w, h) = (
                (area.width() * scale).max(1) as u32,
                (area.height() * scale).max(1) as u32,
            );
            session::frame_drawn(&st.session);
            if st.saver {
                // CRT saver: the screen is blank until a key or host data.
                if let Some(gl) = st.gl.as_ref() {
                    clear_black(gl);
                    capture_when_due(&mut st.capture, st.epoch, gl, w, h);
                }
                return glib::Propagation::Stop;
            }
            let bell = st.bell_lit();
            if st
                .bell_flash
                .is_some_and(|start| start.elapsed() >= BELL_FLASH)
            {
                st.bell_flash = None;
            }
            let mut animating = st.bell_flash.is_some();
            if let (Some(gl), Some(renderer)) = (st.gl.as_ref(), st.renderer.as_mut()) {
                let term = st.session.terminal();
                if let Some(setup) = st.setup.as_ref() {
                    // Set-Up replaces the page; the status line stays.
                    let mut indicator = indicator_line(&term, setup.was_held, bell);
                    let cols = setup.screen.grid().cols();
                    indicator = format!("{indicator:<cols$}").chars().take(cols).collect();
                    drop(term);
                    let screen = &setup.screen;
                    let layout = vt_render::page_layout(w, h, screen);
                    let frame = FrameState {
                        cursor_on: false,
                        focused: true,
                        selection: None,
                        ..frame
                    };
                    // SAFETY: GTK makes the context current before emitting `render`.
                    #[allow(unsafe_code)]
                    let more = unsafe {
                        renderer.draw(
                            gl,
                            screen,
                            &layout,
                            (w, h),
                            frame,
                            &st.theme,
                            &indicator,
                            None,
                        )
                    };
                    animating |= more;
                } else {
                    let held = st.session.is_held();
                    let layout = vt_render::page_layout(w, h, &term);
                    let indicator = indicator_line(&term, held, bell);
                    let animation = st.session.scroll_animation();
                    let scroll = animation.as_ref().map(|a| vt_render::ScrollFrame {
                        scroll: &a.scroll,
                        progress: a.progress(),
                    });
                    // SAFETY: GTK makes the context current before emitting `render`.
                    #[allow(unsafe_code)]
                    let more = unsafe {
                        renderer.draw(
                            gl,
                            &term,
                            &layout,
                            (w, h),
                            frame,
                            &st.theme,
                            &indicator,
                            scroll,
                        )
                    };
                    animating |= more;
                }
                capture_when_due(&mut st.capture, st.epoch, gl, w, h);
            }
            st.animating = animating;
            if animating && !st.ticking {
                st.ticking = true;
                let weak = Rc::downgrade(&state);
                area.add_tick_callback(move |area, _| {
                    let Some(state) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let mut st = state.borrow_mut();
                    if st.animating {
                        area.queue_render();
                        glib::ControlFlow::Continue
                    } else {
                        st.ticking = false;
                        glib::ControlFlow::Break
                    }
                });
            }
            glib::Propagation::Stop
        });
    }

    fn connect_input(&self, keymap: SharedKeymap) {
        // The input method only sees keys veetee does not map itself. Given
        // first refusal it would turn keypad digits into text, so a host that
        // selected application keypad mode (EDT, EVE) would receive `7`
        // instead of `ESC O w`.
        let keys = gtk::EventControllerKey::new();
        let im = gtk::IMMulticontext::new();
        im.set_client_widget(Some(&self.area));

        let state = self.state.clone();
        im.connect_commit(move |_, text| {
            let session = state.borrow().session.clone();
            session.type_text(text);
            keyclick(&session);
        });

        let view = self.clone();
        let im_keys = im.clone();
        keys.connect_key_pressed(move |controller, keyval, keycode, modifiers| {
            // A key that wakes the screen from the CRT saver does nothing else.
            if view.wake() {
                return glib::Propagation::Stop;
            }
            if view.state.borrow().setup.is_some() {
                let mods = Mods {
                    shift: modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK),
                    ctrl: modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK),
                    alt: modifiers.contains(gtk::gdk::ModifierType::ALT_MASK),
                };
                let is_setup_key = matches!(
                    keymap
                        .borrow()
                        .map(keyval.into_glib(), keyval.to_unicode(), mods),
                    Some(Action::Local(Local::SetUp))
                );
                if let Some(input) = setup_input(keyval, mods, is_setup_key) {
                    view.setup_input(input);
                }
                return glib::Propagation::Stop;
            }
            let to_input_method = || {
                controller
                    .current_event()
                    .is_some_and(|event| im_keys.filter_keypress(&event))
            };
            // A composition in progress keeps every key.
            if !im_keys.preedit_string().0.is_empty() && to_input_method() {
                return glib::Propagation::Stop;
            }
            let mods = Mods {
                shift: modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK),
                ctrl: modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK),
                alt: modifiers.contains(gtk::gdk::ModifierType::ALT_MASK),
            };
            // A main keypad key the host programmed (DECPAK) takes priority.
            if let Some(station) = vt_keyboard::station_for_keycode(keycode) {
                // 🔎 GTK 4 does not report AltGr separately, so group 2 codes are not used yet.
                let alt_graph = false;
                let session = view.state.borrow().session.clone();
                match session.alphanumeric_key(station, mods, alt_graph) {
                    vt_core::KeyOutcome::Handled => {
                        keyclick(&session);
                        return glib::Propagation::Stop;
                    }
                    vt_core::KeyOutcome::LocalFunction(n) => {
                        keyclick(&session);
                        view.programmed_local_function(n);
                        return glib::Propagation::Stop;
                    }
                    vt_core::KeyOutcome::NotProgrammed => {}
                }
            }
            let action = keymap
                .borrow()
                .map(keyval.into_glib(), keyval.to_unicode(), mods);
            let Some(action) = action else {
                // Ordinary typing, dead keys and compose go through the input method.
                return if to_input_method() {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                };
            };
            let session = view.state.borrow().session.clone();
            let outcome = match action {
                Action::Key(key) => session.key(key),
                Action::ModifiedKey(key, mods) => session.key_with(key, mods),
                Action::Text(text) => {
                    session.type_text(&text);
                    vt_core::KeyOutcome::Handled
                }
                Action::Local(local) => {
                    view.local_function(local);
                    vt_core::KeyOutcome::Handled
                }
            };
            if outcome != vt_core::KeyOutcome::NotProgrammed {
                keyclick(&session);
            }
            if let vt_core::KeyOutcome::LocalFunction(n) = outcome {
                view.programmed_local_function(n);
            }
            glib::Propagation::Stop
        });
        let im_keys = im.clone();
        keys.connect_key_released(move |controller, _, _, _| {
            if let Some(event) = controller.current_event() {
                im_keys.filter_keypress(&event);
            }
        });
        self.area.add_controller(keys);

        let focus = gtk::EventControllerFocus::new();
        let (state, area, im_in) = (self.state.clone(), self.area.clone(), im.clone());
        focus.connect_enter(move |_| {
            let callbacks = {
                let mut st = state.borrow_mut();
                st.focused = true;
                st.callbacks.clone()
            };
            (callbacks.focused)();
            im_in.focus_in();
            area.queue_render();
        });
        let (state, area) = (self.state.clone(), self.area.clone());
        focus.connect_leave(move |_| {
            state.borrow_mut().focused = false;
            im.focus_out();
            area.queue_render();
        });
        self.area.add_controller(focus);
    }

    /// Selection with the mouse: drag for a stream, double-click for a word,
    /// triple-click for a line. Middle-click pastes the primary selection;
    /// right-click opens Copy/Paste.
    fn connect_mouse(&self) {
        let actions = gio::SimpleActionGroup::new();
        let copy = gio::SimpleAction::new("copy", None);
        copy.connect_activate({
            let view = self.clone();
            move |_, _| view.copy_to_clipboard()
        });
        actions.add_action(&copy);
        let paste = gio::SimpleAction::new("paste", None);
        paste.connect_activate({
            let view = self.clone();
            move |_, _| view.paste(clipboard_for(&view.area, false))
        });
        actions.add_action(&paste);
        self.area.insert_action_group("view", Some(&actions));

        let menu = gio::Menu::new();
        menu.append(Some("Copy"), Some("view.copy"));
        menu.append(Some("Paste"), Some("view.paste"));
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&self.area);
        popover.set_has_arrow(false);
        self.area.connect_unrealize({
            let popover = popover.clone();
            move |_| popover.unparent()
        });

        let click = gtk::GestureClick::new();
        click.set_button(0);
        let view = self.clone();
        click.connect_pressed(move |gesture, n_press, x, y| {
            view.area.grab_focus();
            match gesture.current_button() {
                gdk::BUTTON_PRIMARY => {
                    let Some(p) = view.point_at(x, y) else { return };
                    let selection = {
                        let st = view.state.borrow();
                        let term = st.session.terminal();
                        match n_press {
                            2 => Some(term.word_at(p)),
                            n if n >= 3 => Some(term.line_at(p)),
                            _ => None,
                        }
                    };
                    view.set_selection(selection);
                    if selection.is_some() {
                        view.copy_to_primary();
                    }
                }
                gdk::BUTTON_MIDDLE => view.paste(clipboard_for(&view.area, true)),
                gdk::BUTTON_SECONDARY => {
                    copy.set_enabled(view.state.borrow().selection.is_some());
                    let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                    popover.set_pointing_to(Some(&rect));
                    popover.popup();
                }
                _ => {}
            }
        });
        self.area.add_controller(click);

        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let view = self.clone();
        drag.connect_drag_begin(move |_, x, y| {
            view.state.borrow_mut().drag_anchor = view.point_at(x, y);
        });
        let view = self.clone();
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some((x, y)) = gesture.start_point() else {
                return;
            };
            // Ignore jitter so a plain click does not start a selection.
            if dx.abs() < 3.0 && dy.abs() < 3.0 {
                return;
            }
            let anchor = view.state.borrow().drag_anchor;
            if let (Some(anchor), Some(head)) = (anchor, view.point_at(x + dx, y + dy)) {
                view.set_selection(Some(Selection::new(anchor, head)));
            }
        });
        let view = self.clone();
        drag.connect_drag_end(move |_, _, _| {
            let dragged = view.state.borrow_mut().drag_anchor.take().is_some();
            if dragged && view.state.borrow().selection.is_some() {
                view.copy_to_primary();
            }
        });
        self.area.add_controller(drag);
    }

    /// The page cell under a widget position, clamped to the page.
    fn point_at(&self, x: f64, y: f64) -> Option<Point> {
        let st = self.state.borrow();
        let term = st.session.terminal();
        let grid = term.display_grid();
        let (window_top, screen_lines) = term.window();
        let scale = f64::from(self.area.scale_factor());
        let (w, h) = (
            (f64::from(self.area.width()) * scale).max(1.0) as u32,
            (f64::from(self.area.height()) * scale).max(1.0) as u32,
        );
        let layout = vt_render::page_layout(w, h, &term);
        let px = ((x * scale) as f32).clamp(layout.x, layout.x + layout.width - 1.0);
        let py = ((y * scale) as f32).clamp(layout.y, layout.y + layout.height - 1.0);
        let (row, col) = layout.cell_at(px, py)?;
        let row = (window_top + row.min(screen_lines - 1)).min(grid.rows() - 1);
        let line = grid.line(row);
        let col = if line.size.is_double_width() {
            col / 2
        } else {
            col
        };
        Some(Point {
            row,
            col: col.min(line.width() - 1),
        })
    }

    fn set_selection(&self, selection: Option<Selection>) {
        let mut st = self.state.borrow_mut();
        if st.selection != selection {
            st.selection = selection;
            self.area.queue_render();
        }
    }

    fn selected_text(&self) -> Option<String> {
        let st = self.state.borrow();
        let selection = st.selection?;
        let text = st.session.terminal().selection_text(&selection);
        (!text.is_empty()).then_some(text)
    }

    fn copy_to_clipboard(&self) {
        match self.selected_text() {
            Some(text) => self.area.clipboard().set_text(&text),
            None => (self.state.borrow().callbacks.notify)("Nothing is selected"),
        }
    }

    fn copy_to_primary(&self) {
        if let Some(text) = self.selected_text() {
            self.area.primary_clipboard().set_text(&text);
        }
    }

    /// Types clipboard text into the session. Line breaks are sent as Return
    /// (CR), as a user would type them.
    fn paste(&self, clipboard: gdk::Clipboard) {
        let session = self.state.borrow().session.clone();
        glib::spawn_future_local(async move {
            if let Ok(Some(text)) = clipboard.read_text_future().await {
                let text = text.replace("\r\n", "\r").replace('\n', "\r");
                session.type_text(&text);
            }
        });
    }

    /// A local function the host assigned to a key (EK-VT520-RM table 8-6).
    fn programmed_local_function(&self, number: u16) {
        let local = match number {
            1 | 30 => Local::HoldScreen,
            2 => Local::PrintScreen,
            3 => Local::SetUp,
            4 | 12 => Local::SwitchSession,
            5 => Local::Break,
            10 => Local::Answerback,
            20 => Local::PanUp,
            21 => Local::PanDown,
            24 => Local::PanPrevPage,
            25 => Local::PanNextPage,
            37 => Local::Paste,
            _ => {
                let st = self.state.borrow();
                (st.callbacks.notify)(&format!("Local function {number} is not available"));
                return;
            }
        };
        self.local_function(local);
    }

    fn local_function(&self, local: Local) {
        // DECLFKC and DECELF let the host reassign or disable local keys.
        let key_number = match local {
            Local::HoldScreen => Some(1),
            Local::PrintScreen => Some(2),
            Local::SetUp => Some(3),
            Local::SwitchSession => Some(4),
            _ => None,
        };
        {
            let session = self.state.borrow().session.clone();
            let term = session.terminal();
            if let Some(n) = key_number {
                match term.local_function_key(n) {
                    vt_core::LocalKeyAction::Local => {}
                    vt_core::LocalKeyAction::Disabled => return,
                    vt_core::LocalKeyAction::SendToHost => {
                        drop(term);
                        session.key(vt_core::Key::Function(n));
                        return;
                    }
                }
            }
            if matches!(local, Local::Copy | Local::Paste) && !term.copy_paste_keys_enabled() {
                return;
            }
        }
        let st = self.state.borrow();
        match local {
            Local::HoldScreen => {
                let held = !st.session.is_held();
                st.session.set_held(held);
                (st.callbacks.status)(if held { "Hold Screen" } else { "" });
                self.area.queue_render();
            }
            Local::Answerback => st.session.send_answerback(),
            Local::Paste => self.paste(self.area.clipboard()),
            Local::SetUp => {
                drop(st);
                self.open_setup();
            }
            Local::PanUp | Local::PanDown | Local::PanPrevPage | Local::PanNextPage => {
                {
                    let mut term = st.session.terminal();
                    match local {
                        Local::PanUp => term.pan_view(-1),
                        Local::PanDown => term.pan_view(1),
                        Local::PanPrevPage => term.view_page(-1),
                        _ => term.view_page(1),
                    }
                }
                self.area.queue_render();
            }
            Local::MarkCheckpoint => match st.session.mark_checkpoint() {
                Some(name) => (st.callbacks.notify)(&format!("Recorded checkpoint {name}")),
                None => (st.callbacks.notify)("This session is not being recorded (--record FILE)"),
            },
            Local::PrintScreen => (st.callbacks.notify)("Printing is not available yet"),
            Local::SwitchSession => {
                let callbacks = st.callbacks.clone();
                drop(st);
                (callbacks.switch_session)();
            }
            Local::Break => {
                if let Err(e) = st.session.send_break() {
                    (st.callbacks.notify)(&format!("Break: {e}"));
                }
            }
            Local::Copy => {
                drop(st);
                self.copy_to_clipboard();
            }
        }
    }

    /// F3: enters Set-Up. The host is held while Set-Up is open, so no data
    /// is lost (Installing and Using the VT420, chapter 5).
    pub fn open_setup(&self) {
        let mut st = self.state.borrow_mut();
        if st.setup.is_some() {
            return;
        }
        let session = st.session.clone();
        let was_held = session.is_held();
        session.set_held(true);
        let (model, features) = {
            let term = session.terminal();
            (term.config().model, term.setup_features())
        };
        let menu = SetupMenu::new(model, short_version(), features);
        let screen = setup_screen(model, &menu);
        st.setup = Some(SetupView {
            menu,
            screen,
            was_held,
        });
        // The window reads this view's state when its status changes.
        let callbacks = st.callbacks.clone();
        drop(st);
        (callbacks.status)("Set-Up");
        self.area.queue_render();
    }

    fn setup_input(&self, input: setup::Input) {
        let mut st = self.state.borrow_mut();
        let session = st.session.clone();
        let (callbacks, number) = (st.callbacks.clone(), st.callbacks.session_number);
        let mut error = None;
        let Some(view) = st.setup.as_mut() else {
            return;
        };
        match view.menu.input(input) {
            setup::Outcome::Redraw => {}
            setup::Outcome::Exit => {
                drop(st);
                self.close_setup();
                return;
            }
            setup::Outcome::Action(action) => {
                let mut term = session.terminal();
                let model = term.config().model;
                let result = match action {
                    setup::Action::ClearComm => {
                        term.clear_comm();
                        Ok(())
                    }
                    setup::Action::ResetSession => {
                        term.reset_session();
                        Ok(())
                    }
                    setup::Action::Recall => {
                        term.recall_setup_features();
                        view.menu.set_features(term.setup_features());
                        Ok(())
                    }
                    setup::Action::Save => {
                        term.apply_setup_features(view.menu.features());
                        term.save_setup_features();
                        crate::setup_store::save(model, number, view.menu.features()).map(|_| ())
                    }
                    setup::Action::Default => {
                        term.restore_factory_setup();
                        view.menu.set_features(term.setup_features());
                        crate::setup_store::remove(model, number)
                    }
                };
                // Local changes never reach the host.
                let _ = term.take_output();
                drop(term);
                match result {
                    Ok(()) => view.menu.done(),
                    Err(e) => error = Some(format!("Set-Up could not be saved: {e}")),
                }
            }
        }
        let model = session.terminal().config().model;
        view.screen = setup_screen(model, &view.menu);
        drop(st);
        if let Some(message) = error {
            (callbacks.notify)(&message);
        }
        self.area.queue_render();
    }

    /// Leaves Set-Up: the features take effect and the host resumes.
    fn close_setup(&self) {
        let mut st = self.state.borrow_mut();
        let Some(view) = st.setup.take() else {
            return;
        };
        let session = st.session.clone();
        let on_line = {
            let mut term = session.terminal();
            term.apply_setup_features(view.menu.features());
            if view.menu.clear_display_on_exit() {
                term.clear_display();
            }
            let _ = term.take_output();
            term.on_line()
        };
        // Local keeps the host on hold.
        session.set_held(view.was_held || !on_line);
        let status = if view.was_held {
            "Hold Screen"
        } else if !on_line {
            "Local"
        } else {
            ""
        };
        let callbacks = st.callbacks.clone();
        drop(st);
        (callbacks.status)(status);
        self.area.queue_render();
    }

    fn connect_notices(&self, notices: async_channel::Receiver<Notice>) {
        let area = self.area.clone();
        let session = self.state.borrow().session.clone();
        let ticking = Rc::new(std::cell::Cell::new(false));
        let view = self.clone();
        let callbacks = self.state.borrow().callbacks.clone();
        glib::spawn_future_local(async move {
            while let Ok(notice) = notices.recv().await {
                match notice {
                    Notice::Redraw => {
                        view.wake();
                        area.queue_render();
                    }
                    Notice::SmoothScroll if !ticking.get() => {
                        // Redraw every frame until scrolling stops.
                        ticking.set(true);
                        let (session, ticking) = (session.clone(), ticking.clone());
                        area.add_tick_callback(move |area, _| {
                            area.queue_render();
                            if session.scroll_animation().is_some() {
                                glib::ControlFlow::Continue
                            } else {
                                ticking.set(false);
                                glib::ControlFlow::Break
                            }
                        });
                    }
                    Notice::SmoothScroll => {}
                    Notice::Sound(sound) => {
                        // Without an audio device the desktop's bell stands in.
                        let bell = matches!(sound, crate::sound::Sound::Bell(v) if v != vt_core::setup::Volume::Off);
                        if matches!(sound, crate::sound::Sound::Bell(_)) {
                            let mut st = view.state.borrow_mut();
                            if st.visible_bell {
                                st.bell_flash = Some(Instant::now());
                                drop(st);
                                area.queue_render();
                            }
                        }
                        if !crate::sound::play(sound) && bell {
                            area.error_bell();
                        }
                    }
                    Notice::Title(name) => (callbacks.title)(&name),
                    Notice::Activate => (callbacks.activate)(),
                    Notice::Exited(reason) => {
                        (callbacks.exited)(reason);
                        break;
                    }
                }
            }
        });
    }

    fn start_blink_timer(&self) {
        if let Some((_, delay)) = self.state.borrow().capture.clone() {
            let area = self.area.downgrade();
            glib::timeout_add_local_once(delay + Duration::from_millis(50), move || {
                if let Some(area) = area.upgrade() {
                    area.queue_render();
                }
            });
        }
        let (state, area) = (Rc::downgrade(&self.state), self.area.downgrade());
        glib::timeout_add_local(Duration::from_millis(40), move || {
            let (Some(state), Some(area)) = (state.upgrade(), area.upgrade()) else {
                return glib::ControlFlow::Break;
            };
            let mut st = state.borrow_mut();
            if st.phases() != st.last_phases {
                area.queue_render();
            }
            if !st.saver && st.setup.is_none() {
                let idle = st.last_activity.elapsed();
                let timeout = crt_saver_override().or_else(|| {
                    // Checked only once the screen has been idle a while.
                    (idle >= Duration::from_secs(60))
                        .then(|| st.session.terminal().crt_saver_timeout())
                        .flatten()
                });
                if timeout.is_some_and(|t| idle >= t) {
                    st.saver = true;
                    area.queue_render();
                }
            }
            glib::ControlFlow::Continue
        });
    }
}

/// The indicator status line, after the VT420's: printer and local
/// state on the left, page number and cursor position on the right.
// 🔎 Field positions are approximate until checked against hardware.
fn indicator_line(term: &vt_core::Terminal, held: bool, bell: bool) -> String {
    let mut left = String::from(" Printer: None");
    if bell {
        left.push_str("   Bell");
    }
    if held {
        left.push_str("   Hold Screen");
    }
    if term.modes().keyboard_locked {
        left.push_str("   Locked");
    }
    if !term.on_line() {
        left.push_str("   Local");
    }
    let cursor = term.cursor();
    let right = format!(
        "Page {}   {:>3},{:<3} ",
        term.page().0 + 1,
        cursor.row + 1,
        cursor.col + 1
    );
    let cols = term.grid().cols();
    let pad = cols.saturating_sub(left.len() + right.len());
    format!("{left}{}{right}", " ".repeat(pad))
}

/// Developer hook: `VEETEE_CRT_SAVER_SECONDS` shortens the CRT saver timeout.
fn crt_saver_override() -> Option<Duration> {
    static OVERRIDE: std::sync::OnceLock<Option<Duration>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| {
        std::env::var("VEETEE_CRT_SAVER_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
    })
}

/// Developer hook (`VEETEE_CAPTURE`): saves the frame once the delay has
/// passed, then quits.
fn capture_when_due(
    capture: &mut Option<(std::path::PathBuf, Duration)>,
    epoch: Instant,
    gl: &glow::Context,
    w: u32,
    h: u32,
) {
    let Some((path, delay)) = capture.clone() else {
        return;
    };
    if epoch.elapsed() < delay {
        return;
    }
    *capture = None;
    match save_frame(gl, w, h, &path) {
        Ok(()) => eprintln!("veetee: captured {}", path.display()),
        Err(e) => eprintln!("veetee: capture failed: {e}"),
    }
    if let Some(app) = gtk::gio::Application::default() {
        app.quit();
    }
}

/// Clears the drawing area to black.
fn clear_black(gl: &glow::Context) {
    use glow::HasContext;
    // SAFETY: called from the render handler with the context current.
    #[allow(unsafe_code)]
    unsafe {
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
    }
}

/// The keyboard clicks for a key that sends a code or acts (Installing and
/// Using the VT420, chapter 4), at the Keyboard Set-Up volume.
fn keyclick(session: &Session) {
    let volume = session.terminal().sound_volumes().keyclick;
    if volume != vt_core::setup::Volume::Off {
        crate::sound::play(crate::sound::Sound::Click(volume));
    }
}

/// The Set-Up key a key press stands for, if any.
fn setup_input(keyval: gdk::Key, mods: Mods, is_setup_key: bool) -> Option<setup::Input> {
    use gdk::Key as K;
    if is_setup_key || keyval == K::F3 {
        return Some(setup::Input::SetUp);
    }
    Some(match keyval {
        K::Up | K::KP_Up => setup::Input::Up,
        K::Down | K::KP_Down => setup::Input::Down,
        K::Left | K::KP_Left => setup::Input::Left,
        K::Right | K::KP_Right => setup::Input::Right,
        K::Return | K::KP_Enter | K::ISO_Enter => setup::Input::Enter,
        K::Tab | K::ISO_Left_Tab | K::KP_Tab => setup::Input::Tab,
        K::BackSpace | K::Delete | K::KP_Delete => setup::Input::Backspace,
        _ => {
            let ch = keyval.to_unicode().filter(|c| !c.is_control())?;
            if mods.ctrl || mods.alt {
                return None;
            }
            setup::Input::Text(ch.to_string())
        }
    })
}

fn clipboard_for(area: &gtk::GLArea, primary: bool) -> gdk::Clipboard {
    if primary {
        area.primary_clipboard()
    } else {
        area.clipboard()
    }
}

/// Reads the current framebuffer and writes it as a binary PPM.
fn save_frame(gl: &glow::Context, w: u32, h: u32, path: &std::path::Path) -> std::io::Result<()> {
    use glow::HasContext;
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    // SAFETY: called from the render handler with the context current.
    #[allow(unsafe_code)]
    unsafe {
        gl.read_pixels(
            0,
            0,
            w as i32,
            h as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
    }
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for row in pixels.chunks((w * 4) as usize).rev() {
        for px in row.chunks(4) {
            out.extend_from_slice(&px[..3]);
        }
    }
    std::fs::write(path, out)
}
