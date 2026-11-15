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
                    self.pressed.borrow_mut().clear();
                    for (_, (_, key)) in self.blinks.borrow_mut().drain() {
                        if let Some(key) = key.upgrade() {
                            key.remove_css_class("key-blink");
                            key.remove_css_class("key-active");
                        }
                    }
                    for key in self.enter_keys.borrow().iter().filter_map(glib::WeakRef::upgrade) {
                        key.remove_css_class("key-blink");
                        key.remove_css_class("key-active");
                    }
                    reset(&self.modifiers);
                    self.pending.borrow_mut().clear();
                    view.add_css_class("authentication-mode");
                } else {
                    view.remove_css_class("authentication-mode");
                }
                view.set_opacity(if private { 0.28 } else { 1.0 });
                view.set_sensitive(!private);
            }
        }
        private
    }

    fn press(&self, widget: &gtk::Widget) {
        self.blinks.borrow_mut().remove(&(widget.as_ptr() as usize));
        widget.remove_css_class("key-blink");
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
        widget.add_css_class("key-blink");

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
    let keyboard = gtk::Box::new(gtk::Orientation::Vertical, 3);
    keyboard.add_css_class("virtual-keyboard");
    keyboard.set_homogeneous(true);
    keyboard.set_vexpand(true);
    let feedback = Rc::new(Feedback {
        view: keyboard.downgrade(),
        manual: Cell::new(false),
        pressed: RefCell::new(HashMap::new()),
        blinks: RefCell::new(HashMap::new()),
        next_blink: Cell::new(0),
        enter_keys: RefCell::new(Vec::new()),
        modifiers: Modifiers::default(),
        pending: RefCell::new(Vec::new()),
    });
    let mut indicators = HashMap::<String, glib::WeakRef<gtk::Widget>>::new();
    for row in ROWS {
        let Some(keys) = layout[row].as_array() else {
            continue;
        };
        let grid = gtk::Grid::builder()
            .column_homogeneous(true)
            .column_spacing(3)
            .hexpand(true)
            .build();
        grid.set_vexpand(true);
        let mut column = 0;
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
            let span = match command {
                " " => 15,
                "\t" | "\r" | "\u{8}" => 3,
                _ if name.len() > 2 => 3,
                _ => 2,
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
                grid.attach(&button, column, 0, span, 1);
            } else {
                let button = gtk::Button::new();
                // Recreate the upstream h1/h2/h3/h4/h5 key legends using
                // GTK overlay positioning rather than a vertical label stack.
                let contents = gtk::Overlay::new();
                contents.add_css_class("key-legends");
                let main = gtk::Label::new(Some(label));
                main.add_css_class("key-main");
                contents.set_child(Some(&main));
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
                });
                grid.attach(&button, column, 0, span, 1);
            }
            column += span;
        }
        keyboard.append(&grid);
    }
    let register = gtk::EventControllerKey::new();
    register.set_propagation_phase(gtk::PropagationPhase::Capture);
    let held = feedback.clone();
    let active_tabs = tabs.clone();
    let enter_on_press = feedback.enter_keys.borrow().clone();
    register.connect_key_pressed(move |_, key, code, _| {
        if !held.refresh(&active_tabs) {
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
