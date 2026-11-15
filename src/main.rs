mod audio;
mod chart;
mod documents;
mod explorer;
mod geo;
mod globe;
mod keyboard;
mod metrics;
mod processes;
mod settings;
mod shortcuts;
mod skin;
mod startup;
mod terminal;
mod theme;
mod traffic;

use chart::{Chart, MemoryMap};
use explorer::Files;
use metrics::{PROCESS_LIMIT, Sampler, Stats, percent, quantity};
use std::{
    env, thread,
    time::{Duration, Instant},
};
use terminal::Terminals;
use theme::Preferences;
use vte4::gtk::{self, gio, glib, pango, prelude::*};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Options {
    windowed: bool,
    intro: Option<bool>,
    interface: Option<String>,
    theme: Option<String>,
    keyboard: Option<String>,
}

fn main() -> glib::ExitCode {
    let mut options = Options::default();
    let mut sample_only = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--windowed" => options.windowed = true,
            "--nointro" => options.intro = Some(false),
            "--intro" => options.intro = Some(true),
            "--interface" => {
                let Some(name) = args
                    .next()
                    .filter(|name| !name.starts_with('-') && !name.is_empty())
                else {
                    eprintln!("--interface requires an interface name");
                    return glib::ExitCode::FAILURE;
                };
                options.interface = Some(name);
            }
            "--theme" | "--keyboard" => {
                let Some(name) = args.next().filter(|name| !name.starts_with('-')) else {
                    eprintln!("{arg} requires a name");
                    return glib::ExitCode::FAILURE;
                };
                if arg == "--theme" {
                    options.theme = Some(name);
                } else {
                    options.keyboard = Some(name);
                }
            }
            "--sample" => sample_only = true,
            "--help" | "-h" => {
                println!(
                    "Usage: edex-native [--windowed] [--interface NAME] [--theme NAME] [--keyboard NAME] [--intro|--nointro] [--sample]\n\nF10  Toggle side panels\nF11  Toggle fullscreen\nCtrl+Shift+Q  Quit"
                );
                return glib::ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("edex-native {}", env!("CARGO_PKG_VERSION"));
                return glib::ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("Unknown option: {arg}. Use --help.");
                return glib::ExitCode::FAILURE;
            }
        }
    }
    if sample_only {
        let mut sampler = Sampler::new(options.interface, true);
        thread::sleep(SAMPLE_INTERVAL);
        let sample = sampler.sample();
        println!(
            "CPU {:.1}% | RAM {} / {} | {} processes in table",
            sample.cpu,
            quantity(sample.memory as f64),
            quantity(sample.memory_total as f64),
            sample.processes.unwrap_or_default().len()
        );
        if let Some(network) = sample.network {
            println!(
                "{} | RX {}/s | TX {}/s",
                network.name,
                quantity(network.rx),
                quantity(network.tx)
            );
        }
        return glib::ExitCode::SUCCESS;
    }

    let prefs = match Preferences::read(options.theme.as_deref(), options.keyboard.as_deref()) {
        Ok(prefs) => prefs,
        Err(error) => {
            eprintln!("{error}");
            return glib::ExitCode::FAILURE;
        }
    };
    if env::var_os("GSK_RENDERER").is_none() {
        let renderer = prefs.values["nativeRenderer"]
            .as_str()
            .filter(|name| matches!(*name, "cairo" | "gl" | "vulkan"))
            .or_else(|| {
                match (
                    prefs.values["disableVulkan"].as_bool(),
                    prefs.values["enableVulkan"].as_bool(),
                ) {
                    (Some(true), _) => Some("gl"),
                    (_, Some(true)) => Some("vulkan"),
                    _ => None,
                }
            });
        if let Some(renderer) = renderer {
            // The application is still on its initial thread and GTK has not started.
            unsafe {
                env::set_var("GSK_RENDERER", renderer);
            }
        }
    }
    theme::register_fonts();
    let app = gtk::Application::builder()
        .application_id("io.edex.Remake")
        .build();
    app.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
            window.present();
        } else {
            build_window(app, &options, &prefs);
        }
    });
    app.run_with_args(&["edex-native"])
}

struct ProcessRow {
    pid: gtk::Label,
    name: gtk::Label,
    cpu: gtk::Label,
    memory: gtk::Label,
}

struct Dashboard {
    clock: gtk::Label,
    uptime: gtk::Label,
    clock_format: &'static str,
    clock_year: gtk::Label,
    clock_date: gtk::Label,
    battery: gtk::Label,
    processor: gtk::Label,
    cpu_speed: gtk::Label,
    cpu_max: gtk::Label,
    tasks: gtk::Label,
    cpu: gtk::Label,
    temperature: gtk::Label,
    memory: gtk::Label,
    swap: gtk::Label,
    swap_bar: gtk::ProgressBar,
    processes: Vec<ProcessRow>,
    inspector: processes::Inspector,
    interface: gtk::Label,
    traffic_rate: gtk::Label,
    link_state: gtk::Label,
    address: gtk::Label,
    ping: gtk::Label,
    network_total: gtk::Label,
    globe: Option<globe::Globe>,
    endpoint: gtk::Label,
    geo_hint: gtk::Label,
    cpu_chart: [Chart; 2],
    cpu_groups: [gtk::Label; 2],
    memory_chart: MemoryMap,
    network_chart: traffic::Traffic,
    interface_name: Option<String>,
    files: [Files; 2],
    tabs: Terminals,
    disk_usage: gtk::ProgressBar,
    mount_label: gtk::Button,
    current_mount: Option<std::path::PathBuf>,
    mount_updated: Option<Instant>,
}

fn build_window(app: &gtk::Application, options: &Options, prefs: &Preferences) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("eDEX-Remake")
        .decorated(false)
        .default_width(1440)
        .default_height(850)
        .build();
    let provider = gtk::CssProvider::new();
    provider.load_from_data(&prefs.stylesheet());
    gtk::style_context_add_provider_for_display(
        &gtk::prelude::WidgetExt::display(&window),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let overlay = gtk::Overlay::new();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.add_css_class("workspace");
    overlay.set_child(Some(&root));
    window.set_child(Some(&overlay));
    if prefs.values["nocursor"].as_bool().unwrap_or(false) {
        window.set_cursor_from_name(Some("none"));
    }
    let sounds = audio::Sounds::new(
        prefs.values["audio"].as_bool().unwrap_or(true),
        prefs.values["audioVolume"].as_f64().unwrap_or(1.0),
    );

    let columns = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    columns.set_vexpand(true);
    root.append(&columns);
    let system = sidebar(240);
    let network = sidebar(230);
    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    let tab_stack = stack.downgrade();
    let tabs = Terminals::new(prefs, &sounds, move || {
        if let Some(stack) = tab_stack.upgrade() {
            stack.set_visible_child_name("terminal");
        }
    });
    let files = Files::new(&tabs, prefs, true);
    let explorer = Files::new(&tabs, prefs, false);
    if prefs.values["fsListView"].as_bool().unwrap_or(false) {
        files.toggle_list();
        explorer.toggle_list();
    }

    let center = gtk::Box::new(gtk::Orientation::Vertical, 4);
    center.add_css_class("main-shell");
    system.add_css_class("system-column");
    network.add_css_class("network-column");
    center.set_hexpand(true);
    center.append(&caption("TERMINAL", "eDEX-Remake"));
    let quick = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    let file_button = quick_button("SYSTEM FILES", "OPEN FILE EXPLORER");
    let app_button = quick_button("APP MANAGER", "OPEN INSTALLED APPS");
    for button in [&file_button, &app_button] {
        button.set_hexpand(true);
        button.add_css_class("quick-action");
        quick.append(button);
    }
    let show_file = prefs.values["showFileManagerButton"]
        .as_bool()
        .unwrap_or(true);
    let show_apps = prefs.values["showInstalledAppsButton"]
        .as_bool()
        .unwrap_or(true);
    file_button.set_visible(show_file);
    app_button.set_visible(show_apps);
    quick.set_visible(show_file || show_apps);
    center.append(&quick);
    let top_buttons = gio::SimpleAction::new_stateful(
        "quick-buttons",
        Some(glib::VariantTy::new("(bb)").expect("two booleans")),
        &(show_file, show_apps).to_variant(),
    );
    let top_file = file_button.clone();
    let top_apps = app_button.clone();
    top_buttons.connect_activate(move |action, parameter| {
        if let Some((files, apps)) = parameter.and_then(|value| value.get::<(bool, bool)>()) {
            top_file.set_visible(files);
            top_apps.set_visible(apps);
            quick.set_visible(files || apps);
            action.set_state(&(files, apps).to_variant());
        }
    });
    window.add_action(&top_buttons);
    let release_notices = gtk::Box::new(gtk::Orientation::Vertical, 0);
    center.append(&release_notices);
    if !prefs.values["disableUpdateCheck"]
        .as_bool()
        .unwrap_or(false)
    {
        startup::release_notice(&release_notices);
    }
    center.append(&tabs.bar);
    stack.add_named(&tabs.view, Some("terminal"));
    stack.add_named(&explorer.view, Some("files"));
    stack.add_named(&explorer::app_manager(), Some("apps"));
    stack.set_visible_child_name("terminal");
    center.append(&stack);
    let toggle = |name, stack: &gtk::Stack, terminals: &Terminals| {
        let stack = stack.clone();
        let terminals = terminals.clone();
        move || {
            if stack.visible_child_name().as_deref() == Some(name) {
                stack.set_visible_child_name("terminal");
                terminals.focus();
            } else {
                stack.set_visible_child_name(name);
            }
        }
    };
    let open_files = toggle("files", &stack, &tabs);
    let search_files = explorer.clone();
    let folder_sound = sounds.clone();
    file_button.connect_clicked(move |_| {
        search_files.sync_to_terminal();
        open_files();
        search_files.focus_search();
        folder_sound.play("folder");
    });
    let app_overlay = toggle("apps", &stack, &tabs);
    let apps_stack = stack.clone();
    let open_apps = move || {
        app_overlay();
        if apps_stack.visible_child_name().as_deref() == Some("apps") {
            if let Some(search) = apps_stack
                .visible_child()
                .and_then(|child| child.first_child())
            {
                search.grab_focus();
            }
        }
    };
    let app_overlay = open_apps.clone();
    app_button.connect_clicked(move |_| app_overlay());
    bind(app, &window, "apps", &["<Primary><Shift>a"], open_apps);
    let file_search = explorer.clone();
    let toggle_files = toggle("files", &stack, &tabs);
    bind(app, &window, "files", &["<Primary><Shift>f"], move || {
        file_search.sync_to_terminal();
        toggle_files();
        file_search.focus_search();
    });
    for page in 0..5 {
        let key = format!("<Primary>{}", page + 1);
        let stack = stack.clone();
        let tabs = tabs.clone();
        bind(app, &window, &format!("tab{page}"), &[&key], move || {
            stack.set_visible_child_name("terminal");
            tabs.switch(page);
            tabs.focus();
        });
    }
    for (name, accelerator, offset) in [
        ("next-tab", "<Primary>Tab", 1),
        ("previous-tab", "<Primary><Shift>Tab", 4),
    ] {
        let stack = stack.clone();
        let tabs = tabs.clone();
        bind(app, &window, name, &[accelerator], move || {
            stack.set_visible_child_name("terminal");
            tabs.switch((tabs.view.current_page().unwrap_or(0) + offset) % 5);
            tabs.focus();
        });
    }

    columns.append(&system);
    columns.append(&center);
    columns.append(&network);

    let dock = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .position(630)
        .height_request(235)
        .build();
    let file_dock = gtk::Box::new(gtk::Orientation::Vertical, 4);
    file_dock.append(&files.view);
    let disk_usage = gtk::ProgressBar::builder().hexpand(true).build();
    let mount = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    mount.add_css_class("mount-progress");
    let mount_label = gtk::Button::with_label("Mount / used —");
    mount_label.add_css_class("mount-button");
    let disks = files.clone();
    let mount_window = window.downgrade();
    mount_label.connect_clicked(move |_| {
        if let Some(window) = mount_window.upgrade() {
            disks.open_mounts(&window);
        }
    });
    mount.append(&mount_label);
    mount.append(&disk_usage);
    file_dock.append(&mount);
    let keyboard_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let keyboard = std::rc::Rc::new(std::cell::RefCell::new(keyboard::panel(
        &prefs.keys,
        &tabs,
        &window,
        &sounds,
    )));
    keyboard_holder.append(&keyboard.borrow().view);
    dock.set_resize_start_child(true);
    dock.set_shrink_start_child(true);
    dock.set_shrink_end_child(true);
    root.append(&dock);
    let dock_control = dock.clone();
    let panel_audio = sounds.clone();
    bind(app, &window, "dock", &["F9"], move || {
        dock_control.set_visible(!dock_control.is_visible());
        panel_audio.play("panels");
    });
    let sizing = gtk::DrawingArea::new();
    sizing.set_can_target(false);
    sizing.set_hexpand(true);
    sizing.set_vexpand(true);
    overlay.add_overlay(&sizing);
    let workspace = skin::Workspace::new(
        &root,
        &columns,
        &center,
        &system,
        &network,
        &dock,
        &file_dock,
        &keyboard_holder,
        &mount,
        &sizing,
    );
    workspace.apply(prefs);
    let password = keyboard.clone();
    bind(
        app,
        &window,
        "password-mode",
        &["<Primary><Shift>p"],
        move || {
            password.borrow().toggle_password_mode();
        },
    );
    let selected_keyboard = gio::SimpleAction::new_stateful(
        "keyboard",
        Some(glib::VariantTy::STRING),
        &prefs.keyboard_name.to_variant(),
    );
    let key_window = window.downgrade();
    let key_tabs = tabs.clone();
    let key_sounds = sounds.clone();
    selected_keyboard.connect_activate(move |action, parameter| {
        let Some(name) = parameter.and_then(glib::Variant::str) else {
            return;
        };
        let Some(window) = key_window.upgrade() else {
            return;
        };
        let Ok(layout) = theme::resource(&theme::config_path(), "keyboards", name) else {
            return;
        };
        let next = keyboard::panel(&layout, &key_tabs, &window, &key_sounds);
        let mut previous = keyboard.borrow_mut();
        if previous.password_mode() {
            next.toggle_password_mode();
        }
        previous.detach(&window);
        keyboard_holder.remove(&previous.view);
        keyboard_holder.append(&next.view);
        *previous = next;
        action.set_state(&name.to_variant());
        key_sounds.play("keyboard");
    });
    window.add_action(&selected_keyboard);
    let settings_window = window.downgrade();
    bind(
        app,
        &window,
        "settings",
        &["<Primary><Shift>s"],
        move || {
            if let Some(window) = settings_window.upgrade() {
                settings::open(&window);
            }
        },
    );
    let shortcuts_window = window.downgrade();
    bind(
        app,
        &window,
        "shortcuts",
        &["<Primary><Shift>k"],
        move || {
            if let Some(window) = shortcuts_window.upgrade() {
                settings::shortcuts(&window);
            }
        },
    );
    let modal_tabs = tabs.clone();
    let modal_stack = stack.downgrade();
    bind(app, &window, "focus-terminal", &[], move || {
        if modal_stack
            .upgrade()
            .and_then(|stack| stack.visible_child_name())
            .as_deref()
            == Some("terminal")
        {
            modal_tabs.focus();
        }
    });
    let clipboard = tabs.clone();
    bind(app, &window, "copy", &["<Primary><Shift>c"], move || {
        clipboard.copy()
    });
    let clipboard = tabs.clone();
    bind(app, &window, "paste", &["<Primary><Shift>v"], move || {
        clipboard.paste()
    });
    let file_mode = [files.clone(), explorer.clone()];
    bind(
        app,
        &window,
        "file-list",
        &["<Primary><Shift>l"],
        move || {
            for panel in &file_mode {
                panel.toggle_list();
            }
        },
    );
    let file_mode = [files.clone(), explorer.clone()];
    bind(
        app,
        &window,
        "dotfiles",
        &["<Primary><Shift>h"],
        move || {
            for panel in &file_mode {
                panel.toggle_dotfiles();
            }
        },
    );

    let left = gtk::Box::new(gtk::Orientation::Vertical, 9);
    left.add_css_class("sidebar-content");
    system.set_child(Some(&left));
    left.append(&caption("PANEL", "SYSTEM"));
    let clock = text("—", "clock", 0.5);
    clock.add_css_class("light");
    left.append(&clock);
    let facts = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    facts.add_css_class("facts");
    let (clock_year, clock_date) = fact(&facts, "—", "—");
    let (_, uptime) = fact(&facts, "UPTIME", "—");
    fact(&facts, "TYPE", "linux");
    let (_, battery) = fact(&facts, "POWER", "—");
    left.append(&facts);
    let hardware = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    hardware.add_css_class("facts");
    fact(
        &hardware,
        "MANUFACTURER",
        &metrics::machine_label("sys_vendor"),
    );
    fact(&hardware, "MODEL", &metrics::machine_label("product_name"));
    fact(&hardware, "CHASSIS", &metrics::chassis_label());
    left.append(&hardware);
    let right = gtk::Box::new(gtk::Orientation::Vertical, 9);
    right.add_css_class("sidebar-content");
    network.set_child(Some(&right));
    right.append(&caption("PANEL", "NETWORK"));

    let cpu = text("—", "value", 0.0);
    let cpu_chart = [
        Chart::new(38, prefs.accent()),
        Chart::new(38, prefs.accent()),
    ];
    let cpu_groups = [text("—", "subtle", 0.0), text("—", "subtle", 0.0)];
    let section = metric_group("CPU USAGE");
    section.add_css_class("cpu-module");
    let processor = text("—", "subtle", 0.0);
    processor.set_ellipsize(pango::EllipsizeMode::End);
    section.append(&processor);
    cpu.set_visible(false);
    for (chart, label) in cpu_chart.iter().zip(&cpu_groups) {
        section.append(label);
        section.append(&chart.view);
    }
    let performance = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    performance.add_css_class("facts");
    let (_, temperature) = fact(&performance, "TEMP", "—");
    let (_, cpu_speed) = fact(&performance, "SPD", "—");
    let (_, cpu_max) = fact(&performance, "MAX", "—");
    let (_, tasks) = fact(&performance, "TASKS", "—");
    section.append(&performance);
    left.append(&section);

    let memory = text("—", "value", 0.0);
    let swap = text("—", "subtle", 0.0);
    let memory_chart = MemoryMap::new(prefs.accent());
    let section = metric_group("");
    section.add_css_class("memory-module");
    let ram = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let caption = text("MEMORY", "section-title", 0.0);
    caption.set_hexpand(true);
    ram.append(&caption);
    memory.set_xalign(1.0);
    memory.remove_css_class("value");
    memory.add_css_class("subtle");
    ram.append(&memory);
    section.append(&ram);
    section.append(&memory_chart.view);
    let swaps = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    swaps.append(&text("SWAP", "section-title", 0.0));
    let swap_bar = gtk::ProgressBar::builder()
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    swap_bar.add_css_class("swap-gauge");
    swaps.append(&swap_bar);
    swaps.append(&swap);
    section.append(&swaps);
    left.append(&section);

    let inspector = processes::Inspector::default();
    let section = metric_group("");
    section.add_css_class("process-module");
    let heading = gtk::Button::with_label("TOP PROCESSES");
    heading.add_css_class("module-button");
    heading.set_halign(gtk::Align::Start);
    let open_inspector = inspector.clone();
    let inspector_parent = window.downgrade();
    heading.connect_clicked(move |_| {
        if let Some(window) = inspector_parent.upgrade() {
            open_inspector.open(&window);
        }
    });
    section.append(&heading);
    let table = gtk::Grid::builder()
        .row_spacing(4)
        .column_spacing(6)
        .build();
    table.attach(&text("PID", "subtle", 0.0), 0, 0, 1, 1);
    table.attach(&text("NAME", "subtle", 0.0), 1, 0, 1, 1);
    table.attach(&text("CPU", "subtle", 1.0), 2, 0, 1, 1);
    table.attach(&text("MEM", "subtle", 1.0), 3, 0, 1, 1);
    let processes = (1..=PROCESS_LIMIT)
        .map(|row| {
            let pid = text("—", "process", 0.0);
            let name = text("—", "process-name", 0.0);
            name.set_hexpand(true);
            name.set_max_width_chars(13);
            name.set_ellipsize(pango::EllipsizeMode::End);
            let cpu = text("—", "process", 1.0);
            let memory = text("—", "process", 1.0);
            for (column, cell) in [&pid, &name, &cpu, &memory].into_iter().enumerate() {
                table.attach(cell, column as i32, row as i32, 1, 1);
            }
            ProcessRow {
                pid,
                name,
                cpu,
                memory,
            }
        })
        .collect();
    section.append(&table);
    left.append(&section);

    let section = metric_group("NETWORK STATUS");
    section.add_css_class("network-status");
    let interface = text("—", "subtle", 0.0);
    interface.set_ellipsize(pango::EllipsizeMode::End);
    let network_chart = traffic::Traffic::new(prefs.accent());
    let network_facts = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    network_facts.add_css_class("facts");
    let (_, link_state) = fact(&network_facts, "STATE", "UNKNOWN");
    let (_, address) = fact(&network_facts, "IPv4", "—");
    let (_, ping) = fact(&network_facts, "PING", "—ms");
    section.append(&interface);
    section.append(&network_facts);
    right.append(&section);
    let endpoint = text("—", "subtle", 1.0);
    let geo_hint = text("GLOBAL NETWORK MAP", "subtle", 1.0);
    let globe = prefs.globe_enabled.then(|| {
        let world = metric_group("");
        world.add_css_class("world-module");
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let heading = text("WORLD VIEW", "section-title", 0.0);
        heading.set_hexpand(true);
        title.append(&heading);
        title.append(&geo_hint);
        world.append(&title);
        let location = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let caption = text("ENDPOINT LAT/LON", "subtle", 0.0);
        caption.set_hexpand(true);
        location.append(&caption);
        location.append(&endpoint);
        world.append(&location);
        let globe = globe::Globe::new(prefs);
        world.append(&globe.view);
        right.append(&world);
        globe
    });
    let section = metric_group("");
    section.add_css_class("network-traffic");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let heading = text("NETWORK TRAFFIC", "section-title", 0.0);
    heading.set_hexpand(true);
    let traffic_rate = text("UP / DOWN, Mb/s", "subtle", 1.0);
    header.append(&heading);
    header.append(&traffic_rate);
    section.append(&header);
    let totals = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let network_total = text("0 B OUT, 0 B IN", "subtle", 1.0);
    network_total.set_hexpand(true);
    totals.append(&text("TOTAL", "subtle", 0.0));
    totals.append(&network_total);
    section.append(&totals);
    section.append(&network_chart.view);
    right.append(&section);

    let selected_theme = gio::SimpleAction::new_stateful(
        "theme",
        Some(glib::VariantTy::STRING),
        &prefs.theme_name.to_variant(),
    );
    let current = prefs.clone();
    let theme_tabs = tabs.clone();
    let theme_panels = [files.clone(), explorer.clone()];
    let theme_charts = cpu_chart.clone();
    let theme_traffic = network_chart.clone();
    let theme_memory = memory_chart.clone();
    let theme_globe = globe.clone();
    let theme_sounds = sounds.clone();
    let current = std::cell::RefCell::new(current);
    selected_theme.connect_activate(move |action, parameter| {
        let Some(name) = parameter.and_then(glib::Variant::str) else {
            return;
        };
        let Ok(next) = current.borrow().with_theme(name) else {
            return;
        };
        provider.load_from_data(&next.stylesheet());
        workspace.apply(&next);
        theme_tabs.apply_theme(&next);
        for panel in &theme_panels {
            panel.apply_theme(&next);
        }
        let accent = next.accent();
        for chart in &theme_charts {
            chart.set_accent(accent);
        }
        theme_memory.set_accent(accent);
        theme_traffic.set_accent(accent);
        if let Some(globe) = &theme_globe {
            globe.apply_theme(&next);
        }
        current.replace(next);
        action.set_state(&name.to_variant());
        theme_sounds.play("theme");
    });
    window.add_action(&selected_theme);

    let panels = gio::SimpleAction::new("panels", None);
    panels.connect_activate(move |_, _| {
        let visible = !system.is_visible();
        system.set_visible(visible);
        network.set_visible(visible);
    });
    window.add_action(&panels);
    app.set_accels_for_action("win.panels", &["F10"]);
    let fullscreen = gio::SimpleAction::new("fullscreen", None);
    let weak_window = window.downgrade();
    fullscreen.connect_activate(move |_, _| {
        if let Some(window) = weak_window.upgrade() {
            if window.is_fullscreen() {
                window.unfullscreen();
            } else {
                window.fullscreen();
            }
        }
    });
    window.add_action(&fullscreen);
    app.set_accels_for_action("win.fullscreen", &["F11"]);
    let quit = gio::SimpleAction::new("quit", None);
    let weak_app = app.downgrade();
    quit.connect_activate(move |_, _| {
        if let Some(app) = weak_app.upgrade() {
            app.quit();
        }
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Primary><Shift>q"]);

    bind(app, &window, "native-inspector", &[], || {
        gtk::Window::set_interactive_debugging(true)
    });
    shortcuts::install(app, &window, &tabs);
    if !options.windowed
        && !prefs.values["allowWindowed"]
            .as_bool()
            .is_some_and(|enabled| {
                enabled
                    && !prefs.values["forceFullscreen"].as_bool().unwrap_or(false)
                    && prefs.values["keepGeometry"].as_bool().unwrap_or(false)
            })
    {
        let monitor_index = prefs.values["monitor"].as_u64().unwrap_or(0) as u32;
        if let Some(monitor) = gtk::prelude::WidgetExt::display(&window)
            .monitors()
            .item(monitor_index)
            .and_downcast::<gtk::gdk::Monitor>()
        {
            window.fullscreen_on_monitor(&monitor);
        } else {
            window.fullscreen();
        }
    }
    window.present();
    let focus = tabs.clone();
    glib::idle_add_local_once(move || focus.focus());
    if options
        .intro
        .unwrap_or(!prefs.values["nointro"].as_bool().unwrap_or(false))
    {
        let username = prefs.values["username"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .or_else(|| env::var("USER").ok())
            .unwrap_or_default();
        startup::intro(&overlay, &username, prefs.accent(), &sounds);
    }

    let dashboard = Dashboard {
        clock,
        uptime,
        clock_format: if prefs.values["clockHours"].as_u64() == Some(12) {
            "%I:%M:%S"
        } else {
            "%H:%M:%S"
        },
        clock_year,
        clock_date,
        battery,
        processor,
        cpu_speed,
        cpu_max,
        tasks,
        cpu,
        temperature,
        memory,
        swap,
        swap_bar,
        processes,
        inspector,
        interface,
        traffic_rate,
        link_state,
        address,
        ping,
        network_total,
        globe,
        endpoint,
        geo_hint,
        cpu_chart,
        cpu_groups,
        memory_chart,
        network_chart,
        interface_name: None,
        files: [files, explorer],
        tabs,
        disk_usage,
        mount_label,
        current_mount: None,
        mount_updated: None,
    };
    let interface = options.interface.clone().or_else(|| {
        prefs.values["iface"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
    });
    let parameters = MetricsOptions {
        interface,
        group_threads: prefs.values["excludeThreadsFromToplist"]
            .as_bool()
            .unwrap_or(true),
        geo_database: geo::database(prefs.values["geoIpDatabase"].as_str()),
        observe_terminal_ips: prefs.globe_enabled
            && prefs.values["experimentalGlobeFeatures"]
                .as_bool()
                .unwrap_or(false),
        ping_target: prefs.values["pingAddr"]
            .as_str()
            .filter(|name| !name.is_empty())
            .unwrap_or("1.1.1.1")
            .to_owned(),
    };
    start_metrics(&window, dashboard, parameters);
}

fn bind(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    name: &str,
    keys: &[&str],
    callback: impl Fn() + 'static,
) {
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| callback());
    window.add_action(&action);
    app.set_accels_for_action(&format!("win.{name}"), keys);
}

fn caption(left: &str, right: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.add_css_class("panel-caption");
    let prefix = text(left, "section-title", 0.0);
    prefix.set_hexpand(true);
    row.append(&prefix);
    row.append(&text(right, "section-title", 1.0));
    row
}

fn sidebar(width: i32) -> gtk::ScrolledWindow {
    let panel = gtk::ScrolledWindow::builder()
        .width_request(width)
        .hexpand(false)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .build();
    panel.add_css_class("side-panel");
    panel
}

fn metric_group(title: &str) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 7);
    section.add_css_class("metric-group");
    if !title.is_empty() {
        section.append(&text(title, "section-title", 0.0));
    }
    section
}

fn text(value: &str, class: &str, align: f32) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.set_xalign(align);
    label.add_css_class(class);
    label
}

fn fact(row: &gtk::Box, title: &str, value: &str) -> (gtk::Label, gtk::Label) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.set_hexpand(true);
    let title = text(title, "fact-title", 0.0);
    let value = text(value, "fact-value", 0.0);
    value.set_ellipsize(pango::EllipsizeMode::End);
    value.set_max_width_chars(16);
    card.append(&title);
    card.append(&value);
    row.append(&card);
    (title, value)
}

fn quick_button(title: &str, value: &str) -> gtk::Button {
    let button = gtk::Button::new();
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 0);
    labels.append(&text(title, "quick-title", 0.0));
    labels.append(&text(value, "quick-value", 0.0));
    button.set_child(Some(&labels));
    button
}

struct MetricsOptions {
    interface: Option<String>,
    group_threads: bool,
    geo_database: std::path::PathBuf,
    ping_target: String,
    observe_terminal_ips: bool,
}

fn start_metrics(
    window: &gtk::ApplicationWindow,
    mut dashboard: Dashboard,
    options: MetricsOptions,
) {
    let (address_sender, address_receiver) = async_channel::bounded(1);
    let (geo_sender, geo_receiver) = async_channel::bounded(1);
    let MetricsOptions {
        interface,
        group_threads,
        geo_database,
        ping_target,
        observe_terminal_ips,
    } = options;
    let _ = thread::Builder::new()
        .name("network status".into())
        .spawn(move || {
            let mut watcher = geo::Watcher::new(geo_database, ping_target);
            while let Ok((address, observed)) = address_receiver.recv_blocking() {
                let view = watcher.sample(address, observed);
                if geo_sender.send_blocking(view).is_err() {
                    break;
                }
            }
        });
    let (sender, receiver) = async_channel::bounded(1);
    let stop = receiver.clone();
    window.connect_close_request(move |_| {
        stop.close();
        glib::Propagation::Proceed
    });
    let worker = thread::Builder::new()
        .name("metrics".into())
        .spawn(move || {
            let mut sampler = Sampler::new(interface, group_threads);
            thread::sleep(SAMPLE_INTERVAL);
            loop {
                let started = Instant::now();
                let sample = sampler.sample();
                if sender.send_blocking(sample).is_err() {
                    break;
                }
                thread::sleep(SAMPLE_INTERVAL.saturating_sub(started.elapsed()));
            }
        });
    if let Err(error) = worker {
        dashboard
            .uptime
            .set_text(&format!("Metrics unavailable: {error}"));
    }
    dashboard.update_clock();
    glib::MainContext::default().spawn_local(async move {
        let mut count = 0_u64;
        let mut previous = Vec::new();
        while let Ok(sample) = receiver.recv().await {
            if count % 3 == 0 {
                let address = sample.network.as_ref().and_then(|network| network.address);
                let current = if observe_terminal_ips {
                    dashboard
                        .tabs
                        .recent_text()
                        .map(|text| geo::terminal_addresses(&text))
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let observed = current
                    .iter()
                    .copied()
                    .filter(|ip| !previous.contains(ip))
                    .collect();
                if address_sender.try_send((address, observed)).is_ok() {
                    previous = current;
                }
            }
            count += 1;
            dashboard.update(sample);
            while let Ok(geo) = geo_receiver.try_recv() {
                dashboard.update_location(geo);
            }
        }
    });
}

impl Dashboard {
    fn update_location(&self, view: geo::NetworkView) {
        self.ping.set_text(
            &view
                .delay_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "—ms".into()),
        );
        self.network_chart.set_online(view.delay_ms.is_some());
        if self.interface_name.is_some() {
            self.link_state.set_text(if view.delay_ms.is_some() {
                "ONLINE"
            } else {
                "OFFLINE"
            });
        }
        self.geo_hint.set_text(if view.geo_available {
            "GLOBAL NETWORK MAP"
        } else {
            "GEOIP DB NEEDED"
        });
        self.endpoint.set_text(
            &view
                .endpoint
                .map(|(lat, lon)| format!("{lat:.4}, {lon:.4}"))
                .unwrap_or_else(|| "—".into()),
        );
        if let Some(globe) = &self.globe {
            globe.update(view.peers, view.endpoint);
        }
    }

    fn update_clock(&self) {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(time) = now.format(self.clock_format) {
                self.clock.set_text(&time);
            }
            if let Ok(year) = now.format("%Y") {
                self.clock_year.set_text(&year);
            }
            if let Ok(date) = now.format("%b %d") {
                self.clock_date.set_text(&date.to_uppercase());
            }
        }
    }

    fn update(&mut self, sample: Stats) {
        self.update_clock();
        self.cpu.set_text(&format!("{:.1}%", sample.cpu));
        let midpoint = sample.cpu_count.div_ceil(2);
        let ranges = [(1, midpoint), (midpoint + 1, sample.cpu_count)];
        for ((chart, label), ((start, end), pct)) in self
            .cpu_chart
            .iter()
            .zip(&self.cpu_groups)
            .zip(ranges.into_iter().zip(sample.cpu_groups))
        {
            chart.view.set_visible(start <= end);
            label.set_visible(start <= end);
            label.set_text(&format!("# {start}–{end}   Avg. {pct:.0}%"));
            chart.push(pct);
        }
        self.temperature.set_text(
            &sample
                .temperature
                .map_or_else(|| "—°C".into(), |value| format!("{value:.0}°C")),
        );
        const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
        self.memory.set_text(&format!(
            "USING {:.1} OUT OF {:.1} GiB",
            sample.memory as f64 / GIB,
            sample.memory_total as f64 / GIB
        ));
        self.memory_chart.update(
            percent(sample.memory, sample.memory_total),
            percent(sample.memory_reclaimable, sample.memory_total),
        );
        self.swap
            .set_text(&format!("{:.1} GiB", sample.swap as f64 / GIB));
        self.swap_bar
            .set_fraction(percent(sample.swap, sample.swap_total) / 100.0);
        self.uptime.set_text(&format!(
            "{}d{:02}:{:02}",
            sample.uptime / 86400,
            sample.uptime / 3600 % 24,
            sample.uptime / 60 % 60
        ));
        self.battery
            .set_text(&sample.battery.map_or("ON".into(), |pct| format!("{pct}%")));
        self.processor.set_text(&sample.processor);
        self.cpu_speed
            .set_text(&format!("{:.2}GHz", sample.cpu_speed as f64 / 1000.0));
        self.cpu_max
            .set_text(&format!("{:.2}GHz", sample.cpu_max as f64 / 1000.0));
        self.tasks.set_text(&sample.tasks.to_string());
        if let Some(cwd) = self.tabs.cwd() {
            let old = self.current_mount.as_ref();
            let due = self
                .mount_updated
                .is_none_or(|when| when.elapsed() >= Duration::from_secs(10));
            if due || old.is_none_or(|path| !cwd.starts_with(path)) {
                let mount = metrics::mounts()
                    .into_iter()
                    .filter(|mount| cwd.starts_with(&mount.path))
                    .max_by_key(|mount| mount.path.components().count())
                    .map(|mount| mount.path);
                self.current_mount = mount;
                self.mount_updated = Some(Instant::now());
            }
            for panel in &self.files {
                if panel.view.is_mapped() {
                    panel.follow(&cwd);
                }
            }
        }
        let mount = self
            .current_mount
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("/"));
        if let Some((used, total)) = metrics::mount_usage(mount).or(sample.root) {
            let used = percent(used, total);
            self.disk_usage.set_fraction(used / 100.0);
            self.mount_label
                .set_label(&format!("Mount {} used {:.0}%", mount.display(), used));
        }

        if let Some(processes) = sample.processes {
            for (index, row) in self.processes.iter().enumerate() {
                if let Some(p) = processes.get(index) {
                    row.pid.set_text(&p.pid.to_string());
                    row.name.set_text(&p.name);
                    row.name.set_tooltip_text(Some(&p.name));
                    row.cpu.set_text(&format!("{:.0}%", p.cpu));
                    row.memory.set_text(&format!("{:.1}%", p.memory_percent));
                } else {
                    for cell in [&row.pid, &row.name, &row.cpu, &row.memory] {
                        cell.set_text("—");
                    }
                    row.name.set_tooltip_text(None);
                }
            }
            self.inspector.update(processes);
        }

        let interface = sample.network.as_ref().map(|network| network.name.as_str());
        if self.interface_name.as_deref() != interface {
            self.network_chart.clear();
            self.interface_name = interface.map(str::to_owned);
        }
        if let Some(network) = sample.network {
            self.interface.set_text(&network.name);

            self.address.set_text(
                &network
                    .address
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| "—".into()),
            );
            self.network_total.set_text(&format!(
                "{} OUT, {} IN",
                traffic::total_bytes(network.total_tx),
                traffic::total_bytes(network.total_rx)
            ));
            self.traffic_rate.set_text(&format!(
                "UP {:.2} DOWN {:.2}",
                traffic::megabits(network.tx),
                traffic::megabits(network.rx)
            ));
            self.network_chart.push(network.tx, network.rx);
        } else {
            self.interface.set_text("No interface");
            self.link_state.set_text("OFFLINE");
            self.address.set_text("—");
            self.ping.set_text("—ms");
            self.traffic_rate.set_text("UP / DOWN, Mb/s");
            self.network_total.set_text("0 B OUT, 0 B IN");
        }
    }
}
