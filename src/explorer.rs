use crate::terminal::Terminals;
use std::{
    cell::{Cell, RefCell},
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    rc::Rc,
};
use vte4::gtk::{self, gio, glib, pango, prelude::*};

#[derive(Clone)]
pub struct Files {
    pub view: gtk::Box,
    state: Rc<State>,
}

struct State {
    cwd: RefCell<PathBuf>,
    hidden: Cell<bool>,
    list_mode: Cell<bool>,
    system_view: Cell<bool>,
    query: RefCell<String>,
    shell_linked: bool,
    colors: RefCell<(String, String)>,
    path: gtk::Label,
    mode: gtk::Button,
    tools: gtk::Box,
    search: gtk::SearchEntry,
    list: gtk::FlowBox,
    status: gtk::Label,
    tabs: Terminals,
}

impl Files {
    pub fn new(tabs: &Terminals, prefs: &crate::theme::Preferences, shell_linked: bool) -> Self {
        let view = gtk::Box::new(gtk::Orientation::Vertical, 5);
        view.add_css_class("filesystem-panel");
        view.set_hexpand(true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        let title = gtk::Label::new(Some(if shell_linked {
            "SYSTEM"
        } else {
            "SYSTEM FILES"
        }));
        title.add_css_class("section-title");
        let mode = gtk::Button::with_label("Files");
        mode.set_tooltip_text(Some(
            "Show the active terminal directory or SYSTEM shortcuts",
        ));
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let up = gtk::Button::with_label("↑");
        up.set_tooltip_text(Some("Parent directory"));
        let refresh = gtk::Button::with_label("↻");
        refresh.set_tooltip_text(Some("Refresh directory"));
        let hidden = gtk::ToggleButton::with_label("Dotfiles");
        hidden.set_active(!prefs.hide_dotfiles);
        let path = gtk::Label::new(None);
        path.set_xalign(0.0);
        path.set_hexpand(true);
        path.set_ellipsize(pango::EllipsizeMode::Start);
        path.set_max_width_chars(24);
        header.append(&title);
        if shell_linked {
            header.append(&mode);
        }
        tools.append(&up);
        tools.append(&refresh);
        tools.append(&hidden);
        header.append(&tools);
        header.append(&path);
        view.append(&header);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search this directory"));
        view.append(&search);
        let list = gtk::FlowBox::builder()
            .valign(gtk::Align::Start)
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(14)
            .min_children_per_line(1)
            .row_spacing(5)
            .column_spacing(8)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&list)
            .build();
        view.append(&scroll);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_ellipsize(pango::EllipsizeMode::End);
        status.add_css_class("subtle");
        view.append(&status);
        let state = Rc::new(State {
            cwd: RefCell::new(
                tabs.cwd()
                    .or_else(|| {
                        prefs.values["cwd"]
                            .as_str()
                            .filter(|path| !path.is_empty())
                            .map(PathBuf::from)
                            .filter(|path| path.is_dir())
                    })
                    .unwrap_or_else(glib::home_dir),
            ),
            hidden: Cell::new(prefs.hide_dotfiles),
            list_mode: Cell::new(false),
            system_view: Cell::new(shell_linked),
            query: RefCell::new(String::new()),
            shell_linked,
            colors: RefCell::new(theme_colors(prefs)),
            path,
            mode: mode.clone(),
            tools,
            search: search.clone(),
            list,
            status,
            tabs: tabs.clone(),
        });
        let weak = Rc::downgrade(&state);
        up.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let parent = state.cwd.borrow().parent().map(Path::to_path_buf);
                if let Some(parent) = parent {
                    change_directory(&state, parent);
                }
            }
        });
        let weak = Rc::downgrade(&state);
        refresh.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                render(&state);
            }
        });
        let weak = Rc::downgrade(&state);
        hidden.connect_toggled(move |button| {
            if let Some(state) = weak.upgrade() {
                state.hidden.set(!button.is_active());
                render(&state);
            }
        });
        let weak = Rc::downgrade(&state);
        search.connect_search_changed(move |search| {
            if let Some(state) = weak.upgrade() {
                state.query.replace(search.text().to_string());
                render(&state);
            }
        });
        if shell_linked {
            let weak = Rc::downgrade(&state);
            mode.connect_clicked(move |_| {
                if let Some(state) = weak.upgrade() {
                    set_mode(&state, !state.system_view.get());
                }
            });
        }
        render(&state);
        Self { view, state }
    }

    pub fn apply_theme(&self, prefs: &crate::theme::Preferences) {
        self.state.colors.replace(theme_colors(prefs));
        if self.state.system_view.get() {
            for index in 0..SYSTEM_ICONS.len() {
                if let Some(button) = self
                    .state
                    .list
                    .child_at_index(index as i32)
                    .and_then(|row| row.child())
                    .and_downcast::<gtk::Button>()
                {
                    paint_system_icon(&button, SYSTEM_ICONS[index], &self.state.colors.borrow());
                }
            }
        }
    }

    pub fn follow(&self, cwd: &Path) {
        if self.state.shell_linked && *self.state.cwd.borrow() != cwd {
            self.state.cwd.replace(cwd.to_owned());
            if self.state.system_view.get() {
                self.state.path.set_text(&cwd.to_string_lossy());
                self.state
                    .path
                    .set_tooltip_text(Some(&cwd.to_string_lossy()));
            } else {
                render(&self.state);
            }
        }
    }
    pub fn toggle_list(&self) {
        self.state.list_mode.set(!self.state.list_mode.get());
        render(&self.state);
    }

    pub fn toggle_dotfiles(&self) {
        self.state.hidden.set(!self.state.hidden.get());
        render(&self.state);
    }

    pub fn open_mounts(&self, parent: &gtk::ApplicationWindow) {
        let list = gtk::ListBox::new();
        list.add_css_class("installed-apps");
        list.set_selection_mode(gtk::SelectionMode::None);
        for mount in crate::metrics::mounts() {
            let button = gtk::Button::with_label(&format!(
                "{}   {}   {}",
                mount.path.display(),
                mount.source,
                mount.kind
            ));
            let weak = Rc::downgrade(&self.state);
            button.connect_clicked(move |button| {
                if let Some(state) = weak.upgrade() {
                    change_directory(&state, mount.path.clone());
                }
                crate::settings::close_from(button);
            });
            list.append(&button);
        }
        let scroll = gtk::ScrolledWindow::builder().child(&list).build();
        crate::settings::dialog(parent, "Mounted Filesystems", 690, 450, &scroll).present();
    }

    pub fn sync_to_terminal(&self) {
        if let Some(cwd) = self.state.tabs.cwd() {
            if *self.state.cwd.borrow() != cwd {
                self.state.cwd.replace(cwd);
                render(&self.state);
            }
        }
    }

    pub fn focus_search(&self) {
        self.state.search.grab_focus();
    }
}

fn theme_colors(prefs: &crate::theme::Preferences) -> (String, String) {
    let (r, g, b) = prefs.accent();
    (
        format!("rgb({:.0},{:.0},{:.0})", r * 255.0, g * 255.0, b * 255.0),
        prefs.background().to_string(),
    )
}

fn set_mode(state: &Rc<State>, shortcuts: bool) {
    state.system_view.set(shortcuts);
    if !shortcuts {
        if let Some(cwd) = state.tabs.cwd() {
            state.cwd.replace(cwd);
        }
    }
    render(state);
}

fn change_directory(state: &Rc<State>, path: PathBuf) {
    if !path.is_dir() {
        return;
    }
    if !state.shell_linked {
        state.cwd.replace(path);
        state.query.replace(String::new());
        render(state);
        return;
    }
    let bytes = path.as_os_str().as_bytes();
    if bytes.iter().any(u8::is_ascii_control) {
        state
            .status
            .set_text("This directory name contains terminal control bytes.");
        return;
    }
    // Ctrl+E and Ctrl+U clear the prompt before submitting a separate command.
    let mut command = b"\x05\x15cd '".to_vec();
    for &byte in bytes {
        if matches!(byte, b'\\' | b'\'') {
            // Escape outside single quotes, where fish and POSIX shells agree.
            command.extend_from_slice(&[b'\'', b'\\', byte, b'\'']);
        } else {
            command.push(byte);
        }
    }
    command.extend_from_slice(b"'\r");
    if !state.tabs.write_to_idle_shell(&command) {
        state
            .status
            .set_text("The shell is busy. Return to its prompt to change directories.");
        return;
    }
    state.system_view.set(false);
    state.cwd.replace(path);
    render(state);
}

fn render(state: &Rc<State>) {
    let path = state.cwd.borrow().clone();
    let shortcuts = state.system_view.get();
    state
        .mode
        .set_label(if shortcuts { "Files" } else { "Shortcuts" });
    state.tools.set_visible(!shortcuts);
    state.search.set_visible(!shortcuts);
    state.status.set_visible(!shortcuts);
    state.path.set_text(&path.to_string_lossy());
    state.path.set_tooltip_text(Some(&path.to_string_lossy()));
    state
        .list
        .set_max_children_per_line(if state.list_mode.get() { 1 } else { 14 });
    while let Some(child) = state.list.first_child() {
        state.list.remove(&child);
    }
    if state.system_view.get() {
        render_system(state);
        return;
    }
    let entries = match fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(error) => {
            state.status.set_text(&error.to_string());
            return;
        }
    };
    let mut entries = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            (!state.hidden.get() || !name.starts_with('.'))
                && matches_name(&name, &state.query.borrow())
        })
        .map(|entry| (entry.path().is_dir(), entry.file_name(), entry.path()))
        .collect::<Vec<_>>();
    entries.sort_unstable_by(|(a_dir, a_name, _), (b_dir, b_name, _)| {
        b_dir.cmp(a_dir).then_with(|| a_name.cmp(b_name))
    });
    state.status.set_text(&format!("{} items", entries.len()));
    for (is_dir, name, path) in entries {
        let button = tile(
            &name.to_string_lossy(),
            if is_dir {
                "folder-symbolic"
            } else {
                "text-x-generic-symbolic"
            },
            state.list_mode.get(),
        );
        button.set_tooltip_text(Some(&name.to_string_lossy()));
        let weak = Rc::downgrade(state);
        button.connect_clicked(move |button| {
            if let Some(state) = weak.upgrade() {
                if is_dir {
                    change_directory(&state, path.clone());
                } else if let Err(error) = preview(button, &path) {
                    state.status.set_text(&error.to_string());
                }
            }
        });
        state.list.append(&button);
    }
}

fn matches_name(name: &str, query: &str) -> bool {
    let name = name.to_lowercase();
    let mut name = name.chars();
    query
        .to_lowercase()
        .chars()
        .all(|key| name.any(|letter| letter == key))
}

fn tile(name: &str, icon_name: &str, list: bool) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("file-tile");
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(if list { 20 } else { 47 });
    let label = gtk::Label::new(Some(name));
    label.set_max_width_chars(if list { 56 } else { 14 });
    label.set_ellipsize(pango::EllipsizeMode::End);
    let orientation = if list {
        gtk::Orientation::Horizontal
    } else {
        gtk::Orientation::Vertical
    };
    let contents = gtk::Box::new(orientation, 4);
    contents.append(&icon);
    contents.append(&label);
    button.set_child(Some(&contents));
    button
}

const SYSTEM_ICONS: [&str; 3] = [
    include_str!("../assets/system-icons/settings.svg"),
    include_str!("../assets/system-icons/themesDir.svg"),
    include_str!("../assets/system-icons/kblayoutsDir.svg"),
];

fn paint_system_icon(button: &gtk::Button, source: &str, (accent, background): &(String, String)) {
    let Ok(loader) = gtk::gdk_pixbuf::PixbufLoader::with_type("svg") else {
        return;
    };
    loader.set_size(51, 51);
    let svg = source
        .replace("{{ACCENT}}", accent)
        .replace("{{BACKGROUND}}", background);
    if loader
        .write(svg.as_bytes())
        .and_then(|_| loader.close())
        .is_err()
    {
        return;
    }
    if let (Some(bitmap), Some(image)) = (
        loader.pixbuf(),
        button
            .child()
            .and_then(|content| content.first_child())
            .and_downcast::<gtk::Image>(),
    ) {
        image.set_paintable(Some(&gtk::gdk::Texture::for_pixbuf(&bitmap)));
    }
}

fn render_system(state: &Rc<State>) {
    let config = crate::theme::config_path();
    for (index, (name, icon)) in [
        ("settings.json", "emblem-system-symbolic"),
        ("themes", "folder-pictures-symbolic"),
        ("keyboards", "input-keyboard-symbolic"),
        ("Project Repo", "insert-link-symbolic"),
        ("File Manager", "folder-symbolic"),
        ("Installed Apps", "view-app-grid-symbolic"),
    ]
    .into_iter()
    .enumerate()
    {
        let button = tile(name, icon, false);
        if let Some(source) = SYSTEM_ICONS.get(index) {
            paint_system_icon(&button, source, &state.colors.borrow());
        }
        button.add_css_class("system-tile");
        let config = config.clone();
        let file_state = Rc::downgrade(state);
        button.connect_clicked(move |button| {
            let parent = button.root().and_downcast::<gtk::ApplicationWindow>();
            match (index, parent) {
                (0, Some(parent)) => {
                    gio::prelude::ActionGroupExt::activate_action(&parent, "settings", None);
                }
                (1 | 2, Some(parent)) => crate::settings::resources(&parent, name, &config),
                (3, _) => {
                    let _ = gio::AppInfo::launch_default_for_uri(
                        "https://github.com/ACOPS1206/eDEX-Remake",
                        None::<&gio::AppLaunchContext>,
                    );
                }
                (4, _) => {
                    if let Some(state) = file_state.upgrade() {
                        set_mode(&state, false);
                    }
                }
                (5, Some(parent)) => {
                    gio::prelude::ActionGroupExt::activate_action(&parent, "apps", None)
                }
                _ => {}
            }
        });
        state.list.append(&button);
    }
}

fn preview(button: &gtk::Button, path: &Path) -> Result<(), String> {
    let uri = gio::File::for_path(path).uri();
    let Some(parent) = button.root().and_downcast::<gtk::ApplicationWindow>() else {
        return Err("The file panel has no window".into());
    };
    let info = gio::File::for_path(path).query_info("standard::type,standard::size,standard::content-type,time::modified,time::modified-usec",
        gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>).map_err(|error| error.to_string())?;
    let kind = info.content_type().unwrap_or_default();
    let title = path.file_name().unwrap_or_default().to_string_lossy();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.add_css_class("dialog-content");
    let external = gtk::Button::with_label("Open in default application");
    let notice = gtk::Label::new(None);
    let external_notice = notice.clone();
    external.connect_clicked(move |_| {
        if let Err(error) =
            gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        {
            external_notice.set_text(&error.to_string());
        }
    });
    if info.file_type() == gio::FileType::Regular && kind == "application/pdf" {
        content.append(&crate::documents::pdf(path));
    } else if info.file_type() == gio::FileType::Regular
        && matches!(
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("ttf" | "otf" | "woff" | "woff2")
        )
    {
        if let Some(font) = crate::documents::font(path) {
            content.append(&font);
        }
    } else if info.file_type() == gio::FileType::Regular && kind.starts_with("image/") {
        let image = gtk::Picture::for_filename(path);
        image.set_vexpand(true);
        content.append(&image);
    } else if info.file_type() == gio::FileType::Regular
        && (kind.starts_with("audio/") || kind.starts_with("video/"))
    {
        let media = gtk::Video::for_filename(Some(path));
        media.set_vexpand(true);
        content.append(&media);
    } else if info.file_type() == gio::FileType::Regular && (0..=2_000_000).contains(&info.size()) {
        if let Ok(text) = fs::read_to_string(path) {
            if !text.contains('\0') {
                let editor = gtk::TextView::builder().monospace(true).build();
                editor.buffer().set_text(&text);
                let scroll = gtk::ScrolledWindow::builder()
                    .vexpand(true)
                    .child(&editor)
                    .build();
                content.append(&scroll);
                let save = gtk::Button::with_label("Save");
                let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
                let notice = notice.clone();
                let modified = info.modification_date_time();
                let baseline = Rc::new(RefCell::new(modified));
                save.connect_clicked(move |_| {
                    let current = gio::File::for_path(&path).query_info(
                        "time::modified,time::modified-usec",
                        gio::FileQueryInfoFlags::NONE,
                        None::<&gio::Cancellable>,
                    );
                    if let Ok(current) = current {
                        let observed = current.modification_date_time();
                        if observed != *baseline.borrow() {
                            notice.set_text("The file changed on disk. Reopen it before saving.");
                            return;
                        }
                    }
                    let buffer = editor.buffer();
                    let data = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
                    match glib::file_set_contents(&path, data.as_bytes()) {
                        Ok(()) => {
                            if let Ok(info) = gio::File::for_path(&path).query_info(
                                "time::modified,time::modified-usec",
                                gio::FileQueryInfoFlags::NONE,
                                None::<&gio::Cancellable>,
                            ) {
                                baseline.replace(info.modification_date_time());
                            }
                            notice.set_text("Saved.");
                        }
                        Err(error) => notice.set_text(&error.to_string()),
                    }
                });
                content.append(&save);
            }
        }
    }
    content.append(&external);
    content.append(&notice);
    crate::settings::dialog(&parent, &title, 850, 600, &content).present();
    Ok(())
}

pub fn app_manager() -> gtk::Box {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 7);
    panel.add_css_class("filesystem-panel");
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search installed applications"));
    panel.append(&search);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    list.add_css_class("installed-apps");
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    panel.append(&scroll);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_ellipsize(pango::EllipsizeMode::End);
    panel.append(&status);

    let mut apps: Vec<_> = gio::AppInfo::all()
        .into_iter()
        .filter(|app| app.should_show())
        .collect();
    apps.sort_by_cached_key(|app| app.display_name().to_lowercase());
    let result = Rc::new(RefCell::new(Vec::<gio::AppInfo>::new()));
    let list_ref = list.clone();
    let status_ref = status.clone();
    let matched = result.clone();
    let refresh = move |query: &str| {
        while let Some(row) = list_ref.first_child() {
            list_ref.remove(&row);
        }
        let filtered: Vec<_> = apps
            .iter()
            .filter(|app| {
                matches_name(&app.display_name(), query)
                    || app
                        .description()
                        .is_some_and(|desc| desc.to_lowercase().contains(&query.to_lowercase()))
            })
            .cloned()
            .collect();
        for app in &filtered {
            let row = gtk::ListBoxRow::new();
            let contents = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let image = app.icon().map_or_else(
                || gtk::Image::from_icon_name("application-x-executable-symbolic"),
                |icon| gtk::Image::from_gicon(&icon),
            );
            image.set_pixel_size(28);
            let group = gtk::Box::new(gtk::Orientation::Vertical, 1);
            let name = gtk::Label::builder()
                .label(app.display_name())
                .xalign(0.0)
                .build();
            group.append(&name);
            if let Some(description) = app.description() {
                let description = gtk::Label::builder()
                    .label(description)
                    .xalign(0.0)
                    .ellipsize(pango::EllipsizeMode::End)
                    .build();
                description.add_css_class("subtle");
                group.append(&description);
            }
            contents.append(&image);
            contents.append(&group);
            row.set_child(Some(&contents));
            list_ref.append(&row);
        }
        status_ref.set_text(&format!("{} applications", filtered.len()));
        matched.replace(filtered);
        list_ref.select_row(list_ref.row_at_index(0).as_ref());
    };
    refresh("");
    search.connect_search_changed(move |entry| refresh(entry.text().as_str()));
    let list_ref = list.clone();
    search.connect_activate(move |_| {
        if let Some(row) = list_ref.selected_row() {
            row.emit_activate();
        }
    });
    list.connect_row_activated(move |_, row| {
        if let Some(app) = result.borrow().get(row.index() as usize) {
            if let Err(error) = app.launch(&[], None::<&gio::AppLaunchContext>) {
                status.set_text(&error.to_string());
            }
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        let offset = match key {
            gtk::gdk::Key::Up => -1,
            gtk::gdk::Key::Down => 1,
            _ => return glib::Propagation::Proceed,
        };
        let index = list.selected_row().map(|row| row.index()).unwrap_or(0);
        if let Some(row) = list.row_at_index(index + offset) {
            list.select_row(Some(&row));
        }
        glib::Propagation::Stop
    });
    search.add_controller(keys);
    panel
}
