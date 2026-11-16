use crate::theme::Preferences;
use std::{cell::Cell, rc::Rc};
use vte4::gtk::{self, gdk, prelude::*};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Dock {
    #[default]
    Split,
    Reversed,
    Keyboard,
    Files,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Layout {
    dock: Dock,
    disrupted: bool,
    focus: bool,
}

impl Layout {
    fn for_theme(theme: &Preferences) -> Self {
        let mut layout = match theme.theme_name.as_str() {
            "tron-typeleft" => Self {
                dock: Dock::Reversed,
                ..Self::default()
            },
            "tron-fulltype" => Self {
                dock: Dock::Keyboard,
                ..Self::default()
            },
            "cyborg-focus" => Self {
                focus: true,
                ..Self::default()
            },
            name if name.ends_with("-notype") => Self {
                dock: Dock::Files,
                ..Self::default()
            },
            name if name.ends_with("-disrupted") => Self {
                disrupted: true,
                ..Self::default()
            },
            _ => Self::default(),
        };
        let mut keyboard_left = false;
        let mut files_right = false;
        for (selector, property, value) in declarations(&legacy_source(theme)) {
            let value = value
                .split_whitespace()
                .collect::<String>()
                .to_ascii_lowercase();
            let value = value.trim_end_matches("!important");
            match (selector.as_str(), property, value) {
                ("section#keyboard", "display", "none") => layout.dock = Dock::Files,
                ("section#filesystem", "display", "none") => layout.dock = Dock::Keyboard,
                ("section#keyboard", "right", "44vw") => keyboard_left = true,
                ("section#filesystem", "left", "55vw") => files_right = true,
                ("section#main_shell", "left", "29vh") => layout.disrupted = true,
                _ => {}
            }
        }
        if keyboard_left && files_right && layout.dock == Dock::Split {
            layout.dock = Dock::Reversed;
        }
        layout
    }
}

#[derive(Clone)]
pub struct Workspace {
    root: gtk::Box,
    columns: gtk::Box,
    center: gtk::Box,
    system: gtk::ScrolledWindow,
    network: gtk::ScrolledWindow,
    dock: gtk::Paned,
    files: gtk::Box,
    keyboard: gtk::Box,
    mount: gtk::Box,
    viewport: gtk::DrawingArea,
    layout: Rc<Cell<Layout>>,
}

impl Workspace {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: &gtk::Box,
        columns: &gtk::Box,
        center: &gtk::Box,
        system: &gtk::ScrolledWindow,
        network: &gtk::ScrolledWindow,
        dock: &gtk::Paned,
        files: &gtk::Box,
        keyboard: &gtk::Box,
        mount: &gtk::Box,
        viewport: &gtk::DrawingArea,
    ) -> Self {
        let this = Self {
            root: root.clone(),
            columns: columns.clone(),
            center: center.clone(),
            system: system.clone(),
            network: network.clone(),
            dock: dock.clone(),
            files: files.clone(),
            keyboard: keyboard.clone(),
            mount: mount.clone(),
            viewport: viewport.clone(),
            layout: Rc::new(Cell::new(Layout::default())),
        };
        let (system, network, dock, keyboard, layout) = (
            system.clone(),
            network.clone(),
            dock.clone(),
            keyboard.clone(),
            this.layout.clone(),
        );
        viewport.connect_resize(move |_, width, height| {
            size_workspace(
                &system,
                &network,
                &dock,
                &keyboard,
                layout.get(),
                width,
                height,
            );
        });
        this
    }

    pub fn apply(&self, theme: &Preferences) {
        let layout = Layout::for_theme(theme);
        let previous = self.layout.replace(layout);
        if self.dock.start_child().is_none() || previous.dock != layout.dock {
            self.dock.set_start_child(None::<&gtk::Widget>);
            self.dock.set_end_child(None::<&gtk::Widget>);
            match layout.dock {
                Dock::Split => {
                    self.dock.set_start_child(Some(&self.files));
                    self.dock.set_end_child(Some(&self.keyboard));
                }
                Dock::Reversed => {
                    self.dock.set_start_child(Some(&self.keyboard));
                    self.dock.set_end_child(Some(&self.files));
                }
                Dock::Files => self.dock.set_start_child(Some(&self.files)),
                Dock::Keyboard => self.dock.set_start_child(Some(&self.keyboard)),
            }
        }
        self.columns.reorder_child_after(
            &self.network,
            if layout.disrupted {
                Some(self.system.upcast_ref::<gtk::Widget>())
            } else {
                Some(self.center.upcast_ref::<gtk::Widget>())
            },
        );
        if layout.focus {
            self.root.add_css_class("focus-layout");
            self.keyboard.add_css_class("focus-keyboard");
        } else {
            self.root.remove_css_class("focus-layout");
            self.keyboard.remove_css_class("focus-keyboard");
        }
        self.mount.set_visible(!layout.focus);
        size_workspace(
            &self.system,
            &self.network,
            &self.dock,
            &self.keyboard,
            layout,
            self.viewport.width().max(1),
            self.viewport.height().max(1),
        );
    }
}

fn size_workspace(
    system: &gtk::ScrolledWindow,
    network: &gtk::ScrolledWindow,
    dock: &gtk::Paned,
    keyboard: &gtk::Box,
    layout: Layout,
    width: i32,
    height: i32,
) {
    let sidebar_width = (width * 16 / 100).max(180);
    // EdexUi-2026 keyboard.css:
    // 5 rows × (5.28vh row + 0.46vh top/bottom margins) = 31vh.
    // The special focus theme intentionally retains its compact 108px dock.
    let dock_height = if layout.focus {
        108
    } else if height <= 1 {
        235 // Before the first viewport size is known.
    } else {
        (height * 31 / 100).max(1)
    };
    let position = width
        * if layout.dock == Dock::Reversed {
            56
        } else {
            44
        }
        / 100;
    system.set_width_request(sidebar_width);
    network.set_width_request(sidebar_width);
    dock.set_position(position);
    dock.set_height_request(dock_height);
    keyboard.set_size_request(-1, dock_height);
    // GTK4 has no viewport-relative CSS units. Resolve the upstream
    // 2.7vw / 8.33vh / 47.68vh key widths during every viewport resize.
    let keyboard_width = match layout.dock {
        Dock::Split => (width - position - 28).max(1),
        Dock::Reversed => (position - 28).max(1),
        Dock::Keyboard => (width - 28).max(1),
        Dock::Files => 1,
    };
    if let Some(view) = keyboard.first_child().and_downcast::<gtk::Box>() {
        crate::keyboard::size_to_source(&view, width, height, keyboard_width, dock_height);
    }
}

// Translate explicit legacy panel and widget styles. Reconstruct values rather
// than handing the GTK parser arbitrary browser CSS or resource references.
pub fn theme_css(theme: &Preferences) -> String {
    let mut output = String::new();
    for (selector, property, value) in declarations(&legacy_source(theme)) {
        let target = match selector.as_str() {
            "body" => "window",
            "section#main_shell" => ".main-shell",
            "section#filesystem" => ".filesystem-panel",
            "section#keyboard" => ".virtual-keyboard",
            "section#mod_column_left" => ".system-column",
            "section#mod_column_right" => ".network-column",
            "div#mod_clock" | "#mod_clock" => ".clock",
            "#mod_cpuinfo" => ".cpu-module",
            "#mod_ramwatcher" => ".memory-module",
            "#mod_toplist" => ".process-module",
            "#mod_netstat" | "#mod_netstat_inner" | "div#mod_netstat_inner" => ".network-status",
            "#mod_globe" | "#mod_globe_innercontainer" | "div#mod_globe_innercontainer" => {
                ".world-module"
            }
            "#mod_conninfo" => ".network-traffic",
            "#keyboard .keyboard_key" | "div.keyboard_key" => "button.virtual-key",
            "#keyboard .keyboard_key.active"
            | "section#keyboard .keyboard_key.active"
            | "section#keyboard .keyboard_key.blink" => "button.virtual-key.key-active",
            "ul#main_shell_tabs" => ".tab-bar",
            "ul#main_shell_tabs>li" => "button.shell-tab",
            _ => continue,
        };
        let property = match property {
            "background" => "background-color",
            "color" | "background-color" | "border-color" | "opacity" | "font-size"
            | "font-weight" | "letter-spacing" | "border-radius" => property,
            _ => continue,
        };
        let value = value.trim_end_matches("!important").trim();
        let property_value = match property {
            "color" | "background-color" | "border-color" => legacy_color(theme, value),
            "opacity" => bounded_number(value, 0.0, 1.0),
            "font-weight" => match value {
                "normal" | "bold" => Some(value.to_owned()),
                _ => bounded_number(value, 100.0, 900.0)
                    .filter(|weight| weight.parse::<u32>().is_ok_and(|weight| weight % 100 == 0)),
            },
            "font-size" | "letter-spacing" | "border-radius" => {
                let (number, unit) = value
                    .strip_suffix("px")
                    .map(|value| (value, "px"))
                    .or_else(|| value.strip_suffix("pt").map(|value| (value, "pt")))
                    .unwrap_or((value, ""));
                let minimum = match property {
                    "font-size" => 6.0,
                    "letter-spacing" => -10.0,
                    _ => 0.0,
                };
                bounded_number(number, minimum, 160.0).and_then(|value| {
                    (!unit.is_empty() || value == "0").then(|| format!("{value}{unit}"))
                })
            }
            _ => None,
        };
        if let Some(value) = property_value {
            output.push_str(&format!("{target} {{ {property}: {value}; }}\n"));
        }
    }
    output
}

fn bounded_number(value: &str, low: f64, high: f64) -> Option<String> {
    let value = value.trim().parse::<f64>().ok()?;
    (low..=high).contains(&value).then(|| value.to_string())
}

fn legacy_color(theme: &Preferences, value: &str) -> Option<String> {
    let (r, g, b) = theme.accent();
    let mut color = value.to_owned();
    for (name, rgb) in [("r", r), ("g", g), ("b", b)] {
        color = color.replace(
            &format!("var(--color_{name})"),
            &format!("{:.0}", rgb * 255.0),
        );
    }
    color = color.replace("var(--color_light_black)", &theme.background().to_string());
    gdk::RGBA::parse(&color).ok().map(|color| color.to_string())
}

fn legacy_source(theme: &Preferences) -> String {
    let mut rest = theme.theme["injectCSS"].as_str().unwrap_or_default();
    let mut output = String::with_capacity(rest.len());
    while let Some((before, after)) = rest.split_once("/*") {
        output.push_str(before);
        let Some((_, remaining)) = after.split_once("*/") else {
            return output;
        };
        rest = remaining;
    }
    output.push_str(rest);
    output
}

fn declarations(css: &str) -> Vec<(String, &str, &str)> {
    // Nested browser @media rules depend on browser layout and are omitted.
    let mut depth = 0usize;
    let mut start = 0;
    let mut open = 0;
    let mut output = Vec::new();
    for (index, character) in css.char_indices() {
        match character {
            '{' => {
                if depth == 0 {
                    open = index;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let selector = css[start..open].trim();
                    let body = &css[open + 1..index];
                    if !selector.starts_with('@') && !body.contains(['{', '}']) {
                        for selector in selector.split(',') {
                            let normal = selector.split_whitespace().collect::<Vec<_>>().join(" ");
                            let normal = normal
                                .replace(" > ", ">")
                                .replace("> ", ">")
                                .replace(" >", ">");
                            for (property, value) in
                                body.split(';').filter_map(|line| line.split_once(':'))
                            {
                                output.push((normal.clone(), property.trim(), value.trim()));
                            }
                        }
                    }
                    start = index + 1;
                }
            }
            _ => {}
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_layout_rules_in_legacy_custom_themes() {
        let mut theme = Preferences::read(Some("tron"), None).unwrap();
        theme.theme["injectCSS"] = "section#filesystem { left: 55vw; } section#keyboard {right:44vw} @media (max-width:100px) { section#keyboard {display:none;} }".into();
        assert_eq!(Layout::for_theme(&theme).dock, Dock::Reversed);
        theme.theme["injectCSS"] = "section#keyboard { display: none !important; }".into();
        assert_eq!(Layout::for_theme(&theme).dock, Dock::Files);
    }

    #[test]
    fn translates_legacy_colors_and_rejects_external_resources() {
        let mut theme = Preferences::read(Some("tron"), None).unwrap();
        theme.theme["injectCSS"] = "body {background: #010203;} #mod_globe {opacity:0.7;} #mod_clock {font-size:42px;color:rgb(var(--color_r),0,255);} #mod_conninfo {background:url(https://example.com);}".into();
        let css = theme_css(&theme);
        assert!(
            css.contains("window { background-color: rgb(1,2,3); }"),
            "{css}"
        );
        assert!(css.contains(".world-module { opacity: 0.7; }"), "{css}");
        assert!(css.contains(".clock { font-size: 42px; }"), "{css}");
        assert!(css.contains(".clock { color: rgb(170,0,255); }"), "{css}");
        assert!(!css.contains("https://"), "{css}");
    }
}
