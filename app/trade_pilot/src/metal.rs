//! Parts of the metal design put together from the pictures of `metal_tex`: the slide switch, the top bar and the small clock of the
//! top bar.

use eframe::egui::{self, Color32, Rect, Shape};

/// A slide switch of the metal design over `track`: a recessed slot lit faintly in `lit` and a domed steel knob at `t`
/// (0 = left end, 1 = right end).
#[must_use]
pub fn slide_switch(ctx: &egui::Context, track: Rect, lit: Color32, t: f32) -> Vec<Shape> {
    let radius = track.height() / 2.0;
    let mut shapes = vec![crate::metal_tex::slot(ctx, track, lit)];
    let knob = radius - 2.0;
    let center = egui::pos2(egui::lerp((track.left() + radius)..=(track.right() - radius), t), track.center().y);
    shapes.push(crate::metal_tex::knob(ctx, center, knob, Color32::WHITE));
    shapes
}

/// Paints the top bar of the metal design behind its contents: light brushed metal lit from above (see
/// `metal_tex::top_bar`).
pub fn paint_top_bar(ui: &egui::Ui) {
    let bar = ui.max_rect().expand2(egui::vec2(14.0, 10.0));
    ui.painter().add(crate::metal_tex::top_bar(ui.ctx(), bar));
}

/// A small clock of spun metal around `center` with radius `radius`, its hands at five to twelve: the sign of the top bar that a
/// scheduled trade starts within a few minutes. A ring of hour marks, a dark hour and minute hand and a hand in `accent` that
/// stands at `seconds` (0-60), so the clock visibly runs.
#[must_use]
pub fn five_to_twelve_clock(ctx: &egui::Context, center: egui::Pos2, radius: f32, accent: Color32, seconds: f32) -> Vec<Shape> {
    let mut shapes = vec![crate::metal_tex::hub(ctx, center, radius)];
    let dark = Color32::from_rgb(40, 38, 36);
    // From the top, clockwise: 0 = twelve o'clock, 1 = once around.
    let at = |turn: f32, length: f32| {
        let angle = turn * std::f32::consts::TAU;
        center + egui::vec2(angle.sin(), -angle.cos()) * length
    };
    for hour in 0..12_u8 {
        let turn = f32::from(hour) / 12.0;
        let (inner, width): (f32, f32) = if hour % 3 == 0 { (0.62, 1.4) } else { (0.74, 0.9) };
        shapes.push(Shape::line_segment([at(turn, radius * inner), at(turn, radius * 0.86)], egui::Stroke::new(width, dark)));
    }
    // 11:55: the hour hand just before twelve, the minute hand on the 11.
    shapes.push(Shape::line_segment([center, at(11.0 / 12.0 + 55.0 / 60.0 / 12.0, radius * 0.48)], egui::Stroke::new(2.2_f32, dark)));
    shapes.push(Shape::line_segment([center, at(55.0 / 60.0, radius * 0.74)], egui::Stroke::new(1.5_f32, dark)));
    shapes.push(Shape::line_segment([center, at(seconds / 60.0, radius * 0.8)], egui::Stroke::new(1.0_f32, accent)));
    shapes.push(Shape::circle_filled(center, 1.8, accent));
    shapes
}
