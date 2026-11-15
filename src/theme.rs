use include_dir::{Dir, include_dir};
use serde_json::Value;
use std::{fs, path::PathBuf};
use vte4::gtk::{gdk, pango};
use vte4::{CursorShape, Terminal, prelude::*};

static THEMES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/themes");
static KEYBOARDS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/keyboards");

#[derive(Clone)]
pub struct Preferences {
    pub theme: Value,
    pub theme_name: String,
    pub values: Value,
    pub keys: Value,
    pub keyboard_name: String,
    pub shell: Option<String>,
    pub shell_args: Vec<String>,
    pub font_size: f64,
    pub hide_dotfiles: bool,
    pub globe_enabled: bool,
}

impl Preferences {
    pub fn read(theme: Option<&str>, keyboard: Option<&str>) -> Result<Self, String> {
        let config = config_path();
        let settings = read_values(&config).unwrap_or_default();
        let load = |directory, name, fallback, resource, explicit| {
            load_json(&config, directory, name, resource)
                .map(|data| (name.to_owned(), data))
                .or_else(|error| {
                    if explicit {
                        Err(error)
                    } else {
                        load_json(&config, directory, fallback, resource)
                            .map(|data| (fallback.to_owned(), data))
                    }
                })
        };
        let (theme_name, selected_theme) = load(
            "themes",
            theme.or(settings["theme"].as_str()).unwrap_or("tron"),
            "tron",
            &THEMES,
            theme.is_some(),
        )?;
        let (keyboard_name, keys) = load(
            "keyboards",
            keyboard
                .or(settings["keyboard"].as_str())
                .unwrap_or("en-US"),
            "en-US",
            &KEYBOARDS,
            keyboard.is_some(),
        )?;
        Ok(Self {
            theme: selected_theme,
            theme_name,
            keys,
            keyboard_name,
            shell: settings["shell"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            shell_args: settings["shellArgs"]
                .as_str()
                .and_then(|args| vte4::gtk::glib::shell_parse_argv(args).ok())
                .unwrap_or_default()
                .into_iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            font_size: settings["termFontSize"]
                .as_f64()
                .filter(|size| (7.0..=40.0).contains(size))
                .unwrap_or(15.0),
            hide_dotfiles: settings["hideDotfiles"].as_bool().unwrap_or(false),
            globe_enabled: !settings["disableGlobe"].as_bool().unwrap_or(false),
            values: settings,
        })
    }

    pub fn with_theme(&self, selected: &str) -> Result<Self, String> {
        let mut next = self.clone();
        next.theme = resource(&config_path(), "themes", selected)?;
        next.theme_name = selected.to_owned();
        Ok(next)
    }

    pub fn accent(&self) -> (f64, f64, f64) {
        let get = |key: &str, fallback: u8| {
            self.theme["colors"][key]
                .as_u64()
                .and_then(|v| u8::try_from(v).ok())
                .unwrap_or(fallback)
        };
        (
            f64::from(get("r", 170)) / 255.0,
            f64::from(get("g", 207)) / 255.0,
            f64::from(get("b", 209)) / 255.0,
        )
    }

    pub fn globe_color(&self, name: &str) -> (f64, f64, f64) {
        self.theme["globe"][name]
            .as_str()
            .and_then(|color| gdk::RGBA::parse(color).ok())
            .map(|color| {
                (
                    color.red().into(),
                    color.green().into(),
                    color.blue().into(),
                )
            })
            .unwrap_or_else(|| self.accent())
    }

    pub fn background(&self) -> gdk::RGBA {
        let key = if self.theme_name == "tron" {
            "black"
        } else {
            "light_black"
        };
        parse(&self.theme["colors"][key], "#05080d")
    }

    pub fn stylesheet(&self) -> String {
        let (r, g, b) = self.accent();
        let bg = self.background();
        let font = native_font(
            self.theme["cssvars"]["font_main"]
                .as_str()
                .unwrap_or("monospace"),
        )
        .replace(['"', '\\', '\n', '\r'], "");
        let light = native_font(
            self.theme["cssvars"]["font_main_light"]
                .as_str()
                .unwrap_or("monospace"),
        )
        .replace(['"', '\\', '\n', '\r'], "");
        let overrides = crate::skin::theme_css(self);
        format!(
            "@define-color accent rgb({:.0},{:.0},{:.0});\n@define-color shade {};\nwindow {{ font-family: \"{}\", monospace; }}\n.light {{ font-family: \"{}\", monospace; }}\n{}\n{}",
            r * 255.0,
            g * 255.0,
            b * 255.0,
            bg,
            font,
            light,
            include_str!("style.css"),
            overrides
        )
    }

    pub fn apply_terminal(&self, terminal: &Terminal) {
        let t = &self.theme["terminal"];
        let foreground = parse(&t["foreground"], "#e8edf2");
        let background = parse(&t["background"], "#000000");
        let selection = parse(&t["selection"], "rgba(255,255,255,0.12)");
        terminal.set_color_highlight(Some(&selection));
        terminal.set_color_cursor_foreground(Some(&parse(&t["cursorAccent"], "#000000")));
        let palette = [
            ("black", "#000000"),
            ("red", "#cc0000"),
            ("green", "#4e9a06"),
            ("yellow", "#c4a000"),
            ("blue", "#3465a4"),
            ("magenta", "#75507b"),
            ("cyan", "#06989a"),
            ("white", "#d3d7cf"),
            ("brightBlack", "#555753"),
            ("brightRed", "#ef2929"),
            ("brightGreen", "#8ae234"),
            ("brightYellow", "#fce94f"),
            ("brightBlue", "#729fcf"),
            ("brightMagenta", "#ad7fa8"),
            ("brightCyan", "#34e2e2"),
            ("brightWhite", "#eeeeec"),
        ]
        .map(|(key, fallback)| parse(t.get(key).unwrap_or(&self.theme["colors"][key]), fallback));
        terminal.set_colors(
            Some(&foreground),
            Some(&background),
            &palette.iter().collect::<Vec<_>>(),
        );
        terminal.set_color_cursor(Some(&parse(&t["cursor"], "#e8edf2")));
        terminal.set_cursor_shape(match t["cursorStyle"].as_str() {
            Some("bar") => CursorShape::Ibeam,
            Some("underline") => CursorShape::Underline,
            _ => CursorShape::Block,
        });
        let ligatures = self.theme_name.ends_with("-ligatures")
            || self.values["termLigatures"].as_bool().unwrap_or(false);
        terminal.set_enable_shaping(ligatures);
        let font = if ligatures {
            "Fira Code"
        } else {
            native_font(t["fontFamily"].as_str().unwrap_or("Fira Mono"))
        };
        let size = t["fontSize"].as_f64().unwrap_or(self.font_size) * 0.75;
        terminal.set_font(Some(&pango::FontDescription::from_string(&format!(
            "{font} {size}"
        ))));
    }
}

fn native_font(name: &str) -> &str {
    // The browser loaded these font files under custom @font-face names.
    match name {
        "United Sans Medium" => "United Sans Rg Md",
        "United Sans Light" => "United Sans Rg Lt",
        "Fira Mono" => "FuraMono NF",
        other => other,
    }
}

fn parse(value: &Value, fallback: &str) -> gdk::RGBA {
    value
        .as_str()
        .and_then(|color| gdk::RGBA::parse(color).ok())
        .unwrap_or_else(|| gdk::RGBA::parse(fallback).unwrap())
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| vte4::gtk::glib::home_dir().join(".config"));
    ["EdexUi-2026", "eDEX-UI"]
        .into_iter()
        .map(|name| base.join(name))
        .find(|path| path.join("settings.json").is_file())
        .unwrap_or_else(|| base.join("EdexUi-2026"))
}

pub fn read_values(config: &std::path::Path) -> Result<Value, String> {
    match fs::read(config.join("settings.json")) {
        Ok(data) => serde_json::from_slice::<Value>(&data)
            .map_err(|error| error.to_string())
            .and_then(|value| {
                value
                    .is_object()
                    .then_some(value)
                    .ok_or_else(|| "Settings must be a JSON object".into())
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(error) => Err(error.to_string()),
    }
}

pub fn resource_names(directory: &str) -> Vec<String> {
    let assets = if directory == "themes" {
        &THEMES
    } else {
        &KEYBOARDS
    };
    let files = assets.files().map(|file| file.path().to_path_buf());
    let custom = fs::read_dir(config_path().join(directory))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path());
    let mut names: Vec<_> = files
        .chain(custom)
        .filter_map(|path| {
            (path.extension()?.to_str()? == "json")
                .then(|| path.file_stem()?.to_str().map(str::to_owned))
                .flatten()
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn resource(config: &std::path::Path, directory: &str, name: &str) -> Result<Value, String> {
    let assets = if directory == "themes" {
        &THEMES
    } else {
        &KEYBOARDS
    };
    load_json(config, directory, name, assets)
}

fn load_json(
    config: &std::path::Path,
    dir: &str,
    name: &str,
    builtins: &Dir<'_>,
) -> Result<Value, String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(format!("Invalid {dir} name: {name}"));
    }
    let file = format!("{name}.json");
    let custom = config.join(dir).join(&file);
    let data = if custom.is_file() {
        fs::read(custom).map_err(|error| error.to_string())?
    } else {
        builtins
            .get_file(&file)
            .ok_or_else(|| format!("Unknown {dir} entry: {name}"))?
            .contents()
            .to_vec()
    };
    let value: Value =
        serde_json::from_slice(&data).map_err(|error| format!("Invalid {file}: {error}"))?;
    if !value.is_object() {
        return Err(format!("Invalid {file}: expected a JSON object"));
    }
    if dir == "keyboards"
        && !["row_numbers", "row_1", "row_2", "row_3", "row_space"]
            .iter()
            .all(|row| value[*row].as_array().is_some_and(|keys| !keys.is_empty()))
    {
        return Err(format!("Invalid {file}: expected five keyboard rows"));
    }
    Ok(value)
}

pub fn assets_path() -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join("assets")))
        .filter(|path| path.is_dir())
        .unwrap_or(source)
}

pub fn register_fonts() {
    use std::os::unix::ffi::OsStrExt;
    let directory = assets_path().join("fonts");
    for font in [
        "united_sans_medium.ttf",
        "united_sans_light.ttf",
        "fira_mono.ttf",
        "fira_code.ttf",
    ] {
        let path = directory.join(font);
        if path.is_file() {
            register_font_bytes(path.as_os_str().as_bytes());
        }
    }
}

pub fn register_font_bytes(path: &[u8]) {
    #[link(name = "fontconfig")]
    unsafe extern "C" {
        fn FcConfigAppFontAddFile(config: *mut libc::c_void, file: *const u8) -> libc::c_int;
    }
    if let Ok(path) = std::ffi::CString::new(path) {
        // A null config selects fontconfig's current application font configuration.
        unsafe {
            FcConfigAppFontAddFile(std::ptr::null_mut(), path.as_ptr().cast());
        }
    }
}
