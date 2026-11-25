//! The Trade Templates page: the list, its toolbar, the import of .tpt files and the editor of one template.

use super::{
    build_export, card_bg, default_export_file_name, exports_dir, form_text, header_text, import_candidates, import_templates, muted_text, navy_bar,
    read_export, template_editor, upsert_template, visible_templates, white_card, write_export_to, App, Page, Part, RichText, StoredTrade,
    TemplateEditorOutcome, TemplateForm, TradeTemplate, RED, TEMPLATE_EDITOR_MIN_WIDTH,
};
use crate::widgets::PlasticRows;
use eframe::egui;

impl App {
    /// A blank template form with the defaults of the settings: the Leftover Long Behavior the person chose under Settings > Trading Features.
    fn blank_template_form(&self) -> TemplateForm {
        let mut form = TemplateForm::new_template();
        form.excess_long_behavior = if self.settings.long_strike_reuse == "Sell" { -1 } else { 0 };
        form
    }

    pub fn open_new_template(&mut self, trade_type: &str) {
        let mut form = self.blank_template_form();
        form.trade_type = trade_type.to_string();
        form.on_trade_type_changed();
        self.editing_template = Some(form);
        self.page = Page::Templates;
    }

    /// "Save Template": stores the template, or keeps the editor open with the failing field
    /// outlined (see `template_error`).
    fn save_template_form(&mut self, form: TemplateForm) {
        match form.save() {
            Ok(template) => {
                let name = template.name.clone();
                upsert_template(&mut self.templates, template);
                self.template.error.clear();
                self.save_templates();
                self.log(&format!("Trade template saved: {name}"));
            }
            Err(errors) => {
                for error in &errors {
                    self.log(&format!("Trade template not saved - {}: {}", error.title.trim(), error.message));
                }
                self.template.error = errors;
                self.template_ui.template_error_scroll = true;
                self.editing_template = Some(form);
            }
        }
    }

    /// Clicks "Save Template" on the open editor without saving anything to disk on failure
    /// (`--try-save`, for screenshots of the validation display).
    pub fn try_save_open_template(&mut self, name: &str) {
        if let Some(mut form) = self.editing_template.take() {
            form.name = name.to_string();
            match form.save() {
                Ok(_) => self.editing_template = Some(form),
                Err(_) => self.save_template_form(form),
            }
        }
    }

    /// The template with the id `id`, also a deleted one (a trade or a schedule may still refer to it).
    pub(super) fn template(&self, id: i32) -> Option<&TradeTemplate> {
        self.templates.iter().find(|t| t.id == id)
    }

    /// The template with the id `id` that is not deleted.
    pub(super) fn live_template(&self, id: i32) -> Option<&TradeTemplate> {
        self.template(id).filter(|t| !t.list_flags.is_deleted)
    }

    /// The template a trade was entered with, also a deleted one; `None` for a trade without its origin or whose template is gone.
    pub(super) fn template_of(&self, trade: &StoredTrade) -> Option<&TradeTemplate> {
        self.template(trade.origin.as_ref()?.template_id)
    }

    /// The templates that can be picked (not deleted), as (id, name) for a dropdown.
    pub(super) fn template_choices(&self) -> Vec<(i32, String)> {
        visible_templates(&self.templates).iter().map(|t| (t.id, t.name.clone())).collect()
    }

    /// The ticked templates that are not deleted, in the order of the list.
    fn checked_visible_templates(&self) -> Vec<TradeTemplate> {
        visible_templates(&self.templates).into_iter().filter(|t| self.checked_templates.contains(&t.id)).cloned().collect()
    }

    /// Imports several `.tpt` files in one go; a file that cannot be read is reported and skipped.
    fn import_template_files(&mut self, paths: &[std::path::PathBuf]) {
        let close = self.market_close_hour_24();
        let mut imported = 0;
        let mut dropped = 0;
        let mut skipped = 0;
        let mut renamed: Vec<String> = Vec::new();
        let mut files_ok = 0;
        let mut messages = Vec::new();
        for path in paths {
            match read_export(path) {
                Ok(export) => {
                    let report = import_templates(&mut self.templates, export, close);
                    imported += report.imported;
                    dropped += report.dropped_exit_conditions;
                    skipped += report.skipped_duplicates;
                    renamed.extend(report.renamed);
                    files_ok += 1;
                }
                Err(error) => messages.push(format!("Could not import {}: {error}", path.display())),
            }
        }
        if files_ok > 0 {
            self.save_templates();
            let mut note = String::new();
            if skipped > 0 {
                let _ = std::fmt::Write::write_fmt(&mut note, format_args!(" ({skipped} identical template(s) already existed and were skipped)"));
            }
            if !renamed.is_empty() {
                let _ = std::fmt::Write::write_fmt(&mut note, format_args!(" (name already taken, renamed: {})", renamed.join(", ")));
            }
            if dropped > 0 {
                let _ = std::fmt::Write::write_fmt(
                    &mut note,
                    format_args!(" ({dropped} early exit condition(s) not imported - choose them again in the templates)"),
                );
            }
            messages.insert(0, format!("Imported {imported} trade template(s) from {files_ok} file(s){note}"));
            self.template_ui.template_import_open = false;
            self.import_checked.clear();
        }
        for message in &messages {
            self.log(message);
        }
        self.template.status = Some(messages.join("; "));
    }

    /// The Trade Templates list: Add / Edit / Duplicate /
    /// Delete / Export / Import over the saved templates, sorted by name.
    /// Opens the editor for a saved template (Edit button, or a double-click on its name).
    pub(super) fn edit_template(&mut self, id: i32) {
        if let Some(t) = self.live_template(id) {
            self.editing_template = Some(TemplateForm::load(t, false));
            self.template.error.clear();
        }
    }

    pub(super) fn templates_page(&mut self, ui: &mut egui::Ui) {
        if let Some(mut form) = self.editing_template.take() {
            match self.template_editor_page(ui, &mut form) {
                TemplateEditorOutcome::Keep => self.editing_template = Some(form),
                TemplateEditorOutcome::Save => self.save_template_form(form),
                TemplateEditorOutcome::Cancel => self.template.error.clear(),
            }
            return;
        }

        ui.heading(RichText::new(self.tr("Trade Templates", "Trade-Vorlagen")).color(self.heading_color()));
        ui.add_space(8.0);
        self.templates_load_problem(ui);
        self.templates_toolbar(ui);
        if let Some(status) = &self.template.status {
            ui.add_space(4.0);
            ui.colored_label(muted_text(), status.as_str());
        }
        ui.add_space(8.0);

        self.template_import_card(ui);

        let rows: Vec<(i32, String, String, String, String)> = visible_templates(&self.templates)
            .iter()
            .map(|t| (t.id, t.name.clone(), t.trade_type.clone(), t.strategy.clone(), t.underlying_symbol.clone()))
            .collect();
        // The list scrolls sideways when it is wider than the page, so its frame always fits.
        egui::ScrollArea::horizontal().id_salt("templates_list").show(ui, |ui| {
            egui::Frame::none().fill(card_bg()).show(ui, |ui| {
                let mut plastic = PlasticRows::default();
                let grid = egui::Grid::new("templates_grid").min_col_width(24.0).spacing([24.0, 6.0]).show(ui, |ui| {
                    plastic.row(ui, true);
                    let all_checked = !rows.is_empty() && rows.iter().all(|r| self.checked_templates.contains(&r.0));
                    let mut all = all_checked;
                    if crate::widgets::metal_checkbox(ui, &mut all, "").on_hover_text(self.tr("Tick all", "Alle anhaken")).changed() {
                        if all {
                            self.checked_templates.extend(rows.iter().map(|r| r.0));
                        } else {
                            self.checked_templates.clear();
                        }
                    }
                    for c in ["Name", "Trade Type", "Strategy", "Symbol"] {
                        ui.label(RichText::new(c).strong().color(header_text()));
                    }
                    ui.end_row();
                    let mut open_editor = None;
                    for (id, name, trade_type, strategy, symbol) in &rows {
                        plastic.row(ui, false);
                        let is_selected = self.checked_templates.contains(id);
                        let mut checked = self.checked_templates.contains(id);
                        if crate::widgets::metal_checkbox(ui, &mut checked, "").changed() {
                            if checked {
                                self.checked_templates.insert(*id);
                            } else {
                                self.checked_templates.remove(id);
                            }
                        }
                        // The box ticks the template for the buttons above; a click on the row ticks or unticks it too, a
                        // double-click opens it for editing.
                        let mut cells = vec![ui.selectable_label(is_selected, name.as_str())];
                        for text in [trade_type, strategy, symbol] {
                            let text = if is_selected { RichText::new(text.as_str()).strong() } else { RichText::new(text.as_str()) };
                            cells.push(ui.add(egui::Label::new(text).sense(egui::Sense::click())));
                        }
                        if cells.iter().any(egui::Response::double_clicked) {
                            self.checked_templates.insert(*id);
                            open_editor = Some(*id);
                        } else if cells.iter().any(egui::Response::clicked) && !self.checked_templates.remove(id) {
                            self.checked_templates.insert(*id);
                        }
                        ui.end_row();
                    }
                    if let Some(id) = open_editor {
                        self.edit_template(id);
                    }
                });
                plastic.paint(ui, grid.response.rect, 6.0);
                if rows.is_empty() {
                    ui.separator();
                    ui.add_space(26.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(self.tr(
                                "No saved trade templates yet. Use Add to create one, or Import a .tpt export.",
                                "Noch keine gespeicherten Trade-Vorlagen. Mit Hinzufügen eine erstellen oder einen .tpt-Export importieren.",
                            ))
                            .color(muted_text())
                            .italics(),
                        );
                    });
                    ui.add_space(26.0);
                }
            })
        });
    }

    /// The bar above the list of templates: add, edit, duplicate, delete, import and export.
    fn templates_toolbar(&mut self, ui: &mut egui::Ui) {
        let checked = self.checked_visible_templates();
        // Edit and Copy work on exactly one ticked template, Delete and Export on all ticked ones.
        let single = if checked.len() == 1 { checked.first().cloned() } else { None };
        let count = |label: &str| if checked.len() > 1 { format!("{label} ({})", checked.len()) } else { label.to_string() };
        egui::Frame::none().fill(card_bg()).inner_margin(egui::vec2(10.0, 8.0)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add(crate::widgets::MetalButton::new(self.tr("+ Add", "+ Hinzufügen"))).clicked() {
                    self.editing_template = Some(self.blank_template_form());
                    self.template.error.clear();
                }
                if ui.add_enabled(single.is_some(), crate::widgets::MetalButton::new(self.tr("Edit", "Bearbeiten"))).clicked() {
                    if let Some(t) = &single {
                        self.edit_template(t.id);
                    }
                }
                if ui.add_enabled(single.is_some(), crate::widgets::MetalButton::new(self.tr("Copy", "Kopieren"))).clicked() {
                    if let Some(t) = &single {
                        self.editing_template = Some(TemplateForm::load(t, true));
                        self.template.error.clear();
                    }
                }
                if ui.add_enabled(!checked.is_empty(), crate::widgets::MetalButton::new(count(self.tr("Delete", "Löschen")))).clicked() {
                    // Soft delete: re-entry links from other templates keep
                    // pointing at a template that still exists.
                    for t in self.templates.iter_mut().filter(|t| self.checked_templates.contains(&t.id)) {
                        t.list_flags.is_deleted = true;
                    }
                    self.checked_templates.clear();
                    self.save_templates();
                }
                let export_ids: Vec<i32> = checked.iter().map(|t| t.id).collect();
                if ui.add_enabled(!export_ids.is_empty(), crate::widgets::MetalButton::new(count(self.tr("Export", "Exportieren")))).clicked() {
                    // The user picks where the file goes; closing the dialog cancels the export.
                    let mut dialog = rfd::FileDialog::new()
                        .set_title(self.tr("Export trade templates", "Trade-Vorlagen exportieren"))
                        .set_file_name(default_export_file_name())
                        .add_filter("TradePilot templates (.tpt)", &["tpt"]);
                    if let Ok(dir) = exports_dir() {
                        if dir.is_dir() {
                            dialog = dialog.set_directory(dir);
                        }
                    }
                    if let Some(path) = dialog.save_file() {
                        let export = build_export(&self.templates, &export_ids, self.market_close_hour_24());
                        let message = match write_export_to(&export, &path) {
                            Ok(path) => format!("Exported to file: {}", path.display()),
                            Err(error) => format!("Error Creating Export: {error}"),
                        };
                        self.log(&message);
                        self.template.status = Some(message);
                    }
                }
                if ui.add(crate::widgets::MetalButton::new(self.tr("Import", "Importieren"))).clicked() {
                    self.template_ui.template_import_open = !self.template_ui.template_import_open;
                }
            });
        });
    }

    /// The red line that says why the templates file could not be loaded.
    fn templates_load_problem(&mut self, ui: &mut egui::Ui) {
        if let Some(problem) = &self.templates_load_problem {
            ui.colored_label(
                RED,
                format!("{}: {problem}", self.tr("The templates file could not be loaded", "Die Vorlagen-Datei konnte nicht geladen werden")),
            );
            ui.add_space(6.0);
        }
    }

    /// The card to import templates from a `.tpt` export file.
    fn template_import_card(&mut self, ui: &mut egui::Ui) {
        if self.template_ui.template_import_open {
            let mut import_paths: Vec<std::path::PathBuf> = Vec::new();
            white_card(ui, |ui| {
                navy_bar(ui, self.tr("Import Templates (.tpt)", "Vorlagen importieren (.tpt)"));
                ui.add_space(8.0);
                let folder = exports_dir().map(|d| d.display().to_string()).unwrap_or_default();
                ui.colored_label(muted_text(), format!("Template exports found in {folder} and in Downloads:"));
                let candidates = import_candidates();
                if candidates.is_empty() {
                    ui.colored_label(muted_text(), "No .tpt files found.");
                }
                self.import_checked.retain(|p| candidates.contains(p));
                for path in &candidates {
                    ui.horizontal(|ui| {
                        let mut checked = self.import_checked.contains(path);
                        if crate::widgets::metal_checkbox(ui, &mut checked, path.display().to_string()).changed() {
                            if checked {
                                self.import_checked.insert(path.clone());
                            } else {
                                self.import_checked.remove(path);
                            }
                        }
                    });
                }
                if !candidates.is_empty() {
                    ui.horizontal(|ui| {
                        if ui.add(crate::widgets::MetalButton::new(self.tr("Select all", "Alle auswählen"))).clicked() {
                            self.import_checked.extend(candidates.iter().cloned());
                        }
                        if ui.add(crate::widgets::MetalButton::new(self.tr("Select none", "Keine auswählen"))).clicked() {
                            self.import_checked.clear();
                        }
                        let label = format!("{} ({})", self.tr("Import selected", "Auswahl importieren"), self.import_checked.len());
                        if ui.add_enabled(!self.import_checked.is_empty(), crate::widgets::MetalButton::new(label)).clicked() {
                            import_paths = candidates.iter().filter(|p| self.import_checked.contains(*p)).cloned().collect();
                        }
                    });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(self.tr("Or file path:", "Oder Dateipfad:"));
                    form_text(ui, &mut self.template.import_path, 420.0);
                    let can_import = !self.template.import_path.trim().is_empty();
                    if ui.add_enabled(can_import, crate::widgets::MetalButton::new(self.tr("Import", "Importieren"))).clicked() {
                        import_paths = vec![std::path::PathBuf::from(self.template.import_path.trim())];
                    }
                });
            });
            if !import_paths.is_empty() {
                self.import_template_files(&import_paths);
            }
            ui.add_space(8.0);
        }
    }

    /// The distance between the strikes of `symbol` as TWS reported them in this session: from the options of the chains the app
    /// loaded (each contract belongs to its symbol). `None` when TWS reported no strikes of the symbol.
    pub(super) fn reported_strike_step(&self, symbol: &str) -> Option<super::Decimal> {
        let strikes: Vec<super::Decimal> = self.chains.contracts.iter().filter(|c| c.symbol == symbol).filter_map(|c| crate::pricing::dec(c.strike)).collect();
        crate::strikes::strike_step_of(&strikes)
    }

    /// The Add/Edit Trade Template form. Labels, sections and
    /// which parts show for which selections are defined in `template_form`;
    /// the labels are English-only. The cards are in `template_editor`.
    fn template_editor_page(&mut self, ui: &mut egui::Ui, form: &mut TemplateForm) -> TemplateEditorOutcome {
        let mut outcome = TemplateEditorOutcome::Keep;
        // Widths are checked against the strikes TWS reported for this symbol in the current session.
        form.strike_step = self.reported_strike_step(&form.symbol);
        let layout = form.layout();
        // The last save's errors outline every faulty field in red (and scroll to the first one).
        let errors = self.template.error.clone();
        let scroll_to_error = std::cell::Cell::new(self.template_ui.template_error_scroll);
        let other_templates = self.template_choices();
        let conditions: Vec<(u32, String)> = self.conditions.list.iter().map(|c| (c.id, c.name.clone())).collect();
        let ctx = template_editor::EditorContext {
            errors: &errors,
            scroll_to_error: &scroll_to_error,
            layout: &layout,
            other_templates: &other_templates,
            conditions: &conditions,
        };
        ui.heading(RichText::new(self.tr("Trade Templates", "Trade-Vorlagen")).color(self.heading_color()));
        ui.add_space(8.0);
        // The widest editor rows need ~900px; below that the form scrolls sideways instead of
        // cutting fields off at the right edge. Above it, cards still span the full width.
        let content_width = ui.available_width().max(TEMPLATE_EDITOR_MIN_WIDTH);
        egui::ScrollArea::both().id_salt("template_editor_scroll").show(ui, |ui| {
            ui.set_width(content_width);
            template_editor::details_card(ui, form, &ctx);
            ui.add_space(10.0);

            if layout.shows(Part::CalendarSetup) {
                template_editor::calendar_card(ui, form, &ctx);
                ui.add_space(10.0);
            }

            if layout.shows(Part::CustomSetup) {
                template_editor::custom_card(ui, form);
                ui.add_space(10.0);
            }

            if layout.shows(Part::ShortStrikeHeader) {
                template_editor::short_strikes_card(ui, form, &ctx);
                ui.add_space(10.0);
            }

            if layout.shows(Part::LongStrikeSelection) {
                template_editor::long_strikes_card(ui, form, &ctx);
                ui.add_space(10.0);
            }

            template_editor::entry_cards(ui, form, &ctx);
            ui.add_space(10.0);

            template_editor::stop_card(ui, form, &ctx);
            ui.add_space(10.0);

            template_editor::adjustments_card(ui, form, &ctx);
            ui.add_space(10.0);

            template_editor::profit_target_card(ui, form, &ctx);
            ui.add_space(10.0);

            template_editor::timed_exit_card(ui, form, &ctx);
            ui.add_space(10.0);

            template_editor::reentry_card(ui, form, &ctx);
            ui.add_space(10.0);

            template_editor::fill_and_strategy_cards(ui, form, &ctx);
            ui.add_space(14.0);

            outcome = template_editor::footer(ui, &ctx);
            ui.add_space(14.0);
        });
        self.template_ui.template_error_scroll = scroll_to_error.get();
        outcome
    }
}
