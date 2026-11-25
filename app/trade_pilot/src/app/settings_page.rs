//! The Settings page: a navigation of its sections on the left, the settings of the chosen section as rows on the right and a bar
//! with the state of the changes and the Save button at the bottom. What the settings mean is kept in `PersistedSettings`.

use super::{
    data_table_rows, framed_card, list_backups, muted_text, nav_item, next_selection, placeholder_color, section_header, setting_row, storage, symbol_combo,
    text_primary, time_picker, toggle_switch, ui_mode_name, warning_banner_bg, warning_banner_text, white_card, write_backup, App, Color32, Duration, Lang,
    PathBuf, RichText, SettingsTab, SymbolConfig, TableCell, TableColumn, ACCENT_BLUE, MAINTAIN_CONNECTION_OPTIONS, ORANGE_WARN, STATUS_ERROR, STATUS_OK,
};
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
        self.settings_help_window(ui.ctx());
    }

    /// The help window of the setting whose "?" was clicked: what it is for and the background, in the language of the screens. It
    /// closes with its button, with Escape, or when another "?" is clicked.
    fn settings_help_window(&mut self, ctx: &egui::Context) {
        let Some(entry) = crate::widgets::open_help(ctx).and_then(|name| crate::settings_help::help_for(&name)) else { return };
        let lang = self.settings.lang;
        let mut close = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let kicker = format!("{} \u{b7} {}", self.tr("Settings", "Einstellungen"), self.settings_tab_name(self.settings_tab));
        let got_it = self.tr("Got it", "Verstanden");
        egui::Window::new("settings_help")
            .id(egui::Id::new("settings_help_window"))
            .title_bar(false)
            .frame(egui::Frame::none())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                close |= crate::widgets::help_card(ui, &kicker, entry.title(lang), entry.text(lang), got_it);
            });
        if close {
            crate::widgets::close_help(ctx);
        }
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

    /// The name of a section of the Settings page.
    fn settings_tab_name(&self, tab: SettingsTab) -> &'static str {
        match tab {
            SettingsTab::General => self.tr("General / Broker", "Allgemein / Broker"),
            SettingsTab::Notifications => self.tr("Notifications", "Benachrichtigungen"),
            SettingsTab::TradingFeatures => self.tr("Trading Features", "Handelsfunktionen"),
            SettingsTab::MarketData => self.tr("Market Data", "Marktdaten"),
            SettingsTab::SafetyFeatures => self.tr("Safety Features", "Sicherheitsfunktionen"),
            SettingsTab::DatabaseBackup => self.tr("Database Backup", "Datenbank-Backup"),
        }
    }

    /// The navigation of the Settings page: one entry per section, each with its symbol.
    fn settings_nav(&mut self, ui: &mut egui::Ui) {
        let entries = [
            (SettingsTab::General, "\u{2699}"),
            (SettingsTab::Notifications, "\u{2709}"),
            (SettingsTab::TradingFeatures, "\u{26a1}"),
            (SettingsTab::MarketData, "\u{1f4ca}"),
            (SettingsTab::SafetyFeatures, "\u{1f6e1}"),
            (SettingsTab::DatabaseBackup, "\u{1f4be}"),
        ];
        for (tab, symbol) in entries {
            let label = self.settings_tab_name(tab);
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
                        ui.colored_label(warning_banner_text(), "(i)");
                        ui.vertical(|ui| {
                            ui.label(RichText::new(self.tr("Time Zone", "Zeitzone")).strong().color(warning_banner_text()));
                            ui.label(RichText::new(self.tr(
                                "Your computer is not on US Eastern time. Both times above are New York time (as the exchange keeps it), not the time of this computer.",
                                "Dein Computer läuft nicht auf US-Eastern-Zeit. Beide Zeiten oben sind New Yorker Zeit (wie an der Börse), nicht die Zeit dieses Computers.",
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
                    "Light shows only the most important pages: Dashboard, Instant Order, Trade Templates, Schedules, News, Logs and Settings.",
                    "Light zeigt nur die wichtigsten Seiten: Übersicht, Sofortorder, Trade-Vorlagen, Zeitpläne, Nachrichten, Protokolle und Einstellungen.",
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

    /// Settings > Notifications. "Send Sample" sends at once with what is in the fields at the moment of the click; Save Settings keeps
    /// it. "Entry filled" fires when TWS reports an order of this app filled, "Stop filled / closed" when a trade ends by its stop, its
    /// profit target or a close order.
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

    /// Settings > Trading Features. The stop conversions act through `stop_conversion`, the full stop split through `stop_split`, the
    /// daily profit target through `alerts`.
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
            setting_row(
                ui,
                self.tr("Split short stops on IF and IC-Full", "Short-Stops bei IF und IC-Full aufteilen"),
                self.tr(
                    "IronFly, IronCondor-Full and ShortStrangle with a Short stop: once a short is at the bid price or below, the trade becomes one trade per side, each with its own stop and target",
                    "IronFly, IronCondor-Full und ShortStrangle mit Short-Stop: sobald ein Short beim Bid-Preis oder darunter liegt, wird der Trade zu einem Trade je Seite, jeder mit eigenem Stop und Ziel",
                ),
                |ui| {
                    toggle_switch(ui, &mut self.settings.convert_flags.iron_fly_stop_split_enabled);
                },
            );
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
                    "Credit trades only. The stop keeps its price; it is cancelled and placed again on the new legs. Vertical to Short: a long without a bid, or with one at the price or below, leaves the stop. Short to Vertical: the long comes back at a bid of 0.10 or more, before the 0-DTE time.",
                    "Nur Credit-Trades. Der Stop beh\u{e4}lt seinen Preis; er wird storniert und auf den neuen Beinen neu platziert. Vertical zu Short: ein Long ohne Bid oder mit Bid zum Preis oder darunter verl\u{e4}sst den Stop. Short zu Vertical: der Long kommt ab einem Bid von 0,10 zur\u{fc}ck, vor der 0-DTE-Zeit.",
                ))
                .small()
                .color(muted_text()),
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
        self.new_symbol_window.put_otm.clone_from(&config.put_otm);
        self.new_symbol_window.call_otm.clone_from(&config.call_otm);
        self.new_symbol_window.put_itm.clone_from(&config.put_itm);
        self.new_symbol_window.call_itm.clone_from(&config.call_itm);
        self.template_order_flags.new_symbol_extended_quote_strikes = config.extended_quote_strikes;
    }

    /// The backups as a list: a click selects one, Restore loads its settings.
    fn backups_list(&mut self, ui: &mut egui::Ui, backups: &[crate::storage::BackupInfo]) {
        let position = self.settings_selection.backup.as_ref().and_then(|name| backups.iter().position(|b| &b.file_name == name));
        let selected = position.and_then(|index| backups.get(index));
        ui.horizontal(|ui| {
            if ui.add_enabled(selected.is_some(), crate::widgets::MetalButton::new(self.tr("Restore", "Wiederherstellen"))).clicked() {
                if let Some(backup) = selected {
                    self.restore_backup_file(backup);
                }
            }
            if ui.add_enabled(selected.is_some(), crate::widgets::MetalButton::new(self.tr("Delete", "L\u{f6}schen"))).clicked() {
                self.ui_state.confirm_delete_backup = selected.map(|b| b.file_name.clone());
            }
        });
        if let Some(name) = self.ui_state.confirm_delete_backup.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} {name}?", self.tr("Really delete the backup", "Backup wirklich l\u{f6}schen:"))).color(ORANGE_WARN));
                if ui.add(crate::widgets::MetalButton::new(self.tr("Yes, delete", "Ja, l\u{f6}schen"))).clicked() {
                    if let Some(backup) = backups.iter().find(|b| b.file_name == name) {
                        self.delete_backup_file(backup);
                    }
                    self.ui_state.confirm_delete_backup = None;
                }
                if ui.add(crate::widgets::MetalButton::new(self.tr("Cancel", "Abbrechen"))).clicked() {
                    self.ui_state.confirm_delete_backup = None;
                }
            });
        }
        if let Some((text, problem)) = &self.ui_state.backup_status {
            ui.add_space(4.0);
            ui.label(RichText::new(text).color(if *problem { ORANGE_WARN } else { STATUS_OK }));
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

    /// "Restore": lays the backup over the settings in use and says how it went, also where the backup does not fit this version.
    fn restore_backup_file(&mut self, backup: &crate::storage::BackupInfo) {
        let current = self.current_settings();
        let status = match storage::restore_backup(&backup.path, &current) {
            Ok(restored) => {
                let (text, problem) = restore_message(&backup.file_name, &restored, self.german());
                self.log(&format!(
                    "Restored settings from {} (missing: {:?}, unknown: {:?}, unusable: {:?})",
                    backup.file_name, restored.missing, restored.unknown, restored.unusable
                ));
                self.apply_settings(restored.settings);
                (text, problem)
            }
            Err(error) => {
                self.log(&format!("Could not restore backup {}: {error}", backup.file_name));
                let text = if self.german() {
                    format!("Backup {} NICHT wiederhergestellt: {error}. Die Einstellungen sind unver\u{e4}ndert.", backup.file_name)
                } else {
                    format!("Backup {} NOT restored: {error}. The settings are unchanged.", backup.file_name)
                };
                (text, true)
            }
        };
        self.ui_state.backup_status = Some(status);
    }

    /// Deletes a backup after its confirmation and says so.
    fn delete_backup_file(&mut self, backup: &crate::storage::BackupInfo) {
        let de = self.german();
        let status = match storage::delete_backup(&backup.path) {
            Ok(()) => {
                self.log(&format!("Backup deleted: {}", backup.file_name));
                self.settings_selection.backup = None;
                (if de { format!("Backup {} gel\u{f6}scht.", backup.file_name) } else { format!("Backup {} deleted.", backup.file_name) }, false)
            }
            Err(error) => {
                self.log(&format!("Could not delete backup {}: {error}", backup.file_name));
                (
                    if de {
                        format!("Backup {} NICHT gel\u{f6}scht: {error}", backup.file_name)
                    } else {
                        format!("Backup {} NOT deleted: {error}", backup.file_name)
                    },
                    true,
                )
            }
        };
        self.ui_state.backup_status = Some(status);
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
            let window_note = self.tr(
                "Points around the underlying price that get live quotes; empty = the default of the symbol.",
                "Punkte um den Kurs des Basiswerts, die Live-Kurse bekommen; leer = die Vorgabe des Symbols.",
            );
            let window = &mut self.new_symbol_window;
            for (label, field) in [
                (if self.settings.lang == Lang::De { "Put OTM (Punkte)" } else { "Put OTM (points)" }, &mut window.put_otm),
                (if self.settings.lang == Lang::De { "Call OTM (Punkte)" } else { "Call OTM (points)" }, &mut window.call_otm),
                (if self.settings.lang == Lang::De { "Put ITM (Punkte)" } else { "Put ITM (points)" }, &mut window.put_itm),
                (if self.settings.lang == Lang::De { "Call ITM (Punkte)" } else { "Call ITM (points)" }, &mut window.call_itm),
            ] {
                setting_row(ui, label, window_note, |ui| {
                    crate::widgets::metal_field(ui, egui::TextEdit::singleline(field).desired_width(80.0));
                });
            }
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
                        put_otm: window_value(&self.new_symbol_window.put_otm),
                        call_otm: window_value(&self.new_symbol_window.call_otm),
                        put_itm: window_value(&self.new_symbol_window.put_itm),
                        call_itm: window_value(&self.new_symbol_window.call_itm),
                        extended_quote_strikes: self.template_order_flags.new_symbol_extended_quote_strikes,
                    };
                    if let Some(existing) = self.settings.symbol_configs.iter_mut().find(|s| s.symbol == symbol) {
                        *existing = entry;
                    } else {
                        self.settings.symbol_configs.push(entry);
                    }
                    self.new_symbol.clear();
                    self.new_symbol_required_dtes.clear();
                    self.new_symbol_window = super::SymbolWindowForm::default();
                    self.template_order_flags.new_symbol_extended_quote_strikes = false;
                }
            }
        });
    }

    /// Settings > Safety Features. The "Trade Beyond Stop" warning (`alerts.rs`) is always sent after 60 seconds; closing the trade
    /// beyond its stop is an extra step with its own percent and wait.
    fn safety_features_tab(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Safety Features", "Sicherheitsfunktionen"));

            setting_row(
                ui,
                self.tr("Close trade on stop margin rejection", "Trade bei Margin-Ablehnung des Stops schlie\u{df}en"),
                self.tr(
                    "TWS refused the stop for margin: the trade is closed at market",
                    "Die TWS lehnt den Stop wegen Margin ab: der Trade wird per Market geschlossen",
                ),
                |ui| {
                    toggle_switch(ui, &mut self.settings.stop_flags.close_on_stop_margin_rejection);
                },
            );
            setting_row(
                ui,
                self.tr("Close trade beyond stop price", "Trade jenseits des Stop-Preises schlie\u{df}en"),
                self.tr(
                    "Off = warning only (always sent after 60 seconds). On = also closed once the cost to close stays the percent above the stop for the wait",
                    "Aus = nur Warnung (kommt immer nach 60 Sekunden). An = zus\u{e4}tzlich geschlossen, wenn der Schlie\u{df}preis die Wartezeit lang um den Prozentsatz \u{fc}ber dem Stop liegt",
                ),
                |ui| {
                    toggle_switch(ui, &mut self.settings.stop_flags.close_beyond_stop_enabled);
                },
            );
            setting_row(ui, self.tr("% Beyond Stop", "% jenseits des Stops"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.close_beyond_stop_pct).desired_width(80.0));
            });
            setting_row(ui, self.tr("Wait (seconds)", "Warten (Sek.)"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.close_beyond_stop_wait_seconds).desired_width(80.0));
            });
            setting_row(ui, self.tr("Close Order Type", "Order-Typ zum Schlie\u{df}en"), "", |ui| {
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
            setting_row(
                ui,
                self.tr("Retry trade entry on margin rejection", "Trade-Eintrag bei Margin-Ablehnung wiederholen"),
                self.tr(
                    "On = sent again up to 3 times, 20 seconds apart. Off = cancel the trade",
                    "An = bis zu 3-mal erneut gesendet, im Abstand von 20 Sekunden. Aus = Trade abbrechen",
                ),
                |ui| {
                    toggle_switch(ui, &mut self.settings.alert_flags.retry_on_margin_rejection);
                },
            );
        });
        ui.add_space(14.0);
        white_card(ui, |ui| {
            section_header(ui, self.tr("Risk Limits", "Risikogrenzen"));
            ui.label(
                RichText::new(self.tr(
                    "Checked for every order that opens a position; orders that close or protect one are never held up. 0 or empty = no limit.",
                    "Geprüft bei jeder Order, die eine Position eröffnet; Orders, die schließen oder absichern, werden nie aufgehalten. 0 oder leer = keine Grenze.",
                ))
                .small()
                .color(muted_text()),
            );
            setting_row(ui, self.tr("Max. open contracts per symbol", "Max. offene Kontrakte je Symbol"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.risk_max_contracts_per_symbol).desired_width(80.0));
            });
            setting_row(ui, self.tr("Daily loss limit ($)", "Tagesverlust-Grenze ($)"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.risk_max_daily_loss).desired_width(100.0));
            });
            setting_row(ui, self.tr("Max. new orders per minute", "Max. neue Orders pro Minute"), "", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.settings.risk_max_orders_per_minute).desired_width(80.0));
            });
        });
    }

    /// Settings > Database Backup: the daily backup and the CSV export, "Backup Now", and the backups in the backups folder with
    /// Restore and Delete.
    fn database_backup_tab(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Daily Database Backup", "T\u{e4}gliches Datenbank-Backup"));
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

/// A typed value of the quote window as it is stored: a number above 0 (a comma taken as the decimal point), else empty, which means
/// the default of the symbol.
fn window_value(text: &str) -> String {
    let text = text.trim().replace(',', ".");
    text.parse::<f64>().ok().filter(|value| *value > 0.0 && value.is_finite()).map_or_else(String::new, |_| text)
}

/// What the page says after a restore, and whether it is a warning: restored, and - if the backup is of another version - which
/// settings kept their current value and which entries were left out. The restored settings still have to be saved.
fn restore_message(file: &str, restored: &crate::storage::Restored, de: bool) -> (String, bool) {
    use std::fmt::Write as _;
    let mut text = if de {
        format!("Backup {file} wiederhergestellt. Mit \u{201e}Einstellungen speichern\u{201c} behalten, sonst gilt es nur bis zum Beenden.")
    } else {
        format!("Backup {file} restored. Click \"Save Settings\" to keep it, else it lasts only until the app is closed.")
    };
    if restored.exact() {
        return (text, false);
    }
    text.push_str(if de {
        " Das Backup stammt von einer anderen Version, die Einstellungen haben sich seither ge\u{e4}ndert:"
    } else {
        " The backup is from another version; the settings have changed since:"
    });
    let parts = [
        (&restored.missing, "not in the backup, they keep their current value", "nicht im Backup, sie behalten ihren aktuellen Wert"),
        (&restored.unusable, "do not fit any more, they keep their current value", "passen nicht mehr, sie behalten ihren aktuellen Wert"),
        (&restored.unknown, "unknown to this version, left out", "dieser Version unbekannt, weggelassen"),
    ];
    for (names, en, german) in parts {
        if !names.is_empty() {
            let _ = write!(text, " {} {} ({}).", names.len(), if de { german } else { en }, names.join(", "));
        }
    }
    (text, true)
}

#[cfg(test)]
mod tests {
    use super::{restore_message, window_value};
    use crate::storage::Restored;

    #[test]
    fn a_restore_says_it_worked_and_names_what_did_not_fit_this_version() {
        let exact = Restored { settings: super::super::default_settings(), missing: Vec::new(), unknown: Vec::new(), unusable: Vec::new() };
        let (text, problem) = restore_message("backup-1.json", &exact, true);
        assert!(text.starts_with("Backup backup-1.json wiederhergestellt.") && !problem, "{text}");
        let older = Restored { missing: vec!["light_mode".into()], unknown: vec!["old_switch".into()], ..exact };
        let (text, problem) = restore_message("backup-1.json", &older, false);
        assert!(problem);
        assert!(text.contains("restored") && text.contains("1 not in the backup") && text.contains("light_mode") && text.contains("old_switch"), "{text}");
    }

    #[test]
    fn a_window_value_is_a_number_above_zero_or_empty() {
        assert_eq!(window_value(" 220 "), "220");
        assert_eq!(window_value("12,5"), "12.5");
        assert_eq!(window_value("0"), "");
        assert_eq!(window_value("-5"), "");
        assert_eq!(window_value("abc"), "");
        assert_eq!(window_value(""), "");
    }
}
