use crate::{audio::Sounds, theme::Preferences};
use std::{
    cell::{Cell, RefCell},
    env, fs,
    path::{Path, PathBuf},
    rc::Rc,
};
use vte4::gtk::{self, gio, glib, prelude::*};
use vte4::{PtyFlags, Terminal, prelude::*};

#[derive(Clone)]
struct Session {
    terminal: Terminal,
    pid: Rc<Cell<Option<glib::Pid>>>,
}

#[derive(Clone)]
pub struct Terminals {
    pub view: gtk::Notebook,
    pub bar: gtk::Box,
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
        let sessions = Rc::new(RefCell::new(vec![None; 5]));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.set_homogeneous(true);
        bar.add_css_class("tab-bar");
        let tabs = Self {
            view,
            bar,
            sessions,
            preferences: Rc::new(RefCell::new(settings.clone())),
        };
        let mut buttons = Vec::<gtk::ToggleButton>::with_capacity(5);
        for number in 0..5 {
            let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let title = if number == 0 {
                "Main".to_owned()
            } else {
                number.to_string()
            };
            tabs.view.append_page(&placeholder, None::<&gtk::Widget>);
            let button = gtk::ToggleButton::with_label(&title);
            button.add_css_class("shell-tab");
            button.set_focusable(false);
            if let Some(first) = buttons.first() {
                button.set_group(Some(first));
            }
            let notebook = tabs.view.downgrade();
            let sessions = tabs.sessions.clone();
            let on_tab = on_tab.clone();
            button.connect_clicked(move |_| {
                on_tab();
                if let Some(notebook) = notebook.upgrade() {
                    notebook.set_current_page(Some(number));
                    if let Some(session) = sessions
                        .borrow()
                        .get(number as usize)
                        .and_then(Option::as_ref)
                    {
                        session.terminal.grab_focus();
                    }
                }
            });
            tabs.bar.append(&button);
            buttons.push(button);
        }
        buttons[0].set_active(true);
        let buttons: Vec<_> = buttons.iter().map(|button| button.downgrade()).collect();
        let slots = tabs.sessions.clone();
        let preferences = tabs.preferences.clone();
        let tab_sounds = sounds.clone();
        tabs.view.connect_switch_page(move |notebook, page, index| {
            if let Some(button) = buttons[index as usize].upgrade() {
                button.set_active(true);
            }
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
        let (Some(pid), Some(pty)) = (session.pid.get(), session.terminal.pty()) else {
            return false;
        };
        // TIOCGPGRP on the PTY master identifies the process group receiving keystrokes.
        let (foreground, shell_group) = unsafe {
            (
                libc::tcgetpgrp(std::os::fd::AsRawFd::as_raw_fd(&pty.fd())),
                libc::getpgid(pid.0),
            )
        };
        if foreground <= 0 || foreground != shell_group {
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

impl Session {
    fn cwd(&self) -> Option<PathBuf> {
        let pid = self.pid.get()?;
        fs::read_link(format!("/proc/{}/cwd", pid.0))
            .ok()
            .or_else(|| {
                self.terminal
                    .current_directory_uri()
                    .and_then(|uri| gio::File::for_uri(&uri).path())
            })
    }
}

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
                "\x1b[1mWelcome to EdexUi-2026 - native port ",
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
    key.connect_key_pressed(move |_, _, _, _| {
        sounds_on_key.play("stdin");
        glib::Propagation::Proceed
    });
    session.terminal.add_controller(key);
    let sound_on_output = sounds.clone();
    let last_feedback = Cell::new(std::time::Instant::now());
    session.terminal.connect_contents_changed(move |_| {
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
    let argv = std::iter::once(shell.as_str())
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>();
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
