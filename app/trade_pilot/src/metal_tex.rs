//! Images of the metal design made pixel by pixel when they are first needed: satin steel plates, framed displays
//! with a sunken inside, the silver dial of the instruments and the page background. Each pixel is lit from the
//! upper left, so edges, rings and brushing look physical. No image files: the pictures are computed, kept per size
//! and only drawn afterwards.

// The pixel code converts pixel counts and colour values between integers and floats (none of the values comes near the
// limits) and uses the short names of the formulas (x, y, r, d for distance, c for centre).
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::many_single_char_names)]

use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, Shape, TextureHandle, TextureId, TextureOptions};
use std::cell::RefCell;
use std::collections::HashMap;

/// Room around a plate, a frame or a dial for its shadow, in points.
pub const SHADOW: f32 = 12.0;
/// Sizes are rounded up to this many points, so a card that grows a little reuses its picture.
const STEP: f32 = 8.0;
/// Where the chrome bezel of a dial ends inside, as part of its outer radius.
pub const BEZEL_INNER: f32 = 0.88;
/// The dark track for the fine marks at the rim of the face, as parts of the outer radius.
pub const TRACK_INNER: f32 = 0.80;
/// See `TRACK_INNER`.
pub const TRACK_OUTER: f32 = 0.87;
/// How far below the middle the flat lower part of the face begins, as part of the outer radius.
pub const CHORD: f32 = 0.62;
/// More pictures than this and the store starts over (sizes of windows that are gone).
const CACHE_LIMIT: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Plate,
    Frame,
    Dial,
    Page,
    Knob,
    Glow,
    Slot,
    Hub,
    Button,
    TopBar,
    Sidebar,
    Field,
}

thread_local! {
    static CACHE: RefCell<HashMap<(Kind, u32, u32, u32), TextureHandle>> = RefCell::new(HashMap::new());
}

type Rgb = [f32; 3];

fn rgb(hex: u32) -> Rgb {
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0]
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn add(c: Rgb, v: f32) -> Rgb {
    [c[0] + v, c[1] + v, c[2] + v]
}

fn scale(c: Rgb, v: f32) -> Rgb {
    [c[0] * v, c[1] * v, c[2] * v]
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A repeatable pseudo-random value in 0..1 for a pixel.
fn hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ 0x9e37_79b9;
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

/// Signed distance of a point to a rounded rectangle (centre `c`, half size `half`, corner radius `r`): below 0 inside.
fn rounded_sdf(p: [f32; 2], c: [f32; 2], half: [f32; 2], r: f32) -> f32 {
    let qx = (p[0] - c[0]).abs() - (half[0] - r);
    let qy = (p[1] - c[1]).abs() - (half[1] - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// The outward direction of the rounded rectangle at `p` (from the change of its distance).
fn rounded_normal(p: [f32; 2], c: [f32; 2], half: [f32; 2], r: f32) -> [f32; 2] {
    let dx = rounded_sdf([p[0] + 1.0, p[1]], c, half, r) - rounded_sdf([p[0] - 1.0, p[1]], c, half, r);
    let dy = rounded_sdf([p[0], p[1] + 1.0], c, half, r) - rounded_sdf([p[0], p[1] - 1.0], c, half, r);
    let len = (dx * dx + dy * dy).sqrt().max(1e-4);
    [dx / len, dy / len]
}

/// `top` laid over `bottom` (straight alpha).
fn over(top: (Rgb, f32), bottom: (Rgb, f32)) -> (Rgb, f32) {
    let alpha = top.1 + bottom.1 * (1.0 - top.1);
    if alpha <= 0.0 {
        return ([0.0; 3], 0.0);
    }
    let channel = |a: f32, b: f32| (a * top.1 + b * bottom.1 * (1.0 - top.1)) / alpha;
    let ([r1, g1, b1], [r2, g2, b2]) = (top.0, bottom.0);
    ([channel(r1, r2), channel(g1, g2), channel(b1, b2)], alpha)
}

/// The soft shadow a shape throws below itself: `distance` is the distance of the pixel to the shape moved down a little.
fn shadow(distance: f32, px: f32, strength: f32) -> (Rgb, f32) {
    ([0.0; 3], strength * (1.0 - smoothstep(-2.0 * px, 11.0 * px, distance)))
}

/// The picture of `kind` at `size` points (already rounded) for `pixels_per_point`, made by `paint` the first time.
fn texture(
    ctx: &egui::Context,
    kind: Kind,
    size: egui::Vec2,
    pixels_per_point: f32,
    extra: u32,
    paint: &dyn Fn(f32, f32, f32, f32) -> (Rgb, f32),
) -> TextureId {
    let width = (size.x * pixels_per_point).round().max(1.0) as usize;
    let height = (size.y * pixels_per_point).round().max(1.0) as usize;
    let key = (kind, width as u32, height as u32, extra);
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(handle) = cache.get(&key) {
            return handle.id();
        }
        if cache.len() > CACHE_LIMIT {
            cache.clear();
        }
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let (color, alpha) = paint(x as f32 + 0.5, y as f32 + 0.5, width as f32, height as f32);
                let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                pixels.extend_from_slice(&[byte(color[0]), byte(color[1]), byte(color[2]), byte(alpha)]);
            }
        }
        let image = ColorImage::from_rgba_unmultiplied([width, height], &pixels);
        let handle = ctx.load_texture(format!("metal_{kind:?}_{width}x{height}_{extra}"), image, TextureOptions::LINEAR);
        let id = handle.id();
        cache.insert(key, handle);
        id
    })
}

fn rounded_up(size: egui::Vec2) -> egui::Vec2 {
    egui::vec2((size.x / STEP).ceil() * STEP, (size.y / STEP).ceil() * STEP)
}

fn image(id: TextureId, rect: Rect) -> Shape {
    Shape::image(id, rect, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE)
}

/// A raised plate of satin steel over `rect` with corner radius `radius`, its bevelled edge and its shadow.
pub fn plate(ctx: &egui::Context, rect: Rect, radius: f32) -> Shape {
    tinted_plate(ctx, rect, radius, Color32::WHITE)
}

/// A plate like `plate`, its colour multiplied by `tint` (grey for a darker plate).
pub fn tinted_plate(ctx: &egui::Context, rect: Rect, radius: f32, tint: Color32) -> Shape {
    satin(ctx, rect, radius, tint, (Kind::Plate, rgb(0xb2_ab_a1), rgb(0x5c_57_51)))
}

/// A satin plate: `look` is the kind of picture and the colours at its top and bottom.
fn satin(ctx: &egui::Context, rect: Rect, radius: f32, tint: Color32, look: (Kind, Rgb, Rgb)) -> Shape {
    let (kind, top_color, bottom_color) = look;
    let size = rounded_up(rect.size()) + egui::Vec2::splat(2.0 * SHADOW);
    let ppp = ctx.pixels_per_point();
    let id = texture(ctx, kind, size, ppp, radius as u32, &|x, y, w, h| {
        let m = SHADOW * ppp;
        let (c, half, r) = ([w / 2.0, h / 2.0], [w / 2.0 - m, h / 2.0 - m], radius * ppp);
        let d = rounded_sdf([x, y], c, half, r);
        let below = shadow(rounded_sdf([x, y - 3.0 * ppp], c, half, r), ppp, 0.6);
        if d > 1.0 {
            return below;
        }
        let from_top = y - (c[1] - half[1]);
        let t = from_top / (2.0 * half[1]);
        let across = (x - (c[0] - half[0])) / (2.0 * half[0]);
        let mut color = mix(top_color, bottom_color, t);
        color = add(color, 0.11 * (-((across - 0.3) / 0.4).powi(2)).exp() * (1.0 - t));
        let inside = -d;
        let bevel = 3.5 * ppp;
        if inside < bevel {
            let n = rounded_normal([x, y], c, half, r);
            let k = 1.0 - inside / bevel;
            color = add(color, k * (-n[1] * 0.3 - n[0] * 0.08));
        }
        color = mix(color, rgb(0x14_13_12), smoothstep(-1.6 * ppp, -0.2 * ppp, d) * 0.85);
        color = add(color, (hash(x as u32, y as u32) - 0.5) * 0.012);
        over((color, (0.5 - d).clamp(0.0, 1.0)), below)
    });
    Shape::image(id, rect.expand(SHADOW), Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), tint)
}

/// A framed display over `rect`: a thick rounded metal frame with a light upper edge and a dark, sunken inside.
pub fn frame(ctx: &egui::Context, rect: Rect, radius: f32) -> Shape {
    let size = rounded_up(rect.size()) + egui::Vec2::splat(2.0 * SHADOW);
    let ppp = ctx.pixels_per_point();
    let id = texture(ctx, Kind::Frame, size, ppp, radius as u32, &|x, y, w, h| {
        let m = SHADOW * ppp;
        let (c, half, r) = ([w / 2.0, h / 2.0], [w / 2.0 - m, h / 2.0 - m], radius * ppp);
        let d = rounded_sdf([x, y], c, half, r);
        let below = shadow(rounded_sdf([x, y - 3.0 * ppp], c, half, r), ppp, 0.65);
        if d > 1.0 {
            return below;
        }
        let band = 6.0 * ppp;
        let inside = -d;
        let t = (y - (c[1] - half[1])) / (2.0 * half[1]);
        let color = if inside < band {
            // The frame: a rounded bar, lit from above (its outer half faces out, its inner half faces in).
            let p = inside / band;
            let slope = (std::f32::consts::PI * p).cos();
            let n = rounded_normal([x, y], c, half, r);
            let lit = -n[1] * slope * 0.9 - n[0] * slope * 0.25;
            let mut color = mix(rgb(0x86_82_7d), rgb(0x4a_47_44), t);
            color = add(color, lit * 0.33 + lit.max(0.0).powi(6) * 0.25);
            color = scale(color, 1.0 - 0.55 * (1.0 - smoothstep(0.0, 0.14, p)) - 0.65 * smoothstep(0.86, 1.0, p));
            color
        } else {
            // The sunken inside: dark, shaded most just below the upper edge of the frame.
            let depth = inside - band;
            let top_edge = y - (c[1] - half[1] + band);
            let mut color = mix(rgb(0x1c_1b_1a), rgb(0x29_28_26), t);
            color = scale(color, 1.0 - 0.5 * (1.0 - smoothstep(0.0, 12.0 * ppp, top_edge)));
            color = scale(color, 1.0 - 0.25 * (1.0 - smoothstep(0.0, 6.0 * ppp, depth)));
            add(color, (hash(x as u32, y as u32) - 0.5) * 0.01)
        };
        let color = mix(color, rgb(0x0c_0b_0b), smoothstep(-1.4 * ppp, -0.2 * ppp, d) * 0.85);
        over((color, (0.5 - d).clamp(0.0, 1.0)), below)
    });
    image(id, rect.expand(SHADOW))
}

/// Brightness of a value that changes with the angle (degrees, 0 = right, 90 = down): a smooth curve through
/// `table` (angle, brightness), which runs from -180 to 180.
fn by_angle(table: &[(f32, f32)], degrees: f32) -> f32 {
    for pair in table.windows(2) {
        if let [(a0, v0), (a1, v1)] = pair {
            if degrees >= *a0 && degrees <= *a1 {
                let t = (degrees - a0) / (a1 - a0);
                let eased = (1.0 - (std::f32::consts::PI * t).cos()) / 2.0;
                return v0 + (v1 - v0) * eased;
            }
        }
    }
    table.first().map_or(0.5, |(_, v)| *v)
}

/// How bright the spun face is at an angle: lit from above, brightest towards the upper left, dark at the sides.
const FACE_LIGHT: [(f32, f32); 9] =
    [(-180.0, 0.38), (-135.0, 0.76), (-90.0, 0.79), (-45.0, 0.62), (0.0, 0.36), (45.0, 0.5), (90.0, 0.44), (135.0, 0.32), (180.0, 0.38)];
/// How bright the chrome bezel is at an angle: nearly white on top, middle at the sides.
const BEZEL_LIGHT: [(f32, f32); 9] =
    [(-180.0, 0.42), (-135.0, 0.9), (-90.0, 0.92), (-45.0, 0.62), (0.0, 0.42), (45.0, 0.8), (90.0, 0.6), (135.0, 0.3), (180.0, 0.42)];

/// The dial of an instrument around `center` with outer radius `outer`, and the shadow below it: a polished chrome
/// bezel lit from above; a plate of spun steel whose light turns with the angle, with a dark track for the fine marks
/// along the scale (from `TRACK_INNER` to `TRACK_OUTER`), ending in a straight, shining lower edge that meets the bezel in sharp corners
/// (at `CHORD` of `outer` below the middle); and inside the bezel below the plate a darker, matt housing.
pub fn dial(ctx: &egui::Context, center: Pos2, outer: f32) -> Shape {
    let side = (outer * 2.0 / STEP).ceil() * STEP + 2.0 * SHADOW;
    let ppp = ctx.pixels_per_point();
    let id = texture(ctx, Kind::Dial, egui::vec2(side, side), ppp, 3, &|x, y, w, _| {
        let m = SHADOW * ppp;
        let big = w / 2.0 - m;
        let face = big * BEZEL_INNER;
        let chord = CHORD * big;
        let (dx, dy) = (x - w / 2.0, y - w / 2.0);
        let r = (dx * dx + dy * dy).sqrt();
        let below = ([0.0; 3], 0.8 * (1.0 - smoothstep(big - 6.0 * ppp, big + 12.0 * ppp, (dx * dx + (dy - 7.0 * ppp).powi(2)).sqrt())));
        if r > big + 1.0 {
            return below;
        }
        let degrees = dy.atan2(dx).to_degrees();
        // The plate: inside the face circle and above the chord; its lower edge runs straight across and meets the bezel
        // in a sharp corner.
        let plate = (r - face).max(dy - chord);
        let housing = {
            let depth = ((dy - chord) / (big - chord)).clamp(0.0, 1.0);
            // Spun like the plate, only darker.
            let rings = (hash((r / (0.9 * ppp)) as u32, 17) - 0.5) * 0.03;
            let sheen = 0.04 * (2.0 * (degrees.to_radians() + 0.785)).cos();
            let v = (0.36 - 0.1 * depth + rings + sheen) * (1.0 - 0.4 * (1.0 - smoothstep(0.0, 4.0 * ppp, plate.max(0.0))));
            [v, v * 0.93, v * 0.86]
        };
        let color = if plate < -0.8 * ppp {
            let rings = (hash((r / (0.9 * ppp)) as u32, 11) - 0.5) * 0.035 + (hash((r / (2.7 * ppp)) as u32, 5) - 0.5) * 0.02;
            let mut v = by_angle(&FACE_LIGHT, degrees) + rings;
            v *= 1.0 - 0.15 * smoothstep(0.8, 1.0, r / face);
            let on_scale = !(35.0..=145.0).contains(&degrees);
            let rel = r / big;
            if on_scale && (TRACK_INNER..TRACK_OUTER).contains(&rel) {
                v = v * 0.72 + 0.01;
            }
            if on_scale {
                v *= 1.0 - 0.5 * (1.0 - smoothstep(0.0, 1.0 * ppp, (r - TRACK_INNER * big).abs()));
            }
            // The bevelled lower edge catches the light.
            let edge = (1.0 - smoothstep(0.0, 1.8 * ppp, -plate - 0.8 * ppp)) * smoothstep(chord - 0.2 * big, chord - 0.04 * big, dy);
            v += 0.38 * edge;
            [v, v * 0.94, v * 0.86]
        } else if r < face + 0.6 * ppp {
            // Inside the bezel, below the plate (and in the seam around it), the housing shows.
            housing
        } else {
            // Polished chrome all around, lit from above, a dark edge outside.
            let p = (r - face) / (big - face);
            let ridge = 0.22 * (1.0 - smoothstep(0.0, 0.2, (p - 0.3).abs())) - 0.12 * (1.0 - smoothstep(0.0, 0.15, (p - 0.65).abs()));
            let c = by_angle(&BEZEL_LIGHT, degrees) + ridge;
            let chrome = scale([c, c * 0.97, c * 0.93], 1.0 - 0.8 * smoothstep(0.85, 1.0, p));
            let seam = if p < 0.12 { 0.55 } else { 1.0 };
            scale(chrome, seam)
        };
        over((color, (big + 0.5 - r).clamp(0.0, 1.0)), below)
    });
    image(id, Rect::from_center_size(center, egui::Vec2::splat(side)))
}

/// The page background: warm grey lit by a soft spot light from above (a little left of the middle), darker to the
/// edges and the bottom. Made at a quarter of the size
/// (it has no fine detail) and stretched.
pub fn page(ctx: &egui::Context, rect: Rect) -> Shape {
    let size = egui::vec2((rect.width() / 32.0).ceil() * 32.0, (rect.height() / 32.0).ceil() * 32.0);
    let id = texture(ctx, Kind::Page, size, 0.25, 0, &|x, y, w, h| {
        // A soft spot light from above, a little left of the middle, falling off faster downwards than sideways.
        let nx = (x / w - 0.42) / 0.62;
        let ny = (y / h - 0.02) / 0.5;
        let distance = (nx * nx + ny * ny).sqrt();
        (mix(rgb(0x77_6d_62), rgb(0x24_21_1f), smoothstep(0.0, 1.25, distance)), 1.0)
    });
    image(id, rect)
}

/// A domed steel knob of radius `radius` around `center` (a little shadow below it). It is grey; `tint` colours it.
pub fn knob(ctx: &egui::Context, center: Pos2, radius: f32, tint: Color32) -> Shape {
    const MARGIN: f32 = 3.0;
    let side = (radius * 2.0).ceil() + 2.0 * MARGIN;
    let ppp = ctx.pixels_per_point();
    let id = texture(ctx, Kind::Knob, egui::vec2(side, side), ppp, 0, &|x, y, w, _| {
        let m = MARGIN * ppp;
        let big = w / 2.0 - m;
        let (dx, dy) = (x - w / 2.0, y - w / 2.0);
        let r = (dx * dx + dy * dy).sqrt();
        let below = ([0.0; 3], 0.55 * (1.0 - smoothstep(big - 1.5 * ppp, big + 2.5 * ppp, (dx * dx + (dy - 1.5 * ppp).powi(2)).sqrt())));
        if r > big + 1.0 {
            return below;
        }
        // A dome lit from the upper left: diffuse light, a sharp highlight and a dark rim.
        let (nx, ny) = (dx / big.max(1.0), dy / big.max(1.0));
        let nz = (1.0 - (nx * nx + ny * ny).min(1.0)).sqrt();
        let (lx, ly, lz) = (-0.45_f32, -0.6_f32, 0.66_f32);
        let diffuse = (nx * lx + ny * ly + nz * lz).max(0.0);
        let half = {
            let (hx, hy, hz) = (lx, ly, lz + 1.0);
            let len = (hx * hx + hy * hy + hz * hz).sqrt();
            ((nx * hx + ny * hy + nz * hz) / len).max(0.0)
        };
        let mut b = 0.22 + 0.62 * diffuse + half.powi(40) * 0.6;
        b *= 1.0 - 0.6 * smoothstep(0.82, 1.0, r / big);
        over(([b, b * 0.985, b * 0.96], (big + 0.5 - r).clamp(0.0, 1.0)), below)
    });
    Shape::image(id, Rect::from_center_size(center, egui::Vec2::splat(side)), Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), tint)
}

/// A round soft glow of `radius` around `center` in `color`: brightest in the middle, fading out smoothly.
pub fn glow_dot(ctx: &egui::Context, center: Pos2, radius: f32, color: Color32) -> Shape {
    let id = texture(ctx, Kind::Glow, egui::vec2(64.0, 64.0), 1.0, 0, &|x, y, w, h| {
        let (dx, dy) = ((x / w - 0.5) * 2.0, (y / h - 0.5) * 2.0);
        ([1.0; 3], (-(dx * dx + dy * dy) * 5.0).exp() * (1.0 - smoothstep(0.85, 1.0, (dx * dx + dy * dy).sqrt())))
    });
    Shape::image(id, Rect::from_center_size(center, egui::Vec2::splat(radius * 2.0)), Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), color)
}

/// A soft glow along the line from `from` to `to`, `width` wide, in `color`: round glows close together, so the light
/// seems to spread evenly around the line and past its ends.
pub fn glow_along(ctx: &egui::Context, from: Pos2, to: Pos2, width: f32, color: Color32) -> Shape {
    let radius = width / 2.0;
    let length = (to - from).length();
    let steps = (length / (radius * 0.4).max(0.5)).ceil().max(1.0) as usize;
    // Each glow adds to its neighbours: the more there are, the fainter each one.
    let each = color.gamma_multiply((1.6 / steps as f32).sqrt().min(1.0));
    Shape::Vec((0..=steps).map(|i| glow_dot(ctx, from + (to - from) * (i as f32 / steps as f32), radius, each)).collect())
}

/// The slot of a slide switch over `rect`: a capsule sunk into the metal, dark with a shadow under its upper edge,
/// lit faintly in `lit` inside, with a light lip along its lower edge. Smooth edges at any scale.
pub fn slot(ctx: &egui::Context, rect: Rect, lit: Color32) -> Shape {
    const MARGIN: f32 = 2.0;
    let size = egui::vec2(rect.width().ceil(), rect.height().ceil()) + egui::Vec2::splat(2.0 * MARGIN);
    let ppp = ctx.pixels_per_point();
    let tint = [f32::from(lit.r()) / 255.0, f32::from(lit.g()) / 255.0, f32::from(lit.b()) / 255.0];
    let key = u32::from_be_bytes([lit.r(), lit.g(), lit.b(), 0]);
    let id = texture(ctx, Kind::Slot, size, ppp, key, &|x, y, w, h| {
        let m = MARGIN * ppp;
        let (c, half) = ([w / 2.0, h / 2.0], [w / 2.0 - m, h / 2.0 - m]);
        let r = half[1];
        let d = rounded_sdf([x, y], c, half, r);
        if d > 0.0 {
            // The light lip just outside the lower half of the edge.
            let lower = smoothstep(0.35, 0.8, (y - c[1]) / half[1]);
            let lip = 0.3 * lower * (1.0 - smoothstep(0.0, 1.2 * ppp, d));
            return ([0.9, 0.88, 0.85], lip);
        }
        let depth = -d;
        let from_top = y - (c[1] - half[1]);
        let mut color = mix(rgb(0x10_0f_0e), mix(rgb(0x16_15_14), tint, 0.55), smoothstep(1.0 * ppp, 3.0 * ppp, depth));
        color = scale(color, 1.0 - 0.35 * (1.0 - smoothstep(0.0, half[1] * 0.9, from_top)));
        color = scale(color, 1.0 - 0.25 * (1.0 - smoothstep(0.0, 1.0 * ppp, depth)));
        (color, (0.5 + depth).clamp(0.0, 1.0))
    });
    image(id, rect.expand(MARGIN))
}

/// The cap over the middle of an instrument, radius `radius` around `center`: a flat disc of spun metal whose light
/// falls in sectors (bright towards the upper left and the lower right), a slightly domed edge, a dark rim and a little
/// shadow below it.
pub fn hub(ctx: &egui::Context, center: Pos2, radius: f32) -> Shape {
    const MARGIN: f32 = 3.0;
    let side = (radius * 2.0).ceil() + 2.0 * MARGIN;
    let ppp = ctx.pixels_per_point();
    let id = texture(ctx, Kind::Hub, egui::vec2(side, side), ppp, 0, &|x, y, w, _| {
        let m = MARGIN * ppp;
        let big = w / 2.0 - m;
        let (dx, dy) = (x - w / 2.0, y - w / 2.0);
        let r = (dx * dx + dy * dy).sqrt();
        let below = ([0.0; 3], 0.6 * (1.0 - smoothstep(big - 1.0 * ppp, big + 2.5 * ppp, (dx * dx + (dy - 1.5 * ppp).powi(2)).sqrt())));
        if r > big + 1.0 {
            return below;
        }
        let angle = dy.atan2(dx);
        let rings = (hash((r / (0.7 * ppp)) as u32, 23) - 0.5) * 0.04;
        let mut v = 0.66 + 0.22 * (2.0 * (angle + 0.785)).cos() + rings;
        // The edge rounds off: lighter where it faces the light, darker away from it, and a dark rim.
        let edge = smoothstep(0.7, 1.0, r / big);
        let towards_light = (dx * -0.6 + dy * -0.8) / r.max(1e-3);
        v += 0.18 * edge * towards_light;
        v *= 1.0 - 0.7 * smoothstep(0.86, 1.0, r / big);
        over(([v, v * 0.96, v * 0.9], (big + 0.5 - r).clamp(0.0, 1.0)), below)
    });
    image(id, Rect::from_center_size(center, egui::Vec2::splat(side)))
}

/// A raised button of satin metal over `rect`: a soft gradient from a lighter top to a darker bottom, a light upper edge,
/// a dark outline and a small shadow. It is grey; `tint` colours it (red for an action that ends something).
pub fn button(ctx: &egui::Context, rect: Rect, radius: f32, tint: Color32) -> Shape {
    const MARGIN: f32 = 3.0;
    let size = egui::vec2(rect.width().ceil(), rect.height().ceil()) + egui::Vec2::splat(2.0 * MARGIN);
    let ppp = ctx.pixels_per_point();
    let id = texture(ctx, Kind::Button, size, ppp, radius as u32, &|x, y, w, h| {
        let m = MARGIN * ppp;
        let (c, half, r) = ([w / 2.0, h / 2.0], [w / 2.0 - m, h / 2.0 - m], radius * ppp);
        let d = rounded_sdf([x, y], c, half, r);
        let below = ([0.0; 3], 0.5 * (1.0 - smoothstep(-ppp, 3.0 * ppp, rounded_sdf([x, y - 1.5 * ppp], c, half, r))));
        if d > 1.0 {
            return below;
        }
        let t = (y - (c[1] - half[1])) / (2.0 * half[1]);
        let mut v = 0.62 - 0.22 * t;
        // A light upper edge and a slightly darker lower edge: the face is gently domed.
        let inside = -d;
        if inside < 2.0 * ppp {
            let n = rounded_normal([x, y], c, half, r);
            v += (1.0 - inside / (2.0 * ppp)) * (-n[1] * 0.22);
        }
        v = mix([v; 3], [0.08; 3], smoothstep(-1.2 * ppp, -0.1 * ppp, d) * 0.85)[0];
        over(([v, v * 0.98, v * 0.95], (0.5 - d).clamp(0.0, 1.0)), below)
    });
    Shape::image(id, rect.expand(MARGIN), Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), tint)
}

/// The top bar over `rect`: light brushed metal, brightest a little left of the middle where the light falls and darker
/// towards the sides and the bottom, with a sharp shining upper edge and a dark lower edge.
pub fn top_bar(ctx: &egui::Context, rect: Rect) -> Shape {
    let size = egui::vec2((rect.width() / 16.0).ceil() * 16.0, rect.height().ceil());
    let ppp = ctx.pixels_per_point();
    let id = texture(ctx, Kind::TopBar, size, ppp, 0, &|x, y, w, h| {
        let across = (x / w - 0.42) / 0.33;
        let spot = 0.40 + 0.30 * (-across * across).exp();
        let t = y / h;
        let mut v = spot * (1.18 - 0.42 * t);
        // Brushed along the bar: each row a little lighter or darker.
        v += (hash(7, (y / (0.8 * ppp)) as u32) - 0.5) * 0.02;
        // The shining upper edge and the dark lower edge.
        v += 0.3 * (1.0 - smoothstep(0.0, 1.6 * ppp, y));
        v *= 1.0 - 0.75 * smoothstep(h - 1.6 * ppp, h, y);
        ([v, v * 0.97, v * 0.93], 1.0)
    });
    image(id, rect)
}

/// The shadow the top bar throws on the page below it: dark right under the bar, gone after a few points.
pub fn bar_shadow(rect: Rect) -> Shape {
    use eframe::egui::epaint::{Mesh, Vertex};
    let strip = Rect::from_min_size(rect.min, egui::vec2(rect.width(), 10.0));
    let mut mesh = Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    let (dark, clear) = (Color32::from_black_alpha(150), Color32::TRANSPARENT);
    for (pos, color) in [(strip.left_top(), dark), (strip.right_top(), dark), (strip.left_bottom(), clear), (strip.right_bottom(), clear)] {
        mesh.vertices.push(Vertex { pos, uv, color });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 1, 3, 2]);
    Shape::mesh(mesh)
}

/// The narrow soft shadow the sidebar throws on the page to its right.
pub fn side_shadow(rect: Rect) -> Shape {
    use eframe::egui::epaint::{Mesh, Vertex};
    let strip = Rect::from_min_size(rect.min, egui::vec2(5.0, rect.height()));
    let mut mesh = Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    let (dark, clear) = (Color32::from_black_alpha(90), Color32::TRANSPARENT);
    for (pos, color) in [(strip.left_top(), dark), (strip.right_top(), clear), (strip.left_bottom(), dark), (strip.right_bottom(), clear)] {
        mesh.vertices.push(Vertex { pos, uv, color });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 1, 3, 2]);
    Shape::mesh(mesh)
}

/// The sidebar over `rect`: a panel of dark, horizontally brushed metal lit from the upper right (lighter towards the
/// right and the top, darker downwards) that fills the whole side; a light edge along its top and a brighter one along
/// its right side, where it meets the page; the logo area at the top (`header` points high) a lighter field of its own
/// with a joint below it.
pub fn sidebar(ctx: &egui::Context, rect: Rect, header: f32) -> Shape {
    let size = egui::vec2(rect.width().ceil(), (rect.height() / 16.0).ceil() * 16.0);
    let ppp = ctx.pixels_per_point();
    let id = texture(ctx, Kind::Sidebar, size, ppp, header.round() as u32, &|x, y, w, h| {
        let (u, t) = (x / w, y / h);
        let mut v = 0.10 + (0.22 - 0.13 * t) * u.powf(1.4);
        let joint = header * ppp;
        if y < joint {
            v += 0.04 + 0.04 * u;
        }
        // Brushed along the rows.
        v += (hash(3, (y / (0.9 * ppp)) as u32) - 0.5) * 0.02 + (hash((x / (37.0 * ppp)) as u32, (y / (0.9 * ppp)) as u32) - 0.5) * 0.008;
        // The joint under the logo area: a dark line with a hair of light below it.
        v *= 1.0 - 0.6 * (1.0 - smoothstep(0.0, 1.6 * ppp, (y - joint).abs()));
        v += 0.05 * (1.0 - smoothstep(0.0, 0.9 * ppp, (y - joint - 2.4 * ppp).abs()));
        // The light edge along the top, right under the top bar.
        v += 0.12 * (1.0 - smoothstep(0.0, 1.0 * ppp, (y - 1.5 * ppp).abs()));
        // The brighter edge along the right side, where the panel meets the page.
        let from_right = w - x;
        let edge = 1.0 - smoothstep(0.0, 1.0 * ppp, (from_right - 1.8 * ppp).abs() - 0.6 * ppp);
        v = v + (0.44 - 0.28 * t - v) * edge;
        ([v, v * 0.97, v * 0.93], 1.0)
    });
    Shape::image(id, Rect::from_min_size(rect.min, size), Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE)
}

/// An input field sunk into the metal over `rect`: dark inside with a shadow under its upper edge, a dark outline and a
/// light lip along its lower edge.
pub fn field(ctx: &egui::Context, rect: Rect) -> Shape {
    const MARGIN: f32 = 2.0;
    let size = egui::vec2(rect.width().ceil(), rect.height().ceil()) + egui::Vec2::splat(2.0 * MARGIN);
    let ppp = ctx.pixels_per_point();
    let id = texture(ctx, Kind::Field, size, ppp, 0, &|x, y, w, h| {
        let m = MARGIN * ppp;
        let (c, half, r) = ([w / 2.0, h / 2.0], [w / 2.0 - m, h / 2.0 - m], 3.0 * ppp);
        let d = rounded_sdf([x, y], c, half, r);
        if d > 0.0 {
            // The light lip just below the lower edge.
            let lip = 0.28 * smoothstep(0.3, 0.9, (y - c[1]) / half[1]) * (1.0 - smoothstep(0.0, 1.2 * ppp, d));
            return ([0.85, 0.83, 0.8], lip);
        }
        let depth = -d;
        let from_top = y - (c[1] - half[1]);
        let mut v = 0.085 + 0.035 * (from_top / (2.0 * half[1]));
        v *= 1.0 - 0.5 * (1.0 - smoothstep(0.0, 4.0 * ppp, from_top));
        v *= 1.0 - 0.5 * (1.0 - smoothstep(0.0, 1.0 * ppp, depth));
        ([v, v * 0.97, v * 0.93], (0.5 + depth).clamp(0.0, 1.0))
    });
    image(id, rect.expand(MARGIN))
}
