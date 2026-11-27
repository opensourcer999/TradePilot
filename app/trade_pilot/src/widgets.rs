use crate::i18n::Page;
use crate::theme::{card_bg, card_border, muted_text, placeholder_color, text_primary, ACCENT_BLUE, GREEN_TXT, ORANGE_WARN, RED};
use eframe::egui::{self, Color32, RichText};
use rust_decimal::prelude::ToPrimitive;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// Visual building blocks shared by several pages (gauges, grids, icons).
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Top bar building blocks: every element has the same height, so the row lines up.
// ---------------------------------------------------------------------------

/// Height of chips, buttons and the mode switch in the top bar.
pub const BAR_ITEM_HEIGHT: f32 = 30.0;

/// Corner radius of chips, buttons and the mode switch in the top bar: square-ish, not pills.
pub const BAR_CORNER: f32 = 3.0;

/// Status colours of the top bar (brighter than the text colours, readable as dots in both themes).
pub const STATUS_OK: Color32 = Color32::from_rgb(0x2e, 0xb8, 0x5c);
pub const STATUS_ERROR: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
pub const STATUS_PENDING: Color32 = Color32::from_rgb(0xe0, 0xa1, 0x2e);
pub const STATUS_NEUTRAL: Color32 = Color32::from_rgb(0x8a, 0x93, 0xa0);

/// How far shapes beside text (a status dot, the slider's track) are moved down so that they sit on
/// the middle of the text's capitals: the capitals lie a little below the middle of the text box.
const BESIDE_TEXT_SHIFT: f32 = 1.0;

/// A small filled status marker: a small square with barely rounded corners.
pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
    ui.painter().rect_filled(rect.translate(egui::vec2(0.0, BESIDE_TEXT_SHIFT)), 1.5, color);
}

/// The status as an LED of the metal design: a small lamp in a chrome setting, lit in the status colour, with its glow.
fn status_led(ui: &mut egui::Ui, accent: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let center = rect.center() + egui::vec2(0.0, BESIDE_TEXT_SHIFT);
    let ctx = ui.ctx().clone();
    let painter = ui.painter();
    painter.add(crate::metal_tex::glow_dot(&ctx, center, 12.0, accent.gamma_multiply(0.6)));
    painter.add(crate::metal_tex::knob(&ctx, center, 6.5, Color32::from_rgb(0xc8, 0xc4, 0xbe)));
    painter.circle_filled(center, 4.6, Color32::from_rgb(0x0e, 0x0d, 0x0c));
    painter.add(crate::metal_tex::knob(&ctx, center, 4.0, accent));
}

/// A quiet status in the top bar: a small dot in the status colour and the text in the normal
/// text colour - no box, no fill; the colour is only in the dot. The dot and the text go straight
/// into the bar's row (a nested `horizontal` would be placed at the top of it, not centred), so
/// they sit on the same line as the buttons and the other texts.
pub fn status_item(ui: &mut egui::Ui, accent: Color32, text: &str) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        status_led(ui, accent);
        ui.label(RichText::new(text).color(text_primary()).size(13.0));
    })
    .response
}

/// A thin vertical line between the groups of the top bar.
pub fn bar_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 18.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, card_border());
}

/// A top-bar button of the metal design: a raised button of satin metal tinted with `tint`, a little lighter while the
/// pointer is on it and darker while it is pressed; the label in the light text colour with a fine shadow.
fn metal_bar_button(ui: &mut egui::Ui, label: &str, tint: Color32) -> egui::Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let button = egui::Button::new(RichText::new(label).strong().size(13.0).color(Color32::from_rgb(0xf6, 0xf3, 0xee)))
        .min_size(egui::vec2(0.0, BAR_ITEM_HEIGHT - 4.0))
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE);
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(14.0, 4.0);
            ui.add(button)
        })
        .inner;
    let tint = if response.is_pointer_button_down_on() {
        tint.gamma_multiply(0.82)
    } else if response.hovered() {
        tint.lerp_to_gamma(Color32::WHITE, 0.15)
    } else {
        tint
    };
    ui.painter().set(background, crate::metal_tex::button(ui.ctx(), response.rect, BAR_CORNER, tint));
    response
}

/// A top-bar button of raised metal: `primary` (the step to take) a little lighter.
pub fn bar_button(ui: &mut egui::Ui, label: &str, primary: bool) -> egui::Response {
    // Grey metal; the main action a little lighter.
    let tint = if primary { Color32::from_rgb(0xd8, 0xd4, 0xce) } else { Color32::from_rgb(0xb4, 0xb0, 0xaa) };
    metal_bar_button(ui, label, tint)
}

/// A button of the top bar for an action that ends something (Disconnect): red anodised metal.
pub fn bar_button_danger(ui: &mut egui::Ui, label: &str) -> egui::Response {
    // Red anodised metal.
    metal_bar_button(ui, label, Color32::from_rgb(0xff, 0x6a, 0x5c))
}

/// A segmented control (e.g. "Paper | Live"): the selected segment is filled in its own colour.
/// Returns the index clicked, if it is not the selected one.
pub fn segmented(ui: &mut egui::Ui, options: &[(&str, Color32)], selected: usize, min_width: f32) -> (Option<usize>, egui::Response) {
    let mut clicked = None;
    // The segments sit in a slot sunk into the metal; the chosen one is a raised metal button in its colour.
    let track = ui.painter().add(egui::Shape::Noop);
    let response = egui::Frame::none()
        .inner_margin(egui::Margin::same(3.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = egui::vec2(14.0, 2.0);
            // In a right-aligned row egui places items from the right: add them in reverse so
            // they still read left to right.
            let mut order: Vec<usize> = (0..options.len()).collect();
            if ui.layout().prefer_right_to_left() {
                order.reverse();
            }
            for index in order {
                let Some((label, color)) = options.get(index) else { continue };
                let active = index == selected;
                let text = RichText::new(*label).strong().size(13.0).color(if active { Color32::WHITE } else { muted_text() });
                let size = egui::vec2(min_width, BAR_ITEM_HEIGHT - 8.0);
                let pressed = if active {
                    ui.add(MetalButton::new(text).fill(*color).min_size(size)).clicked()
                } else {
                    ui.add(egui::Button::new(text).frame(false).min_size(size)).clicked()
                };
                if pressed && !active {
                    clicked = Some(index);
                }
            }
        })
        .response;
    ui.painter().set(track, crate::metal_tex::field(ui.ctx(), response.rect));
    (clicked, response)
}

/// A two-way slider with a label at each end (e.g. "Paper Trading" and "Live"): the knob sits at
/// the side of the selected option and the track takes that option's colour. A click on the track
/// or on a label moves it. Returns the index clicked, if it is not the selected one.
///
/// It is one element of the bar's height, drawn by hand and placed straight into the bar's row: a
/// row of separate widgets (labels and track in a nested `horizontal`) grows while the taller track
/// is added, which left the first label lower than the second.
pub fn two_way_slider(ui: &mut egui::Ui, options: &[(&str, Color32); 2], selected: usize) -> (Option<usize>, egui::Response) {
    let gap = 8.0;
    let track_size = egui::vec2(54.0, BAR_ITEM_HEIGHT);
    let galleys: Vec<_> = options
        .iter()
        .enumerate()
        .map(|(index, (text, color))| {
            let color = if index == selected { *color } else { muted_text() };
            ui.painter().layout_no_wrap((*text).to_string(), egui::FontId::proportional(13.0), color)
        })
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|galley| galley.size().x).collect();
    let (first, second) = (widths.first().copied().unwrap_or(0.0), widths.get(1).copied().unwrap_or(0.0));
    let (rect, response) = ui.allocate_exact_size(egui::vec2(first + gap + track_size.x + gap + second, BAR_ITEM_HEIGHT), egui::Sense::click());
    let center_y = rect.center().y;
    let track = egui::Rect::from_min_size(egui::pos2(rect.left() + first + gap, center_y - track_size.y / 2.0), track_size);

    // A click on a label selects it; on the track it flips to the other option.
    let mut clicked = None;
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let index = if pos.x < track.left() {
                0
            } else if pos.x > track.right() {
                1
            } else {
                1 - selected.min(1)
            };
            if index != selected {
                clicked = Some(index);
            }
        }
    }

    if ui.is_rect_visible(rect) {
        let mut x = rect.left();
        for (index, galley) in galleys.into_iter().enumerate() {
            let size = galley.size();
            let left = if index == 0 { x } else { track.right() + gap };
            ui.painter().galley(egui::pos2(left, center_y - size.y / 2.0), galley, Color32::PLACEHOLDER);
            x += size.x;
        }
        let color = options.get(selected).map_or(muted_text(), |(_, color)| *color);
        let t = ui.ctx().animate_bool(response.id, selected == 1);
        // A slim metal switch: a slot of 44 x 20 in the middle of the space.
        let slim = egui::Rect::from_center_size(track.center(), egui::vec2(44.0, 20.0));
        for shape in crate::metal::slide_switch(ui.ctx(), slim, color, t) {
            ui.painter().add(shape);
        }
    }
    (clicked, response)
}
pub struct GaugeSpec<'a> {
    pub min: f64,
    pub max: f64,
    pub ticks: &'a [f64],
    pub title: &'a str,
    /// The value as text, shown large inside the gauge (e.g. "62.5%").
    pub value_text: &'a str,
    /// Seconds since the start-up sweep began (the needle runs to the end and back to the real
    /// value, like a dashboard at start-up), or `None` once it is over.
    pub sweep: Option<f64>,
}

/// Seconds the needle takes to run to the end, and to come back to the real value.
pub const SWEEP_UP_SECONDS: f64 = 0.8;
pub const SWEEP_DOWN_SECONDS: f64 = 1.0;

fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The needle value `t` seconds into the start-up sweep: runs from `min` to `max`, then settles on
/// `actual`.
pub fn sweep_value(min: f64, max: f64, actual: f64, t: f64) -> f64 {
    if t < SWEEP_UP_SECONDS {
        min + (max - min) * smoothstep(t / SWEEP_UP_SECONDS)
    } else {
        max + (actual - max) * smoothstep((t - SWEEP_UP_SECONDS) / SWEEP_DOWN_SECONDS)
    }
}

/// Whether the start-up sweep is still running `t` seconds in.
pub fn sweep_active(t: f64) -> bool {
    t < SWEEP_UP_SECONDS + SWEEP_DOWN_SECONDS
}

pub fn gauge_angle(min: f64, max: f64, value: f64) -> f32 {
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0).to_f32().unwrap_or(0.0);
    let start_deg: f32 = 145.0;
    let sweep_deg: f32 = 250.0;
    (start_deg + sweep_deg * t).to_radians()
}

pub fn gauge_point(center: egui::Pos2, radius: f32, angle: f32) -> egui::Pos2 {
    center + radius * egui::vec2(angle.cos(), angle.sin())
}

pub fn format_tick(v: f64) -> String {
    format!("{v:.0}")
}

pub fn draw_gauge(ui: &mut egui::Ui, size: f32, value: f64, spec: &GaugeSpec<'_>) {
    let (min, max) = (spec.min, spec.max);
    let (rect, _resp) = ui.allocate_exact_size(egui::vec2(size, size * GAUGE_ASPECT), egui::Sense::hover());
    // The needle glides to a new value instead of jumping (egui repaints while it moves).
    let value = f64::from(ui.ctx().animate_value_with_time(ui.id().with(("gauge_needle", spec.title)), value.to_f32().unwrap_or(0.0), 0.6));
    // Start-up sweep: min -> max, then back to the real value. The glide above keeps running
    // underneath, so the needle does not move twice when the sweep ends.
    let value = match spec.sweep {
        Some(t) => {
            ui.ctx().request_repaint();
            sweep_value(min, max, value, t)
        }
        None => value,
    };
    crate::metal_gauge::paint(ui.painter(), rect, value, spec);
}

/// Height of a gauge relative to its width.
const GAUGE_ASPECT: f32 = 0.86;
const GAUGE_CARD_PAD: f32 = 8.0;

/// Outer height of `gauge_card(size)`, so a neighbouring card can be made equally tall.
pub fn gauge_card_height(size: f32) -> f32 {
    (size - 2.0 * GAUGE_CARD_PAD).max(30.0) * GAUGE_ASPECT + 2.0 * GAUGE_CARD_PAD + 2.0
}

/// A gauge in its own card, so the two Dashboard gauges sit next to the figures like tiles.
pub fn gauge_card(ui: &mut egui::Ui, size: f32, value: f64, spec: &GaugeSpec<'_>) {
    // The instrument stands on the panel by itself, as large as the neighbouring cards are high.
    draw_gauge(ui, gauge_card_height(size) / GAUGE_ASPECT, value, spec);
}

/// A key figure: small muted caption above a large value.
pub fn stat_tile(ui: &mut egui::Ui, caption: &str, value: &str, value_color: Color32) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        plate_label(ui, RichText::new(caption).color(muted_text()).size(11.5));
        plate_label(ui, RichText::new(value).color(value_color).size(21.0));
    });
}

/// A label on a metal plate, with a soft dark shadow below it so light text stays readable on the light satin metal.
pub fn plate_label(ui: &mut egui::Ui, text: RichText) -> egui::Response {
    let (pos, galley, response) = egui::Label::new(text).layout_in_ui(ui);
    if ui.is_rect_visible(response.rect) {
        let painter = ui.painter();
        painter.galley_with_override_text_color(pos + egui::vec2(0.0, 1.3), galley.clone(), Color32::from_black_alpha(170));
        painter.galley_with_override_text_color(pos + egui::vec2(0.6, 0.6), galley.clone(), Color32::from_black_alpha(70));
        painter.galley(pos, galley, Color32::PLACEHOLDER);
    }
    response
}

/// Rounding of cards and panels.
pub const CARD_ROUNDING: f32 = 8.0;

pub const PCR_TICKS: [f64; 9] = [-100.0, -75.0, -50.0, -25.0, 0.0, 25.0, 50.0, 75.0, 100.0];
pub const WIN_TICKS: [f64; 5] = [0.0, 25.0, 50.0, 75.0, 100.0];

/// The logo: chrome letters and candles on a transparent background.
static LOGO_BYTES: &[u8] = include_bytes!("../assets/logo.png");

/// `None` only if the bundled logo can't be decoded - which should never happen for an asset
/// baked in at compile time, but per `coding_guideline.md` section 4 this isn't `main`
/// initialization or a test, so it gets a real fallback (skip drawing the logo) instead of an
/// `expect()`.
///
/// The picture is scaled down here, with a fine filter, to `height_px` pixels (the height it is shown at): the graphics
/// card only blends neighbouring pixels, so a picture shown at a fraction of its size would look pixelated.
pub fn load_logo_texture(ctx: &egui::Context, height_px: u32) -> Option<egui::TextureHandle> {
    let original = image::load_from_memory(LOGO_BYTES).ok()?.to_rgba8();
    let (full_w, full_h) = original.dimensions();
    let height = height_px.clamp(1, full_h);
    let width = (u64::from(full_w) * u64::from(height) / u64::from(full_h.max(1))).max(1);
    let decoded = image::imageops::resize(&original, u32::try_from(width).unwrap_or(full_w), height, image::imageops::FilterType::Lanczos3);
    let (w, h) = decoded.dimensions();
    let color_image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], decoded.as_raw());
    Some(ctx.load_texture("logo", color_image, egui::TextureOptions::LINEAR))
}

/// The icon of the News page: a newspaper with a picture and lines of text.
fn draw_news_icon(painter: &egui::Painter, center: egui::Pos2, r: f32, color: Color32) {
    let rect = egui::Rect::from_center_size(center, egui::vec2(r * 1.7, r * 1.4));
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.5_f32, color));
    painter.rect_filled(egui::Rect::from_min_size(rect.min + egui::vec2(r * 0.25, r * 0.25), egui::vec2(r * 0.5, r * 0.45)), 0.5, color);
    for i in 0..3 {
        let y = rect.top() + r * 0.35 + i.to_f32().unwrap_or(0.0) * r * 0.32;
        painter.line_segment([egui::pos2(rect.left() + r * 0.95, y), egui::pos2(rect.right() - r * 0.2, y)], egui::Stroke::new(1.2_f32, color));
    }
}

pub fn draw_nav_icon(painter: &egui::Painter, center: egui::Pos2, page: Page, color: Color32) {
    let r = 8.0_f32;
    let stroke = egui::Stroke::new(1.5_f32, color);
    match page {
        Page::Dashboard => {
            painter.circle_stroke(center, r * 0.8, stroke);
            painter.line_segment([center, center + egui::vec2(r * 0.5, -r * 0.5)], stroke);
            painter.circle_filled(center, 1.4, color);
        }
        Page::LiveData => {
            let y = center.y;
            let pts = vec![
                egui::pos2(center.x - r, y),
                egui::pos2(center.x - r * 0.4, y),
                egui::pos2(center.x - r * 0.1, y - r * 0.9),
                egui::pos2(center.x + r * 0.2, y + r * 0.9),
                egui::pos2(center.x + r * 0.5, y),
                egui::pos2(center.x + r, y),
            ];
            painter.add(egui::Shape::line(pts, stroke));
        }
        Page::Quotes => {
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let c = center + egui::vec2(dx * r * 0.5, dy * r * 0.5);
                painter.rect_stroke(egui::Rect::from_center_size(c, egui::vec2(r * 0.78, r * 0.78)), 1.0, stroke);
            }
        }
        Page::InstantOrder => {
            let pts = vec![
                egui::pos2(center.x + r * 0.1, center.y - r),
                egui::pos2(center.x - r * 0.5, center.y + r * 0.15),
                egui::pos2(center.x - r * 0.05, center.y + r * 0.15),
                egui::pos2(center.x - r * 0.2, center.y + r),
                egui::pos2(center.x + r * 0.5, center.y - r * 0.05),
                egui::pos2(center.x + r * 0.05, center.y - r * 0.05),
            ];
            painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
        }
        Page::Templates => {
            let rect = egui::Rect::from_center_size(center, egui::vec2(r * 1.2, r * 1.7));
            painter.rect_stroke(rect, 1.0, stroke);
            for i in 0..3 {
                let y = rect.top() + rect.height() * (0.28 + 0.22 * i.to_f32().unwrap_or(0.0));
                painter.line_segment([egui::pos2(rect.left() + r * 0.25, y), egui::pos2(rect.right() - r * 0.25, y)], egui::Stroke::new(1.2_f32, color));
            }
        }
        Page::Schedules => {
            let rect = egui::Rect::from_center_size(center, egui::vec2(r * 1.6, r * 1.45));
            painter.rect_stroke(rect, 2.0, stroke);
            painter.line_segment(
                [egui::pos2(rect.left(), rect.top() + r * 0.45), egui::pos2(rect.right(), rect.top() + r * 0.45)],
                egui::Stroke::new(1.2_f32, color),
            );
            for dx in [-0.4_f32, 0.4] {
                let x = rect.center().x + dx * rect.width();
                painter.line_segment([egui::pos2(x, rect.top() - r * 0.15), egui::pos2(x, rect.top() + r * 0.2)], egui::Stroke::new(1.2_f32, color));
            }
        }
        Page::Webhooks => {
            painter.circle_stroke(center + egui::vec2(-r * 0.35, 0.0), r * 0.5, stroke);
            painter.circle_stroke(center + egui::vec2(r * 0.35, 0.0), r * 0.5, stroke);
        }
        Page::Conditions => {
            let rect = egui::Rect::from_center_size(center, egui::vec2(r * 1.3, r * 1.3));
            painter.rect_stroke(rect, 2.0, stroke);
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(rect.left() + r * 0.22, rect.center().y),
                    egui::pos2(rect.center().x - r * 0.05, rect.bottom() - r * 0.22),
                    egui::pos2(rect.right() - r * 0.18, rect.top() + r * 0.2),
                ],
                egui::Stroke::new(1.4_f32, color),
            ));
        }
        Page::Reports => {
            for (i, h) in [0.5_f32, 0.85, 1.15].iter().enumerate() {
                let x = center.x - r * 0.55 + i.to_f32().unwrap_or(0.0) * r * 0.55;
                let bar = egui::Rect::from_min_max(egui::pos2(x - r * 0.18, center.y + r * 0.6 - h * r * 0.55), egui::pos2(x + r * 0.18, center.y + r * 0.6));
                painter.rect_filled(bar, 1.0, color);
            }
        }
        Page::News => draw_news_icon(painter, center, r, color),
        Page::Logs => {
            for i in 0..3 {
                let y = center.y - r * 0.55 + i.to_f32().unwrap_or(0.0) * r * 0.55;
                painter.line_segment([egui::pos2(center.x - r * 0.75, y), egui::pos2(center.x + r * 0.75, y)], egui::Stroke::new(1.5_f32, color));
            }
        }
        Page::Settings => {
            painter.circle_stroke(center, r * 0.48, stroke);
            for i in 0..6 {
                let a = i.to_f32().unwrap_or(0.0) * std::f32::consts::TAU / 6.0;
                let dir = egui::vec2(a.cos(), a.sin());
                painter.line_segment([center + dir * r * 0.55, center + dir * r * 0.9], egui::Stroke::new(1.5_f32, color));
            }
        }
    }
}

pub fn toggle_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = egui::vec2(34.0, 18.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
    }
    if ui.is_rect_visible(rect) {
        // A small metal slide switch: the slot lit blue while on, a dark slot while off.
        let lit = if *on { ACCENT_BLUE } else { Color32::from_rgb(0x3a, 0x37, 0x34) };
        let t = ui.ctx().animate_bool(response.id, *on);
        for shape in crate::metal::slide_switch(ui.ctx(), rect, lit, t) {
            ui.painter().add(shape);
        }
    }
    response
}

/// One cell of a picker list: a fixed-size button that is filled in the accent colour while it is the chosen one.
fn picker_cell(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let (fill, color) = if selected { (ACCENT_BLUE, Color32::WHITE) } else { (Color32::TRANSPARENT, text_primary()) };
    ui.add_sized(egui::vec2(46.0, 24.0), egui::Button::new(RichText::new(text).color(color).size(14.0)).fill(fill).frame(selected).rounding(4.0))
}

/// A popup time-of-day picker (hour 1-12, minute 00-59 and AM/PM as three columns, each a scrollable list that opens at the
/// chosen value) for every wall-clock time setting in the app, instead of text fields a user could type an out-of-range value
/// into. Edits only take effect with "OK"; "Cancel" or Escape leave `hour`/`minute`/`is_pm` as they were, using a staged copy
/// kept in egui's per-widget temporary data while the popup is open.
pub fn time_picker(ui: &mut egui::Ui, id_salt: &str, hour: &mut String, minute: &mut String, is_pm: &mut bool) {
    let popup_id = ui.make_persistent_id(id_salt);
    let fresh_id = popup_id.with("fresh");
    let button_label = if hour.is_empty() { "--:--".to_string() } else { format!("{hour}:{minute} {}", if *is_pm { "PM" } else { "AM" }) };
    let response = ui.add(MetalButton::new(RichText::new(button_label).strong()).min_size(egui::vec2(92.0, 24.0)));
    if response.clicked() {
        // (Re-)seed the staged copy with the real current value every time the popup is opened,
        // so a previous cancel never leaves stale picks behind for next time.
        ui.data_mut(|d| {
            d.insert_temp(popup_id, (hour.clone(), minute.clone(), *is_pm));
            d.insert_temp(fresh_id, true);
        });
        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
    }
    egui::popup::popup_below_widget(ui, popup_id, &response, egui::popup::PopupCloseBehavior::IgnoreClicks, |ui| {
        ui.set_min_width(176.0);
        let (mut staged_hour, mut staged_minute, mut staged_pm): (String, String, bool) =
            ui.data_mut(|d| d.get_temp_mut_or_insert_with(popup_id, || (hour.clone(), minute.clone(), *is_pm)).clone());
        // The first frame after opening scrolls each list to its chosen value.
        let fresh = ui.data_mut(|d| d.remove_temp::<bool>(fresh_id)).unwrap_or(false);
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 2.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("Hour").small().color(muted_text()));
                egui::ScrollArea::vertical().id_salt((id_salt, "h")).max_height(168.0).auto_shrink([true, false]).show(ui, |ui| {
                    for h in 1..=12 {
                        let label = h.to_string();
                        let selected = staged_hour == label;
                        let cell = picker_cell(ui, &label, selected);
                        if selected && fresh {
                            cell.scroll_to_me(Some(egui::Align::Center));
                        }
                        if cell.clicked() {
                            staged_hour = label;
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                ui.label(RichText::new("Minute").small().color(muted_text()));
                egui::ScrollArea::vertical().id_salt((id_salt, "m")).max_height(168.0).auto_shrink([true, false]).show(ui, |ui| {
                    for m in 0..60 {
                        let label = format!("{m:02}");
                        let selected = staged_minute == label;
                        let cell = picker_cell(ui, &label, selected);
                        if selected && fresh {
                            cell.scroll_to_me(Some(egui::Align::Center));
                        }
                        if cell.clicked() {
                            staged_minute = label;
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                ui.label(RichText::new("AM/PM").small().color(muted_text()));
                if picker_cell(ui, "AM", !staged_pm).clicked() {
                    staged_pm = false;
                }
                if picker_cell(ui, "PM", staged_pm).clicked() {
                    staged_pm = true;
                }
            });
        });
        ui.data_mut(|d| d.insert_temp(popup_id, (staged_hour.clone(), staged_minute.clone(), staged_pm)));
        ui.add_space(6.0);
        ui.separator();
        ui.horizontal(|ui| {
            let chosen =
                if staged_hour.is_empty() { "--:--".to_string() } else { format!("{staged_hour}:{staged_minute} {}", if staged_pm { "PM" } else { "AM" }) };
            ui.label(RichText::new(chosen).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(MetalButton::new(RichText::new("OK").color(Color32::WHITE).strong()).fill(ACCENT_BLUE).min_size(egui::vec2(52.0, 24.0))).clicked() {
                    hour.clone_from(&staged_hour);
                    minute.clone_from(&staged_minute);
                    *is_pm = staged_pm;
                    ui.memory_mut(eframe::egui::Memory::close_popup);
                }
                if ui.add(MetalButton::new("Cancel").min_size(egui::vec2(60.0, 24.0))).clicked() {
                    ui.memory_mut(eframe::egui::Memory::close_popup);
                }
            });
        });
        ui.add_space(2.0);
    });
}

/// A grid for the Dashboard's "Trades today" blotter, which - unlike every other
/// page's grid - now has real rows: one per `IBApi.EDecoder.ExecutionDataEvent` from a default
/// `reqExecutions` (see `ExecutionRow`, `parse_execution_data`). Columns this app has no
/// automation for yet (stop/profit-target tracking, a live bid/ask merge, running P/L) show an
/// honest "-" instead of a made-up number, rather than pretend those columns are computed.
/// One cell of `data_table`: text, optionally coloured/bold, optionally led by a status dot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableCell {
    pub text: String,
    pub color: Option<Color32>,
    pub strong: bool,
    /// A coloured dot before the text.
    pub dot: Option<Color32>,
    /// Shown when the cell is hovered.
    pub hover: Option<String>,
}

impl TableCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Self::default() }
    }
}

/// One column of `data_table`: header, relative width, and whether its numbers align right.
#[derive(Clone, Debug)]
pub struct TableColumn {
    pub title: String,
    pub weight: f32,
    pub right: bool,
}

impl TableColumn {
    pub fn new(title: impl Into<String>, weight: f32, right: bool) -> Self {
        Self { title: title.into(), weight, right }
    }
}

/// A cell of fixed size whose content is aligned left or right and centred vertically.
fn table_cell_frame(ui: &mut egui::Ui, width: f32, height: f32, right: bool, add_contents: impl FnOnce(&mut egui::Ui)) {
    let layout = if right { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
    ui.allocate_ui_with_layout(egui::vec2(width, height), layout, |ui| {
        ui.set_min_size(egui::vec2(width, height));
        ui.set_max_width(width);
        add_contents(ui);
    });
}

/// A right click copies the text that is selected - in labels (tables, the trade details) and in
/// text fields (the log) - to the clipboard. The right mouse button is used for nothing else in
/// `TradePilot`, so its press and release are taken out of the frame's input and the press becomes a
/// copy event, as Ctrl+C makes it: a right click must not reach the widgets as a click, because a
/// press on a text field puts its cursor there and so clears the selection before it is copied.
/// With nothing selected nothing changes; a password field is never copied. Once copied, the
/// selection is removed in the next frame (`clear_selection_after_copy`).
pub fn copy_on_right_click(events: &mut Vec<egui::Event>) {
    let is_right = |event: &egui::Event| matches!(event, egui::Event::PointerButton { button: egui::PointerButton::Secondary, .. });
    let right_press = events.iter().any(|event| matches!(event, egui::Event::PointerButton { button: egui::PointerButton::Secondary, pressed: true, .. }));
    events.retain(|event| !is_right(event));
    if right_press {
        CLEAR_SELECTION_PENDING.store(true, Ordering::Relaxed);
        if !events.iter().any(|event| matches!(event, egui::Event::Copy)) {
            events.push(egui::Event::Copy);
        }
    }
}

/// Set when a right click asked for a copy: the selection is cleared in the following frame.
static CLEAR_SELECTION_PENDING: AtomicBool = AtomicBool::new(false);

/// After a right click copied the selected text, removes the selection - in labels and in the
/// focused text field (its cursor stays where the selection ended). Called at the start of every
/// frame; the copy itself was made in the frame before, so nothing is lost.
pub fn clear_selection_after_copy(ctx: &egui::Context) {
    if !CLEAR_SELECTION_PENDING.swap(false, Ordering::Relaxed) {
        return;
    }
    let mut labels = egui::text_selection::LabelSelectionState::load(ctx);
    labels.clear_selection();
    labels.store(ctx);
    if let Some(id) = ctx.memory(egui::Memory::focused) {
        if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
            if let Some(range) = state.cursor.char_range() {
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(range.primary)));
                state.store(ctx, id);
            }
        }
    }
    ctx.request_repaint();
}

/// Settings > Appearance > Grid Font Size, in points: the text of every table and the row height
/// (twice the font size) follow it.
static GRID_FONT_SIZE: AtomicU32 = AtomicU32::new(14);

/// The grid font size a settings text stands for: anything unreadable is the default 14, and the
/// size stays between 10 and 24.
fn grid_font_size_of(text: &str) -> u32 {
    text.trim().parse::<u32>().unwrap_or(14).clamp(10, 24)
}

/// Sets the grid font size from the settings text (see `grid_font_size_of`).
pub fn set_grid_font_size(text: &str) {
    GRID_FONT_SIZE.store(grid_font_size_of(text), Ordering::Relaxed);
}

/// Settings > Appearance > "Mobile Friendly Grid": tables with tighter rows and spacing that leave out columns which would get too
/// narrow, so a narrow window shows every table without cut-off text.
static COMPACT_GRID: AtomicBool = AtomicBool::new(false);

/// Switches the compact tables on or off.
pub fn set_compact_grid(on: bool) {
    COMPACT_GRID.store(on, Ordering::Relaxed);
}

/// The columns (by index) a compact table shows in `room` points: all of them while each gets at least `min_width` after the
/// `spacing` between them; else columns are left out from the second to last one backwards, so the first column (what the row is) and
/// the last one (usually its result) stay. At least those two are shown.
fn visible_columns(weights: &[f32], room: f32, min_width: f32, spacing: f32) -> Vec<usize> {
    let mut shown: Vec<usize> = (0..weights.len()).collect();
    loop {
        let count = shown.len().to_f32().unwrap_or(0.0);
        let total: f32 = shown.iter().filter_map(|i| weights.get(*i)).sum();
        let unit = (room - spacing * (count - 1.0)).max(0.0) / total.max(0.1);
        let fits = shown.iter().filter_map(|i| weights.get(*i)).all(|weight| weight * unit >= min_width);
        if fits || shown.len() <= 2 {
            return shown;
        }
        shown.remove(shown.len() - 2);
    }
}

/// The grid font size in points.
fn grid_font_size() -> f32 {
    GRID_FONT_SIZE.load(Ordering::Relaxed).to_f32().unwrap_or(0.0)
}

/// A quiet table: muted small headers, roomy rows, numbers right-aligned, long text cut with an
/// ellipsis. Column widths are shares of the available width. The text size and row height follow
/// the grid font size of the settings.
pub fn data_table(ui: &mut egui::Ui, id: &str, columns: &[TableColumn], rows: &[Vec<TableCell>]) {
    let _ = table_core(ui, id, columns, rows, (false, None, &[], &[]), false);
}

/// The box in front of a row of a list that can be selected: empty, or filled with a tick for the
/// selected row. It can be clicked like the rest of the row.
/// A small box sunk into the metal at `rect`, with a lit tick while `checked` (a little lighter while the pointer is on it).
fn paint_metal_box(ui: &egui::Ui, rect: egui::Rect, checked: bool, hovered: bool) {
    let painter = ui.painter();
    painter.add(crate::metal_tex::field(ui.ctx(), rect));
    if hovered {
        painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0_f32, Color32::from_rgb(0x6a, 0x65, 0x5f)));
    }
    if checked {
        let size = rect.width();
        let point = |x: f32, y: f32| rect.min + egui::vec2(size * x, size * y);
        let tick = vec![point(0.24, 0.52), point(0.43, 0.70), point(0.76, 0.32)];
        painter.add(crate::metal_tex::glow_along(
            ui.ctx(),
            point(0.3, 0.55),
            point(0.7, 0.4),
            size * 0.9,
            Color32::from_rgba_unmultiplied(0xff, 0xa0, 0x40, 60),
        ));
        painter.add(egui::Shape::line(tick, egui::Stroke::new((size * 0.13).max(1.6), Color32::from_rgb(0xff, 0xd8, 0x9c))));
    }
}

/// A checkbox of the metal design: a small box sunk into the metal with a lit tick, and its label beside it. Used like
/// `ui.checkbox`; the response says `changed` when it was ticked or unticked.
pub fn metal_checkbox(ui: &mut egui::Ui, checked: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    let text: egui::WidgetText = text.into();
    let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let box_size = 16.0;
    let label_width = if galley.size().x > 0.0 { galley.size().x + 7.0 } else { 0.0 };
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(box_size + label_width, box_size.max(galley.size().y)), egui::Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let mark = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - box_size / 2.0), egui::vec2(box_size, box_size));
        paint_metal_box(ui, mark, *checked, response.hovered());
        let color = ui.visuals().text_color();
        ui.painter().galley(egui::pos2(mark.right() + 7.0, rect.center().y - galley.size().y / 2.0), galley, color);
    }
    response
}

pub fn selection_mark(ui: &mut egui::Ui, selected: bool, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    paint_metal_box(ui, rect, selected, response.hovered());
    response
}

/// The fill and the outline of the box of a row that cannot be selected.
const LOCKED_MARK_FILL: Color32 = Color32::from_rgb(0x34, 0x32, 0x2f);
const LOCKED_MARK_OUTLINE: Color32 = Color32::from_rgb(0x5c, 0x59, 0x55);

/// The box of a row that cannot be selected, drawn like a disabled checkbox of Windows: flat, a thin grey outline around a fill
/// just lighter than the row, no tick, and it does not react to the pointer.
fn locked_mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let rect = rect.shrink(0.5);
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, LOCKED_MARK_FILL);
    painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0_f32, LOCKED_MARK_OUTLINE));
}

/// What the person did with the rows of a table: the row clicked and the row double-clicked, by index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowClicks {
    /// The row clicked once.
    pub clicked: Option<usize>,
    /// The row double-clicked.
    pub double_clicked: Option<usize>,
}

/// The selection after this frame's `clicks`, the same for every table with actions: a click on a row selects it,
/// a click on the selected row takes the tick away again, a double-click selects the row (and the caller
/// acts on it). `key_of` turns a row index into the key the page keeps its selection by.
pub fn next_selection<K: Clone + PartialEq>(current: Option<K>, clicks: RowClicks, key_of: impl Fn(usize) -> Option<K>) -> Option<K> {
    if let Some(key) = clicks.double_clicked.and_then(&key_of) {
        return Some(key);
    }
    match clicks.clicked.and_then(&key_of) {
        Some(key) if current.as_ref() == Some(&key) => None,
        Some(key) => Some(key),
        None => current,
    }
}

/// The rows chosen together after this frame's `clicks`, for a table where several rows are chosen at once: a click on a row ticks it, a
/// click on a ticked row takes the tick away. `key_of` turns a row index into the key the page keeps its selection by.
pub fn toggle_selection<K: PartialEq>(current: &mut Vec<K>, clicks: RowClicks, key_of: impl Fn(usize) -> Option<K>) {
    let Some(key) = clicks.clicked.or(clicks.double_clicked).and_then(key_of) else { return };
    match current.iter().position(|known| *known == key) {
        Some(place) => {
            current.remove(place);
        }
        None => current.push(key),
    }
}

/// `data_table` whose rows can be selected: a column of boxes in front shows the `selected` row with a tick
/// (its text is bold too), and the clicks of this frame are returned.
pub fn data_table_rows(ui: &mut egui::Ui, id: &str, columns: &[TableColumn], rows: &[Vec<TableCell>], selected: Option<usize>) -> RowClicks {
    table_core(ui, id, columns, rows, (true, selected, &[], &[]), false).0
}

/// How far the text of a table stays from the ends of its row strips.
const TABLE_INSET: f32 = 10.0;

/// How the rows of an `arranged_table` can be selected: the selected row (with a tick), further ticked rows of a table where several
/// rows are chosen together (see `toggle_selection`), and the rows that cannot be selected (their box greyed out, a click on them not
/// returned).
#[derive(Clone, Copy, Debug, Default)]
pub struct RowSelection<'a> {
    pub selected: Option<usize>,
    pub ticked: &'a [usize],
    pub locked: &'a [usize],
}

/// A table the person arranges, the one function for every such table: a column header dragged onto another one moves the
/// column there, the border between two headers dragged sideways sizes the two columns (a double click on it gives the table its
/// own widths back). Order and widths are remembered per table (`column_order`, under `id`) by the columns' `keys`, which stay
/// the same in every language. `keys`, `columns` and the cells of each row are given in the table's own order. With `selection`
/// the rows can be selected (a column of boxes in front); the clicks of this frame are returned.
pub fn arranged_table(
    ui: &mut egui::Ui,
    id: &str,
    keys: &[&str],
    columns: &[TableColumn],
    rows: &[Vec<TableCell>],
    selection: Option<RowSelection<'_>>,
) -> RowClicks {
    let defaults: Vec<f32> = columns.iter().map(|c| c.weight).collect();
    let (order, widths) = crate::column_order::layout(id, keys, &defaults);
    let arranged: Vec<TableColumn> =
        order.iter().filter_map(|i| columns.get(*i).map(|c| TableColumn { weight: widths.get(*i).copied().unwrap_or(c.weight), ..c.clone() })).collect();
    let rows: Vec<Vec<TableCell>> = rows.iter().map(|row| order.iter().filter_map(|i| row.get(*i).cloned()).collect()).collect();
    let select = selection.unwrap_or_default();
    let (clicks, edit) = table_core(ui, id, &arranged, &rows, (selection.is_some(), select.selected, select.ticked, select.locked), true);
    if let Some((from, to)) = edit.moved {
        crate::column_order::move_column(id, keys, from, to);
        ui.ctx().request_repaint();
    }
    // The places of the two columns at the border, as indexes into `keys`.
    let column_at = |place: usize| order.get(place).copied();
    if let Some((left, right, delta, done)) = edit.resized {
        if let (Some(l), Some(r)) = (column_at(left), column_at(right)) {
            let (l_width, r_width) = (widths.get(l).copied().unwrap_or(1.0), widths.get(r).copied().unwrap_or(1.0));
            let (new_l, new_r) = crate::column_order::resized(l_width, r_width, delta);
            if let (Some(l_key), Some(r_key)) = (keys.get(l), keys.get(r)) {
                crate::column_order::set_widths(id, &[(l_key, new_l), (r_key, new_r)], done);
            }
        }
    }
    if edit.widths_reset {
        crate::column_order::reset_widths(id);
        ui.ctx().request_repaint();
    }
    clicks
}

/// What the person did to the columns of an arranged table in this frame (see `arranged_table`); places are in the columns
/// given to `table_core`.
#[derive(Clone, Copy, Debug, Default)]
struct TableEdit {
    /// A column dragged onto another one: its place and the place it was dropped on.
    moved: Option<(usize, usize)>,
    /// The border between two columns dragged: their places, the change in weights (positive to the right), and whether the
    /// mouse was let go (then the widths are kept in the file).
    resized: Option<(usize, usize, f32, bool)>,
    /// A border was double-clicked: the table's own widths again.
    widths_reset: bool,
}

/// How wide the grip on the border between two column headers is.
const RESIZE_GRIP: f32 = 8.0;
/// The fine line that marks the border between two column headers of an arranged table.
const COLUMN_BORDER: Color32 = Color32::from_rgb(0x6a, 0x66, 0x61);

/// What is dragged when a column header is moved: the table and the place of the column in it.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ColumnDrag {
    table: egui::Id,
    column: usize,
}

/// The colour of the header a dragged column would be dropped on.
const COLUMN_DROP_MARK: Color32 = Color32::from_rgb(0x6f, 0x9f, 0xe8);

/// The header of the column at `place` of the table `table_id` that can be dragged onto another header: the whole cell takes the
/// drag and the drop, the header it would land on is outlined. Returns the move (the place of the dragged column, `place`) when a
/// header of the same table is dropped on it.
fn movable_header(ui: &mut egui::Ui, title: RichText, table_id: egui::Id, place: usize) -> Option<(usize, usize)> {
    let payload = ColumnDrag { table: table_id, column: place };
    let header = ui
        .dnd_drag_source(table_id.with(place), payload, |ui| {
            ui.set_min_size(ui.available_size());
            ui.label(title);
        })
        .response
        .on_hover_cursor(egui::CursorIcon::Grab);
    let ours = |drag: &ColumnDrag| drag.table == table_id;
    if header.dnd_hover_payload::<ColumnDrag>().is_some_and(|drag| ours(&drag) && drag.column != place) {
        ui.painter().rect_stroke(header.rect.expand(2.0), 4.0, egui::Stroke::new(1.5_f32, COLUMN_DROP_MARK));
    }
    header.dnd_release_payload::<ColumnDrag>().filter(|drag| ours(drag)).map(|drag| (drag.column, place))
}

/// The grip on the border right of `cell` (the header of the column at `place`, its right neighbour at `next`): dragging it sizes
/// the two columns, a double click gives the table its own widths back. `unit` is the width of one weight in points.
fn resize_grip(ui: &egui::Ui, cell: egui::Rect, spacing_x: f32, table_id: egui::Id, (place, next): (usize, usize), unit: f32, edit: &mut TableEdit) {
    let center = egui::pos2(cell.right() + spacing_x / 2.0, cell.center().y);
    let rect = egui::Rect::from_center_size(center, egui::vec2(RESIZE_GRIP, cell.height()));
    let grip = ui.interact(rect, table_id.with(("resize", place)), egui::Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    // The border is always shown as a fine line, so the extent of every column can be seen; lit while it is hovered or dragged.
    let line = [egui::pos2(center.x, rect.top() + 4.0), egui::pos2(center.x, rect.bottom() - 4.0)];
    let active = grip.hovered() || grip.dragged();
    let stroke = if active { egui::Stroke::new(2.0_f32, COLUMN_DROP_MARK) } else { egui::Stroke::new(1.0_f32, COLUMN_BORDER) };
    ui.painter().line_segment(line, stroke);
    if grip.double_clicked() {
        edit.widths_reset = true;
    } else if grip.dragged() && unit > 0.0 {
        edit.resized = Some((place, next, grip.drag_delta().x / unit, false));
    } else if grip.drag_stopped() {
        edit.resized = Some((place, next, 0.0, true));
    }
}

/// The one table behind `data_table`, `data_table_rows` and `arranged_table`: `selection` (selectable, selected row, further ticked
/// rows, locked rows) adds the column of boxes in front; when `arranged` the headers can be dragged onto each other and their borders sized,
/// and what the person did is returned (see `TableEdit`, places in `columns`).
fn table_core(
    ui: &mut egui::Ui,
    id: &str,
    columns: &[TableColumn],
    rows: &[Vec<TableCell>],
    selection: (bool, Option<usize>, &[usize], &[usize]),
    arranged: bool,
) -> (RowClicks, TableEdit) {
    let (selectable, selected, ticked, locked) = selection;
    let is_ticked = |row: usize| selected == Some(row) || ticked.contains(&row);
    let mut clicks = RowClicks::default();
    let mut edit = TableEdit::default();
    let table_id = egui::Id::new(("table_columns", id));
    let font = grid_font_size();
    let compact = COMPACT_GRID.load(Ordering::Relaxed);
    let (header_height, row_height, spacing_x) = if compact { (font * 1.5, font * 1.6, 6.0) } else { (font * 1.9, font * 2.0, 12.0) };
    let mark_width = if selectable { font * 1.2 + spacing_x } else { 0.0 };
    let room = (ui.available_width() - mark_width - 4.0 - 2.0 * TABLE_INSET).max(0.0);
    // A compact table leaves out the columns that would get too narrow to read, so it never needs more than the width there is.
    let all_weights: Vec<f32> = columns.iter().map(|c| c.weight).collect();
    let shown: Vec<usize> = if compact { visible_columns(&all_weights, room, font * 3.5, spacing_x) } else { (0..columns.len()).collect() };
    let columns: Vec<TableColumn> = shown.iter().filter_map(|i| columns.get(*i).cloned()).collect();
    let rows: Vec<Vec<TableCell>> = rows.iter().map(|row| shown.iter().filter_map(|i| row.get(*i).cloned()).collect()).collect();
    let (columns, rows) = (columns.as_slice(), rows.as_slice());
    let usable = (room - spacing_x * (columns.len().to_f32().unwrap_or(0.0) - 1.0)).max(0.0);
    let unit = usable / columns.iter().map(|c| c.weight).sum::<f32>().max(0.1);
    let width = |i: usize| columns.get(i).map_or(40.0, |c| (c.weight * unit).max(24.0));
    let mut plastic = PlasticRows::default();
    // The text keeps a little distance from the ends of the strips.
    let grid = egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(TABLE_INSET, 0.0))
        .show(ui, |ui| {
            egui::Grid::new(id).spacing([spacing_x, 2.0]).show(ui, |ui| {
                plastic.row(ui, true);
                if selectable {
                    table_cell_frame(ui, font * 1.2, header_height, false, |_| {});
                }
                for (i, column) in columns.iter().enumerate() {
                    // The place of the column among all columns (a compact table leaves some out).
                    let place = shown.get(i).copied().unwrap_or(i);
                    let mut cell = egui::Rect::NOTHING;
                    table_cell_frame(ui, width(i), header_height, column.right, |ui| {
                        cell = ui.max_rect();
                        let title = RichText::new(&column.title).size(font * 0.85).color(muted_text());
                        if arranged {
                            if let Some(drop) = movable_header(ui, title, table_id, place) {
                                edit.moved = Some(drop);
                            }
                        } else {
                            ui.label(title);
                        }
                    });
                    // Every border but the one after the last column can be dragged.
                    if let (true, Some(next)) = (arranged, shown.get(i + 1)) {
                        resize_grip(ui, cell, spacing_x, table_id, (place, *next), unit, &mut edit);
                    }
                }
                ui.end_row();
                for (row_index, row) in rows.iter().enumerate() {
                    plastic.row(ui, false);
                    let row_locked = locked.contains(&row_index);
                    if selectable {
                        table_cell_frame(ui, font * 1.2, row_height, false, |ui| {
                            if row_locked {
                                locked_mark(ui, font);
                                return;
                            }
                            let mark = selection_mark(ui, is_ticked(row_index), font);
                            if mark.double_clicked() {
                                clicks.double_clicked = Some(row_index);
                            } else if mark.clicked() {
                                clicks.clicked = Some(row_index);
                            }
                        });
                    }
                    for (i, cell) in row.iter().enumerate() {
                        let right = columns.get(i).is_some_and(|c| c.right);
                        table_cell_frame(ui, width(i), row_height, right, |ui| {
                            let response = ui
                                .horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 7.0;
                                    if let Some(dot) = cell.dot {
                                        status_dot(ui, dot);
                                    }
                                    let mut text = RichText::new(&cell.text).size(font).color(cell.color.unwrap_or_else(text_primary));
                                    if cell.strong || is_ticked(row_index) {
                                        text = text.strong();
                                    }
                                    let sense = if row_locked { egui::Sense::hover() } else { egui::Sense::click() };
                                    let label = ui.add(egui::Label::new(text).truncate().sense(sense));
                                    if !row_locked && label.double_clicked() {
                                        clicks.double_clicked = Some(row_index);
                                    } else if !row_locked && label.clicked() {
                                        clicks.clicked = Some(row_index);
                                    }
                                })
                                .response;
                            if let Some(hover) = &cell.hover {
                                response.on_hover_text(hover);
                            }
                        });
                    }
                    ui.end_row();
                }
            })
        })
        .inner;
    plastic.paint(ui, grid.response.rect, 2.0);
    (clicks, edit)
}

/// A centred, muted message for an empty list: a short title and a hint what fills it.
pub fn empty_state(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.add_space(14.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).size(14.0).color(muted_text()).strong());
        ui.add_space(2.0);
        ui.label(RichText::new(hint).size(12.0).color(muted_text()));
    });
    ui.add_space(14.0);
}

/// A KPI card of fixed outer size (`width` x `height`), like a gauge card.
pub fn kpi_card(ui: &mut egui::Ui, width: f32, height: f32, add_contents: impl FnOnce(&mut egui::Ui)) {
    let margin = 16.0;
    // Frame's total margin is inner margin + stroke on each side.
    let chrome = 2.0 * (margin + 1.0);
    metal_frame(ui, egui::Margin::same(margin), false, |ui| {
        // A frame inherits its parent's layout (a row, on the Dashboard): stack the content.
        ui.vertical(|ui| {
            ui.set_width((width - chrome).max(40.0));
            ui.set_min_height((height - chrome).max(40.0));
            add_contents(ui);
        });
    });
}

/// One row of the Dashboard's trade list: the cells as shown, and the colour of the profit.
#[derive(Clone, Debug, PartialEq)]
pub struct TradeRowView {
    /// The index of the trade in the app's list of stored trades (what a click on the row selects).
    pub index: usize,
    pub account: String,
    /// The order reference of the opening order, as TWS shows it (`TP-137`).
    pub order_id: String,
    pub expiry: String,
    pub kind: String,
    pub strikes: String,
    pub qty: String,
    pub open: String,
    /// The stop and the profit target as they stand (`-` where there is none).
    pub stop: String,
    pub target: String,
    /// Set when the stop covers fewer contracts than the trade holds: what to say about it (shown on the stop cell, which turns orange).
    pub stop_note: Option<String>,
    pub bid: String,
    /// The middle between bid and ask (`-` without both).
    pub mid: String,
    pub ask: String,
    pub profit: String,
    /// Gain green, loss red, else the normal text colour.
    pub profit_color: Color32,
    /// "Open", "Expired": the status dot and its hover text; a trade that is not open is muted.
    pub status: String,
}

/// The Dashboard's list of the trades this app opened today, with live bid, mid, ask and profit.
/// `titles` are the twelve column headers (Trade, Order Ref, Strikes, Expiry, Qty, Open, Stop, Target, Bid, Mid, Ask, P/L).
/// The list of today's trades. The result holds the stored indexes of the trades clicked and double-clicked in this
/// frame (see `next_selection`). `selected` is shown with a tick.
pub fn trade_table(ui: &mut egui::Ui, rows: &[TradeRowView], titles: [&str; 12], selected: Option<usize>) -> RowClicks {
    let weights = [2.0, 1.2, 2.4, 1.6, 0.7, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.6];
    let columns: Vec<TableColumn> = titles.iter().zip(weights).enumerate().map(|(i, (title, weight))| TableColumn::new(*title, weight, i >= 4)).collect();
    let cells: Vec<Vec<TableCell>> = rows
        .iter()
        .map(|r| {
            let open = r.status == crate::trade_summary::TradeStatus::Open.as_str();
            let muted = (!open).then(muted_text);
            let plain = |text: &str| TableCell { text: text.to_string(), color: muted, ..TableCell::default() };
            vec![
                TableCell {
                    text: r.kind.clone(),
                    color: muted,
                    strong: open,
                    dot: Some(if open { STATUS_OK } else { STATUS_NEUTRAL }),
                    hover: Some(format!("{} - {}", r.status, r.account)),
                },
                plain(&r.order_id),
                plain(&r.strikes),
                plain(&r.expiry),
                plain(&r.qty),
                plain(&r.open),
                match &r.stop_note {
                    Some(note) => {
                        TableCell { text: format!("{} !", r.stop), color: Some(ORANGE_WARN), strong: true, hover: Some(note.clone()), ..TableCell::default() }
                    }
                    None => plain(&r.stop),
                },
                plain(&r.target),
                plain(&r.bid),
                plain(&r.mid),
                plain(&r.ask),
                TableCell { text: r.profit.clone(), color: Some(r.profit_color), strong: true, ..TableCell::default() },
            ]
        })
        .collect();
    let position = selected.and_then(|index| rows.iter().position(|r| r.index == index));
    // Only an open trade can be selected (its buttons are under the list); a closed one has nothing to act on.
    let locked: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.status != crate::trade_summary::TradeStatus::Open.as_str()).map(|(i, _)| i).collect();
    let keys = ["trade", "order_ref", "strikes", "expiry", "qty", "open", "stop", "target", "bid", "mid", "ask", "profit"];
    let clicks = arranged_table(ui, "trade_table", &keys, &columns, &cells, Some(RowSelection { selected: position, ticked: &[], locked: &locked }));
    let index_of = |row: Option<usize>| row.and_then(|row| rows.get(row)).map(|r| r.index);
    RowClicks { clicked: index_of(clicks.clicked), double_clicked: index_of(clicks.double_clicked) }
}

/// Formats a derived delta for the Live Data grid ("Change" = Price-YesterdayClose, "Intraday
/// Move" = Price-TodayOpen, "Gap" = TodayOpen-YesterdayClose) the way the page shows them: the
/// absolute move on one line, the percent move on the next, colored green/red by sign. Both
/// values are real ticks already received from TWS - this only subtracts them, it never invents
/// either side. Shows "-" (muted) until both ticks this depends on have actually arrived.
pub fn format_delta(current: Option<f64>, base: Option<f64>) -> (String, Color32) {
    match (current, base) {
        (Some(c), Some(b)) if b != 0.0 => {
            let delta = c - b;
            let pct = delta / b * 100.0;
            let color = if delta > 0.0 {
                GREEN_TXT
            } else if delta < 0.0 {
                RED
            } else {
                text_primary()
            };
            (format!("{delta:+.2}\n{pct:+.2}%"), color)
        }
        _ => ("-".to_string(), muted_text()),
    }
}

/// The text a list value has on the screens: the stored value of some choices reads differently from the text the person picks.
const SHOWN_AS: &[(&str, &str)] = &[
    ("Highest premium", "Highest Premium/Delta"),
    ("Middle premium", "Mid Premium/Delta"),
    ("Lowest premium", "Lowest Premium/Delta"),
    ("Narrowest", "Narrow Width"),
    ("Widest", "Wide Width"),
    ("Same width", "Matching Width"),
    ("Furthest OTM", "Most OTM"),
    ("Nearest OTM", "Least OTM"),
    ("Short premium", "ShortPremium"),
    ("Percent OTM", "PercentageOTM"),
    ("Offset from price", "StrikeOffset_Current"),
    ("Offset from open", "StrikeOffset_Open"),
    ("Offset from leg 1", "StrikeOffset_Leg1"),
    ("Offset from leg 2", "StrikeOffset_Leg2"),
    ("Offset from leg 3", "StrikeOffset_Leg3"),
    ("Fixed premium", "FixedPremium"),
    ("Points OTM", "PointsOTM"),
    ("Spread width", "SpreadWidth"),
    ("Premium cap", "Max Premium"),
    ("Market stop", "StopMarket"),
    ("Limit stop with OCO", "StopLimitOCO"),
    ("Limit stop", "StopLimit"),
    ("Given price", "Exact Price"),
    ("Mean price", "Average Price"),
    ("Short leg price", "Short Price"),
    ("Long leg price", "Long Price"),
    ("Profit percent", "Profit Level"),
    ("Target 1", "ProfitTarget1"),
    ("Target 2", "ProfitTarget2"),
    ("Target 3", "ProfitTarget3"),
    ("Stop factor", "Stop Multiple"),
    ("Fixed price", "Fixed Amount"),
    ("Target price", "Exit Price"),
];

/// How `value` reads in a drop-down list.
fn shown(value: &str) -> &str {
    SHOWN_AS.iter().find(|(stored, _)| *stored == value).map_or(value, |(_, text)| *text)
}

/// A dropdown over fixed text options; returns whether the user picked a different one.
pub fn form_combo(ui: &mut egui::Ui, id: impl std::hash::Hash, value: &mut String, options: &[&str], width: f32) -> bool {
    let mut changed = false;
    crate::widgets::metal_combo(ui, |ui| {
        egui::ComboBox::from_id_salt(id).width(width).selected_text(shown(value.as_str())).show_ui(ui, |ui| {
            for option in options {
                if ui.selectable_label(value == option, shown(option)).clicked() && value != option {
                    *value = option.to_string();
                    changed = true;
                }
            }
        })
    });
    changed
}

/// A number field that is a drop-down: a click on it opens the list of values to pick from, the values the person added
/// before, and "Custom...". Picking a value shows only that value. "Custom..." turns the field into a text field for one
/// entry; a number typed there is kept in the list from then on (see `number_lists::remember`). Without a list it is a
/// plain text field.
pub fn form_number(ui: &mut egui::Ui, value: &mut String, width: f32, list: &[&str]) {
    form_number_offering(ui, value, width, list, &|_| true);
}

/// `form_number` that offers only the values `offer` accepts, of the list and of those the person added (a width that fits the
/// strikes of the symbol, say). What is typed after "Custom..." is taken as it is; the check on saving reports a value that does
/// not fit.
pub fn form_number_offering(ui: &mut egui::Ui, value: &mut String, width: f32, list: &[&str], offer: &dyn Fn(&str) -> bool) {
    if list.is_empty() {
        form_text(ui, value, width);
        return;
    }
    let id = ui.next_auto_id();
    let typing_id = id.with("typing");
    if let Some(mut typed) = ui.data(|data| data.get_temp::<String>(typing_id)) {
        let response = crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut typed).desired_width(width));
        let focused_id = id.with("focused");
        if !ui.data(|data| data.get_temp::<bool>(focused_id).unwrap_or(false)) {
            response.request_focus();
            ui.data_mut(|data| data.insert_temp(focused_id, true));
        }
        if response.lost_focus() {
            if !ui.input(|input| input.key_pressed(egui::Key::Escape)) && !typed.trim().is_empty() {
                let key = crate::number_lists::list_key(list);
                *value = crate::number_lists::remember(&key, list, &typed).unwrap_or_else(|| typed.trim().to_string());
            }
            ui.data_mut(|data| {
                data.remove::<String>(typing_id);
                data.remove::<bool>(focused_id);
            });
        } else {
            ui.data_mut(|data| data.insert_temp(typing_id, typed));
        }
        return;
    }
    let key = crate::number_lists::list_key(list);
    let own = crate::number_lists::custom_values(&key);
    let shown = if value.trim().is_empty() { "-".to_string() } else { value.clone() };
    let mut start_typing = false;
    crate::widgets::metal_combo(ui, |ui| {
        egui::ComboBox::from_id_salt(id).width(width).selected_text(shown).show_ui(ui, |ui| {
            for item in list.iter().copied().chain(own.iter().map(String::as_str)).filter(|item| offer(item)) {
                if ui.selectable_label(value.as_str() == item, item).clicked() {
                    *value = item.to_string();
                }
            }
            ui.separator();
            if ui.selectable_label(false, "Custom...").clicked() {
                start_typing = true;
            }
        })
    });
    if start_typing {
        let current = value.clone();
        ui.data_mut(|data| data.insert_temp(typing_id, current));
    }
}

pub fn form_text(ui: &mut egui::Ui, value: &mut String, width: f32) {
    crate::widgets::metal_field(ui, egui::TextEdit::singleline(value).desired_width(width));
}

/// A switch followed by the state text shown next to it (e.g. "On"/"Off",
/// "Active"/"Not Active"); returns whether it was toggled.
pub fn form_toggle(ui: &mut egui::Ui, value: &mut bool, on: &str, off: &str) -> bool {
    ui.horizontal(|ui| {
        let changed = toggle_switch(ui, value).clicked();
        ui.label(if *value { on } else { off });
        changed
    })
    .inner
}

/// A field with its caption above it, like a header-over-control layout. Neither the
/// caption nor the control's text ever wraps: a group squeezed for width would otherwise break its
/// words into letter columns instead of keeping its natural size.
pub fn labeled<R>(ui: &mut egui::Ui, caption: &str, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        ui.label(caption);
        add_contents(ui)
    })
    .inner
}

/// Renders `add_contents`; when `error` is set, outlines it in red with the message below and, the
/// first time (`scroll_pending`), scrolls it into view so the user sees at once what to fix.
pub fn validated<R>(ui: &mut egui::Ui, error: Option<&str>, scroll_pending: &std::cell::Cell<bool>, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let inner = ui.scope(add_contents);
    if let Some(message) = error {
        ui.painter().rect_stroke(inner.response.rect.expand(3.0), 3.0, egui::Stroke::new(2.0_f32, RED));
        ui.add(egui::Label::new(RichText::new(message).color(RED).small()).wrap_mode(egui::TextWrapMode::Extend));
        if scroll_pending.replace(false) {
            inner.response.scroll_to_me(Some(egui::Align::Center));
        }
    }
    inner.inner
}

pub fn navy_bar(ui: &mut egui::Ui, title: &str) {
    // A small raised strip of metal with the title in light text.
    let background = ui.painter().add(egui::Shape::Noop);
    let out = egui::Frame::none().inner_margin(egui::vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(RichText::new(title).color(Color32::from_rgb(0xf4, 0xf1, 0xec)).strong());
    });
    ui.painter().set(background, crate::metal_tex::button(ui.ctx(), out.response.rect, 4.0, Color32::from_rgb(0x9a, 0x94, 0x8c)));
}

/// Room kept free at the right edge of full-width rows so egui's floating scrollbar doesn't cover
/// their right border.
pub const CARD_SCROLLBAR_GAP: f32 = 10.0;

/// A card stacked in a vertical layout spans the full available width, so every card on a page
/// lines up at the same width instead of each shrinking to its own content. Cards placed side by
/// side (inside `ui.horizontal`) keep their content width so they can share the row.
pub fn white_card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    framed_card(ui, add_contents);
}

thread_local! {
    /// Whether what is drawn now already sits inside a metal frame: a card inside it then needs no frame of its own.
    static IN_FRAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A card as on the Dashboard: a display sunk into a metal frame. Inside another frame it is only a group of its own,
/// so frames are never nested.
pub fn framed_card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let fill_width = !ui.layout().main_dir().is_horizontal();
    let body = |ui: &mut egui::Ui| {
        if fill_width {
            ui.set_width((ui.available_width() - CARD_SCROLLBAR_GAP).max(0.0));
        }
        add_contents(ui);
    };
    if IN_FRAME.with(std::cell::Cell::get) {
        egui::Frame::none().inner_margin(egui::vec2(0.0, 6.0)).show(ui, body);
        return;
    }
    let outer = IN_FRAME.with(|cell| cell.replace(true));
    metal_frame(ui, egui::Margin::symmetric(21.0, 19.0), true, body);
    IN_FRAME.with(|cell| cell.set(outer));
}

/// A frame of the metal design: the contents on a raised steel plate, or, with `recessed`, in a display sunk into the panel.
fn metal_frame(ui: &mut egui::Ui, margin: egui::Margin, recessed: bool, add_contents: impl FnOnce(&mut egui::Ui)) {
    let background = ui.painter().add(egui::Shape::Noop);
    let out = egui::Frame::none().inner_margin(margin).show(ui, add_contents);
    let rect = out.response.rect;
    let shape = if recessed { crate::metal_tex::frame(ui.ctx(), rect, CARD_ROUNDING + 4.0) } else { crate::metal_tex::plate(ui.ctx(), rect, CARD_ROUNDING) };
    ui.painter().set(background, shape);
}

/// A simple line chart of a running total: one point per day (spread evenly), the zero line, the
/// highest and lowest value on the left, the first and last day below. Hovering a point shows its
/// day and value. `points` are (day yyyy-MM-dd, value in dollars), oldest first.
pub fn line_chart(ui: &mut egui::Ui, points: &[(String, f64)], height: f32) {
    let width = ui.available_width().max(200.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CARD_ROUNDING, card_bg());
    let plot = egui::Rect::from_min_max(rect.min + egui::vec2(78.0, 16.0), rect.max - egui::vec2(20.0, 30.0));
    let (lo, hi) = points.iter().fold((0.0_f64, 0.0_f64), |(lo, hi), (_, v)| (lo.min(*v), hi.max(*v)));
    let span = if (hi - lo).abs() < f64::EPSILON { 1.0 } else { hi - lo };
    let y_of = |v: f64| plot.bottom() - ((v - lo) / span).to_f32().unwrap_or(0.0) * plot.height();
    let x_of = |i: usize| {
        if points.len() <= 1 {
            plot.center().x
        } else {
            plot.left() + plot.width() * (i.to_f32().unwrap_or(0.0) / (points.len() - 1).to_f32().unwrap_or(0.0))
        }
    };
    let font = egui::FontId::proportional(12.0);
    let dmy = |ymd: &str| ymd.split('-').rev().collect::<Vec<_>>().join("-");
    let money = |v: f64| format!("{}${:.0}", if v < 0.0 { "-" } else { "" }, v.abs());
    // Horizontal guides at the highest value, zero and the lowest value.
    for (value, strong) in [(hi, false), (0.0, true), (lo, false)] {
        let y = y_of(value);
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(1.0_f32, if strong { muted_text() } else { card_border() }),
        );
        painter.text(egui::pos2(plot.left() - 8.0, y), egui::Align2::RIGHT_CENTER, money(value), font.clone(), muted_text());
    }
    if let (Some((first, _)), Some((last, _))) = (points.first(), points.last()) {
        painter.text(egui::pos2(plot.left(), plot.bottom() + 8.0), egui::Align2::LEFT_TOP, dmy(first), font.clone(), muted_text());
        if points.len() > 1 {
            painter.text(egui::pos2(plot.right(), plot.bottom() + 8.0), egui::Align2::RIGHT_TOP, dmy(last), font.clone(), muted_text());
        }
    }
    let positions: Vec<egui::Pos2> = points.iter().enumerate().map(|(i, (_, v))| egui::pos2(x_of(i), y_of(*v))).collect();
    painter.add(egui::Shape::line(positions.clone(), egui::Stroke::new(2.0_f32, ACCENT_BLUE)));
    for (pos, (_, v)) in positions.iter().zip(points) {
        painter.circle_filled(*pos, 3.5, if *v >= 0.0 { STATUS_OK } else { STATUS_ERROR });
    }
    if let Some(((day, value), pos)) = points.last().zip(positions.last()) {
        painter.text(*pos + egui::vec2(12.0, -10.0), egui::Align2::RIGHT_BOTTOM, format!("{} {}", dmy(day), money(*value)), font, text_primary());
    }
    // Hover: the point nearest to the pointer along the x axis.
    if let Some(pointer) = response.hover_pos() {
        let nearest = positions.iter().enumerate().min_by(|(_, a), (_, b)| (a.x - pointer.x).abs().total_cmp(&(b.x - pointer.x).abs())).map(|(i, _)| i);
        if let Some((day, value)) = nearest.and_then(|i| points.get(i)) {
            response.on_hover_text_at_pointer(format!("{}: {}", dmy(day), money(*value)));
        }
    }
}

/// The market status as a banner: heavy white capital letters on a glowing green (open) or red (closed) field. While the
/// market is open the glow breathes slowly (a live signal); while it is closed it is still.
pub fn market_badge(ui: &mut egui::Ui, text: &str, open: bool) {
    let upper = text.to_uppercase();
    // The state as lit text in its colour (green open, red closed) with a soft glow of the same colour.
    let color = if open { Color32::from_rgb(0x5a, 0xe0, 0x96) } else { Color32::from_rgb(0xff, 0x6e, 0x60) };
    let glow = ui.painter().add(egui::Shape::Noop);
    let response = ui.label(RichText::new(upper).strong().size(14.0).color(color));
    let (left, right) = (response.rect.left_center(), response.rect.right_center());
    ui.painter().set(glow, crate::metal_tex::glow_along(ui.ctx(), left, right, response.rect.height() * 1.6, color.gamma_multiply(0.18)));
}

/// The points of a curve inside `rect`, oldest first, and the height of `base` (a reference line such as the prior close) on the
/// same scale. The scale is the curve's own low to high, so even small moves fill the height of the box; `base` gets a height only
/// when it lies within that range (a reference far away, after a gap at the open, would squeeze the curve into a thin band). A flat
/// curve runs through the middle.
#[must_use]
pub fn sparkline_points(values: &[f64], base: Option<f64>, rect: egui::Rect) -> (Vec<egui::Pos2>, Option<f32>) {
    let (min, max) = values.iter().copied().fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), v| (low.min(v), high.max(v)));
    if values.is_empty() || !min.is_finite() || !max.is_finite() {
        return (Vec::new(), None);
    }
    let span = max - min;
    let y_of = |value: f64| -> f32 {
        let share = if span.abs() < f64::EPSILON { 0.5 } else { ((value - min) / span).to_f32().unwrap_or(0.5) };
        rect.bottom() - share * rect.height()
    };
    let steps = values.len().saturating_sub(1).max(1).to_f32().unwrap_or(1.0);
    let points = values
        .iter()
        .enumerate()
        .map(|(index, value)| egui::pos2(rect.left() + rect.width() * (index.to_f32().unwrap_or(0.0) / steps), y_of(*value)))
        .collect();
    (points, base.filter(|base| (min..=max).contains(base)).map(y_of))
}

/// The colour of a good (green) or bad (red) day, for the fill of the curve and the change chip.
#[must_use]
pub fn market_badge_fill(up: bool) -> Color32 {
    if up {
        Color32::from_rgb(0x1e, 0x9e, 0x57)
    } else {
        Color32::from_rgb(0xc4, 0x33, 0x33)
    }
}

/// A small curve: a line over a soft fill in the colour of the day (green above the reference, red below), a dashed
/// reference line, and a dot at the newest value. Nothing is drawn for fewer than two values.
pub fn sparkline(ui: &mut egui::Ui, values: &[f64], base: Option<f64>, size: egui::Vec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let up = match (values.last(), base.or_else(|| values.first().copied())) {
        (Some(last), Some(start)) => *last >= start,
        _ => true,
    };
    let color = market_badge_fill(up);
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, color.gamma_multiply(0.10));
    painter.rect_stroke(rect, 6.0, egui::Stroke::new(1.0_f32, color.gamma_multiply(0.35)));
    let inner = rect.shrink2(egui::vec2(6.0, 5.0));
    let (points, base_y) = sparkline_points(values, base, inner);
    if points.len() < 2 {
        return response;
    }
    if let Some(y) = base_y {
        let stroke = egui::Stroke::new(1.0_f32, Color32::from_gray(150).gamma_multiply(0.6));
        painter.extend(egui::Shape::dashed_line(&[egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)], stroke, 3.0, 3.0));
    }
    let mut fill = egui::Mesh::default();
    let (top, bottom) = (color.gamma_multiply(0.40), Color32::TRANSPARENT);
    for pair in points.windows(2) {
        if let [a, b] = pair {
            let first = u32::try_from(fill.vertices.len()).unwrap_or(0);
            fill.colored_vertex(*a, top);
            fill.colored_vertex(*b, top);
            fill.colored_vertex(egui::pos2(b.x, inner.bottom()), bottom);
            fill.colored_vertex(egui::pos2(a.x, inner.bottom()), bottom);
            fill.add_triangle(first, first + 1, first + 2);
            fill.add_triangle(first, first + 2, first + 3);
        }
    }
    painter.add(egui::Shape::mesh(fill));
    painter.add(egui::Shape::line(points.clone(), egui::Stroke::new(1.6_f32, color)));
    if let Some(last) = points.last() {
        painter.circle_filled(*last, 3.0, color);
        painter.circle_stroke(*last, 3.0, egui::Stroke::new(1.2_f32, Color32::from_white_alpha(200)));
    }
    response
}

/// The change as a rounded chip: a white triangle and the percentage on the colour of the day (green for a gain, red for a
/// loss).
pub fn change_chip(ui: &mut egui::Ui, percent: f64) -> egui::Response {
    let up = percent >= 0.0;
    let galley = ui.painter().layout_no_wrap(format!("{percent:+.2}%"), egui::FontId::proportional(12.5), Color32::WHITE);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(galley.size().x + 34.0, BAR_ITEM_HEIGHT), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, BAR_ITEM_HEIGHT / 2.0, market_badge_fill(up));
    let (cx, cy) = (rect.left() + 15.0, rect.center().y);
    let triangle = if up {
        vec![egui::pos2(cx - 4.0, cy + 3.0), egui::pos2(cx + 4.0, cy + 3.0), egui::pos2(cx, cy - 4.0)]
    } else {
        vec![egui::pos2(cx - 4.0, cy - 3.0), egui::pos2(cx + 4.0, cy - 3.0), egui::pos2(cx, cy + 4.0)]
    };
    painter.add(egui::Shape::convex_polygon(triangle, Color32::WHITE, egui::Stroke::NONE));
    painter.galley(egui::pos2(rect.left() + 25.0, rect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    response
}

/// The heading of a section of the Settings page: bold text above its rows, no filled bar.
pub fn section_header(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(15.0).strong().color(text_primary()));
    ui.add_space(4.0);
}

/// One setting as a row: the name (and a short grey hint under it) on the left, the control on the right, a thin line below.
/// The control is laid out from the right edge; put several widgets into a `ui.horizontal` of their own to keep their order.
pub fn setting_row(ui: &mut egui::Ui, title: &str, hint: &str, control: impl FnOnce(&mut egui::Ui)) {
    setting_row_noted(ui, title, hint, "", control);
}

/// Like `setting_row`, with an amber `note` under the hint, e.g. for a setting that is saved but has no effect yet.
pub fn setting_row_noted(ui: &mut egui::Ui, title: &str, hint: &str, note: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(6.0);
    let width = (ui.available_width() - CARD_SCROLLBAR_GAP).max(0.0);
    ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.allocate_ui_with_layout(egui::vec2(width * 0.5, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).color(text_primary()));
                if crate::settings_help::help_for(title).is_some() {
                    help_button(ui, title);
                }
            });
            if !hint.is_empty() {
                ui.label(RichText::new(hint).small().color(muted_text()));
            }
            if !note.is_empty() {
                ui.label(RichText::new(note).small().color(placeholder_color()));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(6.0);
    ui.add(egui::Separator::default().spacing(0.0));
}

/// The egui memory slot of the help window that is open: the name of its setting.
fn help_slot() -> egui::Id {
    egui::Id::new("settings_help_open")
}

/// A small round "?" button that opens the help of the setting `name` (see `settings_help`).
pub fn help_button(ui: &mut egui::Ui, name: &str) {
    let button = egui::Button::new(RichText::new("?").small().strong().color(text_primary())).rounding(9.0).min_size(egui::vec2(18.0, 18.0));
    if ui.add(button).on_hover_text(if name.is_empty() { "?" } else { name }).clicked() {
        let name = name.to_string();
        ui.ctx().data_mut(|data| data.insert_temp(help_slot(), name));
    }
}

/// The sentences of a help text: it is split after a full stop, an exclamation or a question mark followed by a space, but not after
/// an abbreviation (a word before the stop of one or two letters, or one with a stop inside, such as "z. B." or "e.g.").
pub fn sentences(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for window in chars.windows(2) {
        let [(at, mark), (_, next)] = window else { continue };
        if !matches!(mark, '.' | '!' | '?') || *next != ' ' {
            continue;
        }
        let before = text.get(start..*at).unwrap_or_default();
        let word = before.rsplit(' ').next().unwrap_or_default();
        if word.chars().count() <= 2 || word.contains('.') {
            continue;
        }
        let end = at + mark.len_utf8();
        if let Some(sentence) = text.get(start..end) {
            parts.push(sentence.trim());
        }
        start = end;
    }
    if let Some(rest) = text.get(start..).map(str::trim).filter(|rest| !rest.is_empty()) {
        parts.push(rest);
    }
    parts
}

/// The help of a setting as a card of the metal design: a "?" badge, `kicker` (where it belongs) and `title` on top, the first sentence of
/// `text` as the lead in bold, the rest in paragraphs of two sentences, and a button `close` at the bottom right. True when it or the
/// cross on top was clicked.
pub fn help_card(ui: &mut egui::Ui, kicker: &str, title: &str, text: &str, close: &str) -> bool {
    let mut closed = false;
    framed_card(ui, |ui| {
        ui.set_width(460.0);
        ui.horizontal(|ui| {
            let (badge, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
            ui.painter().circle_filled(badge.center(), 16.0, ACCENT_BLUE);
            ui.painter().circle_stroke(badge.center(), 16.0, egui::Stroke::new(1.0_f32, Color32::from_white_alpha(60)));
            ui.painter().text(badge.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(19.0), Color32::WHITE);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(kicker).size(11.5).color(muted_text()));
                ui.label(RichText::new(title).size(17.0).strong().color(text_primary()));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui.add(egui::Button::new(RichText::new("\u{d7}").size(18.0).color(muted_text())).frame(false)).clicked() {
                    closed = true;
                }
            });
        });
        ui.add_space(10.0);
        ui.add(egui::Separator::default().spacing(0.0));
        ui.add_space(12.0);
        let parts = sentences(text);
        if let Some(lead) = parts.first() {
            ui.label(RichText::new(*lead).size(14.5).strong().color(text_primary()));
        }
        for pair in parts.get(1..).unwrap_or_default().chunks(2) {
            ui.add_space(8.0);
            ui.label(RichText::new(pair.join(" ")).size(13.5).color(text_primary().gamma_multiply(0.88)));
        }
        ui.add_space(16.0);
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button = MetalButton::new(RichText::new(close).strong().color(Color32::WHITE)).fill(ACCENT_BLUE).rounding(8.0);
            if ui.add_sized([120.0, 28.0], button).clicked() {
                closed = true;
            }
        });
    });
    closed
}

/// The name of the setting whose help is open, if any.
pub fn open_help(ctx: &egui::Context) -> Option<String> {
    ctx.data(|data| data.get_temp::<String>(help_slot()))
}

/// Closes the help window.
pub fn close_help(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<String>(help_slot()));
}

/// One entry of the navigation of the Settings page: a coloured badge with a symbol and the name; the chosen entry is filled.
pub fn nav_item(ui: &mut egui::Ui, symbol: &str, label: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        // The chosen entry lies in a darker, sunken groove with a steel-blue mark on the left, like in the sidebar.
        let steel_blue = Color32::from_rgb(0x7c, 0x8f, 0xa8);
        if selected {
            ui.painter().rect_filled(rect, 4.0, Color32::from_black_alpha(95));
            ui.painter().line_segment([rect.left_top(), rect.right_top()], egui::Stroke::new(1.0_f32, Color32::from_black_alpha(150)));
            ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0_f32, Color32::from_white_alpha(22)));
            ui.painter().rect_filled(egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, rect.height())), 0.0, steel_blue);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 4.0, Color32::from_white_alpha(10));
        }
        let badge = egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), egui::vec2(22.0, 22.0));
        ui.painter().rect_filled(badge, 6.0, if selected { steel_blue } else { muted_text().gamma_multiply(0.35) });
        ui.painter().text(badge.center(), egui::Align2::CENTER_CENTER, symbol, egui::FontId::proportional(13.0), Color32::WHITE);
        ui.painter().text(egui::pos2(rect.left() + 40.0, rect.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(14.0), text_primary());
    }
    response
}

/// An input field (an `egui::TextEdit`) sunk into the metal: a dark slot with a light lower lip behind it; while it is
/// being edited its outline shows in the selection colour.
pub fn metal_field(ui: &mut egui::Ui, field: impl egui::Widget) -> egui::Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .scope(|ui| {
            let visuals = ui.visuals_mut();
            visuals.extreme_bg_color = Color32::TRANSPARENT;
            visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(0x6a, 0x65, 0x5f));
            ui.add(field)
        })
        .inner;
    ui.painter().set(background, crate::metal_tex::field(ui.ctx(), response.rect));
    response
}

/// A drop-down (an `egui::ComboBox` built in `add`) on a raised metal button like the other buttons; its list opens as
/// before.
pub fn metal_combo<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let background = ui.painter().add(egui::Shape::Noop);
    let inner = ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        for widget in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active, &mut visuals.widgets.open] {
            widget.bg_fill = Color32::TRANSPARENT;
            widget.weak_bg_fill = Color32::TRANSPARENT;
            widget.bg_stroke = egui::Stroke::NONE;
        }
        add(ui)
    });
    let rect = inner.response.rect;
    let tint = if ui.rect_contains_pointer(rect) { Color32::from_rgb(0xd8, 0xd4, 0xce) } else { Color32::from_rgb(0xc0, 0xbc, 0xb6) };
    ui.painter().set(background, crate::metal_tex::button(ui.ctx(), rect, BAR_CORNER, tint));
    inner.inner
}

/// A button of raised satin metal, like the buttons of the top bar: a drop-in for `egui::Button` (same way of building
/// it). A colour given with `fill` tints the metal (blue, green, red for the main, the positive and the ending action);
/// without one it is grey. It is a little lighter while the pointer is on it, darker while pressed, and dimmed while
/// disabled.
pub struct MetalButton {
    text: egui::WidgetText,
    fill: Option<Color32>,
    min_size: egui::Vec2,
}

impl MetalButton {
    /// A grey metal button with `text`.
    pub fn new(text: impl Into<egui::WidgetText>) -> Self {
        Self { text: text.into(), fill: None, min_size: egui::Vec2::ZERO }
    }
    /// Tints the metal with `fill`.
    #[must_use]
    pub fn fill(mut self, fill: impl Into<Color32>) -> Self {
        self.fill = Some(fill.into());
        self
    }
    /// The smallest size of the button.
    #[must_use]
    pub fn min_size(mut self, min_size: egui::Vec2) -> Self {
        self.min_size = min_size;
        self
    }
    /// The corner radius is that of all metal buttons; this only keeps the way of building an `egui::Button`.
    #[must_use]
    pub fn rounding(self, _rounding: impl Into<egui::Rounding>) -> Self {
        self
    }
}

impl egui::Widget for MetalButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let background = ui.painter().add(egui::Shape::Noop);
        let button = egui::Button::new(self.text).min_size(self.min_size).fill(Color32::TRANSPARENT).stroke(egui::Stroke::NONE);
        let response = ui.add(button);
        // The grey of the metal, or the given colour made lighter so the metal still shows through.
        let tint =
            self.fill.filter(|fill| *fill != Color32::TRANSPARENT).map_or(Color32::from_rgb(0xc8, 0xc4, 0xbe), |fill| fill.lerp_to_gamma(Color32::WHITE, 0.25));
        let tint = if !response.enabled() {
            tint.gamma_multiply(0.55)
        } else if response.is_pointer_button_down_on() {
            tint.gamma_multiply(0.82)
        } else if response.hovered() {
            tint.lerp_to_gamma(Color32::WHITE, 0.15)
        } else {
            tint
        };
        ui.painter().set(background, crate::metal_tex::button(ui.ctx(), response.rect, BAR_CORNER, tint));
        response
    }
}

/// One row of a list as a raised strip of metal over `rect`: a little lighter (every other row a shade darker), a light
/// upper edge and a dark lower edge; `header` for the row of column titles, a lighter strip of steel.
pub fn plastic_strip(rect: egui::Rect, index: usize, header: bool) -> egui::Shape {
    let fill = if header {
        Color32::from_rgb(0x4a, 0x46, 0x42)
    } else if index.is_multiple_of(2) {
        Color32::from_rgb(0x36, 0x33, 0x30)
    } else {
        Color32::from_rgb(0x30, 0x2d, 0x2a)
    };
    egui::Shape::Vec(vec![
        egui::Shape::rect_filled(rect.shrink2(egui::vec2(0.0, 1.0)), 3.0, fill),
        egui::Shape::line_segment(
            [rect.left_top() + egui::vec2(3.0, 1.5), rect.right_top() + egui::vec2(-3.0, 1.5)],
            egui::Stroke::new(1.0_f32, Color32::from_white_alpha(if header { 60 } else { 38 })),
        ),
        egui::Shape::line_segment(
            [rect.left_bottom() + egui::vec2(3.0, -1.0), rect.right_bottom() + egui::vec2(-3.0, -1.0)],
            egui::Stroke::new(1.0_f32, Color32::from_black_alpha(150)),
        ),
    ])
}

/// The rows of a list drawn as raised strips of metal: each row a slightly lighter strip with a light upper edge and a
/// dark lower edge, the header row a lighter strip of steel. Call `row` at the start of every row of a grid (the header
/// first, if there is one), then `paint` with the rectangle of the grid.
#[derive(Default)]
pub struct PlasticRows {
    rows: Vec<(egui::layers::ShapeIdx, f32, bool)>,
}

impl PlasticRows {
    /// Marks the start of a row; `header` for the row of column titles.
    pub fn row(&mut self, ui: &egui::Ui, header: bool) {
        self.rows.push((ui.painter().add(egui::Shape::Noop), ui.cursor().top(), header));
    }

    /// Paints the strips behind the rows of the grid over `grid`; `gap` is the space between two rows.
    pub fn paint(self, ui: &egui::Ui, grid: egui::Rect, gap: f32) {
        let tops: Vec<f32> = self.rows.iter().map(|(_, top, _)| *top).collect();
        // The strips run over the whole width of the area the list is in (as its title bar does), at least over the list.
        let area = ui.max_rect();
        let (left, right) = (area.left().min(grid.left()), grid.right().max(area.right().min(ui.clip_rect().right())));
        for (index, (shape, top, header)) in self.rows.into_iter().enumerate() {
            let bottom = tops.get(index + 1).map_or(grid.bottom(), |next| next - gap);
            let rect = egui::Rect::from_min_max(egui::pos2(left, top - gap / 2.0), egui::pos2(right, bottom + gap / 2.0));
            ui.painter().set(shape, plastic_strip(rect, index, header));
        }
    }
}

/// The light text of a price in `price_readout` that has no direction.
pub const READOUT_TEXT: Color32 = Color32::from_rgb(0xf2, 0xef, 0xea);

/// How much darker the plate of a `price_readout` is than the light steel of the bar it sits on.
const READOUT_PLATE_TINT: Color32 = Color32::from_gray(168);

/// The radius of the rivets at the ends of a `price_readout` plate, and how far their centres sit from its ends.
const READOUT_RIVET_RADIUS: f32 = 2.2;
const READOUT_RIVET_INSET: f32 = 5.5;

/// A price in the top bar as a small raised plate of darker steel, riveted onto the bar at both ends like a name plate: the figures in
/// `color` (`READOUT_TEXT`, or the green, red or orange of its state) with a dark shadow below them, so they stay easy to read on the
/// metal.
pub fn price_readout(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::monospace(15.0), color);
    // Room at the ends for the rivets.
    let margin = egui::vec2(2.0 * READOUT_RIVET_INSET + 3.0, 3.0);
    let (rect, response) = ui.allocate_exact_size(galley.size() + 2.0 * margin, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.add(crate::metal_tex::tinted_plate(ui.ctx(), rect, 4.0, READOUT_PLATE_TINT));
        for x in [rect.left() + READOUT_RIVET_INSET, rect.right() - READOUT_RIVET_INSET] {
            painter.add(crate::metal_tex::knob(ui.ctx(), egui::pos2(x, rect.center().y), READOUT_RIVET_RADIUS, Color32::from_gray(200)));
        }
        let pos = rect.center() - galley.size() / 2.0;
        painter.galley_with_override_text_color(pos + egui::vec2(0.0, 1.3), galley.clone(), Color32::from_black_alpha(200));
        painter.galley_with_override_text_color(pos + egui::vec2(0.6, 0.6), galley.clone(), Color32::from_black_alpha(90));
        painter.galley(pos, galley.clone(), color);
        // Bold like the other figures of the bar: the text once more, a hair to the right.
        painter.galley(pos + egui::vec2(0.5, 0.0), galley, color);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compact_table_drops_columns_from_the_second_to_last_backwards_until_the_rest_is_readable() {
        let weights = [1.0, 1.0, 1.0, 1.0];
        assert_eq!(visible_columns(&weights, 400.0, 50.0, 6.0), [0, 1, 2, 3], "wide enough for all");
        assert_eq!(visible_columns(&weights, 200.0, 50.0, 6.0), [0, 1, 3], "the third column goes first");
        assert_eq!(visible_columns(&weights, 50.0, 50.0, 6.0), [0, 3], "the first and the last always stay");
        assert_eq!(visible_columns(&[2.0], 10.0, 50.0, 6.0), [0]);
        assert_eq!(visible_columns(&[], 100.0, 50.0, 6.0), Vec::<usize>::new());
    }

    #[test]
    fn the_choices_of_a_template_read_as_they_always_did_while_their_stored_values_differ() {
        assert_eq!(shown("Highest premium"), "Highest Premium/Delta");
        assert_eq!(shown("Market stop"), "StopMarket");
        assert_eq!(shown("Target 2"), "ProfitTarget2");
        assert_eq!(shown("Credit"), "Credit", "a value without its own text reads as it is");
        let stored: Vec<&str> = SHOWN_AS.iter().map(|(stored, _)| *stored).collect();
        assert!(crate::template_form::PREFERENCES
            .iter()
            .chain(crate::template_form::STOP_ORDER_TYPES.iter())
            .all(|v| stored.contains(v) || !v.contains(' ') || *v == "None"));
    }

    #[test]
    fn a_curve_fills_its_box_with_the_highest_value_on_top_and_the_reference_line_inside() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 20.0));
        let (points, base_y) = sparkline_points(&[10.0, 20.0, 15.0], Some(12.0), rect);
        assert_eq!(points.len(), 3);
        assert!(points[0].x.abs() < 1e-3 && (points[2].x - 100.0).abs() < 1e-3, "{points:?}");
        assert!(points[1].y.abs() < 1e-3 && (points[0].y - 20.0).abs() < 1e-3, "the highest value is on top, the lowest at the bottom: {points:?}");
        let base_y = base_y.unwrap();
        assert!(base_y > points[1].y && base_y < points[0].y, "the reference line lies between them");
        // A flat curve runs through the middle, and nothing is drawn without values.
        assert!((sparkline_points(&[5.0, 5.0], None, rect).0[0].y - 10.0).abs() < 1e-3);
        assert_eq!(sparkline_points(&[], None, rect).0.len(), 0);
        // A reference far away does not squeeze the curve: a move of 2 points still fills the whole height, and the line is left out.
        let (below, base_y) = sparkline_points(&[7250.0, 7252.0], Some(7300.0), rect);
        assert!((below[0].y - 20.0).abs() < 1e-3 && below[1].y.abs() < 1e-3, "{below:?}");
        assert_eq!(base_y, None);
    }

    /// The whole way through egui with the mouse, on a table with selection boxes like those of the Dashboard: the border between
    /// two headers shows the sizing cursor, dragging it widens the left column by what the right one gives up, and the widths are
    /// kept once the button is let go.
    #[test]
    fn the_border_between_two_headers_is_dragged_with_the_mouse_to_size_the_columns() {
        let ctx = egui::Context::default();
        let id = "test_mouse_resize_table";
        let keys = ["a", "b", "c"];
        let columns = vec![TableColumn::new("Alpha", 1.0, false), TableColumn::new("Beta", 1.0, false), TableColumn::new("Gamma", 1.0, false)];
        let rows = vec![vec![TableCell::new("1"), TableCell::new("2"), TableCell::new("3")]];
        let frame = |events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 300.0));
            let input = egui::RawInput { events, screen_rect: Some(screen), ..egui::RawInput::default() };
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = arranged_table(ui, id, &keys, &columns, &rows, Some(RowSelection::default()));
                });
            })
        };
        let cursor_at = |pos: egui::Pos2| frame(vec![egui::Event::PointerMoved(pos)]).platform_output.cursor_icon;
        let _ = frame(Vec::new());
        // The header row: where the first column shows the cursor that moves it.
        let header_y = (0_u16..80).step_by(2).map(f32::from).find(|y| cursor_at(egui::pos2(150.0, *y)) == egui::CursorIcon::Grab).expect("the header row");
        // The border after the first column: where the sizing cursor shows.
        let grip_x = (0_u16..900).map(f32::from).find(|x| cursor_at(egui::pos2(*x, header_y)) == egui::CursorIcon::ResizeHorizontal).expect("a border");
        let grip = egui::pos2(grip_x, header_y);
        let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = frame(vec![press(grip, true)]);
        for step in 1_u16..=6 {
            let _ = frame(vec![egui::Event::PointerMoved(grip + egui::vec2(10.0 * f32::from(step), 0.0))]);
        }
        let _ = frame(vec![press(grip + egui::vec2(60.0, 0.0), false)]);
        let _ = frame(Vec::new());
        let (_, widths) = crate::column_order::layout(id, &keys, &[1.0, 1.0, 1.0]);
        assert!(widths[0] > 1.1 && widths[1] < 0.9, "the first column grew, the second gave it up: {widths:?}");
        assert!((widths[0] + widths[1] - 2.0).abs() < 1e-3 && (widths[2] - 1.0).abs() < 1e-6, "the table keeps its width: {widths:?}");
    }

    fn button(button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: egui::pos2(10.0, 10.0), button, pressed, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn a_right_click_asks_for_a_copy_of_the_selected_text() {
        // Pressing the right button: a copy (once), and the press itself does not reach the widgets.
        let mut events = vec![button(egui::PointerButton::Secondary, true)];
        copy_on_right_click(&mut events);
        assert!(matches!(events.as_slice(), [egui::Event::Copy]), "{events:?}");
        copy_on_right_click(&mut events);
        assert!(matches!(events.as_slice(), [egui::Event::Copy]), "not twice: {events:?}");
        // Releasing it: no copy, and the release is taken out as well.
        let mut released = vec![button(egui::PointerButton::Secondary, false)];
        copy_on_right_click(&mut released);
        assert!(released.is_empty(), "{released:?}");
        // A left click and no click stay as they are.
        let mut left = vec![button(egui::PointerButton::Primary, true)];
        copy_on_right_click(&mut left);
        assert_eq!(left, vec![button(egui::PointerButton::Primary, true)]);
        let mut none = Vec::new();
        copy_on_right_click(&mut none);
        assert_eq!(none, [] as [eframe::egui::Event; 0]);
    }

    /// The whole way through egui: text selected in a field is in the clipboard after a right click.
    #[test]
    fn a_right_click_puts_the_selected_text_of_a_field_into_the_clipboard() {
        let ctx = egui::Context::default();
        let mut text = String::from("SPX 6650/6640 P");
        let id = egui::Id::new("copy_test_field");
        let frame = |ctx: &egui::Context, text: &mut String, events: Vec<egui::Event>| {
            let mut input = egui::RawInput { events, ..egui::RawInput::default() };
            copy_on_right_click(&mut input.events);
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    crate::widgets::metal_field(ui, egui::TextEdit::singleline(text).id(id));
                });
            })
        };
        // First frame: the field exists; then it gets the focus and "SPX" is selected.
        let _ = frame(&ctx, &mut text, Vec::new());
        ctx.memory_mut(|m| m.request_focus(id));
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(3))));
        state.store(&ctx, id);
        let _ = frame(&ctx, &mut text, Vec::new());
        // The right click.
        let output = frame(&ctx, &mut text, vec![button(egui::PointerButton::Secondary, true)]);
        assert_eq!(output.platform_output.copied_text, "SPX");
        // The next frame removes the selection; the text stays.
        clear_selection_after_copy(&ctx);
        let _ = frame(&ctx, &mut text, Vec::new());
        let range = egui::TextEdit::load_state(&ctx, id).and_then(|s| s.cursor.char_range()).expect("cursor");
        assert_eq!(range.primary, range.secondary, "no selection left");
        assert_eq!(text, "SPX 6650/6640 P");
    }

    /// The same for text selected with the mouse in a label (tables, the trade details).
    #[test]
    fn a_right_click_puts_the_text_selected_in_a_label_into_the_clipboard() {
        let ctx = egui::Context::default();
        let frame = |events: Vec<egui::Event>| {
            let mut input = egui::RawInput { events, ..egui::RawInput::default() };
            copy_on_right_click(&mut input.events);
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Order 11 STP");
                });
            })
        };
        let at = |x: f32| egui::pos2(x, 14.0);
        let primary =
            |pressed: bool, x: f32| egui::Event::PointerButton { pos: at(x), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let _ = frame(Vec::new());
        // Drag across the whole label: hover it, press, move in steps, release.
        let _ = frame(vec![egui::Event::PointerMoved(at(9.0))]);
        let _ = frame(vec![primary(true, 9.0)]);
        for x in [20.0, 60.0, 120.0, 300.0] {
            let _ = frame(vec![egui::Event::PointerMoved(at(x))]);
        }
        let _ = frame(vec![primary(false, 300.0)]);
        // The right click on it.
        let right =
            frame(vec![egui::Event::PointerButton { pos: at(30.0), button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE }]);
        let copied = right.platform_output.copied_text;
        assert!(!copied.is_empty() && "Order 11 STP".starts_with(&copied), "{copied:?}");
        // It copied what Ctrl+C copies - and the selection is still there afterwards.
        let ctrl_c = frame(vec![egui::Event::Copy]);
        assert_eq!(ctrl_c.platform_output.copied_text, copied);
    }

    #[test]
    fn the_grid_font_size_is_read_from_the_setting_and_kept_in_range() {
        // The pure conversion: no shared state, so other tests that draw tables (and set the size of
        // their settings) cannot get in the way.
        for (text, expected) in [("14", 14), ("18", 18), (" 16 ", 16), ("12", 12)] {
            assert_eq!(grid_font_size_of(text), expected, "{text}");
        }
        // Nonsense keeps the default, and too small or too big sizes are held to 10 and 24.
        for (text, expected) in [("", 14), ("big", 14), ("-3", 14), ("2", 10), ("0", 10), ("99", 24)] {
            assert_eq!(grid_font_size_of(text), expected, "{text:?}");
        }
    }

    #[test]
    fn the_start_up_sweep_runs_to_the_end_and_settles_on_the_real_value() {
        // PCR gauge: -100..100, the real value is 30.
        assert_eq!(sweep_value(-100.0, 100.0, 30.0, 0.0), -100.0, "starts at the beginning of the scale");
        assert!((sweep_value(-100.0, 100.0, 30.0, SWEEP_UP_SECONDS / 2.0) - 0.0).abs() < 1e-9, "half way at half time");
        assert_eq!(sweep_value(-100.0, 100.0, 30.0, SWEEP_UP_SECONDS), 100.0, "touches the end");
        let back_half = sweep_value(-100.0, 100.0, 30.0, SWEEP_UP_SECONDS + SWEEP_DOWN_SECONDS / 2.0);
        assert!((back_half - 65.0).abs() < 1e-9, "half way back: {back_half}");
        assert_eq!(sweep_value(-100.0, 100.0, 30.0, SWEEP_UP_SECONDS + SWEEP_DOWN_SECONDS), 30.0, "ends on the real value");
        // Never leaves the scale, whatever the real value.
        for step in 0..=40 {
            let v = sweep_value(0.0, 100.0, 0.0, f64::from(step) * 0.05);
            assert!((0.0..=100.0).contains(&v), "{v}");
        }
        assert!(sweep_active(0.5) && sweep_active(1.7));
        assert!(!sweep_active(SWEEP_UP_SECONDS + SWEEP_DOWN_SECONDS));
    }

    #[test]
    fn live_data_delta_is_computed_only_once_both_real_ticks_have_arrived() {
        assert_eq!(format_delta(None, None), ("-".to_string(), muted_text()));
        assert_eq!(format_delta(Some(101.0), None), ("-".to_string(), muted_text()));
        assert_eq!(format_delta(Some(101.0), Some(100.0)), ("+1.00\n+1.00%".to_string(), GREEN_TXT));
        assert_eq!(format_delta(Some(98.0), Some(100.0)), ("-2.00\n-2.00%".to_string(), RED));
    }
}
