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
    pressed: RefCell<HashMap<u32, glib::WeakRef<gtk::Widget>>>,
    pulses: RefCell<HashMap<usize, (u64, glib::WeakRef<gtk::Widget>)>>,
    next_pulse: Cell<u64>,
    modifiers: Modifiers,
    pending: RefCell<Vec<String>>,
}

impl Feedback {
    fn refresh(&self, tabs: &Terminals) -> bool {
        let private = self.manual.get() || tabs.password_input();
        if let Some(view) = self.view.upgrade() {
            if view.has_css_class("authentication-mode") != private {
                if private {
                    // Keep the keyboard on screen but do not reveal secret keys
                    // through key flashes, active modifiers, or mouse input.
                    self.pressed.borrow_mut().clear();
                    for (_, (_, widget)) in self.pulses.borrow_mut().drain() {
                        if let Some(widget) = widget.upgrade() {
                            widget.remove_css_class("key-flash");
                            widget.remove_css_class("key-echo");
                        }
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

    fn pulse(self: &Rc<Self>, widget: &gtk::Widget) {
        let key = widget.as_ptr() as usize;
        let sequence = self.next_pulse.get().wrapping_add(1);
        self.next_pulse.set(sequence);
        self.pulses
            .borrow_mut()
            .insert(key, (sequence, widget.downgrade()));

        widget.remove_css_class("key-echo");
        widget.add_css_class("key-flash");

        // First, a bright flash; then a short pause and a weaker afterglow.
        // A sequence ID prevents old timeouts from cancelling a newer press.
        for (delay_ms, echo, finished) in [
            (95, false, false),
            (145, true, false),
            (255, false, true),
        ] {
            let feedback = Rc::downgrade(self);
            let weak_widget = widget.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || {
                let Some(feedback) = feedback.upgrade() else {
                    return;
                };
                if !feedback
                    .pulses
                    .borrow()
                    .get(&key)
                    .is_some_and(|(current, _)| *current == sequence)
                {
                    return;
                }

                if let Some(widget) = weak_widget.upgrade() {
                    widget.remove_css_class("key-flash");
                    if echo {
                        widget.add_css_class("key-echo");
                    } else {
                        widget.remove_css_class("key-echo");
                    }
                }
                if finished {
                    feedback.pulses.borrow_mut().remove(&key);
                }
            });
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
        pulses: RefCell::new(HashMap::new()),
        next_pulse: Cell::new(0),
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
                button.set_valign(gtk::Align::Center);
                button.set_vexpand(true);
                button.set_focusable(false);
                let key_feedback = feedback.clone();
                let tabs = tabs.clone();
                let sounds = sounds.clone();
                button.connect_clicked(move |button| {
                    if !key_feedback.refresh(&tabs) {
                        key_feedback.pulse(button.upcast_ref::<gtk::Widget>());
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
                let contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let alternate = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                for field in ["shift_name", "alt_name", "altshift_name"] {
                    if let Some(upper) = key[field].as_str().filter(|name| !name.is_empty()) {
                        let top = gtk::Label::new(Some(upper));
                        top.add_css_class("key-shift");
                        alternate.append(&top);
                    }
                }
                if alternate.first_child().is_some() {
                    contents.append(&alternate);
                }
                contents.append(&gtk::Label::new(Some(label)));
                button.set_child(Some(&contents));
                button.add_css_class("virtual-key");
                button.set_focusable(false);
                if matches!(command, " " | "\r") || name.contains("ICON: ARROW") {
                    button.add_css_class("key-shape");
                }
                if command == "\r" {
                    button.add_css_class(if name.is_empty() {
                        "key-enter-lower"
                    } else {
                        "key-enter-upper"
                    });
                }
                if name.contains("ICON: ARROW") {
                    button.add_css_class("arrow-key");
                    button.set_halign(gtk::Align::Center);
                }
                if name.len() > 2 && !name.starts_with("ESCAPED") {
                    button.add_css_class("key-wide");
                }
                button.set_valign(if command == "\r" {
                    gtk::Align::Fill
                } else {
                    gtk::Align::Center
                });
                button.set_vexpand(true);
                let widget = button.clone().upcast::<gtk::Widget>();
                for field in ["cmd", "shift_cmd", "alt_cmd", "altshift_cmd"] {
                    if let Some(input) = key[field].as_str().filter(|input| {
                        !input.is_empty()
                            && !input.starts_with("ESCAPED|--")
                            && !input.starts_with("~~~")
                    }) {
                        indicators
                            .entry(input.to_owned())
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
                    feedback.pulse(button.upcast_ref::<gtk::Widget>());
                    sounds.play("stdin");
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
    register.connect_key_pressed(move |_, key, code, _| {
        if !held.refresh(&active_tabs) {
            let name = match key {
                gdk::Key::Return | gdk::Key::KP_Enter => "\r".to_owned(),
                gdk::Key::BackSpace => "\u{8}".to_owned(),
                gdk::Key::Tab | gdk::Key::ISO_Left_Tab => "\t".to_owned(),
                gdk::Key::Shift_L | gdk::Key::Shift_R => "shift".to_owned(),
                gdk::Key::Alt_L | gdk::Key::Alt_R | gdk::Key::ISO_Level3_Shift => "alt".to_owned(),
                gdk::Key::Control_L | gdk::Key::Control_R => "ctrl".to_owned(),
                _ => key.to_unicode().map(String::from).unwrap_or_default(),
            };
            if let Some(widget) = indicators.get(&name).and_then(glib::WeakRef::upgrade) {
                let mut pressed = held.pressed.borrow_mut();
                if let std::collections::hash_map::Entry::Vacant(entry) = pressed.entry(code) {
                    entry.insert(widget.downgrade());
                    held.pulse(&widget);
                }
            }
        }
        glib::Propagation::Proceed
    });
    let released = feedback.clone();
    register.connect_key_released(move |_, _, code, _| {
        released.pressed.borrow_mut().remove(&code);
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
