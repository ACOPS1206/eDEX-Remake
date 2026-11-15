use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};
use vte4::gtk::{DrawingArea, cairo::Context, prelude::*};

const HISTORY: usize = 90;
type Color = (f64, f64, f64);

#[derive(Clone)]
pub struct Chart {
    pub view: DrawingArea,
    samples: Rc<RefCell<VecDeque<f64>>>,
    color: Rc<Cell<Color>>,
}

impl Chart {
    pub fn new(height: i32, accent: Color) -> Self {
        let view = DrawingArea::builder()
            .content_height(height)
            .hexpand(true)
            .build();
        let samples = Rc::new(RefCell::new(VecDeque::with_capacity(HISTORY)));
        let history = samples.clone();
        let color = Rc::new(Cell::new(accent));
        let palette = color.clone();
        view.set_draw_func(move |_, cr, width, height| {
            let (width, height) = (f64::from(width), f64::from(height));
            let accent = palette.get();
            cr.set_source_rgba(accent.0, accent.1, accent.2, 0.025);
            let _ = cr.paint();
            if width < 6.0 || height < 6.0 {
                return;
            }
            cr.set_source_rgba(accent.0, accent.1, accent.2, 0.14);
            cr.set_line_width(1.0);
            for i in 1..4 {
                let y = f64::from(i) * height / 4.0;
                cr.move_to(0.0, y);
                cr.line_to(width, y);
            }
            for i in 1..6 {
                let x = f64::from(i) * width / 6.0;
                cr.move_to(x, 0.0);
                cr.line_to(x, height);
            }
            let _ = cr.stroke();

            let history = history.borrow();
            let rect = (width - 4.0, height - 4.0);
            line(cr, &history, rect, 100.0, accent);
        });
        Self {
            view,
            samples,
            color,
        }
    }

    pub fn set_accent(&self, color: Color) {
        self.color.set(color);
        self.view.queue_draw();
    }

    pub fn push(&self, value: f64) {
        let mut samples = self.samples.borrow_mut();
        if samples.len() == HISTORY {
            samples.pop_front();
        }
        samples.push_back(value);
        self.view.queue_draw();
    }
}

fn line(
    cr: &Context,
    data: &VecDeque<f64>,
    (width, height): (f64, f64),
    maximum: f64,
    (r, g, b): Color,
) {
    if data.is_empty() {
        return;
    }
    let step = width / (HISTORY - 1) as f64;
    let start = 2.0 + width - (data.len() - 1) as f64 * step;
    let bottom = height + 2.0;
    let draw_path = || {
        for (index, &value) in data.iter().enumerate() {
            let x = start + index as f64 * step;
            let y = bottom - (value / maximum).clamp(0.0, 1.0) * height;
            if index == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
    };
    cr.new_path();
    draw_path();
    cr.line_to(width + 2.0, bottom);
    cr.line_to(start, bottom);
    cr.close_path();
    cr.set_source_rgba(r, g, b, 0.12);
    let _ = cr.fill();

    draw_path();
    cr.set_source_rgb(r, g, b);
    cr.set_line_width(1.7);
    let _ = cr.stroke();
}

#[derive(Clone)]
pub struct MemoryMap {
    pub view: DrawingArea,
    value: Rc<Cell<(f64, f64)>>,
    color: Rc<Cell<Color>>,
}

impl MemoryMap {
    pub fn new(accent: Color) -> Self {
        let view = DrawingArea::builder()
            .content_height(57)
            .hexpand(true)
            .build();
        let value = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
        let state = value.clone();
        let color = Rc::new(Cell::new(accent));
        let palette = color.clone();
        let mut order = std::array::from_fn::<_, 440, _>(|index| index);
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |duration| duration.subsec_nanos() | 1);
        for index in (1..order.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            order.swap(index, seed as usize % (index + 1));
        }
        view.set_draw_func(move |_, cr, width, height| {
            let (r, g, b) = palette.get();
            let dx = (f64::from(width) - 10.0).max(0.0) / 40.0;
            let dy = (f64::from(height) - 10.0).max(0.0) / 11.0;
            let (used, reclaimable) = state.get();
            let used = (used * 4.4).round().clamp(0.0, 440.0) as usize;
            let available = (used + (reclaimable * 4.4).round() as usize).min(440);
            for (first, last, alpha) in [
                (0, used, 1.0),
                (used, available, 0.3),
                (available, 440, 0.1),
            ] {
                cr.set_source_rgba(r, g, b, alpha);
                for &dot in &order[first..last] {
                    let x = 5.0 + (dot / 11) as f64 * dx;
                    let y = 4.0 + (dot % 11) as f64 * dy;
                    cr.rectangle(x, y, 2.16_f64.min(dx), 2.7_f64.min(dy));
                }
                let _ = cr.fill();
            }
        });
        Self { view, value, color }
    }

    pub fn set_accent(&self, color: Color) {
        self.color.set(color);
        self.view.queue_draw();
    }

    pub fn update(&self, used: f64, reclaimable: f64) {
        self.value
            .set((used.clamp(0.0, 100.0), reclaimable.clamp(0.0, 100.0)));
        self.view.queue_draw();
    }
}
