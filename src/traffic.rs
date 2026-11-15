use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    time::{Duration, Instant},
};
use vte4::gtk::{self, cairo::Context, glib, prelude::*};

type Color = (f64, f64, f64);
const SECONDS_PER_PIXEL: f64 = 0.08;
const GRID_INTERVAL: f64 = 5.0;
const PLOT_DELAY: f64 = 1.2;

#[derive(Clone, Copy)]
struct Sample {
    time: f64,
    upload: f64,
    download: f64,
}

#[derive(Clone)]
pub struct Traffic {
    pub view: gtk::DrawingArea,
    data: Rc<RefCell<VecDeque<Sample>>>,
    online: Rc<Cell<bool>>,
    accent: Rc<Cell<Color>>,
    start: Instant,
}

impl Traffic {
    pub fn new(color: Color) -> Self {
        let view = gtk::DrawingArea::builder()
            .content_height(172)
            .hexpand(true)
            .build();
        let this = Self {
            view,
            data: Rc::new(RefCell::new(VecDeque::with_capacity(120))),
            online: Rc::new(Cell::new(false)),
            accent: Rc::new(Cell::new(color)),
            start: Instant::now(),
        };
        let (data, online, accent, start) = (
            this.data.clone(),
            this.online.clone(),
            this.accent.clone(),
            this.start,
        );
        this.view.set_draw_func(move |_, cr, width, height| {
            let edge = start.elapsed().as_secs_f64() - PLOT_DELAY;
            draw(
                cr,
                f64::from(width),
                f64::from(height),
                edge,
                &data.borrow(),
                online.get(),
                accent.get(),
            );
        });
        let view = this.view.downgrade();
        let online = this.online.clone();
        glib::timeout_add_local(Duration::from_millis(67), move || {
            let Some(view) = view.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if online.get() && view.is_mapped() {
                view.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        this
    }

    pub fn set_accent(&self, color: Color) {
        self.accent.set(color);
        self.view.queue_draw();
    }

    pub fn set_online(&self, online: bool) {
        if self.online.replace(online) != online {
            self.view.queue_draw();
        }
    }

    pub fn clear(&self) {
        self.data.borrow_mut().clear();
        self.view.queue_draw();
    }

    pub fn push(&self, tx_bytes_per_second: f64, rx_bytes_per_second: f64) {
        let mut data = self.data.borrow_mut();
        if data.len() == 120 {
            data.pop_front();
        }
        data.push_back(Sample {
            time: self.start.elapsed().as_secs_f64(),
            upload: megabits(tx_bytes_per_second),
            download: megabits(rx_bytes_per_second),
        });
        self.view.queue_draw();
    }
}

pub fn megabits(bytes_per_second: f64) -> f64 {
    bytes_per_second / 125_000.0
}

pub fn total_bytes(bytes: u64) -> String {
    let unit = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
    let mut amount = bytes as f64;
    let mut index = 0;
    while amount >= 1000.0 && index < unit.len() - 1 {
        amount /= 1000.0;
        index += 1;
    }
    let precision = if index == 0 || amount >= 100.0 {
        0
    } else if amount >= 10.0 {
        1
    } else {
        2
    };
    let text = format!("{amount:.precision$}");
    let value = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    format!("{value} {}", unit[index])
}

fn draw(
    cr: &Context,
    width: f64,
    height: f64,
    edge: f64,
    samples: &VecDeque<Sample>,
    online: bool,
    (r, g, b): Color,
) {
    let center = (height / 2.0).floor() + 0.5;
    let dim = if online { 1.0 } else { 0.3 };
    let x_at = |time: f64| width - (edge - time) / SECONDS_PER_PIXEL;
    let first = edge - width * SECONDS_PER_PIXEL;
    let latest_grid = (edge / GRID_INTERVAL).floor() * GRID_INTERVAL;
    cr.set_line_width(1.0);
    cr.set_source_rgba(r, g, b, 0.4 * dim);
    cr.set_dash(&[2.0, 2.0], 0.0);
    for y in [0.5, height - 0.5] {
        cr.move_to(0.0, y);
        cr.line_to(width, y);
    }
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    for section in 1..6 {
        let y = if section == 3 {
            center
        } else {
            (height * f64::from(section) / 6.0).floor() + 0.5
        };
        cr.move_to(0.0, y);
        cr.line_to(width, y);
    }
    let mut grid = latest_grid;
    while grid >= first {
        let x = x_at(grid).floor() + 0.5;
        cr.move_to(x, 0.0);
        cr.line_to(x, height);
        grid -= GRID_INTERVAL;
    }
    let _ = cr.stroke();
    let max = samples
        .iter()
        .filter(|sample| sample.time >= first - 1.0)
        .fold(0.0_f64, |max, point| {
            max.max(point.upload).max(point.download)
        });
    let scale = max.max(0.01);
    if online {
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(1.7);
        for upload in [true, false] {
            for (index, sample) in samples
                .iter()
                .filter(|sample| sample.time >= first - 2.0)
                .enumerate()
            {
                let (value, direction) = if upload {
                    (sample.upload, -1.0)
                } else {
                    (sample.download, 1.0)
                };
                let (x, y) = (
                    x_at(sample.time),
                    center + direction * (center - 4.0) * (value / scale).clamp(0.0, 1.0),
                );
                if index == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            let _ = cr.stroke();
        }
    }
    cr.select_font_face(
        "monospace",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );
    cr.set_source_rgba(r, g, b, dim);
    cr.set_font_size(10.0);
    for (label, y) in [
        (format!("{max:.2}"), 11.0),
        (
            format!("{:.2}", if max > 0.0 { -max } else { 0.0 }),
            height - 4.0,
        ),
    ] {
        if let Ok(boxed) = cr.text_extents(&label) {
            cr.move_to((width - boxed.width() - 3.0).max(0.0), y);
            let _ = cr.show_text(&label);
        }
    }
    if !online {
        cr.set_source_rgb(r, g, b);
        cr.set_font_size(19.0);
        if let Ok(boxed) = cr.text_extents("OFFLINE") {
            cr.move_to((width - boxed.width()) / 2.0, center + boxed.height() / 2.0);
            let _ = cr.show_text("OFFLINE");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plots_upload_above_download_below_the_shared_baseline() {
        use gtk::cairo::{Format, ImageSurface};
        let mut surface = ImageSurface::create(Format::ARgb32, 320, 172).unwrap();
        let cr = Context::new(&surface).unwrap();
        let mut samples = VecDeque::new();
        for time in 0..24 {
            // 2 Mbps uploads reach the top of the shared scale. 1 Mbps downloads reach its midpoint.
            samples.push_back(Sample {
                time: f64::from(time),
                upload: 2.0,
                download: 1.0,
            });
        }
        render_for_test(&cr, &samples);
        drop(cr);
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let alpha = |x: usize, y: usize| data[y * stride + x * 4 + 3];
        assert!(alpha(150, 4) > 120 && alpha(150, 127) > 120);
        assert!(alpha(150, 166) < 120);
    }

    fn render_for_test(cr: &Context, samples: &VecDeque<Sample>) {
        draw(cr, 320.0, 172.0, 23.0, samples, true, (1.0, 1.0, 1.0));
    }

    #[test]
    fn uses_upstream_megabit_scale_and_decimal_transfer_totals() {
        assert_eq!(megabits(250_000.0), 2.0);
        assert_eq!(total_bytes(1_250_000), "1.25 MB");
        assert_eq!(total_bytes(0), "0 B");
    }
}
