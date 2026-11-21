//! The Settings page: a navigation of its sections on the left, the settings of the chosen section as rows on the right and a bar
//! with the state of the changes and the Save button at the bottom. What the settings mean is kept in `PersistedSettings`.

use super::{
    data_table_rows, framed_card, list_backups, muted_text, nav_item, next_selection, placeholder_color, read_backup, section_header, setting_row, storage,
    symbol_combo, text_primary, time_picker, toggle_switch, ui_mode_name, warning_banner_bg, warning_banner_text, white_card, write_backup, App, Color32,
    Duration, Lang, PathBuf, RichText, SettingsTab, SymbolConfig, TableCell, TableColumn, ACCENT_BLUE, MAINTAIN_CONNECTION_OPTIONS, ORANGE_WARN, STATUS_ERROR,
    STATUS_OK,
};
use crate::widgets::setting_row_noted;
use eframe::egui;

impl App {
    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui) {
        // The bar with the save button sits at the bottom, the navigation at the left; the chosen section fills the rest.
        // The status bar and the navigation each sit in a metal frame, like the rest of the page.
        egui::TopBottomPanel::bottom("settings_status_bar")
            .frame(egui::Frame::none().inner_margin(egui::Margin { top: 10.0, ..egui::Margin::ZERO }))
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                framed_card(ui, |ui| self.settings_status_bar(ui));
            });
        egui::SidePanel::left("settings_navigation").exact_width(230.0).resizable(false).frame(egui::Frame::none()).show_separator_line(false).show_inside(
            ui,
            |ui| {
                framed_card(ui, |ui| {
                    ui.set_min_height((ui.available_height() - 8.0).max(0.0));
                    ui.heading(RichText::new(self.tr("Settings", "Einstellungen")).color(self.heading_color()));
                    ui.add_space(8.0);
                    self.settings_nav(ui);
                });
            },
        );
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin { left: 12.0, ..egui::Margin::ZERO })).show_inside(ui, |ui| {
            self.settings_mode_note(ui);
            ui.add_space(8.0);
            // The sections scroll inside one metal frame that stays put, so no card is ever cut off at its edge; the
            // sections inside it are groups of their own (frames are not nested).
            framed_card(ui, |ui| {
                ui.set_min_height((ui.available_height() - 8.0).max(0.0));
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height((ui.available_height() - 8.0).max(0.0)).show(ui, |ui| {
                    ui.set_max_width(820.0_f32.min(ui.available_width() - 14.0));
                    match self.settings_tab {
                        SettingsTab::General => self.settings_general_tab(ui),
                        SettingsTab::Notifications => self.notifications_tab(ui),
                        SettingsTab::TradingFeatures => self.trading_features_tab(ui),
                        SettingsTab::MarketData => self.market_data_tab(ui),
                        SettingsTab::SafetyFeatures => self.safety_features_tab(ui),
                        SettingsTab::DatabaseBackup => self.database_backup_tab(ui),
                    }
                    ui.add_space(8.0);
                });
            });
        });
    }

    /// The line under the title: a coloured chip with the mode (Live or Paper) the settings belong to, and what that means.
    fn settings_mode_note(&mut self, ui: &mut egui::Ui) {
        let live = self.link.trading_live;
        let (chip, color) = if live { (self.tr("LIVE", "LIVE"), ORANGE_WARN) } else { (self.tr("PAPER", "PAPER"), ACCENT_BLUE) };
        let text = if live {
            self.tr("These settings apply to your Live account.", "Diese Einstellungen gelten für dein Live-Konto.")
        } else {
            self.tr("These settings apply to your Test (paper money) account.", "Diese Einstellungen gelten für dein Test-Konto (Papiergeld).")
        };
        ui.horizontal(|ui| {
            egui::Frame::none().fill(color.gamma_multiply(0.22)).rounding(10.0).inner_margin(egui::vec2(8.0, 2.0)).show(ui, |ui| {
                ui.label(RichText::new(chip).small().strong().color(color));
            });
            ui.label(RichText::new(text).color(muted_text()));
        });
    }

    /// The navigation of the Settings page: one entry per section, each with its symbol.
    fn settings_nav(&mut self, ui: &mut egui::Ui) {
        let entries = [
            (SettingsTab::General, "\u{2699}", self.tr("General / Broker", "Allgemein / Broker")),
            (SettingsTab::Notifications, "\u{2709}", self.tr("Notifications", "Benachrichtigungen")),
            (SettingsTab::TradingFeatures, "\u{26a1}", self.tr("Trading Features", "Handelsfunktionen")),
            (SettingsTab::MarketData, "\u{1f4ca}", self.tr("Market Data", "Marktdaten")),
            (SettingsTab::SafetyFeatures, "\u{1f6e1}", self.tr("Safety Features", "Sicherheitsfunktionen")),
            (SettingsTab::DatabaseBackup, "\u{1f4be}", self.tr("Database Backup", "Datenbank-Backup")),
        ];
        for (tab, symbol, label) in entries {
            if nav_item(ui, symbol, label, self.settings_tab == tab).clicked() {
                self.settings_tab = tab;
            }
            ui.add_space(2.0);
        }
    }

    /// The bar at the bottom of the Settings page: whether there are unsaved changes, the version and the Save button.
    fn settings_status_bar(&mut self, ui: &mut egui::Ui) {
        let dirty = self.settings_dirty();
        ui.horizontal(|ui| {
            let (dot, text) = if dirty {
                (ORANGE_WARN, self.tr("Unsaved changes", "Nicht gespeicherte Änderungen"))
            } else {
                (STATUS_OK, self.tr("All changes saved", "Alle Änderungen gespeichert"))
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, dot);
            ui.label(RichText::new(text).color(text_primary()));
            if dirty {
                ui.label(RichText::new(self.tr("You will be asked when you close the app.", "Beim Beenden wirst du gefragt.")).small().color(muted_text()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save_label = self.tr("Save Settings", "Einstellungen speichern");
                let button = if dirty {
                    crate::widgets::MetalButton::new(RichText::new(save_label).strong().color(Color32::WHITE)).fill(ACCENT_BLUE)
                } else {
                    crate::widgets::MetalButton::new(save_label)
                };
                if ui.add_sized([180.0, 30.0], button.rounding(8.0)).clicked() {
                    self.save_settings();
                }
            });
        });
    }

    /// The General / Broker Settings section: its groups one below the other.
    fn settings_general_tab(&mut self, ui: &mut egui::Ui) {
        self.broker_setup_card(ui);
        ui.add_space(14.0);
        self.market_time_card(ui);
        ui.add_space(14.0);
        self.accounts_card(ui);
        ui.add_space(14.0);
        self.logs_card(ui);
        ui.add_space(14.0);
        self.appearance_card(ui);
        ui.add_space(14.0);
        self.news_translation_card(ui);
        ui.add_space(14.0);
        white_card(ui, |ui| {
            section_header(ui, self.tr("Data location", "Datenablage"));
            self.storage_location_section(ui);
        });
    }

    /// The Broker Setup group: host, port, client id and how the connection is maintained.
    fn broker_setup_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Broker Setup", "Broker-Einrichtung"));
            setting_row(ui, self.tr("IBKR TWS Host", "IBKR TWS Host"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.tws_host).desired_width(160.0));
            });
            setting_row(ui, self.tr("IBKR TWS Port", "IBKR TWS Port"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.tws_port).desired_width(80.0));
            });
            setting_row(ui, self.tr("IBKR Client ID", "IBKR Client-ID"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.settings_client_id).desired_width(80.0));
            });
            let maintain_label = |app: &Self, opt: &str| -> &'static str {
                match opt {
                    "Always" => app.tr("Always", "Immer"),
                    "Market Hours" => app.tr("Market Hours", "Während der Handelszeit"),
                    _ => app.tr("Disabled", "Deaktiviert"),
                }
            };
            setting_row(ui, self.tr("Maintain Connection", "Verbindung aufrechterhalten"), "", |ui| {
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("maintain_connection")
                        .width(180.0)
                        .selected_text(maintain_label(self, &self.settings.settings_maintain_connection))
                        .show_ui(ui, |ui| {
                            for opt in MAINTAIN_CONNECTION_OPTIONS {
                                let label = maintain_label(self, opt);
                                ui.selectable_value(&mut self.settings.settings_maintain_connection, opt.to_string(), label);
                            }
                        })
                });
            });
        });
    }

    /// The Market Time group: the 0-DTE conversion time, the market close and the time zone warning.
    fn market_time_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Market Time", "Marktzeit"));
            setting_row(ui, self.tr("Convert 0 DTE Stops to Short", "0-DTE-Stops in Short umwandeln"), "", |ui| {
                // The row is laid out from the right: the time first, the switch to its left. While the switch is off,
                // the time cannot be changed.
                let enabled = self.settings.time_flags.convert_0dte_enabled;
                ui.add_enabled_ui(enabled, |ui| {
                    time_picker(
                        ui,
                        "convert_0dte_time",
                        &mut self.settings.convert_0dte_hour,
                        &mut self.settings.convert_0dte_minute,
                        &mut self.settings.time_flags.convert_0dte_pm,
                    );
                });
                ui.add_space(8.0);
                toggle_switch(ui, &mut self.settings.time_flags.convert_0dte_enabled);
            });
            setting_row(ui, self.tr("Market Close", "Marktschluss"), "", |ui| {
                time_picker(
                    ui,
                    "market_close_time",
                    &mut self.settings.market_close_hour,
                    &mut self.settings.market_close_minute,
                    &mut self.settings.time_flags.market_close_pm,
                );
            });
            let local_offset_hours = f64::from(self.clock.local().offset().local_minus_utc()) / 3600.0;
            if (local_offset_hours - (-5.0)).abs() > 0.1 && (local_offset_hours - (-4.0)).abs() > 0.1 {
                ui.add_space(8.0);
                egui::Frame::none().fill(warning_banner_bg()).rounding(8.0).inner_margin(10.0).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(warning_banner_text(), "(!)");
                        ui.vertical(|ui| {
                            // Both times above are plain wall-clock triggers that assume the
                            // OS clock already reads US Eastern - a timezone mismatch affects
                            // the 0-DTE conversion time exactly as much as Market Close, so the
                            // banner needs to say so instead of naming Market Close only.
                            ui.label(RichText::new(self.tr("Time Zone", "Zeitzone")).strong().color(warning_banner_text()));
                            ui.label(RichText::new(self.tr(
                                "Your computer's local time zone does not match US Eastern. You may need to adjust both the 0-DTE conversion time and the Market Close time above.",
                                "Die lokale Zeitzone deines Computers stimmt nicht mit US Eastern überein. Du musst eventuell sowohl die 0-DTE-Umwandlungszeit als auch die Marktschluss-Zeit oben anpassen.",
                            )).color(warning_banner_text()));
                            ui.label(RichText::new(format!("UTC{local_offset_hours:+.0}")).color(warning_banner_text()));
                        });
                    });
                });
            }
        });
    }

    /// The News Translation group: the optional address that raises the daily limit of the free translation service.
    fn news_translation_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("News Translation", "Nachrichten-Übersetzung"));
            setting_row(
                ui,
                self.tr("Contact address for the translation service (optional)", "Kontaktadresse für den Übersetzungsdienst (optional)"),
                self.tr(
                    "News that are not in the language of the screens are translated by the free service MyMemory; the text of those news is sent to it. An address of its own (not your private one) raises its daily limit and is sent to it as well. Empty: none is sent.",
                    "Nachrichten, die nicht in der Sprache der Oberfläche sind, übersetzt der kostenlose Dienst MyMemory; der Text dieser Nachrichten wird an ihn gesendet. Eine eigene Adresse (nicht deine private) erhöht sein Tageslimit und wird ebenfalls gesendet. Leer: es wird keine gesendet.",
                ),
                |ui| {
                    crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.translation_contact_email).desired_width(260.0));
                },
            );
        });
    }

    /// The Accounts group: the account numbers to ignore.
    fn accounts_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Accounts", "Konten"));
            setting_row(ui, self.tr("Account numbers to ignore", "Zu ignorierende Kontonummern"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.settings_ignore_accounts).desired_width(260.0));
            });
        });
    }

    /// The Logs group: logging level, clearing and the cleanup of old log files.
    fn logs_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Logs", "Protokolle"));
            let (info, debug) = (self.tr("Info", "Info"), self.tr("Debug (also what TWS answered)", "Debug (auch die Antworten der TWS)"));
            setting_row(ui, self.tr("Logging Level", "Protokollierungsstufe"), "", |ui| {
                let debug_on = &mut self.settings.view_flags.settings_logging_debug;
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("settings_logging_level").width(260.0).selected_text(if *debug_on { debug } else { info }).show_ui(ui, |ui| {
                        ui.selectable_value(debug_on, false, info);
                        ui.selectable_value(debug_on, true, debug);
                    })
                });
            });
            setting_row(ui, self.tr("Clear Logs", "Protokolle löschen"), "", |ui| {
                if ui.add(crate::widgets::MetalButton::new(self.tr("Clear Logs", "Protokolle löschen"))).clicked() {
                    self.ui_state.confirm_clear_logs = true;
                }
            });
            setting_row(ui, self.tr("Clean up old log files", "Alte Protokolldateien bereinigen"), "", |ui| {
                toggle_switch(ui, &mut self.settings.view_flags.settings_clean_logs);
            });
            setting_row(ui, self.tr("Cleanup Interval (days)", "Bereinigungsintervall (Tage)"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.settings_cleanup_interval).desired_width(70.0));
            });
        });
    }

    /// The Appearance group: grid font size, mobile grid, UI mode and language.
    fn appearance_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Appearance", "Darstellung"));
            setting_row(ui, self.tr("Grid Font Size", "Schriftgröße (Tabellen)"), "", |ui| {
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("grid_font_size").width(80.0).selected_text(self.settings.settings_grid_font.clone()).show_ui(ui, |ui| {
                        for size in ["12", "13", "14", "16", "18"] {
                            ui.selectable_value(&mut self.settings.settings_grid_font, size.to_string(), size);
                        }
                    })
                });
            });
            setting_row(ui, self.tr("Mobile Friendly Grid", "Mobilfreundliche Tabellen"), "", |ui| {
                toggle_switch(ui, &mut self.settings.misc_flags.settings_mobile_grid);
            });
            setting_row(
                ui,
                self.tr("UI mode", "Oberflächenmodus"),
                self.tr(
                    "Light shows only the most important pages: Dashboard, Instant Order, Trade Templates, Schedules, News and Settings.",
                    "Light zeigt nur die wichtigsten Seiten: Übersicht, Sofortorder, Trade-Vorlagen, Zeitpläne, Nachrichten und Einstellungen.",
                ),
                |ui| {
                    let light = self.settings.light_mode;
                    crate::widgets::metal_combo(ui, |ui| {
                        egui::ComboBox::from_id_salt("ui_mode").width(120.0).selected_text(ui_mode_name(light)).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.light_mode, true, ui_mode_name(true));
                            ui.selectable_value(&mut self.settings.light_mode, false, ui_mode_name(false));
                        })
                    });
                },
            );
            setting_row(ui, self.tr("Language", "Sprache"), "", |ui| {
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.settings.lang == Lang::En, "EN").clicked() {
                        self.settings.lang = Lang::En;
                    }
                    if ui.selectable_label(self.settings.lang == Lang::De, "DE").clicked() {
                        self.settings.lang = Lang::De;
                    }
                });
            });
        });
    }

    /// The folder of the database, the logs and everything else: shows the folder in use and lets
    /// the user pick another one (typed, or with the system's folder dialog). The new folder takes
    /// effect at the next start; the current data can be copied into it.
    fn storage_location_section(&mut self, ui: &mut egui::Ui) {
        ui.label(self.tr("Database and log file storage location:", "Speicherort für Datenbank und Protokolldateien:"));
        let location = storage::data_location().ok();
        let in_use = location.as_ref().map(|l| l.dir.display().to_string()).unwrap_or_default();
        ui.monospace(&in_use);
        if storage::portable_dir().is_some() {
            ui.label(
                RichText::new(self.tr(
                    "Portable mode: the data lies next to the program (portable.txt).",
                    "Portabler Modus: die Daten liegen neben dem Programm (portable.txt).",
                ))
                .small()
                .color(muted_text()),
            );
        }
        if let Some(reason) = location.as_ref().and_then(|l| l.fallback.clone()) {
            ui.colored_label(ORANGE_WARN, reason);
        }
        // The folder dialog runs on its own thread: take its answer when it is there.
        if let Some(receiver) = &self.folder_pick {
            match receiver.try_recv() {
                Ok(chosen) => {
                    if let Some(folder) = chosen {
                        self.storage_form.dir_input = folder;
                    }
                    self.folder_pick = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(200)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.folder_pick = None,
            }
        }
        if self.storage_form.dir_input.is_empty() && self.folder_pick.is_none() && self.storage_form.dir_status.is_none() {
            self.storage_form.dir_input.clone_from(&in_use);
        }
        ui.add_space(4.0);
        ui.label(self.tr("New folder (takes effect at the next start):", "Neuer Ordner (gilt ab dem nächsten Start):"));
        ui.horizontal(|ui| {
            crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.storage_form.dir_input).desired_width(380.0));
            let picking = self.folder_pick.is_some();
            if ui.add_enabled(!picking, crate::widgets::MetalButton::new(self.tr("Choose folder...", "Ordner wählen..."))).clicked() {
                self.folder_pick = Some(storage::pick_folder(&self.storage_form.dir_input));
            }
        });
        let copy_label = self.tr("Copy the current data into the new folder", "Aktuelle Daten in den neuen Ordner kopieren");
        crate::widgets::metal_checkbox(ui, &mut self.pending.storage_copy_data, copy_label);
        ui.horizontal(|ui| {
            if ui.add(crate::widgets::MetalButton::new(self.tr("Use this folder", "Diesen Ordner verwenden"))).clicked() {
                self.apply_storage_folder(Some(PathBuf::from(self.storage_form.dir_input.trim())));
            }
            if ui.add(crate::widgets::MetalButton::new(self.tr("Back to the default folder", "Zurück zum Standardordner"))).clicked() {
                self.apply_storage_folder(None);
            }
        });
        if let Some(status) = &self.storage_form.dir_status {
            ui.label(status);
        }
    }

    /// Remembers `folder` (`None` = the default) as the storage folder for the next start, after
    /// copying the current data into it when asked. The running session keeps using its folder.
    #[allow(clippy::needless_pass_by_value)] // the caller builds the path from a field of `self`
    fn apply_storage_folder(&mut self, folder: Option<PathBuf>) {
        let de = self.german();
        let Some(current) = storage::data_location().ok() else {
            self.storage_form.dir_status = Some(self.tr("The current storage folder is unknown.", "Der aktuelle Speicherordner ist unbekannt.").to_string());
            return;
        };
        let target = folder.clone().unwrap_or_else(|| current.default_dir.clone());
        let result = (|| -> std::io::Result<usize> {
            storage::check_data_dir(&target)?;
            let copied = if self.pending.storage_copy_data && target != current.dir { storage::copy_data(&current.dir, &target)? } else { 0 };
            storage::choose_data_dir(folder.as_deref())?;
            Ok(copied)
        })();
        self.storage_form.dir_status = Some(match result {
            Ok(copied) => {
                let message = format!(
                    "{} {}{}",
                    if de { "Gespeichert. Ab dem nächsten Start gilt:" } else { "Saved. From the next start on:" },
                    target.display(),
                    if copied > 0 { format!(" ({copied} {})", if de { "Dateien kopiert" } else { "files copied" }) } else { String::new() }
                );
                self.log(&message);
                message
            }
            Err(error) => {
                let message = format!("{} {}: {error}", if de { "Nicht geändert -" } else { "Not changed -" }, target.display());
                self.log(&message);
                message
            }
        });
    }

    /// Settings > Notifications. Every field here is real and persisted (see `PersistedSettings`);
    /// "Send Sample" actually sends via real SMTP/HTTP right now (see `send_notification_email`,
    /// `send_notification_webhook`) using whatever is in the fields at the moment of the click -
    /// click Save Settings separately to keep it for next time. "Entry filled" fires for real the
    /// moment an order this app placed is reported filled by TWS; there's no "Stop filled / closed"
    /// trigger yet.
    /// The SMTP part of the notification settings: the server, the login, the STARTTLS switch, the sample email and its outcome.
    fn smtp_settings_row(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(RichText::new(self.tr("SMTP (for sending)", "SMTP (zum Versenden)")).strong());
        setting_row(ui, self.tr("Host", "Host"), "", |ui| {
            crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.smtp_host).desired_width(200.0));
        });
        setting_row(ui, self.tr("Port", "Port"), "", |ui| {
            crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.smtp_port).desired_width(80.0));
        });
        setting_row(ui, self.tr("Username", "Benutzername"), "", |ui| {
            crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.smtp_username).desired_width(220.0));
        });
        setting_row(ui, self.tr("Password", "Passwort"), "", |ui| {
            crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.smtp_password).password(true).desired_width(220.0));
        });
        setting_row(ui, self.tr("STARTTLS", "STARTTLS"), "", |ui| {
            toggle_switch(ui, &mut self.settings.mail_flags.smtp_use_tls);
        });
        ui.add_space(10.0);
        if ui.add(crate::widgets::MetalButton::new(self.tr("Send Sample Notification Email", "Beispiel-Benachrichtigung per E-Mail senden"))).clicked() {
            self.notify_trade_event(
                "sample",
                self.tr("This is a sample notification from TradePilot.", "Dies ist eine Beispiel-Benachrichtigung von TradePilot."),
                true,
                false,
            );
        }
        if let Some(status) = &self.ui_state.mail_status {
            ui.colored_label(
                if status.starts_with("Notification email sent") {
                    STATUS_OK
                } else if status.starts_with("Sending") {
                    muted_text()
                } else {
                    STATUS_ERROR
                },
                status.as_str(),
            );
        }
    }

    fn notifications_tab(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Email", "E-Mail"));
            setting_row(ui, self.tr("Notification Email Address", "Benachrichtigungs-E-Mail-Adresse"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.notification_email).desired_width(260.0));
            });
            setting_row(ui, self.tr("Entry filled", "Trade eröffnet"), "", |ui| {
                toggle_switch(ui, &mut self.settings.notify_flags.notify_trade_opened_email);
            });
            setting_row(ui, self.tr("Stop filled / closed", "Trade gestoppt/geschlossen"), "", |ui| {
                toggle_switch(ui, &mut self.settings.notify_flags.notify_trade_stopped_email);
            });
            self.smtp_settings_row(ui);
        });
        ui.add_space(14.0);

        white_card(ui, |ui| {
            section_header(ui, self.tr("Webhook", "Webhook"));
            setting_row(ui, self.tr("Notification Webhook URL", "Benachrichtigungs-Webhook-URL"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.notification_webhook_url).desired_width(320.0));
            });
            setting_row(ui, self.tr("Entry filled", "Trade eröffnet"), "", |ui| {
                toggle_switch(ui, &mut self.settings.notify_flags.notify_trade_opened_webhook);
            });
            setting_row(ui, self.tr("Stop filled / closed", "Trade gestoppt/geschlossen"), "", |ui| {
                toggle_switch(ui, &mut self.settings.mail_flags.notify_trade_stopped_webhook);
            });
            ui.add_space(10.0);
            if ui.add(crate::widgets::MetalButton::new(self.tr("Send Sample Notification Webhook", "Beispiel-Benachrichtigung per Webhook senden"))).clicked() {
                self.notify_trade_event(
                    "sample",
                    self.tr("This is a sample notification from TradePilot.", "Dies ist eine Beispiel-Benachrichtigung von TradePilot."),
                    false,
                    true,
                );
            }
        });
    }

    /// Settings > Trading Features. Every field is real and persisted. The rest (stop
    /// conversions, profit target, net-liq override) have no automated engine behind them yet to
    /// act on these values - saved settings, not yet live behavior; see the in-UI notes.
    fn trading_features_tab(&mut self, ui: &mut egui::Ui) {
        self.webhook_settings_card(ui);
        ui.add_space(14.0);

        white_card(ui, |ui| {
            section_header(ui, self.tr("Leftover Long Behavior", "Verhalten bei übrigen Longs"));
            setting_row(
                ui,
                self.tr("Default for new templates", "Vorgabe für neue Templates"),
                self.tr(
                    "After a stop that buys back only the shorts: Ignore leaves the long in the account (a later trade can reuse it), Sell sells it at market if it has a bid. Each template can say otherwise.",
                    "Nach einem Stop, der nur die Shorts zurückkauft: Ignore lässt den Long im Konto (ein späterer Trade kann ihn wiederverwenden), Sell verkauft ihn per Market, wenn er einen Bid hat. Jedes Template kann es anders festlegen.",
                ),
                |ui| {
                    let selected = if self.settings.long_strike_reuse == "Sell" { "Sell" } else { "Ignore" };
                    crate::widgets::metal_combo(ui, |ui| egui::ComboBox::from_id_salt("default_leftover_long").width(120.0).selected_text(selected).show_ui(ui, |ui| {
                        for option in ["Ignore", "Sell"] {
                            ui.selectable_value(&mut self.settings.long_strike_reuse, option.to_string(), option);
                        }
                    }));
                },
            );
        });
        ui.add_space(14.0);

        white_card(ui, |ui| {
            section_header(ui, self.tr("IronFly/IronCondor - Full Stop Split", "IronFly/IronCondor - Full-Stop-Split"));
            setting_row(ui, self.tr("Split short stops on IF and IC-Full", "Short-Stops bei IF und IC-Full aufteilen"), "", |ui| {
                toggle_switch(ui, &mut self.settings.convert_flags.iron_fly_stop_split_enabled);
            });
            setting_row(ui, self.tr("Bid price to trigger split", "Bid-Preis zum Ausl\u{f6}sen des Splits"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.iron_fly_stop_split_price).desired_width(80.0));
            });
        });
        ui.add_space(14.0);

        self.stop_management_card(ui);
        ui.add_space(14.0);

        white_card(ui, |ui| {
            section_header(ui, self.tr("Daily Profit Target", "T\u{e4}gliches Gewinnziel"));
            ui.label(
                RichText::new(self.tr(
                    "Cancel the waiting entries and send a notification when today's profit reaches the target (0 = off)",
                    "Wartende Eintr\u{e4}ge stornieren und eine Meldung senden, wenn der Tagesgewinn das Ziel erreicht (0 = aus)",
                ))
                .color(muted_text()),
            );
            setting_row(ui, self.tr("Target Profit", "Zielgewinn"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.daily_profit_target).desired_width(100.0));
            });
            setting_row(ui, self.tr("Delay Seconds", "Verz\u{f6}gerung (Sek.)"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.daily_profit_target_delay_seconds).desired_width(80.0));
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(self.tr(
                    "The open trades are not closed by this: they keep their stops and profit targets. Please review them yourself.",
                    "Offene Trades werden dabei nicht geschlossen: sie behalten Stop und Profit Target. Bitte pr\u{fc}fe sie selbst.",
                ))
                .small()
                .color(muted_text()),
            );
        });
        ui.add_space(14.0);

        white_card(ui, |ui| {
            section_header(ui, self.tr("Net Liquidity Override", "Net-Liquidity-\u{dc}berschreibung"));
            setting_row(
                ui,
                self.tr("Amount", "Betrag"),
                self.tr(
                    "If provided, this amount is used for all PctOfNetLiq allocation calculations.",
                    "Falls angegeben, wird dieser Betrag f\u{fc}r alle PctOfNetLiq-Berechnungen verwendet.",
                ),
                |ui| {
                    crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.net_liquidity_override).desired_width(100.0));
                },
            );
        });
    }

    /// The Stop Management group of the Trading Features section.
    fn stop_management_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Stop Management", "Stop-Verwaltung"));
            setting_row(ui, self.tr("Convert Short Stop to Vertical", "Short-Stop in Vertical umwandeln"), "", |ui| {
                toggle_switch(ui, &mut self.settings.convert_flags.convert_short_to_vertical_enabled);
            });
            setting_row(ui, self.tr("Convert Vertical Stop to Short", "Vertical-Stop in Short umwandeln"), "", |ui| {
                toggle_switch(ui, &mut self.settings.convert_flags.convert_vertical_to_short_enabled);
            });
            setting_row(ui, self.tr("Convert Vertical to Short bid price", "Bid-Preis f\u{fc}r Vertical-zu-Short"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.convert_vertical_to_short_price).desired_width(80.0));
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(self.tr(
                    "No stop-management engine implemented yet - saved, but not yet acted on automatically.",
                    "Noch keine Stop-Verwaltungs-Engine implementiert - wird gespeichert, aber noch nicht automatisch ausgef\u{fc}hrt.",
                ))
                .small()
                .color(placeholder_color()),
            );
        });
    }

    /// Settings > Market Data. The provider dropdown only ever offers "TWS" - there is no
    /// other data source (a vendor-hosted feed is not available).
    /// Configure Symbols is a real, user-maintained list (starts empty - no row is ever
    /// invented), and the Dashboard Ticker Display dropdowns only offer symbols actually in it.
    fn market_data_tab(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Market Data Settings - Source", "Marktdaten-Einstellungen - Quelle"));
            setting_row(ui, self.tr("Market Data Provider", "Marktdaten-Anbieter"), "", |ui| {
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("market_data_provider").width(120.0).selected_text(self.settings.market_data_provider.clone()).show_ui(
                        ui,
                        |ui| {
                            ui.selectable_value(&mut self.settings.market_data_provider, "TWS".to_string(), "TWS");
                        },
                    )
                });
            });
            setting_row(ui, self.tr("Estimate SPX Extended Hours", "SPX au\u{df}erb\u{f6}rsliche Zeiten sch\u{e4}tzen"), "", |ui| {
                toggle_switch(ui, &mut self.settings.misc_flags.estimate_spx_extended_hours);
            });
        });
        ui.add_space(14.0);

        self.symbol_configs_card(ui);
        ui.add_space(14.0);

        white_card(ui, |ui| {
            section_header(ui, self.tr("Dashboard Ticker Display", "Dashboard-Ticker-Anzeige"));
            if self.settings.symbol_configs.is_empty() {
                ui.label(
                    egui::RichText::new(self.tr(
                        "Configure symbols first - the dropdowns only offer configured symbols.",
                        "Zuerst Symbole konfigurieren - die Auswahl bietet nur konfigurierte Symbole an.",
                    ))
                    .small()
                    .color(placeholder_color()),
                );
            }
            let configs = &self.settings.symbol_configs;
            setting_row(ui, self.tr("Price Symbol", "Preis-Symbol"), "", |ui| {
                symbol_combo(ui, "ticker_price_symbol", &mut self.settings.ticker_price_symbol, "-", configs);
            });
            setting_row(ui, self.tr("Delta Symbol", "Delta-Symbol"), "", |ui| {
                // Same fallback the dashboard uses when no delta symbol is chosen.
                symbol_combo(ui, "ticker_delta_symbol", &mut self.settings.ticker_delta_symbol, "SPX", configs);
            });
        });
    }

    /// The configured symbols as a list: a click selects one, Edit (or a double-click) loads it into the
    /// fields below, Delete removes it.
    fn symbol_configs_list(&mut self, ui: &mut egui::Ui) {
        let configs = self.settings.symbol_configs.clone();
        let position = self.settings_selection.symbol.as_ref().and_then(|symbol| configs.iter().position(|c| &c.symbol == symbol));
        let selected = position.and_then(|index| configs.get(index)).cloned();
        ui.horizontal(|ui| {
            if ui.add_enabled(selected.is_some(), crate::widgets::MetalButton::new(self.tr("Edit", "Bearbeiten"))).clicked() {
                if let Some(config) = &selected {
                    self.load_symbol_config(config);
                }
            }
            if ui.add_enabled(selected.is_some(), crate::widgets::MetalButton::new(self.tr("Delete", "L\u{f6}schen"))).clicked() {
                if let Some(config) = &selected {
                    self.settings.symbol_configs.retain(|c| c.symbol != config.symbol);
                    self.settings_selection.symbol = None;
                }
            }
        });
        ui.add_space(4.0);
        let columns = [
            TableColumn::new("Symbol", 1.0, false),
            TableColumn::new("Required DTEs", 1.4, false),
            TableColumn::new("Put OTM", 1.0, false),
            TableColumn::new("Call OTM", 1.0, false),
            TableColumn::new("Put ITM", 1.0, false),
            TableColumn::new("Call ITM", 1.0, false),
        ];
        let rows: Vec<Vec<TableCell>> = configs
            .iter()
            .map(|c| {
                [&c.symbol, &c.required_dtes, &c.put_otm, &c.call_otm, &c.put_itm, &c.call_itm].into_iter().map(|text| TableCell::new(text.clone())).collect()
            })
            .collect();
        let clicks = data_table_rows(ui, "symbol_configs_table", &columns, &rows, position);
        self.settings_selection.symbol = next_selection(self.settings_selection.symbol.take(), clicks, |index| configs.get(index).map(|c| c.symbol.clone()));
        if let Some(config) = clicks.double_clicked.and_then(|index| configs.get(index)) {
            self.load_symbol_config(config);
        }
    }

    /// Puts a symbol configuration into the fields to edit it.
    fn load_symbol_config(&mut self, config: &SymbolConfig) {
        self.new_symbol.clone_from(&config.symbol);
        self.new_symbol_required_dtes.clone_from(&config.required_dtes);
        self.template_order_flags.new_symbol_extended_quote_strikes = config.extended_quote_strikes;
    }

    /// The backups as a list: a click selects one, Restore loads its settings.
    fn backups_list(&mut self, ui: &mut egui::Ui, backups: &[crate::storage::BackupInfo]) {
        let position = self.settings_selection.backup.as_ref().and_then(|name| backups.iter().position(|b| &b.file_name == name));
        let selected = position.and_then(|index| backups.get(index));
        if ui.add_enabled(selected.is_some(), crate::widgets::MetalButton::new(self.tr("Restore", "Wiederherstellen"))).clicked() {
            if let Some(backup) = selected {
                match read_backup(&backup.path) {
                    Ok(settings) => {
                        self.apply_settings(settings);
                        self.log(&format!("Restored settings from {}", backup.file_name));
                    }
                    Err(error) => self.log(&format!("Could not restore backup: {error}")),
                }
            }
        }
        ui.add_space(4.0);
        let columns = [TableColumn::new("File", 3.0, false), TableColumn::new("Backup Date", 2.0, false), TableColumn::new("Size", 1.0, true)];
        let rows: Vec<Vec<TableCell>> = backups
            .iter()
            .map(|b| vec![TableCell::new(b.file_name.clone()), TableCell::new(b.modified.clone()), TableCell::new(format!("{} KB", b.size_bytes / 1024))])
            .collect();
        let clicks = data_table_rows(ui, "backups_table", &columns, &rows, position);
        self.settings_selection.backup = next_selection(self.settings_selection.backup.take(), clicks, |index| backups.get(index).map(|b| b.file_name.clone()));
    }

    /// The card to configure the symbols of the market data.
    fn symbol_configs_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Market Data Settings - Configure Symbols", "Marktdaten-Einstellungen - Symbole konfigurieren"));
            if self.settings.symbol_configs.is_empty() {
                ui.colored_label(
                    muted_text(),
                    self.tr("No symbols configured yet. Add one below.", "Noch keine Symbole konfiguriert. Unten eines hinzuf\u{fc}gen."),
                );
            } else {
                self.symbol_configs_list(ui);
            }
            ui.add_space(12.0);
            ui.label(RichText::new(self.tr("Add/Edit Symbol Configuration", "Symbol-Konfiguration hinzuf\u{fc}gen/bearbeiten")).strong());
            setting_row(ui, self.tr("Symbol", "Symbol"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.new_symbol).desired_width(100.0));
            });
            setting_row(ui, self.tr("Required DTEs", "Ben\u{f6}tigte DTEs"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.new_symbol_required_dtes).desired_width(100.0));
            });
            setting_row(ui, self.tr("Extended Quote Strikes", "Erweiterte Quote-Strikes"), "", |ui| {
                toggle_switch(ui, &mut self.template_order_flags.new_symbol_extended_quote_strikes);
            });
            ui.add_space(10.0);
            if ui.add(crate::widgets::MetalButton::new(self.tr("Save Symbol", "Symbol speichern"))).clicked() {
                let symbol = self.new_symbol.trim().to_uppercase();
                if !symbol.is_empty() {
                    let entry = SymbolConfig {
                        symbol: symbol.clone(),
                        required_dtes: self.new_symbol_required_dtes.trim().to_string(),
                        put_otm: String::new(),
                        call_otm: String::new(),
                        put_itm: String::new(),
                        call_itm: String::new(),
                        extended_quote_strikes: self.template_order_flags.new_symbol_extended_quote_strikes,
                    };
                    if let Some(existing) = self.settings.symbol_configs.iter_mut().find(|s| s.symbol == symbol) {
                        *existing = entry;
                    } else {
                        self.settings.symbol_configs.push(entry);
                    }
                    self.new_symbol.clear();
                    self.new_symbol_required_dtes.clear();
                    self.template_order_flags.new_symbol_extended_quote_strikes = false;
                }
            }
        });
    }

    /// Settings > Safety Features. The missing-stop and slippage notifications are live; closing a
    /// trade automatically and retrying an entry are only saved, so those rows carry an amber note.
    /// The "Trade Beyond Stop" warning (`alerts.rs`) is always sent, with its own fixed delay.
    fn safety_features_tab(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Safety Features", "Sicherheitsfunktionen"));
            let not_live = self.tr("Saved only - no effect yet", "Nur gespeichert - noch ohne Wirkung");

            setting_row_noted(ui, self.tr("Close trade on stop margin rejection", "Trade bei Margin-Ablehnung des Stops schlie\u{df}en"), "", not_live, |ui| {
                toggle_switch(ui, &mut self.settings.stop_flags.close_on_stop_margin_rejection);
            });
            setting_row_noted(
                ui,
                self.tr("Close trade beyond stop price", "Trade jenseits des Stop-Preises schlie\u{df}en"),
                self.tr("Off = warning only", "Aus = nur Warnung"),
                self.tr(
                    "Saved only - no effect yet; the warning is always sent after 60 seconds",
                    "Nur gespeichert - noch ohne Wirkung; die Warnung kommt immer nach 60 Sekunden",
                ),
                |ui| {
                    toggle_switch(ui, &mut self.settings.stop_flags.close_beyond_stop_enabled);
                },
            );
            setting_row_noted(ui, self.tr("% Beyond Stop", "% jenseits des Stops"), "", not_live, |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.close_beyond_stop_pct).desired_width(80.0));
            });
            setting_row_noted(ui, self.tr("Wait (seconds)", "Warten (Sek.)"), "", not_live, |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.close_beyond_stop_wait_seconds).desired_width(80.0));
            });
            setting_row_noted(ui, self.tr("Close Order Type", "Order-Typ zum Schlie\u{df}en"), "", not_live, |ui| {
                let market = self.settings.stop_flags.close_beyond_stop_market_order;
                ui.label(if market { self.tr("Market", "Market") } else { self.tr("Limit", "Limit") });
                toggle_switch(ui, &mut self.settings.stop_flags.close_beyond_stop_market_order);
            });
            setting_row(ui, self.tr("Missing Stop Notifications", "Benachrichtigungen bei fehlendem Stop"), "", |ui| {
                toggle_switch(ui, &mut self.settings.alert_flags.missing_stop_notifications_enabled);
            });
            setting_row(ui, self.tr("High Slippage Notifications", "Benachrichtigungen bei hoher Slippage"), "", |ui| {
                toggle_switch(ui, &mut self.settings.alert_flags.high_slippage_notifications_enabled);
            });
            setting_row(ui, self.tr("% Slippage to Notify", "% Slippage f\u{fc}r Benachrichtigung"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.high_slippage_pct).desired_width(80.0));
            });
            setting_row_noted(
                ui,
                self.tr("Retry trade entry on margin rejection", "Trade-Eintrag bei Margin-Ablehnung wiederholen"),
                self.tr("Off = cancel the trade", "Aus = Trade abbrechen"),
                not_live,
                |ui| {
                    toggle_switch(ui, &mut self.settings.alert_flags.retry_on_margin_rejection);
                },
            );
        });
    }

    /// Settings > Database Backup. There is no cloud backend, so this is a real
    /// local file export/import:
    /// "Backup Now" really writes a file under
    /// this app's data directory, "Available Backups" really lists what's there, "Restore" really
    /// loads one back. Nothing here is simulated.
    fn database_backup_tab(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Daily Database Backup", "T\u{e4}gliches Datenbank-Backup"));
            ui.label(
                RichText::new(self.tr(
                    "There is no cloud account to back up to - backups are written to a local file on this computer instead.",
                    "Es gibt keinen Cloud-Account zum Sichern - Backups werden stattdessen lokal auf diesem Computer gespeichert.",
                ))
                .small()
                .color(placeholder_color()),
            );
            ui.add_space(6.0);
            setting_row(ui, self.tr("Daily Backups Enabled", "T\u{e4}gliche Backups aktiviert"), "", |ui| {
                toggle_switch(ui, &mut self.settings.misc_flags.daily_backup_enabled);
            });
            setting_row(ui, self.tr("Computer Name", "Computername"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.backup_machine_name).desired_width(200.0));
            });
        });
        ui.add_space(14.0);

        white_card(ui, |ui| {
            section_header(ui, self.tr("Daily CSV Export", "Täglicher CSV-Export"));
            setting_row(
                ui,
                self.tr("Export the trades of the day after the market close", "Trades des Tages nach Handelsschluss exportieren"),
                self.tr(
                    "Writes TradePilot-Trades-<date>.csv to the exports folder of the mode (the same columns as Export to CSV on the Reports page).",
                    "Schreibt TradePilot-Trades-<Datum>.csv in den Ordner exports des Modus (dieselben Spalten wie Als CSV exportieren bei den Berichten).",
                ),
                |ui| {
                    toggle_switch(ui, &mut self.settings.daily_csv_export);
                },
            );
        });
        ui.add_space(14.0);

        white_card(ui, |ui| {
            section_header(ui, self.tr("Instant Database Backup", "Sofortiges Datenbank-Backup"));
            setting_row(ui, self.tr("One-time instant backup.", "Einmaliges Sofort-Backup."), "", |ui| {
                if ui.add(crate::widgets::MetalButton::new(self.tr("Backup Now", "Jetzt sichern"))).clicked() {
                    let settings = self.current_settings();
                    match write_backup(&settings) {
                        Ok(path) => self.log(&format!("Backup written to {}", path.display())),
                        Err(error) => self.log(&format!("Could not write backup: {error}")),
                    }
                }
            });
        });
        ui.add_space(14.0);

        white_card(ui, |ui| {
            section_header(ui, self.tr("Available Backups", "Verf\u{fc}gbare Backups"));
            match list_backups() {
                Ok(backups) if backups.is_empty() => {
                    ui.colored_label(muted_text(), self.tr("No backups yet.", "Noch keine Backups."));
                }
                Ok(backups) => self.backups_list(ui, &backups),
                Err(error) => {
                    ui.colored_label(crate::theme::negative_text(), format!("Could not list backups: {error}"));
                }
            }
        });
    }
}
