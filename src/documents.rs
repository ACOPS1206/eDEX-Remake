use std::{
    cell::Cell,
    path::Path,
    process::{Command, Stdio},
    rc::Rc,
};
use vte4::gtk::{self, gdk, glib, prelude::*};

pub fn pdf(path: &Path) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_vexpand(true);
    let image = gtk::Picture::new();
    image.set_vexpand(true);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.set_halign(gtk::Align::Center);
    let previous = gtk::Button::with_label("Previous Page");
    let next = gtk::Button::with_label("Next Page");
    let status = gtk::Label::new(Some("Opening PDF…"));
    controls.append(&previous);
    controls.append(&status);
    controls.append(&next);
    let page = Rc::new(Cell::new(1u32));
    let pages = Rc::new(Cell::new(None::<u32>));
    let open = {
        let page = page.clone();
        let pages = pages.clone();
        let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        let previous = previous.downgrade();
        let next = next.downgrade();
        let status = status.downgrade();
        let image = image.downgrade();
        move || {
            let current = page.get();
            if let Some(button) = previous.upgrade() {
                button.set_sensitive(false);
            }
            if let Some(button) = next.upgrade() {
                button.set_sensitive(false);
            }
            let path = path.clone();
            let total = pages.get();
            let (send, receive) = async_channel::bounded(1);
            let _ = std::thread::spawn(move || {
                let result = (|| {
                    let total = total.or_else(|| {
                        let output = Command::new("pdfinfo")
                            .env("LC_ALL", "C")
                            .arg(&path)
                            .output()
                            .ok()?;
                        let contents = String::from_utf8_lossy(&output.stdout);
                        contents.lines().find_map(|line| {
                            line.strip_prefix("Pages:")?.trim().parse::<u32>().ok()
                        })
                    });
                    let page = current.to_string();
                    let output = Command::new("pdftoppm")
                        .args([
                            "-png",
                            "-singlefile",
                            "-scale-to",
                            "1500",
                            "-f",
                            &page,
                            "-l",
                            &page,
                        ])
                        .arg(&path)
                        .stdin(Stdio::null())
                        .output()
                        .map_err(|error| error.to_string())?;
                    if !output.status.success() {
                        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
                    }
                    Ok((total, output.stdout))
                })();
                let _ = send.send_blocking(result);
            });
            let (previous, next, status, image, pages) = (
                previous.clone(),
                next.clone(),
                status.clone(),
                image.clone(),
                pages.clone(),
            );
            glib::MainContext::default().spawn_local(async move {
                let Ok(result) = receive.recv().await else {
                    return;
                };
                let Some(status) = status.upgrade() else {
                    return;
                };
                match result.and_then(|(total, data)| {
                    gdk::Texture::from_bytes(&glib::Bytes::from_owned(data))
                        .map(|texture| (total, texture))
                        .map_err(|error| error.to_string())
                }) {
                    Ok((total, texture)) => {
                        pages.set(total);
                        if let Some(image) = image.upgrade() {
                            image.set_paintable(Some(&texture));
                        }
                        status.set_text(&total.map_or_else(
                            || format!("PAGE {current}"),
                            |total| format!("{current} / {total}"),
                        ));
                        if let Some(button) = next.upgrade() {
                            button.set_sensitive(total.is_none_or(|total| current < total));
                        }
                    }
                    Err(error) => status.set_text(&error),
                }
                if let Some(button) = previous.upgrade() {
                    button.set_sensitive(current > 1);
                }
            });
        }
    };
    let prev_page = page.clone();
    let open_previous = open.clone();
    previous.connect_clicked(move |_| {
        prev_page.set(prev_page.get().saturating_sub(1).max(1));
        open_previous();
    });
    let next_page = page;
    let open_next = open.clone();
    next.connect_clicked(move |_| {
        next_page.set(next_page.get().saturating_add(1));
        open_next();
    });
    content.append(&image);
    content.append(&controls);
    open();
    content
}

pub fn font(path: &Path) -> Option<gtk::Widget> {
    use std::os::unix::ffi::OsStrExt;
    let path = path.canonicalize().ok()?;
    let output = Command::new("fc-scan")
        .args(["--format", "%{family[0]}"])
        .arg(&path)
        .output()
        .ok()?;
    let family = std::str::from_utf8(&output.stdout).ok()?;
    if !output.status.success() || family.is_empty() {
        return None;
    }
    crate::theme::register_font_bytes(path.as_os_str().as_bytes());
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 9);
    let title = gtk::Label::new(Some(family));
    panel.append(&title);
    let preview = gtk::Label::new(Some(
        "The quick brown fox jumps over the lazy dog.\n0123456789  !?  []  {}  @  #",
    ));
    let mut font = gtk::pango::FontDescription::new();
    font.set_family(family);
    font.set_size(24 * gtk::pango::SCALE);
    let attrs = gtk::pango::AttrList::new();
    attrs.insert(gtk::pango::AttrFontDesc::new(&font));
    preview.set_attributes(Some(&attrs));
    preview.set_wrap(true);
    preview.set_vexpand(true);
    panel.append(&preview);
    Some(panel.upcast())
}
