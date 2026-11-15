use crate::{palette, theme};
use serde_json::Value;
use vte4::gtk::{self, gio, glib, prelude::*};

#[derive(Clone)]
enum Field {
    Text(gtk::Entry),
    Toggle(gtk::Switch),
    Number(gtk::SpinButton, bool),
    Choice(gtk::DropDown),
    Palette(gtk::Grid, Vec<gtk::Entry>),
}

impl Field {
    fn widget(&self) -> gtk::Widget {
        match self {
            Self::Text(input) => input.clone().upcast(),
            Self::Toggle(input) => input.clone().upcast(),
            Self::Number(input, _) => input.clone().upcast(),
            Self::Choice(input) => input.clone().upcast(),
            Self::Palette(grid, _) => grid.clone().upcast(),
        }
    }

    fn value(&self) -> Value {
        match self {
            Self::Text(input) => input.text().as_str().into(),
            Self::Toggle(input) => input.is_active().into(),
            Self::Number(input, true) => input.value_as_int().into(),
            Self::Number(input, false) => input.value().into(),
            Self::Choice(input) => input
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .map(|item| item.string().as_str().into())
                .unwrap_or(Value::Null),
            Self::Palette(_, inputs) => {
                let colors: Vec<_> = inputs
                    .iter()
                    .map(|input| input.text().trim().to_owned())
                    .collect();
                let mut value =
                    serde_json::json!({"ansi": &colors[palette::PRIMARY_COLORS.len()..]});
                for ((name, _), color) in palette::PRIMARY_COLORS.iter().zip(colors) {
                    value[*name] = color.into();
                }
                value
            }
        }
    }
}

fn choice(options: Vec<String>, current: &str) -> Field {
    let options: Vec<_> = options.iter().map(String::as_str).collect();
    let field = gtk::DropDown::from_strings(&options);
    if let Some(index) = options.iter().position(|value| *value == current) {
        field.set_selected(index as u32);
    }
    Field::Choice(field)
}

fn terminal_fonts(parent: &gtk::ApplicationWindow, current: &str) -> Field {
    let mut fonts: Vec<_> = parent
        .pango_context()
        .list_families()
        .into_iter()
        .filter(|font| font.is_monospace())
        .map(|font| font.name().to_string())
        .collect();
    if current != palette::FOLLOW_THEME {
        fonts.push(current.to_owned());
    }
    fonts.sort_unstable_by_key(|font| font.to_lowercase());
    fonts.dedup();
    fonts.insert(0, palette::FOLLOW_THEME.to_owned());
    let field = choice(fonts, current);
    if let Field::Choice(input) = &field {
        input.set_enable_search(true);
    }
    field
}

fn custom_colors(colors: &palette::TerminalPalette) -> Field {
    let grid = gtk::Grid::builder()
        .column_spacing(3)
        .row_spacing(4)
        .build();
    grid.add_css_class("terminal-color-grid");
    let inputs = palette::PRIMARY_COLORS
        .iter()
        .map(|(_, label)| *label)
        .chain(palette::ANSI_COLORS)
        .zip(colors.primary().into_iter().chain(colors.ansi))
        .enumerate()
        .map(|(index, (label, rgba))| {
            let input = gtk::Entry::builder()
                .text(rgba.to_string())
                .placeholder_text("#rrggbb or CSS color")
                .width_chars(18)
                .build();
            let swatch = gtk::DrawingArea::builder()
                .content_width(23)
                .content_height(21)
                .build();
            let field = input.downgrade();
            swatch.set_draw_func(move |_, cr, _, _| {
                let color = field
                    .upgrade()
                    .and_then(|field| gtk::gdk::RGBA::parse(field.text().trim()).ok())
                    .unwrap_or(rgba);
                cr.set_source_rgba(
                    color.red().into(),
                    color.green().into(),
                    color.blue().into(),
                    color.alpha().into(),
                );
                let _ = cr.paint();
            });
            let display = swatch.clone();
            input.connect_changed(move |field| {
                if gtk::gdk::RGBA::parse(field.text().trim()).is_ok() {
                    field.remove_css_class("error");
                } else {
                    field.add_css_class("error");
                }
                display.queue_draw();
            });
            let editor = gtk::Box::new(gtk::Orientation::Vertical, 6);
            editor.append(&gtk::Label::new(Some(label)));
            editor.append(&input);
            let popup = gtk::Popover::new();
            popup.add_css_class("terminal-color-popup");
            popup.set_child(Some(&editor));
            let close = popup.downgrade();
            input.connect_activate(move |field| {
                if gtk::gdk::RGBA::parse(field.text().trim()).is_ok() {
                    if let Some(popup) = close.upgrade() {
                        popup.popdown();
                    }
                }
            });
            let button = gtk::MenuButton::new();
            button.set_child(Some(&swatch));
            button.set_popover(Some(&popup));
            button.set_tooltip_text(Some(label));
            button.update_property(&[gtk::accessible::Property::Label(label)]);
            let (column, row) = if index < palette::PRIMARY_COLORS.len() {
                (index as i32, 0)
            } else {
                let index = index as i32 - palette::PRIMARY_COLORS.len() as i32;
                (index % 8, 1 + index / 8)
            };
            grid.attach(&button, column, row, 1, 1);
            input
        })
        .collect();
    Field::Palette(grid, inputs)
}

fn current_resource(window: &gtk::ApplicationWindow, action: &str) -> Option<String> {
    window
        .lookup_action(action)?
        .state()?
        .str()
        .map(str::to_owned)
}

fn apply_resource(
    window: &gtk::ApplicationWindow,
    action: &str,
    value: &str,
) -> Result<(), String> {
    gio::prelude::ActionGroupExt::activate_action(window, action, Some(&value.to_variant()));
    if current_resource(window, action).as_deref() == Some(value) {
        Ok(())
    } else {
        Err(format!("Could not apply {value}"))
    }
}

fn save_values(config: &std::path::Path, values: &Value) -> Result<(), String> {
    std::fs::create_dir_all(config).map_err(|error| error.to_string())?;
    let mut encoded = serde_json::to_vec_pretty(values).map_err(|error| error.to_string())?;
    encoded.push(b'\n');
    glib::file_set_contents(config.join("settings.json"), &encoded)
        .map_err(|error| error.to_string())
}

pub fn open(parent: &gtk::ApplicationWindow, prefs: &theme::Preferences) {
    let config = theme::config_path();
    let mut values = theme::read_values(&config).unwrap_or_default();
    if !values.is_object() {
        values = serde_json::json!({});
    }
    for action in ["theme", "keyboard"] {
        if let Some(value) = current_resource(parent, action) {
            values[action] = value.into();
        }
    }
    let active_palette = prefs.terminal_palette();
    let palette_field = custom_colors(&palette::TerminalPalette::custom(
        &values["termCustomColors"],
        &active_palette,
    ));
    let selected_scheme = choice(
        palette::scheme_names(),
        values["termColorScheme"]
            .as_str()
            .unwrap_or(palette::FOLLOW_THEME),
    );
    let text = |name: &str, default: &str| {
        Field::Text(
            gtk::Entry::builder()
                .text(values[name].as_str().unwrap_or(default))
                .width_chars(30)
                .build(),
        )
    };
    let toggle = |name: &str, default: bool| {
        Field::Toggle(
            gtk::Switch::builder()
                .active(values[name].as_bool().unwrap_or(default))
                .halign(gtk::Align::Start)
                .build(),
        )
    };
    let number = |name: &str, default: f64, min, max, step| {
        let input = gtk::SpinButton::with_range(min, max, step);
        input.set_value(values[name].as_f64().unwrap_or(default));
        Field::Number(input, step == 1.0)
    };
    let mut interfaces = vec!["Automatic".to_owned()];
    interfaces.extend(
        std::fs::read_dir("/sys/class/net")
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned()),
    );
    let fields = [
        ("shell", "Shell", text("shell", "")),
        ("shellArgs", "Shell arguments", text("shellArgs", "")),
        ("cwd", "Initial working directory", text("cwd", "")),
        ("username", "Display name", text("username", "")),
        (
            "keyboard",
            "Keyboard layout",
            choice(
                theme::resource_names("keyboards"),
                values["keyboard"].as_str().unwrap_or("en-US"),
            ),
        ),
        (
            "theme",
            "Theme",
            choice(
                theme::resource_names("themes"),
                values["theme"].as_str().unwrap_or("tron"),
            ),
        ),
        (
            "termFontFamily",
            "Terminal font",
            terminal_fonts(
                parent,
                values["termFontFamily"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(palette::FOLLOW_THEME),
            ),
        ),
        (
            "termFontSize",
            "Terminal font size",
            number("termFontSize", prefs.terminal_font_size(), 7.0, 40.0, 1.0),
        ),
        (
            "termColorScheme",
            "Terminal colors",
            selected_scheme.clone(),
        ),
        ("termCustomColors", "Custom terminal colors", palette_field),
        (
            "showFileManagerButton",
            "Show top file manager button",
            toggle("showFileManagerButton", true),
        ),
        (
            "showInstalledAppsButton",
            "Show top installed apps button",
            toggle("showInstalledAppsButton", true),
        ),
        ("audio", "Audio effects", toggle("audio", true)),
        (
            "audioVolume",
            "Audio volume",
            number("audioVolume", 1.0, 0.0, 1.0, 0.05),
        ),
        ("pingAddr", "Ping address", text("pingAddr", "1.1.1.1")),
        (
            "clockHours",
            "Clock (12 or 24 hours)",
            choice(
                vec!["24".to_owned(), "12".to_owned()],
                if values["clockHours"].as_i64() == Some(12) {
                    "12"
                } else {
                    "24"
                },
            ),
        ),
        (
            "monitor",
            "Monitor index",
            number("monitor", 0.0, 0.0, 99.0, 1.0),
        ),
        ("nointro", "Skip the intro", toggle("nointro", false)),
        ("nocursor", "Hide pointer", toggle("nocursor", false)),
        (
            "termLigatures",
            "Terminal font shaping",
            toggle("termLigatures", false),
        ),
        (
            "disableUpdateCheck",
            "Skip upstream release check",
            toggle("disableUpdateCheck", false),
        ),
        (
            "nativeRenderer",
            "Native renderer",
            choice(
                ["Automatic", "cairo", "gl", "vulkan"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                values["nativeRenderer"].as_str().unwrap_or("Automatic"),
            ),
        ),
        (
            "iface",
            "Network interface",
            choice(interfaces, values["iface"].as_str().unwrap_or("Automatic")),
        ),
        (
            "excludeThreadsFromToplist",
            "Group process threads",
            toggle("excludeThreadsFromToplist", true),
        ),
        (
            "hideDotfiles",
            "Hide dotfiles",
            toggle("hideDotfiles", false),
        ),
        (
            "fsListView",
            "List files as rows",
            toggle("fsListView", false),
        ),
        (
            "disableGlobe",
            "Hide the globe",
            toggle("disableGlobe", false),
        ),
        (
            "experimentalGlobeFeatures",
            "Map public IPv4 addresses in terminal output",
            toggle("experimentalGlobeFeatures", false),
        ),
        (
            "geoIpDatabase",
            "GeoLite2 City database path",
            text("geoIpDatabase", ""),
        ),
    ];
    let original = std::cell::RefCell::new(
        fields
            .iter()
            .map(|(_, _, field)| field.value())
            .collect::<Vec<_>>(),
    );
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("dialog-content");
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let grid = gtk::Grid::builder()
        .column_spacing(24)
        .row_spacing(8)
        .build();
    for (index, (key, caption, field)) in fields.iter().enumerate() {
        let label = gtk::Label::builder()
            .label(*caption)
            .xalign(0.0)
            .hexpand(true)
            .build();
        let widget = field.widget();
        if *key == "termCustomColors" {
            if let Field::Choice(input) = selected_scheme.clone() {
                let colors = widget.clone();
                let title = label.clone();
                let update = move |input: &gtk::DropDown| {
                    let custom = input
                        .selected_item()
                        .and_downcast::<gtk::StringObject>()
                        .is_some_and(|item| item.string() == palette::CUSTOM);
                    colors.set_visible(custom);
                    title.set_visible(custom);
                };
                update(&input);
                input.connect_selected_notify(update);
            }
        }
        grid.attach(&label, 0, index as i32, 1, 1);
        grid.attach(&widget, 1, index as i32, 1, 1);
    }
    scroll.set_child(Some(&grid));
    content.append(&scroll);
    let status = gtk::Label::builder().xalign(0.0).wrap(true).build();
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let close = gtk::Button::with_label("Close");
    let save = gtk::Button::with_label("Save to Disk");
    let download = gtk::Button::with_label("Download GeoLite2 City");
    buttons.append(&download);
    buttons.append(&close);
    buttons.append(&save);
    content.append(&status);
    content.append(&buttons);
    let dialog = dialog(parent, "Settings", 680, 650, &content);
    let download_status = status.clone();
    download.connect_clicked(move |button| {
        button.set_sensitive(false);
        download_status.set_text("Downloading GeoLite2 City…");
        let (writer, reader) = async_channel::bounded(1);
        let _ = std::thread::spawn(move || {
            let _ = writer.send_blocking(crate::geo::download());
        });
        let weak = button.downgrade();
        let message = download_status.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let result = reader.recv().await;
            if let Some(button) = weak.upgrade() {
                button.set_sensitive(true);
            }
            if let Some(message) = message.upgrade() {
                message.set_text(&match result {
                    Ok(Ok(path)) => format!("GeoLite2 City saved to {}", path.display()),
                    Ok(Err(error)) => format!("GeoLite2 download failed: {error}"),
                    Err(error) => error.to_string(),
                });
            }
        });
    });
    close.connect_clicked(close_from);
    let weak_window = parent.downgrade();
    save.connect_clicked(move |_| {
        let result = (|| {
            let window = weak_window.upgrade().ok_or("The window was closed")?;
            let mut next = theme::read_values(&config)?;
            let mut apply = Vec::new();
            let mut restart = false;
            let mut quick_buttons = false;
            let mut terminal_style = false;
            let mut custom_palette_edited = false;
            let previous = original.borrow();
            let current: Vec<_> = fields.iter().map(|(_, _, field)| field.value()).collect();
            for (((key, _, _), value), before) in fields.iter().zip(&current).zip(previous.iter()) {
                if value == before {
                    continue;
                }
                if matches!(*key, "theme" | "keyboard") {
                    let name = value.as_str().ok_or("Select a theme or keyboard")?;
                    let directory = if *key == "theme" {
                        "themes"
                    } else {
                        "keyboards"
                    };
                    theme::resource(&config, directory, name)?;
                    apply.push((*key, name));
                } else if matches!(*key, "showFileManagerButton" | "showInstalledAppsButton") {
                    quick_buttons = true;
                } else if palette::STYLE_SETTINGS.contains(key) {
                    terminal_style = true;
                    custom_palette_edited |= matches!(*key, "termCustomColors" | "termColorScheme");
                    if *key == "termColorScheme" && value.as_str() == Some(palette::CUSTOM) {
                        if let Some(position) = fields
                            .iter()
                            .position(|(key, ..)| *key == "termCustomColors")
                        {
                            next["termCustomColors"] = current[position].clone();
                        }
                    }
                } else {
                    restart = true;
                }
                next[*key] = match (*key, value) {
                    ("clockHours", Value::String(value)) => {
                        value.parse::<u32>().unwrap_or(24).into()
                    }
                    ("iface", Value::String(value)) if value == "Automatic" => false.into(),
                    (_, value) => value.clone(),
                };
            }
            drop(previous);
            if custom_palette_edited
                && next["termColorScheme"].as_str() == Some(palette::CUSTOM)
                && !palette::custom_is_valid(&next["termCustomColors"])
            {
                return Err("Use a valid color such as #a6e3a1 for each terminal swatch.".into());
            }
            save_values(&config, &next)?;
            for (action, name) in apply {
                apply_resource(&window, action, name)?;
            }
            if terminal_style {
                gio::prelude::ActionGroupExt::activate_action(&window, "terminal-style", None);
            }
            if quick_buttons {
                let show_file = next["showFileManagerButton"].as_bool().unwrap_or(true);
                let show_apps = next["showInstalledAppsButton"].as_bool().unwrap_or(true);
                gio::prelude::ActionGroupExt::activate_action(
                    &window,
                    "quick-buttons",
                    Some(&(show_file, show_apps).to_variant()),
                );
            }
            original.replace(current);
            Ok::<_, String>(restart)
        })();
        status.set_text(&match result {
            Ok(true) => "Saved. Restart eDEX to apply the other settings.".to_owned(),
            Ok(false) => "Saved and applied.".to_owned(),
            Err(error) => format!("Could not save settings: {error}"),
        });
    });
    dialog.present();
}

pub struct Modal {
    parent: gtk::Overlay,
    pub layer: gtk::Overlay,
}

impl Modal {
    pub fn present(&self) {
        self.parent.add_overlay(&self.layer);
        self.layer.grab_focus();
    }
}

pub fn close_from(widget: &impl IsA<gtk::Widget>) {
    let mut node = Some(widget.clone().upcast::<gtk::Widget>());
    while let Some(widget) = node {
        let parent = widget.parent();
        if widget.has_css_class("modal-layer") {
            if let Some(parent) = parent.and_downcast::<gtk::Overlay>() {
                parent.remove_overlay(&widget);
            }
            return;
        }
        node = parent;
    }
}

pub fn dialog(
    parent: &gtk::ApplicationWindow,
    title: &str,
    width: i32,
    height: i32,
    content: &impl IsA<gtk::Widget>,
) -> Modal {
    let container = parent
        .child()
        .and_downcast::<gtk::Overlay>()
        .expect("workspace overlay");
    let layer = gtk::Overlay::new();
    layer.add_css_class("modal-layer");
    layer.set_focusable(true);
    let shade = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shade.add_css_class("modal-backdrop");
    layer.set_child(Some(&shade));
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("modal-card");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_size_request(
        width.min((parent.width() - 48).max(300)),
        height.min((parent.height() - 48).max(220)),
    );
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("modal-heading");
    let caption = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .hexpand(true)
        .build();
    let close = gtk::Button::with_label("✕");
    close.set_tooltip_text(Some("Close"));
    close.connect_clicked(close_from);
    header.append(&caption);
    header.append(&close);
    card.append(&header);
    content.set_vexpand(true);
    card.append(content);
    layer.add_overlay(&card);

    let old_focus = gtk::prelude::GtkWindowExt::focus(parent).map(|focus| focus.downgrade());
    let weak_parent = parent.downgrade();
    layer.connect_unmap(move |_| {
        let restored = old_focus
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
            .is_some_and(|widget| widget.is_mapped() && widget.grab_focus());
        if !restored {
            if let Some(parent) = weak_parent.upgrade() {
                gio::prelude::ActionGroupExt::activate_action(&parent, "focus-terminal", None);
            }
        }
    });
    let escape = gtk::EventControllerKey::new();
    escape.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = layer.downgrade();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(layer) = weak.upgrade() {
                close_from(&layer);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    layer.add_controller(escape);
    Modal {
        parent: container,
        layer,
    }
}

pub fn shortcuts(parent: &gtk::ApplicationWindow) {
    let labels = [
        ("Ctrl+1–5", "Switch terminal tab"),
        ("Ctrl+Tab / Ctrl+Shift+Tab", "Next / previous terminal tab"),
        ("Ctrl+Shift+T", "New terminal tab"),
        ("Ctrl+Shift+C / V", "Terminal copy / paste"),
        ("Ctrl+Shift+S / K", "Settings / shortcuts"),
        ("Ctrl+Shift+A / F", "App Manager / file search"),
        ("Ctrl+Shift+L / H", "File list / dotfiles"),
        ("Ctrl+Shift+P", "Toggle on-screen keyboard privacy"),
        ("F9 / F10 / F11", "Bottom panels / side panels / fullscreen"),
        ("Ctrl+Shift+Q", "Quit"),
    ];
    let content = gtk::Grid::builder()
        .column_spacing(18)
        .row_spacing(9)
        .build();
    content.add_css_class("dialog-content");
    for (index, (keys, label)) in labels.into_iter().enumerate() {
        content.attach(
            &gtk::Label::builder().label(keys).xalign(0.0).build(),
            0,
            index as i32,
            1,
            1,
        );
        content.attach(
            &gtk::Label::builder().label(label).xalign(0.0).build(),
            1,
            index as i32,
            1,
            1,
        );
    }
    dialog(parent, "Keyboard Shortcuts", 580, 350, &content).present();
}

pub fn resources(parent: &gtk::ApplicationWindow, kind: &str, config: &std::path::Path) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    let config = config.to_owned();
    for name in theme::resource_names(kind) {
        let button = gtk::Button::with_label(&name);
        let is_theme = kind == "themes";
        let config = config.clone();
        let window = parent.downgrade();
        let key = if is_theme { "theme" } else { "keyboard" };
        button.connect_clicked(move |button| {
            let result = (|| {
                let window = window.upgrade().ok_or("The window was closed")?;
                theme::resource(
                    &config,
                    if is_theme { "themes" } else { "keyboards" },
                    &name,
                )?;
                let mut settings = theme::read_values(&config)?;
                settings[key] = name.clone().into();
                save_values(&config, &settings)?;
                apply_resource(&window, key, &name)
            })();
            button.set_label(&match result {
                Ok(()) => format!("{name} — applied"),
                Err(error) => error,
            });
        });
        list.append(&button);
    }
    let scroll = gtk::ScrolledWindow::builder().child(&list).build();
    dialog(parent, kind, 490, 530, &scroll).present();
}
