use crate::{geo::Position, theme::Preferences};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    f64::consts::{PI, TAU},
    rc::Rc,
    sync::LazyLock,
    time::{Duration, Instant},
};
use vte4::gtk::{DrawingArea, cairo, glib, prelude::*};

type Color = (f64, f64, f64);
type Point = [f64; 3];
const CAMERA_DISTANCE: f64 = 1700.0 / 1.1 / 500.0;
const VIEW_ANGLE: f64 = 0.630;
const INTRO_DURATION: f64 = 2.0;
const INTRO_ALTITUDE: f64 = 1.1;
const TILE_SHADE_COUNT: usize = 9;
type TerrainPaths = [Vec<(&'static Tile, f64)>; TILE_SHADE_COUNT * 8];

struct Tile {
    center: Point,
    outline: Vec<Point>,
    longitude: f64,
    shade: usize,
}

static TILES: LazyLock<Vec<Tile>> = LazyLock::new(|| {
    let world: Value =
        serde_json::from_str(include_str!("../assets/grid.json")).expect("upstream globe grid");
    world["tiles"]
        .as_array()
        .expect("upstream tiles")
        .iter()
        .enumerate()
        .filter_map(|(index, tile)| {
            let outline: Vec<_> = tile["b"]
                .as_array()?
                .iter()
                .filter_map(|edge| {
                    Some([
                        edge["x"].as_f64()? / 500.0,
                        edge["y"].as_f64()? / 500.0,
                        edge["z"].as_f64()? / 500.0,
                    ])
                })
                .collect();
            if outline.len() < 3 {
                return None;
            }
            let count = outline.len() as f64;
            let center = outline
                .iter()
                .fold([0.0; 3], |sum, point| {
                    [
                        (sum[0] + point[0]),
                        (sum[1] + point[1]),
                        (sum[2] + point[2]),
                    ]
                })
                .map(|axis| axis / count);
            Some(Tile {
                center,
                outline,
                longitude: tile["lon"].as_f64()?,
                shade: (index as u64).wrapping_mul(0x9e37_79b9) as usize % TILE_SHADE_COUNT,
            })
        })
        .collect()
});

#[derive(Clone, Copy)]
struct Palette {
    land: [Color; TILE_SHADE_COUNT],
    marker: Color,
    pin: Color,
    satellite: Color,
}

impl Palette {
    fn from_theme(theme: &Preferences) -> Self {
        let base = theme.globe_color("base");
        let base = if base.0 + base.1 + base.2 < 0.1 {
            theme.accent()
        } else {
            base
        };
        Self {
            land: std::array::from_fn(|index| {
                let saturation = [1.0, 0.65, 0.3][index / 3];
                let value = [0.97, 0.60, 0.30][index % 3];
                let channels = [base.0, base.1, base.2];
                let high = channels.iter().copied().fold(0.0_f64, f64::max);
                let low = channels.iter().copied().fold(1.0_f64, f64::min);
                let chroma = high - low;
                let mapped = channels.map(|channel| {
                    let hue = if chroma > 0.001 {
                        (channel - low) / chroma
                    } else {
                        1.0
                    };
                    let saturation = if high < 0.001 {
                        0.0
                    } else {
                        saturation * chroma / high
                    };
                    (1.0 - saturation * (1.0 - hue)) * value
                });
                (mapped[0], mapped[1], mapped[2])
            }),
            marker: theme.globe_color("marker"),
            pin: theme.globe_color("pin"),
            satellite: theme.globe_color("satellite"),
        }
    }
}

struct Peer {
    position: Position,
    first_seen: Instant,
}

#[derive(Default)]
struct Markers {
    peers: Vec<Peer>,
    endpoint: Option<Position>,
}

#[derive(Clone)]
pub struct Globe {
    pub view: DrawingArea,
    markers: Rc<RefCell<Markers>>,
    palette: Rc<Cell<Palette>>,
}

impl Globe {
    pub fn new(theme: &Preferences) -> Self {
        let view = DrawingArea::builder()
            .content_height(304)
            .hexpand(true)
            .build();
        let started = Instant::now();
        let palette = Rc::new(Cell::new(Palette::from_theme(theme)));
        let colors = palette.clone();
        let markers = Rc::new(RefCell::new(Markers::default()));
        let shared = markers.clone();
        // Parse the immutable terrain once before GTK requests its first frame.
        LazyLock::force(&TILES);
        let paths = RefCell::new(std::array::from_fn(|_| Vec::new()));
        view.set_draw_func(move |_, cr, width, height| {
            let seconds = started.elapsed().as_secs_f64();
            let camera = Camera::orbit(f64::from(width), f64::from(height), seconds);
            let colors = colors.get();
            draw_terrain(cr, &camera, &colors, seconds, &mut paths.borrow_mut());
            draw_scan(cr, &camera, colors.marker, seconds);
            let state = shared.borrow();
            for peer in &state.peers {
                let grow = (peer.first_seen.elapsed().as_secs_f64() / 0.9).min(1.0);
                draw_marker(cr, &camera, peer.position, colors.pin, grow, None);
            }
            if let Some(home) = state.endpoint {
                draw_marker(cr, &camera, home, colors.marker, 1.0, Some(seconds.fract()));
            }
            draw_constellation(cr, &camera, colors.satellite, seconds);
        });
        let weak = view.downgrade();
        // Keep steady animation at 12 FPS, with a faster sweep for the short intro.
        let mut draw_steady = false;
        glib::timeout_add_local(Duration::from_millis(42), move || {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            draw_steady = !draw_steady;
            if view.is_mapped() && (started.elapsed().as_secs_f64() < 2.5 || draw_steady) {
                view.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        Self {
            view,
            markers,
            palette,
        }
    }

    pub fn apply_theme(&self, theme: &Preferences) {
        self.palette.set(Palette::from_theme(theme));
        self.view.queue_draw();
    }

    pub fn update(&self, mut peers: Vec<Position>, endpoint: Option<Position>) {
        let mut state = self.markers.borrow_mut();
        state.peers.retain(|peer| {
            if let Some(index) = peers.iter().position(|position| *position == peer.position) {
                peers.swap_remove(index);
                true
            } else {
                false
            }
        });
        state.peers.extend(peers.into_iter().map(|position| Peer {
            position,
            first_seen: Instant::now(),
        }));
        state.endpoint = endpoint;
        self.view.queue_draw();
    }
}

struct Camera {
    right: Point,
    up: Point,
    forward: Point,
    center: (f64, f64),
    focal: f64,
}

impl Camera {
    fn orbit(width: f64, height: f64, seconds: f64) -> Self {
        let (sin, cos) = (PI + TAU * seconds / 45.0).sin_cos();
        let (tilt_sin, tilt_cos) = VIEW_ANGLE.sin_cos();
        Self {
            right: [sin, 0.0, -cos],
            up: [-cos * tilt_sin, tilt_cos, -sin * tilt_sin],
            forward: [cos * tilt_cos, tilt_sin, sin * tilt_cos],
            center: (width / 2.0, height / 2.0),
            focal: width.min(height) / (2.0 * 25.0_f64.to_radians().tan()),
        }
    }

    fn project(&self, world: Point, altitude: f64) -> (f64, f64) {
        let distance = CAMERA_DISTANCE - dot(world, self.forward) * altitude;
        let scale = self.focal * altitude / distance;
        (
            self.center.0 + dot(world, self.right) * scale,
            self.center.1 - dot(world, self.up) * scale,
        )
    }
}

fn dot(left: Point, right: Point) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn point((lat, lon): Position) -> Point {
    let lat = lat.to_radians();
    let lon = lon.to_radians();
    [-lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin()]
}

// The upstream vertex shader reveals each longitude in turn, starting above
// the surface before transitioning it down over 200 ms.
fn terrain_phase(seconds: f64, longitude: f64) -> Option<(f64, f64)> {
    let elapsed = seconds - INTRO_DURATION * ((180.0 + longitude) / 360.0);
    let linger = INTRO_DURATION / 8.0;
    if elapsed <= 0.0 {
        None
    } else if elapsed < linger {
        Some((INTRO_ALTITUDE, 0.3))
    } else {
        Some((
            1.0 + (INTRO_ALTITUDE - 1.0) * (1.0 - (elapsed - linger) / 0.2).max(0.0),
            1.0,
        ))
    }
}

fn visibility(depth: f64) -> f64 {
    let fog = (-depth / 0.6).clamp(0.0, 1.0);
    1.0 - fog * fog * (3.0 - 2.0 * fog)
}

fn draw_terrain(
    cr: &cairo::Context,
    camera: &Camera,
    colors: &Palette,
    seconds: f64,
    paths: &mut TerrainPaths,
) {
    // Batch by shade and fog layer to avoid a separate Cairo fill for each tile.
    // Far-side polygons are always painted first.
    paths.iter_mut().for_each(Vec::clear);
    for tile in TILES.iter() {
        let depth = dot(tile.center, camera.forward);
        let Some((altitude, opacity)) = terrain_phase(seconds, tile.longitude) else {
            continue;
        };
        let alpha = (visibility(depth * altitude) * opacity * 4.0).round() as usize;
        if alpha > 0 {
            let layer = if depth < 0.0 { 0 } else { 4 };
            paths[(layer + alpha - 1) * TILE_SHADE_COUNT + tile.shade].push((tile, altitude));
        }
    }
    for (index, path) in paths
        .iter()
        .enumerate()
        .filter(|(_, path)| !path.is_empty())
    {
        let (r, g, b) = colors.land[index % TILE_SHADE_COUNT];
        let alpha = (index / TILE_SHADE_COUNT % 4 + 1) as f64 / 4.0;
        cr.set_source_rgba(r, g, b, alpha);
        for &(tile, altitude) in path {
            for (index, vertex) in tile.outline.iter().enumerate() {
                let (x, y) = camera.project(*vertex, altitude);
                if index == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            cr.close_path();
        }
        let _ = cr.fill();
    }
}

fn draw_scan(cr: &cairo::Context, camera: &Camera, color: Color, seconds: f64) {
    if seconds >= INTRO_DURATION {
        return;
    }
    let progress = seconds / INTRO_DURATION;
    let (r, g, b) = color;
    cr.set_source_rgba(
        r,
        g,
        b,
        (progress * 10.0).min((1.0 - progress) * 5.0).min(1.0),
    );
    cr.set_line_width(1.2);
    for line in 0..60 {
        let lat = -86.0 + line as f64 * 2.8;
        let length = 4 + line % 5;
        let offset = ((line * 37) % 30) as f64 - 15.0;
        let mut started = false;
        for vertex in 0..length {
            let world = point((
                lat,
                -180.0 + progress * 360.0 + offset - vertex as f64 * 5.0,
            ));
            if visibility(dot(world, camera.forward)) < 0.4 {
                started = false;
                continue;
            }
            let (x, y) = camera.project(world, INTRO_ALTITUDE);
            if started {
                cr.line_to(x, y);
            } else {
                cr.move_to(x, y);
            }
            started = true;
        }
    }
    let _ = cr.stroke();
}

fn draw_marker(
    cr: &cairo::Context,
    camera: &Camera,
    position: Position,
    color: Color,
    grow: f64,
    pulse: Option<f64>,
) {
    let world = point(position);
    let alpha = visibility(dot(world, camera.forward));
    if alpha <= 0.05 {
        return;
    }
    let (r, g, b) = color;
    let (vx, vy) = camera.project(world, 1.0);
    let (px, py) = camera.project(world, 1.0 + grow * 0.2);
    cr.set_line_width(1.1);
    cr.set_source_rgba(r, g, b, alpha * 0.9);
    cr.move_to(vx, vy);
    cr.line_to(px, py);
    let _ = cr.stroke();
    cr.arc(px, py, if pulse.is_some() { 3.8 } else { 2.1 }, 0.0, TAU);
    let _ = cr.fill();
    if let Some(progress) = pulse {
        cr.set_source_rgba(r, g, b, alpha * (1.0 - progress));
        cr.arc(px, py, 5.0 + progress * 10.0, 0.0, TAU);
        let _ = cr.stroke();
    }
}

fn draw_constellation(cr: &cairo::Context, camera: &Camera, (r, g, b): Color, seconds: f64) {
    for band in 0..2 {
        for offset in 0..3 {
            let world = point((
                -25.0 + 50.0 * f64::from(band),
                -120.0 + 120.0 * f64::from(offset) + 30.0 * f64::from(band),
            ));
            let alpha = visibility(dot(world, camera.forward));
            if alpha < 0.15 {
                continue;
            }
            let (x, y) = camera.project(world, 1.32 + 0.15 * f64::from(offset));
            let tilt = (seconds * TAU / 45.0 + f64::from(offset) * 2.1).sin() * 0.75
                + if band == 0 { 1.25 } else { -0.70 };
            cr.set_line_width(0.85);
            cr.set_source_rgba(r, g, b, alpha);
            cr.arc(x, y, 2.1, 0.0, TAU);
            let _ = cr.stroke();
            for segment in 0..4 {
                let angle = tilt + f64::from(segment) * TAU / 4.0;
                cr.new_sub_path();
                cr.arc(x, y, 4.0, angle + 0.20, angle + 1.02);
            }
            let _ = cr.stroke();
            for wave in 0..7 {
                let phase = ((seconds + f64::from(wave) * 0.20) % 1.4) / 1.4;
                cr.set_source_rgba(r, g, b, alpha * (1.0 - phase) * 0.75);
                cr.arc(x, y, 5.0 + phase * 27.0, tilt - PI / 12.0, tilt + PI / 12.0);
                let _ = cr.stroke();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_reveals_tiles_by_longitude_and_lowers_them_to_the_surface() {
        assert_eq!(terrain_phase(0.5, 0.0), None);
        assert_eq!(terrain_phase(1.1, 0.0), Some((1.1, 0.3)));
        let (altitude, _) = terrain_phase(1.35, 0.0).unwrap();
        assert!((altitude - 1.05).abs() < 1e-9);
        assert_eq!(terrain_phase(2.45, 180.0), Some((1.0, 1.0)));
    }

    #[test]
    fn camera_rotates_in_one_original_day_and_far_side_fades() {
        let start = Camera::orbit(304.0, 304.0, 0.0);
        let complete = Camera::orbit(304.0, 304.0, 45.0);
        let world = point((17.0, 32.0));
        let (a, b) = start.project(world, 1.0);
        let (c, d) = complete.project(world, 1.0);
        assert!((a - c).abs() + (b - d).abs() < 1e-8);
        assert_eq!(visibility(0.5), 1.0);
        assert_eq!(visibility(-0.6), 0.0);
        assert_eq!(visibility(-0.3), 0.5);
    }
}
