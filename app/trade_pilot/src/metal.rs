//! Parts of the metal design put together from the pictures of `metal_tex`: the slide switch and the top bar.

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
