use crate::{audio::Sounds, terminal::Terminals};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::LazyLock,
    time::Duration,
};
use vte4::gtk::{self, gdk, glib, prelude::*};

const ROWS: [&str; 5] = ["row_numbers", "row_1", "row_2", "row_3", "row_space"];
const CONTROL: [&str; 22] = [
    "", "\x1b", "\x1c", "\x1d", "\x1e", "\x1f", "\x11", "\x17", "\x12", "\x12", "\x19", "\x15",
    "\x10", "\x01", "\x13", "\x04", "\x06", "\x1a", "\x18", "\x03", "\x16", "\x02",
];

type ComposeTable = HashMap<String, HashMap<String, String>>;
static COMPOSE: LazyLock<ComposeTable> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/key-compose.json"))
        .expect("validated upstream keyboard tables")
});

pub struct Keyboard {
    pub view: gtk::Box,
    feedback: Rc<Feedback>,
    controller: gtk::EventControllerKey,
}

impl Keyboard {
    pub fn password_mode(&self) -> bool {
        self.feedback.manual.get()
    }

    pub fn detach(&self, window: &gtk::ApplicationWindow) {
        window.remove_controller(&self.controller);
    }

    pub fn set_password_mode(&self, active: bool, tabs: &Terminals) {
        self.feedback.manual.set(active);
        self.refresh(tabs);
    }

    pub fn refresh(&self, tabs: &Terminals) {
        self.feedback.refresh(tabs);
    }
}

struct Feedback {
    view: glib::WeakRef<gtk::Box>,
    manual: Cell<bool>,
    pressed: RefCell<HashMap<u32, Vec<glib::WeakRef<gtk::Widget>>>>,
    blinks: RefCell<HashMap<usize, (u64, glib::WeakRef<gtk::Widget>)>>,
    next_blink: Cell<u64>,
    enter_keys: RefCell<Vec<glib::WeakRef<gtk::Widget>>>,
    physical_shift: Cell<bool>,
    physical_caps: Cell<bool>,
    modifiers: Modifiers,
    pending: RefCell<Vec<String>>,
}

impl Feedback {
    fn refresh(&self, tabs: &Terminals) -> bool {
        let private = self.manual.get() || tabs.password_input();
        if let Some(view) = self.view.upgrade() {
            if view.has_css_class("authentication-mode") != private {
                if private {
                    // A password prompt must not expose key positions via animation.
                    for (_, keys) in self.pressed.borrow_mut().drain() {
                        for key in keys.into_iter().filter_map(|key| key.upgrade()) {
                            key.remove_css_class("key-active");
                            key.remove_css_class("key-blink");
                            key.remove_css_class("key-blink-peak");
                        }
                    }
                    for (_, (_, key)) in self.blinks.borrow_mut().drain() {
                        if let Some(key) = key.upgrade() {
                            key.remove_css_class("key-blink");
                            key.remove_css_class("key-blink-peak");
                            key.remove_css_class("key-active");
                        }
                    }
                    for key in self.enter_keys.borrow().iter().filter_map(glib::WeakRef::upgrade) {
                        key.remove_css_class("key-blink");
                        key.remove_css_class("key-active");
                    }
                    reset(&self.modifiers);
                    self.physical_shift.set(false);
                    self.physical_caps.set(false);
                    self.pending.borrow_mut().clear();
                    if let Some(view) = self.view.upgrade() {
                        view.remove_css_class("shift-on");
                        view.remove_css_class("fn-on");
                        view.remove_css_class("caps-on");
                    }
                    view.add_css_class("authentication-mode");
                } else {
                    view.remove_css_class("authentication-mode");
                }
                view.set_opacity(if private { 0.5 } else { 1.0 });
                view.set_sensitive(!private);
            }
        }
        private
    }

    fn sync_modifier_classes(&self) {
        let Some(view) = self.view.upgrade() else {
            return;
        };
        let buttons = self.modifiers.borrow();
        for (kind, class) in [
            ("shift", "shift-on"),
            ("caps", "caps-on"),
            ("fn", "fn-on"),
        ] {
            let enabled = buttons.iter().any(|(flag, button)| {
                *flag == kind
                    && button.upgrade().is_some_and(|button| button.is_active())
            }) || match kind {
                "shift" => self.physical_shift.get(),
                "caps" => self.physical_caps.get(),
                _ => false,
            };
            if enabled {
                view.add_css_class(class);
            } else {
                view.remove_css_class(class);
            }
        }
    }

    fn press(&self, widget: &gtk::Widget) {
        self.blinks.borrow_mut().remove(&(widget.as_ptr() as usize));
        widget.remove_css_class("key-blink");
        widget.remove_css_class("key-blink-peak");
        widget.add_css_class("key-active");
    }

    // Adapted from EdexUi-2026 keyboard.class.js: keyup/mouseup applies the
    // original 100ms blink, instead of an unconditional 255ms double pulse.
    fn release(self: &Rc<Self>, widget: &gtk::Widget) {
        widget.remove_css_class("key-active");
        let key = widget.as_ptr() as usize;
        let sequence = self.next_blink.get().wrapping_add(1);
        self.next_blink.set(sequence);
        self.blinks
            .borrow_mut()
            .insert(key, (sequence, widget.downgrade()));
        widget.remove_css_class("key-blink-peak");
        widget.add_css_class("key-blink");

        // The upstream 0.5s blink keyframe is only active for 100ms:
        // it reaches 20% of the keyframe cycle, or about 40% brightness.
        let halfway_feedback = Rc::downgrade(self);
        let halfway_key = widget.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(50), move || {
            let Some(feedback) = halfway_feedback.upgrade() else {
                return;
            };
            if feedback
                .blinks
                .borrow()
                .get(&key)
                .is_some_and(|(current, _)| *current == sequence)
            {
                if let Some(widget) = halfway_key.upgrade() {
                    widget.add_css_class("key-blink-peak");
                }
            }
        });

        let feedback = Rc::downgrade(self);
        let weak_widget = widget.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(100), move || {
            let Some(feedback) = feedback.upgrade() else {
                return;
            };
            let is_current = feedback
                .blinks
                .borrow()
                .get(&key)
                .is_some_and(|(current, _)| *current == sequence);
            if !is_current {
                return;
            }
            if let Some(widget) = weak_widget.upgrade() {
                widget.remove_css_class("key-blink");
                widget.remove_css_class("key-blink-peak");
            }
            feedback.blinks.borrow_mut().remove(&key);
        });
    }

    fn release_enter(self: &Rc<Self>) {
        for widget in self.enter_keys.borrow().iter().filter_map(glib::WeakRef::upgrade) {
            self.release(&widget);
        }
    }
}

pub fn panel(
    layout: &Value,
    tabs: &Terminals,
    window: &gtk::ApplicationWindow,
    sounds: &Sounds,
) -> Keyboard {
    // Source rows are 5.28vh high with 0.46vh vertical margins.
    // GtkFixed keeps independently sized keys and split Enter aligned.
    let keyboard = gtk::Box::new(gtk::Orientation::Vertical, 0);
    keyboard.add_css_class("virtual-keyboard");
    keyboard.set_homogeneous(false);
    keyboard.set_vexpand(true);
    let feedback = Rc::new(Feedback {
        view: keyboard.downgrade(),
        manual: Cell::new(false),
        pressed: RefCell::new(HashMap::new()),
        blinks: RefCell::new(HashMap::new()),
        next_blink: Cell::new(0),
        enter_keys: RefCell::new(Vec::new()),
        physical_shift: Cell::new(false),
        physical_caps: Cell::new(false),
        modifiers: Modifiers::default(),
        pending: RefCell::new(Vec::new()),
    });
    let mut indicators = HashMap::<String, glib::WeakRef<gtk::Widget>>::new();
    for row in ROWS {
        let Some(keys) = layout[row].as_array() else {
            continue;
        };
        // Exact coordinates are calculated after the viewport is sized.
        // GtkBox centering made split Enter and narrow modifier keys drift.
        let row_box = gtk::Fixed::new();
        row_box.add_css_class("keyboard-row");
        row_box.set_halign(gtk::Align::Center);
        row_box.set_valign(gtk::Align::Start);
        row_box.set_vexpand(false);
        for key in keys {
            let key = key.clone();
            let command = key["cmd"].as_str().unwrap_or("");
            let name = key["name"].as_str().unwrap_or("");
            let label = match name {
                "ESCAPED|-- ICON: ARROW_UP" => "↑",
                "ESCAPED|-- ICON: ARROW_DOWN" => "↓",
                "ESCAPED|-- ICON: ARROW_LEFT" => "←",
                "ESCAPED|-- ICON: ARROW_RIGHT" => "→",
                other => other,
            };
            if let Some(kind) = modifier(command) {
                let button = gtk::ToggleButton::with_label(label);
                button.add_css_class("virtual-key");
                button.add_css_class("key-wide");
                button.set_valign(gtk::Align::Fill);
                button.set_vexpand(true);
                button.set_focusable(false);
                let key_feedback = feedback.clone();
                let tabs = tabs.clone();
                let sounds = sounds.clone();
                button.connect_clicked(move |button| {
                    if !key_feedback.refresh(&tabs) {
                        key_feedback.release(button.upcast_ref::<gtk::Widget>());
                        key_feedback.sync_modifier_classes();
                        sounds.play("stdin");
                    }
                });
                feedback
                    .modifiers
                    .borrow_mut()
                    .push((kind, button.downgrade()));
                let widget = button.clone().upcast::<gtk::Widget>();
                indicators
                    .entry(kind.to_owned())
                    .or_insert_with(|| widget.downgrade());
                row_box.put(&button, 0.0, 0.0);
            } else {
                let button = gtk::Button::new();
                // Recreate the upstream h1/h2/h3/h4/h5 key legends using
                // GTK overlay positioning rather than a vertical label stack.
                let contents = gtk::Overlay::new();
                contents.add_css_class("key-legends");
                let main = gtk::Label::new(Some(label));
                main.add_css_class("key-main");
                contents.set_child(Some(&main));
                for (field, class) in [
                    ("shift_name", "key-shift-active"),
                    ("fn_name", "key-fn-active"),
                ] {
                    if let Some(text) = key[field].as_str().filter(|name| !name.is_empty()) {
                        let active = gtk::Label::new(Some(text));
                        active.add_css_class(class);
                        active.set_halign(gtk::Align::Center);
                        active.set_valign(gtk::Align::Center);
                        contents.add_overlay(&active);
                    }
                }
                for (field, class, horizontal, vertical) in [
                    ("shift_name", "key-shift", gtk::Align::Start, gtk::Align::Start),
                    ("alt_name", "key-alt", gtk::Align::End, gtk::Align::End),
                    ("fn_name", "key-fn", gtk::Align::Start, gtk::Align::Start),
                    ("altshift_name", "key-altshift", gtk::Align::End, gtk::Align::Start),
                ] {
                    if let Some(text) = key[field].as_str().filter(|name| !name.is_empty()) {
                        let legend = gtk::Label::new(Some(text));
                        legend.add_css_class(class);
                        legend.set_halign(horizontal);
                        legend.set_valign(vertical);
                        legend.set_margin_start(3);
                        legend.set_margin_end(3);
                        legend.set_margin_top(3);
                        legend.set_margin_bottom(3);
                        contents.add_overlay(&legend);
                    }
                }
                button.set_child(Some(&contents));
                button.add_css_class("virtual-key");
                button.set_focusable(false);
                if matches!(command, " " | "\r") || name.contains("ICON: ARROW") {
                    button.add_css_class("key-shape");
                }
                if command == " " {
                    button.add_css_class("key-spacebar");
                    button.set_valign(gtk::Align::Center);
                }
                if command == "\r" {
                    button.add_css_class(if name.is_empty() {
                        "key-enter-lower"
                    } else {
                        "key-enter-upper"
                    });
                    feedback
                        .enter_keys
                        .borrow_mut()
                        .push(button.upcast_ref::<gtk::Widget>().downgrade());
                }
                if name.contains("ICON: ARROW") {
                    button.add_css_class("arrow-key");
                    button.set_halign(gtk::Align::Center);
                }
                if name.len() > 2 && !name.starts_with("ESCAPED") {
                    button.add_css_class("key-wide");
                }
                // The original spacebar is shorter than the letter keys.
                button.set_valign(if command == " " {
                    gtk::Align::Center
                } else {
                    gtk::Align::Fill
                });
                button.set_vexpand(true);
                let widget = button.clone().upcast::<gtk::Widget>();
                for field in ["cmd", "shift_cmd", "alt_cmd", "altshift_cmd"] {
                    if let Some(input) = key[field].as_str().filter(|input| {
                        !input.is_empty() && !input.starts_with("ESCAPED|--")
                    }) {
                        indicators
                            .entry(decode(input))
                            .or_insert_with(|| widget.downgrade());
                    }
                }
                let tabs = tabs.clone();
                let feedback = feedback.clone();
                let sounds = sounds.clone();
                let window = window.downgrade();
                button.connect_clicked(move |button| {
                    if feedback.refresh(&tabs) {
                        return;
                    }
                    if key["cmd"].as_str() == Some("\r") {
                        feedback.release_enter();
                        sounds.play("granted");
                    } else {
                        feedback.release(button.upcast_ref::<gtk::Widget>());
                        sounds.play("stdin");
                    }
                    let mods = &feedback.modifiers;
                    let pending = &feedback.pending;
                    let active = |kind| {
                        mods.borrow().iter().any(|(flag, button)| {
                            *flag == kind
                                && button.upgrade().is_some_and(|button| button.is_active())
                        })
                    };
                    let shift = active("shift");
                    let caps = active("caps");
                    let alt = active("alt");
                    let mut command = key["cmd"].as_str().unwrap_or("");
                    let ctrl = active("ctrl");
                    if (ctrl || alt)
                        && command.chars().count() == 1
                        && !command.chars().any(char::is_control)
                    {
                        let held = [
                            if ctrl { "Ctrl" } else { "" },
                            if alt { "Alt" } else { "" },
                            if shift { "Shift" } else { "" },
                        ];
                        if window.upgrade().is_some_and(|window| {
                            crate::shortcuts::invoke(
                                &window,
                                if command == " " { "Space" } else { command },
                                held,
                            )
                        }) {
                            reset(mods);
                            feedback.sync_modifier_classes();
                            return;
                        }
                    }
                    for (enabled, field) in [
                        (shift || caps, "shift_cmd"),
                        (caps, "capslck_cmd"),
                        (active("ctrl"), "ctrl_cmd"),
                        (alt, "alt_cmd"),
                        (alt && shift, "altshift_cmd"),
                        (active("fn"), "fn_cmd"),
                    ] {
                        if enabled {
                            if let Some(value) =
                                key[field].as_str().filter(|value| !value.is_empty())
                            {
                                command = value;
                            }
                        }
                    }
                    if let Some(command) = command.strip_prefix("ESCAPED|-- ") {
                        if COMPOSE.contains_key(command) {
                            let mut pending = pending.borrow_mut();
                            if !pending.iter().any(|entry| entry == command) {
                                pending.push(command.to_owned());
                            }
                        }
                    } else {
                        let command = compose(command, &mut pending.borrow_mut());
                        let editable = window.upgrade().and_then(|window| {
                            let mut focus = gtk::prelude::GtkWindowExt::focus(&window);
                            while let Some(widget) = focus {
                                if let Some(entry) = widget.dynamic_cast_ref::<gtk::Editable>() {
                                    return Some(entry.clone());
                                }
                                focus = widget.parent();
                            }
                            None
                        });
                        if let Some(editable) = editable {
                            edit(&editable, &command);
                        } else {
                            tabs.write(decode(&command).as_bytes());
                        }
                    }
                    reset(mods);
                    feedback.sync_modifier_classes();
                });
                row_box.put(&button, 0.0, 0.0);
            }
        }
        keyboard.append(&row_box);
    }
    // When a layout is changed at runtime, its new GTK box may be mapped
    // without a window resize; recompute the source-proportional key widths.
    let window_for_size = window.downgrade();
    keyboard.connect_map(move |view| {
        let Some(window) = window_for_size.upgrade() else {
            return;
        };
        let viewport_width = window.width().max(window.default_width()).max(1);
        let viewport_height = window.height().max(window.default_height()).max(1);
        let parent = view.parent();
        let dock_width = parent.as_ref().map(|widget| widget.width())
            .filter(|width| *width > 100)
            .unwrap_or((viewport_width * 56 / 100).max(1));
        let dock_height = if parent.as_ref().is_some_and(|widget| widget.has_css_class("focus-keyboard")) {
            108
        } else {
            (viewport_height * 31 / 100).max(1)
        };
        size_to_source(view, viewport_width, viewport_height, dock_width, dock_height);
    });
    let register = gtk::EventControllerKey::new();
    register.set_propagation_phase(gtk::PropagationPhase::Capture);
    let held = feedback.clone();
    let active_tabs = tabs.clone();
    let enter_on_press = feedback.enter_keys.borrow().clone();
    register.connect_key_pressed(move |_, key, code, _| {
        if !held.refresh(&active_tabs) {
            if matches!(key, gdk::Key::Shift_L | gdk::Key::Shift_R) {
                held.physical_shift.set(true);
                held.sync_modifier_classes();
            }
            if key == gdk::Key::Caps_Lock && !held.pressed.borrow().contains_key(&code) {
                held.physical_caps.set(!held.physical_caps.get());
                held.sync_modifier_classes();
            }
            let name = match key {
                gdk::Key::Return | gdk::Key::KP_Enter => "\r".to_owned(),
                gdk::Key::Escape => "\x1b".to_owned(),
                gdk::Key::BackSpace => "\u{8}".to_owned(),
                gdk::Key::Tab | gdk::Key::ISO_Left_Tab => "\t".to_owned(),
                gdk::Key::Up => "\x1bOA".to_owned(),
                gdk::Key::Down => "\x1bOB".to_owned(),
                gdk::Key::Left => "\x1bOD".to_owned(),
                gdk::Key::Right => "\x1bOC".to_owned(),
                gdk::Key::Shift_L | gdk::Key::Shift_R => "shift".to_owned(),
                gdk::Key::Alt_L | gdk::Key::Alt_R | gdk::Key::ISO_Level3_Shift => "alt".to_owned(),
                gdk::Key::Control_L | gdk::Key::Control_R => "ctrl".to_owned(),
                _ => key.to_unicode().map(String::from).unwrap_or_default(),
            };
            // GDK reports uppercase letters while Shift is held; the
            // source layouts store letter commands in lowercase.
            let name = if name.is_ascii() && name.len() == 1 {
                name.to_ascii_lowercase()
            } else {
                name
            };
            let keys: Vec<gtk::Widget> = if name == "\r" {
                enter_on_press.iter().filter_map(glib::WeakRef::upgrade).collect()
            } else {
                indicators
                    .get(&name)
                    .and_then(glib::WeakRef::upgrade)
                    .into_iter()
                    .collect()
            };
            if !keys.is_empty() {
                let mut pressed = held.pressed.borrow_mut();
                if let std::collections::hash_map::Entry::Vacant(entry) = pressed.entry(code) {
                    entry.insert(keys.iter().map(|widget| widget.downgrade()).collect());
                    for widget in keys {
                        held.press(&widget);
                    }
                }
            }
        }
        glib::Propagation::Proceed
    });
    let released = feedback.clone();
    let released_tabs = tabs.clone();
    let key_sounds = sounds.clone();
    register.connect_key_released(move |_, key, code, _| {
        let was_private = released.refresh(&released_tabs);
        if matches!(key, gdk::Key::Shift_L | gdk::Key::Shift_R) {
            released.physical_shift.set(false);
            released.sync_modifier_classes();
        }
        if let Some(widgets) = released.pressed.borrow_mut().remove(&code) {
            if !was_private {
                for widget in widgets.into_iter().filter_map(|widget| widget.upgrade()) {
                    released.release(&widget);
                }
            }
        }
        if !was_private && matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
            key_sounds.play("granted");
        }
    });
    window.add_controller(register.clone());
    Keyboard {
        view: keyboard,
        feedback,
        controller: register,
    }
}


// EdexUi-2026 keyboard.css (2321bd5): ordinary keys 2.7vw,
// edge keys 8.33vh, the two Enter parts 9.72/7.78vh,
// and the spacebar 47.68vh.
pub fn size_to_source(
    keyboard: &gtk::Box,
    viewport_width: i32,
    viewport_height: i32,
    available_width: i32,
    available_height: i32,
) {
    let vw = viewport_width.max(1) as f64;
    let vh = viewport_height.max(1) as f64;
    let regular = vw * 0.027;
    let edge = vh * 0.0833;
    let enter_upper = vh * 0.0972;
    let enter_lower = vh * 0.0778;
    let spacebar = vh * 0.4768;
    let gutter = vh * 0.0046;
    let regular_height = vw * 0.027;
    // The source uses 3.52vh; increase only the spacebar to 4.5vh
    // as requested, keeping it within the 5.28vh key-row envelope.
    let space_height = vh * 0.045;
    let enter_lower_height = vh * 0.06389;
    let row_height = vh * 0.0528;
    let row_pitch = vh * 0.062; // 5.28vh row + 2 × 0.46vh margin
    let row_margin = gutter;

    struct KeyGeometry {
        widget: gtk::Widget,
        width: f64,
        height: f64,
        gutter: f64,
        enter_lower: bool,
    }
    let mut rows: Vec<(gtk::Fixed, Vec<KeyGeometry>, f64)> = Vec::new();
    let mut maximum: f64 = 1.0;
    for (row_index, row) in children(keyboard.upcast_ref::<gtk::Widget>())
        .into_iter()
        .enumerate()
    {
        let Ok(fixed) = row.downcast::<gtk::Fixed>() else {
            continue;
        };
        let keys = children(fixed.upcast_ref::<gtk::Widget>());
        let last = keys.len().saturating_sub(1);
        let mut total = 0.0;
        let mut geometry = Vec::new();
        for (index, widget) in keys.into_iter().enumerate() {
            let width = if widget.has_css_class("key-spacebar") {
                spacebar
            } else if widget.has_css_class("key-enter-upper") {
                enter_upper
            } else if widget.has_css_class("key-enter-lower") {
                enter_lower
            } else if index == 0 || (index == last && row_index == 0) {
                edge
            } else {
                regular
            };
            let height = if widget.has_css_class("key-spacebar") {
                space_height
            } else if widget.has_css_class("key-enter-lower") {
                enter_lower_height
            } else {
                regular_height
            };
            let gap = if row_index == 4 && index >= last.saturating_sub(2) {
                0.0
            } else {
                gutter
            };
            total += width + 2.0 * gap;
            geometry.push(KeyGeometry {
                enter_lower: widget.has_css_class("key-enter-lower"),
                widget,
                width,
                height,
                gutter: gap,
            });
        }
        maximum = maximum.max(total);
        rows.push((fixed, geometry, total));
    }

    // Scale every key and every offset together. In particular, never
    // independently stretch a row or make all keys the same width.
    let fit_width = (available_width - 16).max(1) as f64 / maximum;
    let fit_height = available_height.max(1) as f64 / (rows.len() as f64 * row_pitch);
    let fit = fit_width.min(fit_height).clamp(0.01, 1.0);
    let cell_height = (row_pitch * fit).round().max(1.0) as i32;
    let fixed_width = (maximum * fit).round().max(1.0) as i32;

    // The upper and lower pieces of Enter are right-aligned to form the
    // original stepped outline, despite belonging to different rows.
    let upper_row_width = rows.get(1).map(|row| row.2).unwrap_or(maximum);
    let lower_row_width = rows.get(2).map(|row| row.2).unwrap_or(maximum);
    let enter_right_adjust = (upper_row_width - lower_row_width) / 2.0;

    for (row_index, (fixed, keys, row_width)) in rows.into_iter().enumerate() {
        fixed.set_size_request(fixed_width, cell_height);
        let remaining = (maximum - row_width).max(0.0);
        let extra_offset = if row_index == 4 {
            (vh * 0.024).min(remaining / 2.0)
        } else {
            0.0
        };
        let mut x = (remaining / 2.0 + extra_offset).max(0.0);

        for key in keys {
            x += key.gutter;
            let width = (key.width * fit).round().max(1.0) as i32;
            let height = (key.height * fit).round().max(1.0) as i32;
            key.widget.set_size_request(width, height);
            key.widget.set_hexpand(false);
            key.widget.set_vexpand(false);
            key.widget.set_halign(gtk::Align::Start);
            key.widget.set_valign(gtk::Align::Start);
            // The fixed container owns spacing. Previous CSS margins added
            // another offset and visibly disconnected the Enter outline.
            key.widget.set_margin_start(0);
            key.widget.set_margin_end(0);
            key.widget.set_margin_top(0);
            key.widget.set_margin_bottom(0);
            let mut key_x = x;
            let mut key_y = row_margin + (row_height - key.height) / 2.0;
            if key.enter_lower {
                key_x += enter_right_adjust;
                // Upstream: top:-0.37vh; join the lower left stroke exactly
                // to the upper key's bottom at differing desktop resolutions.
                key_y -= vh * 0.007;
            }
            fixed.move_(&key.widget, key_x * fit, key_y * fit);
            x += key.width + key.gutter;
        }
    }
}

fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut output = Vec::new();
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        output.push(current);
    }
    output
}

type Modifiers = RefCell<Vec<(&'static str, glib::WeakRef<gtk::ToggleButton>)>>;

fn reset(buttons: &Modifiers) {
    for (flag, button) in buttons.borrow().iter() {
        if let Some(button) = button.upgrade().filter(|_| *flag != "caps") {
            button.set_active(false);
        }
    }
}

fn compose(command: &str, pending: &mut Vec<String>) -> String {
    pending.drain(..).fold(command.to_owned(), |value, key| {
        COMPOSE
            .get(&key)
            .and_then(|table| table.get(&value))
            .cloned()
            .unwrap_or(value)
    })
}

fn edit(editable: &gtk::Editable, command: &str) {
    let mut cursor = editable.position();
    match command {
        "\r" | "\n" => {
            if let Some(widget) = editable.dynamic_cast_ref::<gtk::Widget>() {
                widget.activate();
            }
        }
        "\u{8}" | "\x7f" => {
            if let Some((start, end)) = editable.selection_bounds() {
                editable.delete_text(start, end);
            } else if cursor > 0 {
                editable.delete_text(cursor - 1, cursor);
            }
        }
        value if !value.is_empty() && !value.chars().any(char::is_control) => {
            if let Some((start, end)) = editable.selection_bounds() {
                editable.delete_text(start, end);
                cursor = start;
            }
            editable.insert_text(value, &mut cursor);
            editable.set_position(cursor);
        }
        _ => {}
    }
}

fn modifier(command: &str) -> Option<&'static str> {
    let flag = command.strip_prefix("ESCAPED|-- ")?.split(':').next()?;
    match flag {
        "SHIFT" => Some("shift"),
        "CTRL" => Some("ctrl"),
        "ALT" => Some("alt"),
        "FN" => Some("fn"),
        "CAPSLCK" => Some("caps"),
        _ => None,
    }
}

fn decode(command: &str) -> String {
    if !command.contains("~~~CTRLSEQ") {
        return command.to_owned();
    }
    CONTROL
        .iter()
        .enumerate()
        .fold(command.to_owned(), |command, (index, bytes)| {
            command.replace(&format!("~~~CTRLSEQ{index}~~~"), bytes)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composes_original_dead_keys() {
        assert_eq!(compose("e", &mut vec!["CIRCUM".to_owned()]), "ê");
        assert_eq!(compose("3", &mut vec!["CIRCUM".to_owned()]), "³");
        assert_eq!(compose("x", &mut vec!["OVERRING".to_owned()]), "x");
    }
    #[test]
    fn preserves_upstream_control_sequences() {
        assert_eq!(decode("~~~CTRLSEQ1~~~OA"), "\x1bOA");
        assert_eq!(decode("~~~CTRLSEQ19~~~"), "\x03");
        assert_eq!(modifier("ESCAPED|-- SHIFT: RIGHT"), Some("shift"));
    }
}
