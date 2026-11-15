use crate::{audio::Sounds, theme::Preferences};
use std::{
    cell::{Cell, RefCell},
    env, fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};
use vte4::gtk::{self, gio, glib, prelude::*};
use vte4::{PtyFlags, Terminal, prelude::*};

#[derive(Clone)]
struct Session {
    terminal: Terminal,
    pid: Rc<Cell<Option<glib::Pid>>>,
    initial_cwd: Option<PathBuf>,
}

pub const DEFAULT_TAB_COUNT: u32 = 5;

#[derive(Clone)]
pub struct Terminals {
    pub view: gtk::Notebook,
    pub bar: gtk::Revealer,
    buttons: gtk::Box,
    on_tab: Rc<dyn Fn()>,
    sessions: Rc<RefCell<Vec<Option<Session>>>>,
    preferences: Rc<RefCell<Preferences>>,
}

impl Terminals {
    pub fn new(
        settings: &Preferences,
        sounds: &Sounds,
        on_tab: impl Fn() + Clone + 'static,
    ) -> Self {
        let view = gtk::Notebook::builder()
            .show_tabs(false)
            .hexpand(true)
            .vexpand(true)
            .build();
        view.add_css_class("terminal-tabs");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        buttons.set_homogeneous(true);
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&buttons)
            .build();
        let add = gtk::Button::with_label("+");
        add.add_css_class("shell-tab");
        add.add_css_class("new-tab");
        add.set_focusable(false);
        add.set_tooltip_text(Some("New terminal tab (Ctrl+Shift+T)"));
        add.set_action_name(Some("win.new-tab"));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("tab-bar");
        row.append(&scroll);
        row.append(&add);
        let bar = gtk::Revealer::builder()
            .height_request(7)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(140)
            .reveal_child(true)
            .child(&row)
            .build();
        bar.add_css_class("tab-strip");
        let tabs = Self {
            view,
            bar,
            buttons,
            on_tab: Rc::new(on_tab),
            sessions: Rc::new(RefCell::new(Vec::new())),
            preferences: Rc::new(RefCell::new(settings.clone())),
        };
        for _ in 0..DEFAULT_TAB_COUNT {
            tabs.append_tab();
        }
        let buttons = tabs.buttons.observe_children();
        if let Some(button) = buttons.item(0).and_downcast::<gtk::ToggleButton>() {
            button.set_active(true);
        }
        let adjustment = scroll.hadjustment();
        let scroll_selected = {
            let notebook = tabs.view.downgrade();
            let row = tabs.buttons.downgrade();
            let adjustment = adjustment.downgrade();
            move || {
                let (notebook, row, adjustment) =
                    (notebook.clone(), row.clone(), adjustment.clone());
                glib::idle_add_local_once(move || {
                    let (Some(notebook), Some(row), Some(adjustment)) =
                        (notebook.upgrade(), row.upgrade(), adjustment.upgrade())
                    else {
                        return;
                    };
                    let Some(button) = notebook
                        .current_page()
                        .and_then(|index| row.observe_children().item(index))
                        .and_downcast::<gtk::Widget>()
                    else {
                        return;
                    };
                    if let Some(bounds) = button.compute_bounds(&row) {
                        let start = f64::from(bounds.x());
                        adjustment.clamp_page(start, start + f64::from(bounds.width()));
                    }
                });
            }
        };
        let scroll_on_resize = scroll_selected.clone();
        adjustment.connect_changed(move |_| scroll_on_resize());

        let slots = tabs.sessions.clone();
        let preferences = tabs.preferences.clone();
        let tab_sounds = sounds.clone();
        tabs.view.connect_switch_page(move |notebook, page, index| {
            if let Some(button) = buttons.item(index).and_downcast::<gtk::ToggleButton>() {
                button.set_active(true);
            }
            scroll_selected();
            let previous = notebook
                .current_page()
                .and_then(|i| slots.borrow().get(i as usize)?.clone());
            let cwd = previous.and_then(|session| session.cwd());
            if let Some(panel) = page.downcast_ref::<gtk::Box>() {
                open_tab(
                    &slots,
                    panel,
                    index as usize,
                    &preferences.borrow(),
                    cwd,
                    &tab_sounds,
                );
                tab_sounds.play("folder");
            }
        });
        if let Some(panel) = tabs.view.nth_page(Some(0)).and_downcast::<gtk::Box>() {
            open_tab(&tabs.sessions, &panel, 0, settings, None, sounds);
        }
        tabs
    }

    fn append_tab(&self) -> u32 {
        let number = self.view.n_pages();
        let title = if number == 0 {
            "Main".into()
        } else {
            number.to_string()
        };
        let button = gtk::ToggleButton::with_label(&title);
        button.add_css_class("shell-tab");
        button.set_focusable(false);
        if let Some(first) = self
            .buttons
            .first_child()
            .and_downcast::<gtk::ToggleButton>()
        {
            button.set_group(Some(&first));
        }
        let notebook = self.view.downgrade();
        let sessions = self.sessions.clone();
        let on_tab = self.on_tab.clone();
        button.connect_clicked(move |button| {
            on_tab();
            if let Some(notebook) = notebook.upgrade() {
                notebook.set_current_page(Some(number));
                button.set_active(true);
                if let Some(session) = sessions
                    .borrow()
                    .get(number as usize)
                    .and_then(Option::as_ref)
                {
                    session.terminal.grab_focus();
                }
            }
        });

        self.sessions.borrow_mut().push(None);
        self.buttons.append(&button);
        self.view.append_page(
            &gtk::Box::new(gtk::Orientation::Vertical, 0),
            None::<&gtk::Widget>,
        );
        number
    }

    pub fn add_tab(&self) {
        (self.on_tab)();
        self.switch(self.append_tab());
        self.focus();
    }

    pub fn cycle(&self, forward: bool) {
        let count = self.view.n_pages();
        if count > 0 {
            let page = self.view.current_page().unwrap_or(0);
            let offset = if forward { 1 } else { count - 1 };
            self.switch((page + offset) % count);
        }
    }

    pub fn install_auto_hide(&self, header: &gtk::Box) -> impl Fn() + 'static {
        let pointer = gtk::EventControllerMotion::new();
        let bar = self.bar.downgrade();
        let timeout = Rc::new(Cell::new(None::<glib::SourceId>));
        let update = move |hovering: bool| {
            if let Some(id) = timeout.take() {
                id.remove();
            }
            if let Some(revealer) = bar.upgrade() {
                revealer.set_reveal_child(true);
            }
            if !hovering {
                let (timeout_state, bar) = (timeout.clone(), bar.clone());
                timeout.set(Some(glib::timeout_add_local_once(
                    Duration::from_secs(3),
                    move || {
                        timeout_state.take();
                        if let Some(revealer) = bar.upgrade() {
                            revealer.set_reveal_child(false);
                        }
                    },
                )));
            }
        };
        let enter = update.clone();
        pointer.connect_enter(move |_, _, _| enter(true));
        let leave = update.clone();
        pointer.connect_leave(move |_| leave(false));
        header.add_controller(pointer.clone());
        move || update(pointer.contains_pointer())
    }

    pub fn preferences(&self) -> Preferences {
        self.preferences.borrow().clone()
    }

    pub fn apply_theme(&self, settings: &Preferences) {
        self.preferences.replace(settings.clone());
        for session in self.sessions.borrow().iter().flatten() {
            settings.apply_terminal(&session.terminal);
        }
    }

    fn active(&self) -> Option<Session> {
        self.view
            .current_page()
            .and_then(|i| self.sessions.borrow().get(i as usize)?.clone())
    }
    pub fn recent_text(&self) -> Option<glib::GString> {
        let terminal = self.active()?.terminal;
        let (_, row) = terminal.cursor_position();
        let end = terminal.column_count().saturating_sub(1);
        terminal
            .text_range_format(
                vte4::Format::Text,
                row.saturating_sub(8).max(0),
                0,
                row,
                end,
            )
            .0
    }

    pub fn cwd(&self) -> Option<PathBuf> {
        self.active()?.cwd()
    }

    pub fn password_input(&self) -> bool {
        self.active()
            .is_some_and(|session| password_input(&session.terminal))
    }
    pub fn focus(&self) {
        if let Some(session) = self.active() {
            session.terminal.grab_focus();
        }
    }
    pub fn write(&self, data: &[u8]) {
        if let Some(session) = self.active() {
            session.terminal.feed_child(data);
            session.terminal.grab_focus();
        }
    }
    pub fn write_to_idle_shell(&self, data: &[u8]) -> bool {
        let Some(session) = self.active() else {
            return false;
        };
        let Some(foreground) = session.foreground_group() else {
            return false;
        };
        let Some(shell) = session.pid.get() else {
            return false;
        };
        if foreground != unsafe { libc::getpgid(shell.0) } {
            return false;
        }
        session.terminal.feed_child(data);
        session.terminal.grab_focus();
        true
    }

    pub fn copy(&self) {
        if let Some(session) = self.active() {
            session.terminal.copy_clipboard_format(vte4::Format::Text);
        }
    }
    pub fn paste(&self) {
        if let Some(session) = self.active() {
            session.terminal.paste_clipboard();
        }
    }
    pub fn switch(&self, page: u32) {
        self.view.set_current_page(Some(page));
    }
}

fn password_input(terminal: &Terminal) -> bool {
    use std::{mem::MaybeUninit, os::fd::AsRawFd};

    let Some(pty) = terminal.pty() else {
        return false;
    };
    let fd = pty.fd().as_raw_fd();
    let mut mode = MaybeUninit::<libc::termios>::uninit();
    // Linux exposes the slave's terminal settings through the PTY master.
    if unsafe { libc::tcgetattr(fd, mode.as_mut_ptr()) } != 0 {
        return false;
    }
    let mode = unsafe { mode.assume_init() };
    let local = mode.c_lflag;
    if local & libc::ECHO != 0 {
        return false;
    }
    // Password readers normally keep canonical input. sudo's pwfeedback mode
    // uses cbreak, preserving signal handling and the normal CR-to-NL input mapping.
    if local & libc::ICANON != 0
        || (local & (libc::ISIG | libc::IEXTEN) == libc::ISIG && mode.c_iflag & libc::ICRNL != 0)
    {
        return true;
    }
    // Dedicated password readers can disable all terminal line processing.
    // Linux limits the process name reported by /proc to 15 bytes.
    let group = unsafe { libc::tcgetpgrp(fd) };
    group > 0
        && fs::read_to_string(format!("/proc/{group}/comm")).is_ok_and(|name| {
            matches!(
                name.trim_end(),
                "systemd-ask-pas" | "pinentry-tty" | "pinentry-curses"
            )
        })
}

impl Session {
    fn foreground_group(&self) -> Option<libc::pid_t> {
        let pty = self.terminal.pty()?;
        let group = unsafe { libc::tcgetpgrp(std::os::fd::AsRawFd::as_raw_fd(&pty.fd())) };
        (group > 0).then_some(group)
    }

    fn cwd(&self) -> Option<PathBuf> {
        let Some(pid) = self.pid.get() else {
            return self.initial_cwd.clone();
        };
        let pid = pid.0;
        // A fish started interactively from the configured shell has its own foreground job.
        let pid = self
            .foreground_group()
            .filter(|&group| {
                group != pid
                    && fs::read_link(format!("/proc/{group}/exe"))
                        .is_ok_and(|path| path.file_name().is_some_and(|name| name == "fish"))
            })
            .unwrap_or(pid);
        fs::read_link(format!("/proc/{pid}/cwd")).ok().or_else(|| {
            let uri = self.terminal.current_directory_uri()?;
            let (path, host) = glib::filename_from_uri(&uri).ok()?;
            host.is_none_or(|name| {
                name.is_empty() || name == "localhost" || name == glib::host_name()
            })
            .then_some(path)
        })
    }
}

// Install a portable OSC 7 report without replacing the user's prompt or fish config.
const FISH_CWD_HOOK: &str = r#"function __edex_report_cwd --on-event fish_prompt
    printf '\e]7;file://%s\a' (string escape --style=url -- "$PWD")
end"#;

fn open_tab(
    slots: &RefCell<Vec<Option<Session>>>,
    panel: &gtk::Box,
    index: usize,
    settings: &Preferences,
    cwd: Option<PathBuf>,
    sounds: &Sounds,
) {
    if let Some(session) = slots.borrow().get(index).and_then(Option::as_ref) {
        session.terminal.grab_focus();
        return;
    }
    let default_cwd = settings.values["cwd"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(Path::new)
        .filter(|path| path.is_dir());
    let home = glib::home_dir();
    let session = create_session(
        panel,
        settings,
        cwd.as_deref().or(default_cwd).or(Some(&home)),
        sounds,
        index == 0,
    );
    slots.borrow_mut()[index] = Some(session);
}

fn create_session(
    panel: &gtk::Box,
    settings: &Preferences,
    cwd: Option<&Path>,
    sounds: &Sounds,
    main: bool,
) -> Session {
    panel.add_css_class("terminal-panel");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    header.add_css_class("terminal-heading");
    let status = gtk::Label::new(None);
    status.add_css_class("subtle");
    let restart = gtk::Button::with_label("Restart shell");
    restart.add_css_class("shell-restart");
    restart.set_visible(false);
    header.append(&status);
    header.append(&restart);
    header.set_visible(false);
    panel.append(&header);

    let terminal = Terminal::new();
    terminal.add_css_class("shell");
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);
    terminal.set_scrollback_lines(3000);
    terminal.set_scroll_on_output(false);
    terminal.set_scroll_on_keystroke(true);
    terminal.set_audible_bell(false);
    settings.apply_terminal(&terminal);
    if main {
        terminal.feed(
            concat!(
                "\x1b[1mWelcome to eDEX-Remake ",
                env!("CARGO_PKG_VERSION"),
                "\x1b[0m\r\n"
            )
            .as_bytes(),
        );
    }
    panel.append(&terminal);
    let session = Session {
        terminal,
        pid: Rc::new(Cell::new(None)),
        initial_cwd: cwd.map(Path::to_path_buf),
    };

    let focus = session.terminal.downgrade();
    panel.connect_map(move |_| {
        if let Some(terminal) = focus.upgrade() {
            terminal.grab_focus();
        }
    });
    let status_when_exited = status.clone();
    let header_when_exited = header.clone();
    let restart_when_exited = restart.clone();
    let pid = session.pid.clone();
    session
        .terminal
        .connect_child_exited(move |_, wait_status| {
            pid.set(None);
            status_when_exited.set_text(&match glib::spawn_check_exit_status(wait_status) {
                Ok(()) => "Shell exited".to_owned(),
                Err(error) => error.to_string(),
            });
            restart_when_exited.set_visible(true);
            header_when_exited.set_visible(true);
        });

    let shell = settings
        .shell
        .clone()
        .or_else(|| env::var("SHELL").ok().filter(|name| !name.is_empty()))
        .unwrap_or_else(|| "/bin/sh".to_owned());
    let command = Rc::new((shell, settings.shell_args.clone()));
    let terminal_ref = session.terminal.downgrade();
    let pid = session.pid.clone();
    let object = status.clone();
    let restart_command = command.clone();
    let header_when_restarted = header.downgrade();
    restart.connect_clicked(move |button| {
        if let Some(terminal) = terminal_ref.upgrade() {
            spawn(&terminal, &pid, &object, button, &restart_command, None);
            if let Some(header) = header_when_restarted.upgrade() {
                header.set_visible(false);
            }
            terminal.grab_focus();
        }
    });
    let sounds_on_key = sounds.clone();
    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    key.connect_key_pressed(move |controller, _, _, _| {
        if controller
            .widget()
            .and_downcast::<Terminal>()
            .is_some_and(|terminal| !password_input(&terminal))
        {
            sounds_on_key.play("stdin");
        }
        glib::Propagation::Proceed
    });
    session.terminal.add_controller(key);
    let sound_on_output = sounds.clone();
    let last_feedback = Cell::new(std::time::Instant::now());
    session.terminal.connect_contents_changed(move |terminal| {
        // Do not turn password-feedback characters into an audible side channel.
        if password_input(terminal) {
            return;
        }
        if last_feedback.get().elapsed() >= std::time::Duration::from_millis(140) {
            last_feedback.set(std::time::Instant::now());
            sound_on_output.play("stdout");
        }
    });
    spawn(
        &session.terminal,
        &session.pid,
        &status,
        &restart,
        &command,
        cwd,
    );
    session
}

fn spawn(
    terminal: &Terminal,
    pid: &Rc<Cell<Option<glib::Pid>>>,
    status: &gtk::Label,
    restart: &gtk::Button,
    (shell, args): &(String, Vec<String>),
    cwd: Option<&Path>,
) {
    status.set_text("Starting shell…");
    restart.set_visible(false);
    let pid = pid.clone();
    let status = status.clone();
    let restart = restart.clone();
    let shell_for_label = shell.clone();
    let mut argv = vec![shell.as_str()];
    if Path::new(shell)
        .file_name()
        .is_some_and(|name| name == "fish")
    {
        argv.extend(["--init-command", FISH_CWD_HOOK]);
    }
    argv.extend(args.iter().map(String::as_str));
    let cwd = cwd.and_then(Path::to_str);
    terminal.spawn_async(
        PtyFlags::DEFAULT,
        cwd,
        &argv,
        &["COLORTERM=truecolor", "TERM_PROGRAM=edex-native"],
        glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        None::<&gio::Cancellable>,
        move |result| match result {
            Ok(child) => {
                pid.set(Some(child));
                status.set_text(&shell_for_label);
            }
            Err(error) => {
                status.set_text(&format!("Shell failed: {error}"));
                restart.set_visible(true);
            }
        },
    );
}
