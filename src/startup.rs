use crate::audio::Sounds;
use std::{
    cell::Cell,
    process::Command,
    rc::Rc,
    time::{Duration, Instant},
};
use vte4::gtk::{self, cairo, gio, glib, prelude::*};

type Color = (f64, f64, f64);

pub fn intro(
    overlay: &gtk::Overlay,
    username: &str,
    color: Color,
    sounds: &Sounds,
    on_finish: impl Fn() + 'static,
) {
    let scene = gtk::Stack::new();
    scene.add_css_class("opening");
    scene.set_hexpand(true);
    scene.set_vexpand(true);
    scene.set_halign(gtk::Align::Fill);
    scene.set_valign(gtk::Align::Fill);
    let log = gtk::Label::builder().xalign(0.0).yalign(1.0).build();
    log.add_css_class("opening-log");
    scene.add_named(&log, Some("boot"));

    let display = gtk::DrawingArea::new();
    let greeting = if username.is_empty() {
        "Welcome back".into()
    } else {
        format!("Welcome back, {username}")
    };
    let frame = Rc::new(Cell::new(0.0));
    let logo_time = frame.clone();
    display.set_draw_func(move |_, cr, width, height| {
        draw_title(
            cr,
            f64::from(width),
            f64::from(height),
            logo_time.get(),
            color,
            &greeting,
        );
    });
    scene.add_named(&display, Some("title"));
    scene.set_visible_child_name("boot");
    overlay.add_overlay(&scene);

    let lines: Vec<_> = include_str!("../assets/boot_log.txt").lines().collect();
    let mut due = Duration::ZERO;
    let schedule: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let next = (due, *line);
            due += Duration::from_millis(boot_pause(index + 1, lines.len()));
            next
        })
        .collect();
    let boot_end = due + Duration::from_millis(300);
    let started = Instant::now();
    let mut cursor = 0;
    let mut shown_logo = false;
    let mut welcomed = false;
    let weak = scene.downgrade();
    let parent = overlay.downgrade();
    let sounds = sounds.clone();
    glib::timeout_add_local(Duration::from_millis(25), move || {
        let (Some(scene), Some(parent)) = (weak.upgrade(), parent.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        let elapsed = started.elapsed();
        if elapsed < boot_end {
            let before = cursor;
            while cursor < schedule.len() && schedule[cursor].0 <= elapsed {
                if schedule[cursor].1 == "Boot Complete" {
                    sounds.play("granted");
                }
                cursor += 1;
            }
            if before != cursor {
                let visible_lines = ((scene.height().max(240) - 16) / 17) as usize;
                log.set_text(&lines[cursor.saturating_sub(visible_lines)..cursor].join("\n"));
                sounds.play("stdout");
            }
        } else {
            if !shown_logo {
                shown_logo = true;
                scene.set_visible_child_name("title");
                sounds.play("theme");
            }
            let seconds = (elapsed - boot_end).as_secs_f64();
            if seconds >= 3.7 && !welcomed {
                welcomed = true;
                sounds.play("expand");
            }
            if seconds >= 4.65 {
                parent.remove_overlay(&scene);
                sounds.play("keyboard");
                on_finish();
                return glib::ControlFlow::Break;
            }
            frame.set(seconds);
            display.queue_draw();
        }
        glib::ControlFlow::Continue
    });
}

fn boot_pause(index: usize, lines: usize) -> u64 {
    match index {
        4 => 500,
        25 => 400,
        42 => 300,
        5..=24 => 30,
        43..=83 => 25,
        index if index >= lines.saturating_sub(2) && index < lines => 300,
        _ => ((1.0 - index as f64 / 1000.0).powi(3) * 25.0).max(1.0) as u64,
    }
}

fn draw_title(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    seconds: f64,
    (r, g, b): Color,
    greeting: &str,
) {
    if seconds < 0.45 {
        return;
    }
    let welcome = seconds >= 3.7;
    let title = if welcome { greeting } else { "eDEX-Remake" };
    cr.select_font_face(
        "United Sans Rg Md",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Bold,
    );
    cr.set_font_size((height * if welcome { 0.058 } else { 0.1 }).max(28.0));
    let Ok(extents) = cr.text_extents(title) else {
        return;
    };
    let left = (width - extents.width()) * 0.5;
    let top = (height - extents.height()) * 0.5;
    let x = left - extents.x_bearing();
    let y = top - extents.y_bearing();
    let pulse = ((seconds * 24.0) as i32 % 2) as f64;
    let pad = height * 0.016;
    if (1.25..1.79).contains(&seconds) {
        for (section, offset) in [(0, -(0.012 + pulse * 0.032)), (1, 0.013 + pulse * 0.022)] {
            let _ = cr.save();
            let boundary = top + extents.height() * 0.40;
            let (start, pane) = if section == 0 {
                (top - 3.0, boundary - top + 3.0)
            } else {
                (boundary, extents.height() + top - boundary + 3.0)
            };
            cr.rectangle(0.0, start, width, pane);
            cr.clip();
            cr.set_source_rgba(r, g, b, 0.9);
            cr.move_to(x + extents.width() * offset, y);
            let _ = cr.show_text(title);
            let _ = cr.restore();
        }
        return;
    }
    let alpha = ((seconds - 0.45) / 0.30).clamp(0.0, 1.0);
    let filled = (0.79..1.13).contains(&seconds);
    cr.set_source_rgba(r, g, b, alpha);
    cr.set_line_width((height * 0.0046).max(3.0));
    if filled || ((1.13..3.7).contains(&seconds)) {
        cr.rectangle(
            left - pad,
            top - pad,
            extents.width() + 2.0 * pad,
            extents.height() + 2.0 * pad,
        );
        if filled {
            let _ = cr.fill();
            cr.set_source_rgb(0.025, 0.04, 0.055);
        } else {
            let _ = cr.stroke();
        }
    } else if !welcome {
        cr.move_to(left - pad, top + extents.height() + pad);
        cr.line_to(left + extents.width() + pad, top + extents.height() + pad);
        let _ = cr.stroke();
    }
    cr.move_to(x, y);
    let _ = cr.show_text(title);
}

pub fn release_notice(parent: &gtk::Box) {
    let (send, receive) = async_channel::bounded(1);
    let _ = std::thread::spawn(move || {
        let version = (|| {
            let output = Command::new("curl")
                .args([
                    "-fsS",
                    "--max-time",
                    "8",
                    "--max-filesize",
                    "250000",
                    "-H",
                    "Accept: application/vnd.github+json",
                    "https://api.github.com/repos/Ali-A-Alwahed/EdexUi-2026/releases/latest",
                ])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let data: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
            let tag = data["tag_name"].as_str()?;
            let version: Vec<_> = tag
                .trim_start_matches('v')
                .split('.')
                .map(str::parse::<u32>)
                .collect::<Result<_, _>>()
                .ok()?;
            (version.as_slice() > &[2026, 4, 22]).then(|| tag.to_owned())
        })();
        let _ = send.send_blocking(version);
    });
    let parent = parent.downgrade();
    glib::MainContext::default().spawn_local(async move {
        if let (Ok(Some(version)), Some(parent)) = (receive.recv().await, parent.upgrade()) {
            let notice =
                gtk::Button::with_label(&format!("EdexUi-2026 upstream release {version}"));
            notice.add_css_class("update-notice");
            notice.connect_clicked(|_| {
                let _ = gio::AppInfo::launch_default_for_uri(
                    "https://github.com/Ali-A-Alwahed/EdexUi-2026/releases/latest",
                    None::<&gio::AppLaunchContext>,
                );
            });
            parent.append(&notice);
        }
    });
}
