use eframe::egui::Color32;

// The palette of the metal design. Most of it is dark steel with light text; on light metal (the top bar, the light
// cards) the texts are dark instead, which `set_on_light_metal` switches while such a surface is drawn.
thread_local! {
    static ON_LIGHT_METAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Whether what is drawn now sits on light metal (the top bar, a light card): its texts are then dark.
fn on_light_metal() -> bool {
    ON_LIGHT_METAL.with(std::cell::Cell::get)
}
/// Marks what is drawn from now on as sitting on light metal (the top bar, a light card), or not any more; returns
/// what was set before, so a nested light card can put it back.
pub fn set_on_light_metal(on: bool) -> bool {
    ON_LIGHT_METAL.with(|cell| cell.replace(on))
}
pub fn content_bg() -> Color32 {
    Color32::from_rgb(0x2d, 0x2b, 0x29)
}
pub fn topbar_bg() -> Color32 {
    Color32::from_rgb(0x2f, 0x2d, 0x2b)
}
pub fn card_bg() -> Color32 {
    Color32::from_rgb(0x2d, 0x2b, 0x29)
}
pub fn card_border() -> Color32 {
    Color32::from_rgb(0x0c, 0x0b, 0x0b)
}
pub fn nav_inactive() -> Color32 {
    Color32::from_rgb(0xe0, 0xdc, 0xd6)
}
pub fn chrome_icon() -> Color32 {
    Color32::from_rgb(0xe0, 0xdc, 0xd6)
}
pub fn text_primary() -> Color32 {
    if on_light_metal() {
        Color32::from_rgb(0x1c, 0x1a, 0x17)
    } else {
        Color32::from_rgb(0xf2, 0xef, 0xea)
    }
}
pub fn muted_text() -> Color32 {
    if on_light_metal() {
        Color32::from_rgb(0x36, 0x32, 0x2d)
    } else {
        Color32::from_rgb(0xe8, 0xe4, 0xde)
    }
}
pub fn warning_banner_bg() -> Color32 {
    Color32::from_rgb(0x3a, 0x1f, 0x1f)
}
pub fn warning_banner_text() -> Color32 {
    Color32::from_rgb(0xf3, 0xc9, 0xc9)
}
/// A subtle, friendly (not alarming) red used only for page headings while connected to a
/// paper-trading account, as a passive reminder of the account mode - not a loud warning color.
pub fn paper_heading_color() -> Color32 {
    Color32::from_rgb(0xe0, 0x9a, 0x9a)
}
pub fn header_text() -> Color32 {
    if on_light_metal() {
        Color32::from_rgb(0x2a, 0x26, 0x22)
    } else {
        Color32::from_rgb(0xd8, 0xd4, 0xce)
    }
}
/// The soft yellow of the debug lines in the log: visible, but quieter than the normal text.
pub fn debug_log_color() -> Color32 {
    Color32::from_rgb(0xb5, 0xa8, 0x6a)
}

/// Amber, reserved exclusively for controls that render as if interactive but currently have no
/// effect when used (no backend behind them yet) - e.g. "Add" on a list that can't be saved
/// to yet. Lets a glance tell "this really works" apart from "this is still a UI look-alike".
pub fn placeholder_color() -> Color32 {
    Color32::from_rgb(0xe0, 0xa8, 0x3a)
}
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0x3f, 0x7a, 0xd4);
pub const RED: Color32 = Color32::from_rgb(0xc7, 0x1f, 0x1f);
pub const GREEN_TXT: Color32 = Color32::from_rgb(0x1f, 0x8a, 0x44);
/// Warnings that need attention but are not an error of this app (e.g. TWS sends no prices).
pub const ORANGE_WARN: Color32 = Color32::from_rgb(0xd9, 0x6c, 0x06);

/// The widget look of the metal design: dark steel buttons with a dark outline, a muted white text and an amber
/// selection.
pub fn metal_visuals() -> eframe::egui::Visuals {
    use eframe::egui::{Stroke, Visuals};
    let mut v = Visuals::dark();
    let steel = Color32::from_rgb(0x3a, 0x3d, 0x41);
    let outline = Stroke::new(1.0_f32, Color32::from_rgb(0x0c, 0x0c, 0x0d));
    v.panel_fill = Color32::from_rgb(0x2d, 0x2b, 0x29);
    // Dialogs and drop-down lists: dark warm metal with a light metal rim and a deep shadow, like a raised plate.
    v.window_fill = Color32::from_rgb(0x2b, 0x28, 0x26);
    v.window_stroke = Stroke::new(1.5_f32, Color32::from_rgb(0x8a, 0x84, 0x7c));
    v.window_rounding = eframe::egui::Rounding::same(8.0);
    v.window_shadow = eframe::egui::epaint::Shadow { offset: eframe::egui::vec2(0.0, 8.0), blur: 24.0, spread: 2.0, color: Color32::from_black_alpha(170) };
    v.menu_rounding = eframe::egui::Rounding::same(6.0);
    v.extreme_bg_color = Color32::from_rgb(0x12, 0x13, 0x15);
    v.faint_bg_color = Color32::from_rgb(0x31, 0x33, 0x36);
    v.widgets.noninteractive.bg_fill = Color32::from_rgb(0x2b, 0x2d, 0x30);
    v.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(0x2b, 0x2d, 0x30);
    v.widgets.noninteractive.bg_stroke = outline;
    v.widgets.inactive.bg_fill = steel;
    v.widgets.inactive.weak_bg_fill = steel;
    v.widgets.inactive.bg_stroke = outline;
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x4a, 0x4d, 0x52);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x4a, 0x4d, 0x52);
    v.widgets.active.bg_fill = Color32::from_rgb(0x55, 0x58, 0x5d);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x55, 0x58, 0x5d);
    v.selection.bg_fill = Color32::from_rgb(0x7a, 0x55, 0x20);
    v.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(0xff, 0xc0, 0x6a));
    v
}

/// The green of good news in a text: brighter on dark metal, so it stays readable there.
pub fn positive_text() -> Color32 {
    if on_light_metal() {
        GREEN_TXT
    } else {
        Color32::from_rgb(0x5a, 0xd3, 0x8a)
    }
}

/// The red of bad news in a text: brighter on dark metal, so it stays readable there.
pub fn negative_text() -> Color32 {
    if on_light_metal() {
        RED
    } else {
        Color32::from_rgb(0xff, 0x7a, 0x6e)
    }
}
