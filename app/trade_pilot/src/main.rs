//! `TradePilot`: a desktop app for options trades on Interactive Brokers TWS - templates, entry,
//! stop and profit-target orders, live figures and reports. Orders go to paper trading accounts only.
// `windows_subsystem` only means anything to the Windows PE linker; gating it to that target
// keeps a cross-platform build (Linux/macOS) from tripping over a Windows-only attribute.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing))]

use strategy::{
    bars, fees, legacy_names, market_calendar, position_value, pricing, reconcile, reports, strikes, template_form, trade_entry, trade_exit, trade_live,
    trade_rebuild, trade_summary,
};
mod app;
mod calendar;
mod clock;
mod column_order;
mod condition;
mod condition_editor;
mod condition_form;
mod database;
mod http_error;
mod i18n;
mod list_file;
mod metal;
mod metal_gauge;
mod metal_tex;
mod news;
mod notifications;
mod number_lists;
mod option_chain;
mod schedule;
mod schedule_editor;
mod schedule_export;
mod schedule_form;
mod schedule_run;
mod settings_help;
mod storage;
mod templates;
#[cfg(test)]
mod test_support;
mod theme;
mod trade_mail;
mod trigger;
mod trigger_editor;
mod trigger_form;
mod watch;
mod webhook;
mod widgets;

use app::App;
use eframe::egui;
use i18n::Page;
use std::sync::Arc;
use theme::text_primary;

fn save_png(img: &Arc<egui::ColorImage>, path: &str) {
    let [w, h] = img.size;
    let mut buf = Vec::with_capacity(w * h * 4);
    for p in &img.pixels {
        buf.extend_from_slice(&p.to_array());
    }
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = image::save_buffer(path, &buf, u32::try_from(w).unwrap_or(u32::MAX), u32::try_from(h).unwrap_or(u32::MAX), image::ColorType::Rgba8);
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let shot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1).cloned());
    let start_page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).and_then(|name| match name.as_str() {
        "dashboard" => Some(Page::Dashboard),
        "live_data" => Some(Page::LiveData),
        "quotes" => Some(Page::Quotes),
        "instant_order" => Some(Page::InstantOrder),
        "templates" => Some(Page::Templates),
        "schedules" => Some(Page::Schedules),
        "webhooks" => Some(Page::Webhooks),
        "conditions" => Some(Page::Conditions),
        "reports" => Some(Page::Reports),
        "news" => Some(Page::News),
        "logs" => Some(Page::Logs),
        "settings" => Some(Page::Settings),
        _ => None,
    });
    // With --screenshot: draws the UI as if TWS were connected (no connection is made).
    let preview_connected = args.iter().any(|a| a == "--preview-connected");
    // With --screenshot: made-up trades and prices on a dummy paper account instead of the person's own data (pictures for the README).
    let demo = args.iter().any(|a| a == "--demo");
    // Screenshots of the Reports tabs: --reports-tab overview|equity|daily|log.
    let reports_tab = args.iter().position(|a| a == "--reports-tab").and_then(|i| args.get(i + 1)).cloned();
    // With --screenshot --preview-connected: an example Instant Order with its confirmation open.
    let preview_confirm = args.iter().position(|a| a == "--preview-confirm").and_then(|i| args.get(i + 1)).cloned();
    // Opens the Add Trade Template editor for the given trade type (for screenshots of a layout).
    let new_template = args.iter().position(|a| a == "--new-template").and_then(|i| args.get(i + 1)).cloned();
    // With --new-template: clicks Save with this template name (shows the validation display).
    let try_save = args.iter().position(|a| a == "--try-save").and_then(|i| args.get(i + 1)).cloned();
    let window_width = args.iter().position(|a| a == "--width").and_then(|i| args.get(i + 1)).and_then(|value| value.parse::<f32>().ok()).unwrap_or(1430.0);
    let window_height = args.iter().position(|a| a == "--height").and_then(|i| args.get(i + 1)).and_then(|value| value.parse::<f32>().ok()).unwrap_or(800.0);
    // The window's title bar names the version after the app.
    let title = format!("TradePilot {}", app::APP_VERSION);
    // The dashboard's credit-trades card + two gauges are fixed-pixel widgets (250 + 24 + 200 +
    // 12 + 200 = 686px) inside a 320px fixed sidebar and a 20px CentralPanel margin on each side;
    // below ~1100px window width that row has nowhere to shrink and overflows the window, as
    // confirmed by testing at 1000px. A minimum window size prevents resizing into that broken
    // state instead of silently clipping content.
    let icon = image::load_from_memory(include_bytes!("../assets/icon.png")).map_err(|error| eframe::Error::AppCreation(Box::new(error)))?.into_rgba8();
    let (width, height) = icon.dimensions();
    let mut viewport = egui::ViewportBuilder::default()
        .with_icon(egui::IconData { rgba: icon.into_raw(), width, height })
        .with_inner_size([window_width, window_height])
        .with_min_inner_size([1100.0, 700.0])
        .with_title(title.clone());
    if shot.is_some() {
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        &title,
        options,
        Box::new(move |creation_context| {
            creation_context.egui_ctx.set_fonts(app_fonts());
            // One look, the metal design, whatever the system prefers.
            creation_context.egui_ctx.set_visuals_of(egui::Theme::Dark, theme::metal_visuals());
            creation_context.egui_ctx.set_visuals_of(egui::Theme::Light, theme::metal_visuals());
            creation_context.egui_ctx.all_styles_mut(|style| {
                use egui::{FontFamily, FontId, TextStyle};
                style.text_styles.insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
                style.text_styles.insert(TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
                style.text_styles.insert(TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional));
                style.text_styles.insert(TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace));
                style.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
                style.visuals.override_text_color = Some(text_primary());
            });
            let mut app = App::new(shot, start_page);
            if let Some(tab) = &reports_tab {
                app.preview_reports_tab(tab);
            }
            if preview_connected {
                app.preview_connected();
            }
            if demo {
                app.preview_demo();
            }
            if let Some(action) = &preview_confirm {
                app.preview_instant_confirm(action);
            }
            if let Some(trade_type) = &new_template {
                app.open_new_template(trade_type);
                if let Some(name) = &try_save {
                    app.try_save_open_template(name);
                }
            }
            Ok(Box::new(app))
        }),
    )
}

/// egui's default fonts, with Segoe UI (Windows' interface font) in front when it is installed:
/// the text and the numbers of the whole app use it. Elsewhere, or if the file is missing, the
/// default fonts stay.
fn app_fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    let path = std::path::Path::new(&windir).join("Fonts").join("segoeui.ttf");
    if let Ok(bytes) = std::fs::read(path) {
        fonts.font_data.insert("segoe-ui".to_string(), egui::FontData::from_owned(bytes));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().insert(0, "segoe-ui".to_string());
        }
    }
    fonts
}
