use serde_json::Value;
use vte4::gtk::gdk::RGBA;

pub const FOLLOW_THEME: &str = "Follow theme";
pub const CUSTOM: &str = "Custom";
pub const PRIMARY_COLORS: [(&str, &str); 6] = [
    ("foreground", "Text"),
    ("background", "Background"),
    ("cursor", "Cursor"),
    ("cursorText", "Cursor text"),
    ("selection", "Selection"),
    ("selectionText", "Selected text"),
];
pub const ANSI_COLORS: [&str; 16] = [
    "Black",
    "Red",
    "Green",
    "Yellow",
    "Blue",
    "Magenta",
    "Cyan",
    "White",
    "Bright black",
    "Bright red",
    "Bright green",
    "Bright yellow",
    "Bright blue",
    "Bright magenta",
    "Bright cyan",
    "Bright white",
];
pub const TERMINAL_BACKGROUND: &str = "termBackground";
pub const STYLE_SETTINGS: [&str; 6] = [
    "termFontFamily",
    "termFontSize",
    "termLigatures",
    "termColorScheme",
    "termCustomColors",
    TERMINAL_BACKGROUND,
];

pub struct TerminalPalette {
    pub foreground: RGBA,
    pub background: RGBA,
    pub cursor: RGBA,
    pub cursor_text: RGBA,
    pub selection: RGBA,
    pub selection_text: Option<RGBA>,
    pub ansi: [RGBA; 16],
}

impl TerminalPalette {
    pub fn primary(&self) -> [RGBA; 6] {
        [
            self.foreground,
            self.background,
            self.cursor,
            self.cursor_text,
            self.selection,
            self.selection_text.unwrap_or(self.foreground),
        ]
    }

    pub fn custom(colors: &Value, fallback: &Self) -> Self {
        let primary = std::array::from_fn(|i| {
            color(&colors[PRIMARY_COLORS[i].0]).unwrap_or(fallback.primary()[i])
        });
        let ansi = std::array::from_fn(|i| color(&colors["ansi"][i]).unwrap_or(fallback.ansi[i]));
        Self::from_colors(primary, ansi)
    }

    fn from_colors(
        [
            foreground,
            background,
            cursor,
            cursor_text,
            selection,
            selected,
        ]: [RGBA; 6],
        ansi: [RGBA; 16],
    ) -> Self {
        Self {
            foreground,
            background,
            cursor,
            cursor_text,
            selection,
            selection_text: Some(selected),
            ansi,
        }
    }
}

pub fn color(value: &Value) -> Option<RGBA> {
    RGBA::parse(value.as_str()?).ok()
}

pub fn custom_is_valid(colors: &Value) -> bool {
    PRIMARY_COLORS
        .iter()
        .all(|(key, _)| color(&colors[*key]).is_some())
        && colors["ansi"].as_array().is_some_and(|ansi| {
            ansi.len() == ANSI_COLORS.len() && ansi.iter().all(|value| color(value).is_some())
        })
}

pub fn scheme_names() -> Vec<String> {
    std::iter::once(FOLLOW_THEME)
        .chain(PRESETS.iter().map(|preset| preset.name))
        .chain(std::iter::once(CUSTOM))
        .map(str::to_owned)
        .collect()
}

pub fn scheme(name: &str) -> Option<TerminalPalette> {
    let preset = PRESETS.iter().find(|preset| preset.name == name)?;
    let parse = |color| RGBA::parse(color).expect("valid terminal preset color");
    Some(TerminalPalette::from_colors(
        preset.primary.map(parse),
        preset.ansi.map(parse),
    ))
}

struct Preset {
    name: &'static str,
    primary: [&'static str; 6],
    ansi: [&'static str; 16],
}

// Catppuccin Alacritty ports and the Nord terminal palette.
const PRESETS: &[Preset] = &[
    Preset {
        name: "Catppuccin Latte",
        primary: [
            "#4c4f69", "#eff1f5", "#dc8a78", "#eff1f5", "#dc8a78", "#eff1f5",
        ],
        ansi: [
            "#5c5f77", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5", "#ea76cb", "#179299", "#acb0be",
            "#6c6f85", "#de293e", "#49af3d", "#eea02d", "#456eff", "#fe85d8", "#2d9fa8", "#bcc0cc",
        ],
    },
    Preset {
        name: "Catppuccin Frappé",
        primary: [
            "#c6d0f5", "#303446", "#f2d5cf", "#303446", "#f2d5cf", "#303446",
        ],
        ansi: [
            "#51576d", "#e78284", "#a6d189", "#e5c890", "#8caaee", "#f4b8e4", "#81c8be", "#a5adce",
            "#626880", "#e67172", "#8ec772", "#d9ba73", "#7b9ef0", "#f2a4db", "#5abfb5", "#b5bfe2",
        ],
    },
    Preset {
        name: "Catppuccin Macchiato",
        primary: [
            "#cad3f5", "#24273a", "#f4dbd6", "#24273a", "#f4dbd6", "#24273a",
        ],
        ansi: [
            "#494d64", "#ed8796", "#a6da95", "#eed49f", "#8aadf4", "#f5bde6", "#8bd5ca", "#a5adcb",
            "#5b6078", "#ec7486", "#8ccf7f", "#e1c682", "#78a1f6", "#f2a9dd", "#63cbc0", "#b8c0e0",
        ],
    },
    Preset {
        name: "Catppuccin Mocha",
        primary: [
            "#cdd6f4", "#1e1e2e", "#f5e0dc", "#1e1e2e", "#f5e0dc", "#1e1e2e",
        ],
        ansi: [
            "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8",
            "#585b70", "#f37799", "#89d88b", "#ebd391", "#74a8fc", "#f2aede", "#6bd7ca", "#bac2de",
        ],
    },
    Preset {
        name: "Nord",
        primary: [
            "#d8dee9", "#2e3440", "#d8dee9", "#2e3440", "#434c5e", "#eceff4",
        ],
        ansi: [
            "#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
            "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4",
        ],
    },
];
