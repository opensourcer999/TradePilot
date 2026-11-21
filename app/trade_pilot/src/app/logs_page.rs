//! The Logs page: the lines of the log with the debug lines marked, clearing and the export.

use super::{card_bg, debug_log_color, fs, text_primary, App, RichText};
use eframe::egui;

impl App {
    /// Empties the log on the screen and the log file; a file that cannot be emptied is said in the log.
    pub(super) fn clear_logs(&mut self) {
        self.log_lines.clear();
        self.ui_state.log_view = None;
        if let Some(path) = self.log_path.clone() {
            if let Err(error) = fs::write(&path, "") {
                self.log(&format!("Could not empty the log file {}: {error}", path.display()));
            }
        }
    }

    /// Asks before the logs are cleared (on the screen and in the log file): that cannot be undone.
    pub(super) fn clear_logs_dialog(&mut self, ctx: &egui::Context) {
        if !self.ui_state.confirm_clear_logs {
            return;
        }
        let (mut yes, mut no) = (false, false);
        egui::Window::new(self.tr("Clear the logs?", "Protokolle löschen?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(self.tr(
                    "All lines of the log are removed, on the screen and in the log file. This cannot be undone.",
                    "Alle Zeilen des Protokolls werden gelöscht, auf dem Bildschirm und in der Protokolldatei. Das lässt sich nicht rückgängig machen.",
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    yes = ui.add(crate::widgets::MetalButton::new(self.tr("Yes, clear the logs", "Ja, Protokolle löschen")).fill(crate::theme::RED)).clicked();
                    no = ui.add(crate::widgets::MetalButton::new(self.tr("No, keep them", "Nein, behalten"))).clicked();
                });
            });
        if yes {
            self.clear_logs();
            self.log("Logs cleared by the user");
        }
        if yes || no {
            self.ui_state.confirm_clear_logs = false;
        }
    }

    pub(super) fn logs_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(self.tr("Logs", "Protokolle")).color(self.heading_color()));
            ui.add_space(10.0);
            if ui.add(crate::widgets::MetalButton::new(self.tr("Refresh", "Aktualisieren"))).clicked() {
                self.log("Log view refreshed");
            }
            if ui.add(crate::widgets::MetalButton::new(self.tr("Clear Log", "Protokoll leeren"))).clicked() {
                self.ui_state.confirm_clear_logs = true;
            }
            if ui.add(crate::widgets::MetalButton::new(self.tr("Export to .TXT file", "Als .TXT-Datei exportieren"))).clicked() {
                if let Some(path) = self.log_path.clone() {
                    let export_path = path.with_file_name("app-export.txt");
                    let contents: String = self.log_lines.iter().rev().cloned().collect::<Vec<_>>().join("\n");
                    match fs::write(&export_path, contents) {
                        Ok(()) => self.log(&format!("Exported logs to {}", export_path.display())),
                        Err(error) => self.log(&format!("Could not export logs: {error}")),
                    }
                }
            }
        });
        ui.add_space(10.0);
        if let Some(error) = &self.log_error {
            ui.colored_label(crate::theme::negative_text(), error);
        }
        // The text of the log is put together only when a line came in, not on every frame.
        let log_lines = &self.log_lines;
        let log_view = self.ui_state.log_view.get_or_insert_with(|| log_lines.iter().rev().map(String::as_str).collect::<Vec<_>>().join("\n"));
        egui::Frame::none().fill(card_bg()).inner_margin(10.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // A plain `Label` has no keyboard shortcuts at all. A read-only `TextEdit` (over a `&str`, so nothing can be typed
                // into it) gets real Ctrl+A/Ctrl+C support for free; a right click copies its selection like everywhere else
                // (`widgets::copy_on_right_click`).
                let mut log_text: &str = log_view;
                // The debug lines get a soft yellow; the layouter colours each line by its content.
                let (normal, debug) = (text_primary(), debug_log_color());
                let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
                    let font = egui::TextStyle::Monospace.resolve(ui.style());
                    let mut job = egui::text::LayoutJob::default();
                    for line in text.split_inclusive('\n') {
                        let color = if line.contains(" DEBUG ") { debug } else { normal };
                        job.append(line, 0.0, egui::TextFormat::simple(font.clone(), color));
                    }
                    job.wrap.max_width = wrap_width;
                    ui.fonts(|fonts| fonts.layout_job(job))
                };
                egui::TextEdit::multiline(&mut log_text).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace).layouter(&mut layouter).show(ui);
            });
        });
    }
}
