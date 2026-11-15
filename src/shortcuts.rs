use crate::{terminal::Terminals, theme};
use serde_json::Value;
use vte4::gtk::{self, gio, prelude::*};

pub fn install(app: &gtk::Application, window: &gtk::ApplicationWindow, tabs: &Terminals) {
    let path = theme::config_path().join("shortcuts.json");
    let Ok(data) = std::fs::read(path) else {
        return;
    };
    let Ok(entries) = serde_json::from_slice::<Vec<Value>>(&data) else {
        return;
    };
    for (index, entry) in entries.iter().enumerate() {
        let enabled = entry["enabled"].as_bool().unwrap_or(false);
        let Some(trigger) = entry["trigger"].as_str() else {
            continue;
        };
        let Some(action) = entry["action"].as_str() else {
            continue;
        };
        match entry["type"].as_str() {
            Some("app") if action == "TAB_X" => {
                for tab in 0..crate::terminal::DEFAULT_TAB_COUNT {
                    let trigger = trigger.replace('X', &(tab + 1).to_string());
                    if let Some(accel) = accelerator(&trigger) {
                        let selected = [accel.as_str()];
                        app.set_accels_for_action(
                            &format!("win.tab{tab}"),
                            if enabled { &selected } else { &[] },
                        );
                    }
                }
            }
            Some("app") => {
                let name = match action {
                    "COPY" => "copy",
                    "PASTE" => "paste",
                    "NEXT_TAB" => "next-tab",
                    "PREVIOUS_TAB" => "previous-tab",
                    "TAB_1" => "tab0",
                    "TAB_2" => "tab1",
                    "TAB_3" => "tab2",
                    "TAB_4" => "tab3",
                    "TAB_5" => "tab4",
                    "SETTINGS" => "settings",
                    "SHORTCUTS" => "shortcuts",
                    "APP_MANAGER" => "apps",
                    "FUZZY_SEARCH" => "files",
                    "FS_LIST_VIEW" => "file-list",
                    "FS_DOTFILES" => "dotfiles",
                    "KB_PASSMODE" => "password-mode",
                    "DEV_DEBUG" => "native-inspector",
                    _ => continue,
                };
                if let Some(accel) = accelerator(trigger) {
                    let selected = [accel.as_str()];
                    app.set_accels_for_action(
                        &format!("win.{name}"),
                        if enabled { &selected } else { &[] },
                    );
                }
            }
            Some("shell") if enabled => {
                if let Some(accel) = accelerator(trigger) {
                    let mut command = action.as_bytes().to_vec();
                    if entry["linebreak"].as_bool() == Some(true) {
                        command.push(b'\r');
                    }
                    let connection = tabs.clone();
                    let name = format!("shell-shortcut-{index}");
                    let action = gio::SimpleAction::new(&name, None);
                    action.connect_activate(move |_, _| connection.write(&command));
                    window.add_action(&action);
                    app.set_accels_for_action(&format!("win.{name}"), &[&accel]);
                }
            }
            _ => {}
        }
    }
}

pub fn invoke(window: &gtk::ApplicationWindow, key: &str, held: [&str; 3]) -> bool {
    let Some(app) = window.application() else {
        return false;
    };
    let parts = held
        .into_iter()
        .filter(|part| !part.is_empty())
        .chain(std::iter::once(key))
        .collect::<Vec<_>>();
    let Some(accel) = accelerator(&parts.join("+")) else {
        return false;
    };
    for action in app.actions_for_accel(&accel) {
        if gtk::prelude::WidgetExt::activate_action(window, &action, None).is_ok() {
            return true;
        }
    }
    false
}

fn accelerator(combination: &str) -> Option<String> {
    let result = format_accelerator(combination)?;
    let (key, modifiers) = gtk::accelerator_parse(&result)?;
    (gtk::accelerator_valid(key, modifiers) || key == gtk::gdk::Key::Tab).then_some(result)
}

fn format_accelerator(combination: &str) -> Option<String> {
    let mut parts = combination.split('+').peekable();
    let mut result = String::new();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            match part {
                "Space" => result.push_str("space"),
                "Plus" => result.push_str("plus"),
                "Esc" => result.push_str("Escape"),
                key if key.len() == 1 => result.push_str(&key.to_ascii_lowercase()),
                key => result.push_str(key),
            }
            break;
        }
        result.push_str(match part.to_ascii_lowercase().as_str() {
            "cmdorctrl" | "commandorcontrol" | "ctrl" | "control" => "<Control>",
            "shift" => "<Shift>",
            "alt" | "option" => "<Alt>",
            "super" | "cmd" | "command" | "meta" => "<Super>",
            _ => return None,
        });
    }
    (!result.is_empty()).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_electron_shortcuts_to_gtk() {
        assert_eq!(
            format_accelerator("Ctrl+Shift+K").as_deref(),
            Some("<Control><Shift>k")
        );
        assert_eq!(
            format_accelerator("Ctrl+Shift+Alt+Space").as_deref(),
            Some("<Control><Shift><Alt>space")
        );
        assert_eq!(format_accelerator("Invalid+K"), None);
    }
}
