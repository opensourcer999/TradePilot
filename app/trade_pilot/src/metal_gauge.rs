//! The dial instrument of the metal design: a silver dial (see `metal_tex::dial`) with short glowing amber marks,
//! fine dark marks in its rim track, the numbers around it, a glowing needle that runs to a point on a small chrome
//! hub, and the value on the edge of its dark lower part with the name below it.

use crate::metal_tex::{CHORD, TRACK_INNER, TRACK_OUTER};
use crate::widgets::{format_tick, gauge_angle, gauge_point, GaugeSpec};
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke};

/// The glow around lit parts: an orange that gets fainter the wider it spreads.
fn glow(opacity: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(0xff, 0x8a, 0x1e, opacity)
}

/// A short scale mark that glows: a soft glow around a thin warm core.
fn glowing_mark(painter: &egui::Painter, from: Pos2, to: Pos2, scale: f32) {
    painter.add(crate::metal_tex::glow_along(painter.ctx(), from, to, 6.0 * scale, glow(70)));
    painter.line_segment([from, to], Stroke::new(1.3 * scale, Color32::from_rgb(0xff, 0xc4, 0x78)));
    painter.line_segment([from, to], Stroke::new(0.7 * scale, Color32::from_rgb(0xff, 0xf0, 0xd8)));
}

/// The needle: wide at the hub, running to a point at `tip`, lit from inside: a wide soft glow and a narrower brighter
/// one around it, an orange body and a nearly white core.
fn glowing_needle(painter: &egui::Painter, center: Pos2, tip: Pos2, angle: f32, scale: f32) {
    let across = egui::vec2(-angle.sin(), angle.cos());
    let back = egui::vec2(angle.cos(), angle.sin());
    // The needle starts under the middle of the cap, so nothing of it shows behind the cap.
    let base = center;
    // The point is blunted by a fraction of a pixel: the edge smoothing of a needle-sharp corner would otherwise
    // draw a dotted line far past the tip.
    let blunt = across * (0.35 * scale);
    let wedge = |half: f32, tip_out: f32, color: Color32| {
        let point = tip + back * tip_out;
        Shape::convex_polygon(vec![base + across * half, point + blunt, point - blunt, base - across * half], color, Stroke::NONE)
    };
    // The shadow the needle throws on the face below it (light from the upper left).
    let offset = egui::vec2(2.0, 3.0) * scale;
    painter.add(Shape::convex_polygon(
        vec![base + across * (2.4 * scale) + offset, tip + blunt + offset, tip - blunt + offset, base - across * (2.4 * scale) + offset],
        Color32::from_black_alpha(70),
        Stroke::NONE,
    ));
    painter.add(crate::metal_tex::glow_along(painter.ctx(), center, tip, 12.0 * scale, glow(40)));
    painter.add(crate::metal_tex::glow_along(painter.ctx(), center, tip, 5.0 * scale, glow(80)));
    painter.add(wedge(2.4 * scale, 0.0, Color32::from_rgb(0xff, 0xb8, 0x60)));
    painter.add(wedge(1.0 * scale, -2.0 * scale, Color32::from_rgb(0xff, 0xf2, 0xdc)));
}

/// `text` printed on the metal around `at`: spaced letters in a paint that lets the brushed metal show through, lighter
/// at the top of the letters and darker at the bottom like the sheen of the surface, with a hair of shadow above and of
/// light below where the paint sits in it.
fn printed_text(painter: &egui::Painter, at: Pos2, text: &str, size: f32) {
    let format = egui::TextFormat { font_id: egui::FontId::proportional(size), extra_letter_spacing: size * 0.08, color: Color32::WHITE, ..Default::default() };
    let galley = painter.layout_job(egui::text::LayoutJob::single_section(text.to_string(), format));
    let pos = at - galley.size() / 2.0;
    let rect = Rect::from_min_size(pos, galley.size());
    let paint = |offset: egui::Vec2, clip: Rect, color: Color32| {
        painter.with_clip_rect(clip.intersect(painter.clip_rect())).galley_with_override_text_color(pos + offset, galley.clone(), color);
    };
    let all = rect.expand(2.0);
    paint(egui::vec2(0.0, -0.7), all, Color32::from_black_alpha(140));
    paint(egui::vec2(0.0, 0.8), all, Color32::from_white_alpha(28));
    let middle = rect.center().y + size * 0.05;
    paint(egui::Vec2::ZERO, Rect::from_min_max(all.min, egui::pos2(all.max.x, middle)), Color32::from_rgba_unmultiplied(0xf2, 0xed, 0xe4, 235));
    paint(egui::Vec2::ZERO, Rect::from_min_max(egui::pos2(all.min.x, middle), all.max), Color32::from_rgba_unmultiplied(0xcc, 0xc5, 0xba, 235));
}

/// Paints the instrument into `rect`; `value` is where the needle points.
pub fn paint(painter: &egui::Painter, rect: Rect, value: f64, spec: &GaugeSpec<'_>) {
    let center = rect.center();
    // Room around the dial for the numbers.
    let outer = rect.height().min(rect.width()) * 0.5 - 17.0;
    let scale = (outer / 80.0).clamp(0.7, 1.6);
    painter.add(crate::metal_tex::dial(painter.ctx(), center, outer));
    let (min, max) = (spec.min, spec.max);

    // Fine dark marks across the rim track, like a ruler.
    if let (Some(first), Some(last)) = (spec.ticks.first(), spec.ticks.last()) {
        let fine = u32::try_from(spec.ticks.len().saturating_sub(1) * 10).unwrap_or(0);
        for step in 0..=fine {
            let at = first + (last - first) * f64::from(step) / f64::from(fine.max(1));
            let angle = gauge_angle(min, max, at);
            painter.line_segment(
                [gauge_point(center, outer * (TRACK_INNER + 0.015), angle), gauge_point(center, outer * (TRACK_OUTER - 0.01), angle)],
                Stroke::new(0.9_f32, Color32::from_rgb(0x1c, 0x1a, 0x18)),
            );
        }
    }
    // Short glowing amber marks just inside the track, the numbers outside the bezel.
    for tick in spec.ticks {
        let angle = gauge_angle(min, max, *tick);
        glowing_mark(painter, gauge_point(center, outer * 0.65, angle), gauge_point(center, outer * 0.79, angle), scale);
        let label = gauge_point(center, outer + 12.0 * scale, angle);
        let font = egui::FontId::proportional(10.5 * scale);
        painter.text(label + egui::vec2(0.0, 1.0), egui::Align2::CENTER_CENTER, format_tick(*tick), font.clone(), Color32::BLACK);
        painter.text(label, egui::Align2::CENTER_CENTER, format_tick(*tick), font, Color32::from_rgb(0xdc, 0xd8, 0xd2));
    }

    // The needle into the track, and a small chrome hub on top of it.
    let angle = gauge_angle(min, max, value);
    glowing_needle(painter, center, gauge_point(center, outer * 0.86, angle), angle, scale);
    painter.add(crate::metal_tex::hub(painter.ctx(), center, outer * 0.11));

    let text = |pos: Pos2, text: &str, size: f32| {
        painter.text(pos + egui::vec2(0.0, 1.5), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(size), Color32::from_black_alpha(220));
        painter.text(pos, egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(size), Color32::from_rgb(0xfc, 0xfa, 0xf6));
    };
    // The value glows softly, like the lit marks.
    let value_at = egui::pos2(center.x, center.y + outer * (CHORD - 0.12));
    let reach = egui::vec2(outer * 0.22, 0.0);
    painter.add(crate::metal_tex::glow_along(
        painter.ctx(),
        value_at - reach,
        value_at + reach,
        outer * 0.3,
        Color32::from_rgba_unmultiplied(0xff, 0xe8, 0xc8, 16),
    ));
    text(value_at, spec.value_text, outer * 0.26);
    printed_text(painter, egui::pos2(center.x, center.y + outer * (CHORD + 0.13)), spec.title, outer * 0.13);
}
