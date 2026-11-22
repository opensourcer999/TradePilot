//! The News screen: the newest economic news of the public feeds, US news leading, in a table (date, time, source, headline), as many
//! as fit on the screen. A click on a headline opens the whole item below it.

use super::{card_bg, muted_text, text_primary, white_card, App, Instant, RichText, ACCENT_BLUE, ORANGE_WARN};
use crate::i18n::date_pattern_in;
use crate::news::{self, NewsItem};
use crate::storage::{data_file, read_json_file, write_json_file};
use chrono::{DateTime, Local};
use eframe::egui;
use rust_decimal::prelude::ToPrimitive;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

/// The feed is asked for new items this often.
const REFRESH_AFTER: Duration = Duration::from_secs(300);

/// The height of a line with a one-line headline, to estimate how many lines fit.
const ROW_HEIGHT: f32 = 28.0;

/// The room a line of the table needs: two lines of headline, the gaps and the edge of the card.
const ROW_ROOM: f32 = 70.0;

/// After a load that failed (no network, the feed is down) the next try comes this soon.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// What the person has seen: the time of the newest item (seconds since 1970). It is kept in `news_seen.json`, so a restart does not
/// show the same news as new again.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct SeenNews {
    up_to: Option<i64>,
}

/// The file in the data folder that keeps what has been seen.
const SEEN_FILE: &str = "news_seen.json";

/// The loaded news and what is going on with them.
#[derive(Default)]
pub(super) struct NewsState {
    pub(super) items: Vec<NewsItem>,
    /// The answer of the load that is running.
    receiver: Option<Receiver<Result<news::Loaded, String>>>,
    /// The sources the last load could not read.
    failed_sources: Vec<String>,
    /// When the items were loaded: on the monotonic clock and on the wall clock.
    loaded: Option<(Instant, DateTime<Local>)>,
    error: Option<String>,
    /// When the last load was started.
    attempted: Option<Instant>,
    /// The lower edge of the room the page has on the screen (0 before the first frame).
    pub(super) room_bottom: f32,
    /// The translations of the headlines and texts.
    pub(super) translation: super::news_translate::TranslationState,
    /// What the person has seen; `None` until it is read from its file.
    seen: Option<SeenNews>,
    /// The item that is opened (its address, or its headline without one).
    opened: Option<String>,
}

/// Whether the feed is to be loaded now: when it was never asked for, every `REFRESH_AFTER`, and after a failed load every `RETRY_AFTER`.
/// `waited` is the time since the last load was started.
fn load_is_due(failed: bool, waited: Option<Duration>) -> bool {
    match waited {
        None => true,
        Some(waited) if failed => waited >= RETRY_AFTER,
        Some(waited) => waited >= REFRESH_AFTER,
    }
}

/// One line of the table as it is shown: the item, its headline and text (translated when that is there), and whether they are.
struct NewsRow {
    item: NewsItem,
    title: String,
    text: String,
    translated: bool,
}

/// The strip behind a line of the list: the whole width of the card, a little taller than the line.
fn strip_of(ui: &egui::Ui, rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(ui.max_rect().left(), rect.top() - 3.0), egui::pos2(ui.max_rect().right(), rect.bottom() + 3.0))
}

impl App {
    /// Starts loading the feeds (not in tests: they have no network).
    fn start_news_load(&mut self) {
        self.news.attempted = Some(self.clock.instant());
        if cfg!(test) {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(news::load());
        });
        self.news.receiver = Some(receiver);
    }

    /// Called on every frame, whichever screen is open: takes the answer of a load that has finished and starts the next one when it is due -
    /// at the start of the app, every `REFRESH_AFTER`, and after a failed load every `RETRY_AFTER`, so the
    /// news come back by themselves when the network does.
    pub(super) fn news_tick(&mut self, ctx: &egui::Context) {
        self.poll_news(ctx);
        self.poll_translations();
        // Wakes the app for the next check even when nothing else is going on.
        ctx.request_repaint_after(RETRY_AFTER);
        if self.news.receiver.is_some() {
            return;
        }
        let waited = self.news.attempted.map(|at| self.clock.instant().saturating_duration_since(at));
        if load_is_due(self.news.error.is_some(), waited) {
            self.start_news_load();
        }
    }

    /// Reads what has been seen from its file, once; a file that cannot be read is logged and nothing counts as seen.
    fn load_seen(&mut self) {
        if self.news.seen.is_some() {
            return;
        }
        let seen = match data_file(SEEN_FILE).map_or_else(|| Ok(SeenNews::default()), |path| read_json_file(&path)) {
            Ok(seen) => seen,
            Err(reason) => {
                self.log(&format!("WARNING: News: {reason}"));
                SeenNews::default()
            }
        };
        self.news.seen = Some(seen);
    }

    /// Takes everything that is shown as seen: the bulletin on the Dashboard goes away until newer news arrive.
    pub(super) fn mark_news_seen(&mut self) {
        let Some(newest) = self.news.items.iter().map(|item| item.published.timestamp()).max() else { return };
        self.load_seen();
        let seen = self.news.seen.get_or_insert_with(SeenNews::default);
        if seen.up_to == Some(newest) {
            return;
        }
        seen.up_to = Some(newest);
        if let Some(path) = data_file(SEEN_FILE) {
            if let Err(reason) = write_json_file(&path, seen) {
                self.log(&format!("WARNING: News: {reason}"));
            }
        }
    }

    /// The time of the newest item the person has seen; `None` before anything was seen.
    fn seen_up_to(&self) -> Option<i64> {
        self.news.seen.as_ref().and_then(|seen| seen.up_to)
    }

    /// The items that arrived since the person last looked at the news, the newest first.
    pub(super) fn unseen_news(&self) -> Vec<&NewsItem> {
        let Some(seen) = self.seen_up_to() else { return Vec::new() };
        self.news.items.iter().filter(|item| item.published.timestamp() > seen).collect()
    }

    /// Whether any item arrived since the person last looked at the news (the dot in the sidebar, asked on every frame).
    pub(super) fn has_unseen_news(&self) -> bool {
        self.seen_up_to().is_some_and(|seen| self.news.items.iter().any(|item| item.published.timestamp() > seen))
    }

    /// The bulletin at the top of the Dashboard: new news have arrived. "Show news" opens the News screen, the cross dismisses it.
    pub(super) fn news_bulletin(&mut self, ui: &mut egui::Ui) {
        let (count, newest) = {
            let unseen = self.unseen_news();
            (unseen.len(), unseen.first().map(|item| item.title.clone()))
        };
        let Some(newest) = newest else { return };
        let title = if count == 1 {
            self.tr("1 new news item", "1 neue Nachricht").to_string()
        } else {
            format!("{count} {}", self.tr("new news items", "neue Nachrichten"))
        };
        let mut show = false;
        let mut dismiss = false;
        egui::Frame::none()
            .fill(ACCENT_BLUE.gamma_multiply(0.18))
            .stroke(egui::Stroke::new(1.0_f32, ACCENT_BLUE))
            .rounding(8.0)
            .inner_margin(egui::vec2(12.0, 8.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    egui::Frame::none().fill(ACCENT_BLUE).rounding(10.0).inner_margin(egui::vec2(8.0, 1.0)).show(ui, |ui| {
                        ui.label(RichText::new(self.tr("NEWS", "NEWS")).small().strong().color(egui::Color32::WHITE));
                    });
                    ui.label(RichText::new(title).strong().color(text_primary()));
                    ui.label(RichText::new(newest).color(muted_text()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // "×" is in every font; the heavier cross signs are not and showed as an empty box.
                        let cross = egui::Button::new(RichText::new("\u{d7}").size(18.0).color(text_primary())).frame(false);
                        dismiss = ui.add(cross).on_hover_text(self.tr("Dismiss", "Ausblenden")).clicked();
                        show = ui.add(crate::widgets::MetalButton::new(self.tr("Show news", "Nachrichten anzeigen"))).clicked();
                    });
                });
            });
        ui.add_space(10.0);
        if show {
            self.page = crate::i18n::Page::News;
        }
        if show || dismiss {
            self.mark_news_seen();
        }
    }

    /// Takes the answer of a load that has finished.
    fn poll_news(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.news.receiver else { return };
        match receiver.try_recv() {
            Ok(Ok(loaded)) => {
                self.log(&format!("News: {} items loaded", loaded.items.len()));
                for source in &loaded.failed_sources {
                    self.log(&format!("News: could not read {source}"));
                }
                self.news.items = loaded.items;
                self.news.failed_sources = loaded.failed_sources;
                self.news.loaded = Some((self.clock.instant(), self.clock.local().with_timezone(&Local)));
                self.news.error = None;
                self.news.receiver = None;
                self.load_seen();
                if self.seen_up_to().is_none() {
                    // The very first load: everything on it is the state of today, not news to point out.
                    self.mark_news_seen();
                } else {
                    let fresh = self.unseen_news().len();
                    if fresh > 0 {
                        self.log(&format!("News: {fresh} new items"));
                    }
                }
            }
            Ok(Err(reason)) => {
                self.log(&format!("News: could not load the feed: {reason}"));
                self.news.error = Some(reason);
                self.news.receiver = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(300)),
            Err(TryRecvError::Disconnected) => self.news.receiver = None,
        }
    }

    pub(super) fn news_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(self.tr("Economic News", "Wirtschaftsnachrichten")).color(self.heading_color()));
            ui.add_space(10.0);
            let loading = self.news.receiver.is_some();
            if ui.add_enabled(!loading, crate::widgets::MetalButton::new(self.tr("Refresh", "Aktualisieren"))).clicked() {
                self.start_news_load();
            }
            if loading {
                ui.spinner();
            } else if let Some((_, at)) = self.news.loaded {
                ui.label(RichText::new(format!("{} {}", self.tr("Updated", "Stand"), at.format("%H:%M:%S"))).small().color(muted_text()));
            }
        });
        ui.label(RichText::new(news::source_names()).small().color(muted_text()));
        if let Some(reason) = self.news.translation.error.clone() {
            ui.label(
                RichText::new(format!(
                    "{}: {reason}",
                    self.tr("Translation failed, the original text is shown", "Übersetzung fehlgeschlagen, es erscheint der Originaltext")
                ))
                .small()
                .color(ORANGE_WARN),
            );
        }
        if !self.news.failed_sources.is_empty() {
            ui.label(
                RichText::new(format!("{}: {}", self.tr("Not reachable", "Nicht erreichbar"), self.news.failed_sources.join("; "))).small().color(muted_text()),
            );
        }
        ui.add_space(8.0);
        if let Some(error) = &self.news.error {
            ui.colored_label(
                crate::theme::negative_text(),
                format!("{}: {error}", self.tr("The news could not be loaded", "Die Nachrichten konnten nicht geladen werden")),
            );
            ui.add_space(6.0);
        }
        if self.news.items.is_empty() {
            if self.news.receiver.is_some() {
                ui.label(RichText::new(self.tr("Loading the news...", "Nachrichten werden geladen...")).color(muted_text()));
            }
            return;
        }
        self.news_table(ui);
        // What is on the screen has been seen.
        self.mark_news_seen();
    }

    /// The table: a header line and one line per item; the opened item shows its text and a link to the whole article below its line.
    fn news_table(&mut self, ui: &mut egui::Ui) {
        let lang = self.settings.lang;
        let pattern = date_pattern_in(lang);
        let (date_title, time_title, headline_title) = (self.tr("Date", "Datum"), self.tr("Time", "Uhrzeit"), self.tr("Headline", "Meldung"));
        let region_title = self.tr("From", "Quelle");
        let translated_note = self.tr("Translated automatically", "Automatisch übersetzt");
        let open_label = self.tr("Open the whole article", "Ganzen Artikel öffnen");
        // What is on the screen is translated first: the headlines of the lines that fit, and the text of the opened item.
        let ctx = ui.ctx().clone();
        let visible = if self.news.room_bottom > 0.0 {
            ((self.news.room_bottom - ui.cursor().top()).max(0.0) / ROW_HEIGHT).to_usize().unwrap_or(0) + 2
        } else {
            usize::MAX
        };
        let mut rows: Vec<NewsRow> = Vec::new();
        let shown: Vec<NewsItem> = self.news.items.iter().take(visible).cloned().collect();
        for (index, item) in shown.into_iter().enumerate() {
            let opened = self.news.opened.as_deref() == Some(item.key());
            if index < visible {
                self.want_translation(&ctx, item.region, &item.title);
            }
            if opened {
                self.want_translation(&ctx, item.region, &item.text);
            }
            let title = self.translated(item.region, &item.title);
            let text = if opened { self.translated(item.region, &item.text) } else { None };
            let translated = title.is_some() || text.is_some();
            rows.push(NewsRow { title: title.unwrap_or_else(|| item.title.clone()), text: text.unwrap_or_else(|| item.text.clone()), translated, item });
        }
        let mut toggled: Option<String> = None;
        white_card(ui, |ui| {
            let cell = |ui: &mut egui::Ui, width: f32, text: RichText| {
                ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.label(text);
                });
            };
            // The column titles and every item on a raised strip of metal, like the rows of the other lists.
            let header_strip = ui.painter().add(egui::Shape::Noop);
            let header = ui.horizontal_top(|ui| {
                cell(ui, 24.0, RichText::new(""));
                cell(ui, 96.0, RichText::new(date_title).small().strong().color(muted_text()));
                cell(ui, 56.0, RichText::new(time_title).small().strong().color(muted_text()));
                cell(ui, 40.0, RichText::new(region_title).small().strong().color(muted_text()));
                cell(ui, 200.0, RichText::new(headline_title).small().strong().color(muted_text()));
            });
            ui.painter().set(header_strip, crate::widgets::plastic_strip(strip_of(ui, header.response.rect), 0, true));
            ui.add_space(6.0);
            let room_bottom = self.news.room_bottom;
            for (line_index, NewsRow { item, title, text, translated }) in rows.iter().enumerate() {
                // As many items as fit: no scroll bar. A line needs room for two lines of headline and the card's edge.
                if room_bottom > 0.0 && room_bottom - ui.cursor().top() < ROW_ROOM {
                    break;
                }
                let key = item.key().to_string();
                let opened = self.news.opened.as_deref() == Some(key.as_str());
                let local = item.published.with_timezone(&Local);
                let strip = ui.painter().add(egui::Shape::Noop);
                let line = ui.horizontal_top(|ui| {
                    cell(ui, 24.0, RichText::new(if opened { "\u{25be}" } else { "\u{25b8}" }).color(muted_text()));
                    cell(ui, 96.0, RichText::new(local.format(pattern).to_string()).color(text_primary()));
                    cell(ui, 56.0, RichText::new(local.format("%H:%M").to_string()).color(text_primary()));
                    cell(ui, 40.0, RichText::new(item.region.label()).small().strong().color(muted_text()));
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        let title = RichText::new(title.as_str()).color(if opened { ACCENT_BLUE } else { text_primary() });
                        ui.add(egui::Label::new(if opened { title.strong() } else { title }).wrap());
                    });
                });
                ui.painter().set(strip, crate::widgets::plastic_strip(strip_of(ui, line.response.rect), line_index, false));
                let click =
                    ui.interact(line.response.rect, ui.id().with(("news_line", &key)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                if click.clicked() {
                    toggled = Some(key.clone());
                }
                if opened {
                    egui::Frame::none().fill(card_bg()).inner_margin(egui::vec2(24.0, 4.0)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if text.is_empty() {
                            ui.label(
                                RichText::new(self.tr("This item has no text in the feed.", "Zu dieser Meldung liefert der Feed keinen Text."))
                                    .color(muted_text())
                                    .italics(),
                            );
                        } else {
                            ui.add(egui::Label::new(RichText::new(text.as_str()).color(text_primary())).wrap());
                        }
                        if !item.link.is_empty() {
                            ui.add_space(4.0);
                            ui.hyperlink_to(open_label, &item.link);
                        }
                        if *translated {
                            ui.label(RichText::new(translated_note).small().color(muted_text()));
                        }
                    });
                }
                ui.add_space(6.0);
            }
        });
        if let Some(key) = toggled {
            self.news.opened = if self.news.opened.as_deref() == Some(key.as_str()) { None } else { Some(key) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_at(minute: i64) -> NewsItem {
        news::test_item(news::Region::Us, minute)
    }

    fn deliver(app: &mut App, items: Vec<NewsItem>) {
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok(news::Loaded { items, ..news::Loaded::default() })).unwrap();
        app.news.receiver = Some(receiver);
        app.poll_news(&egui::Context::default());
    }

    #[test]
    fn the_first_load_is_not_new_but_later_arrivals_are_until_the_person_has_seen_them() {
        let mut app = App::new(Some("test".to_string()), None);
        deliver(&mut app, vec![item_at(2), item_at(1)]);
        assert!(app.unseen_news().is_empty() && !app.has_unseen_news(), "the state of today is not pointed out");
        // Two items arrive that are newer than the newest one seen.
        deliver(&mut app, vec![item_at(4), item_at(3), item_at(2), item_at(1)]);
        assert_eq!(app.unseen_news().iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["US4", "US3"]);
        assert!(app.has_unseen_news());
        app.mark_news_seen();
        assert!(app.unseen_news().is_empty() && !app.has_unseen_news());
        deliver(&mut app, vec![item_at(5), item_at(4), item_at(3), item_at(2), item_at(1)]);
        assert_eq!(app.unseen_news().len(), 1);
    }

    #[test]
    fn the_news_are_asked_for_at_the_start_every_five_minutes_and_half_a_minute_after_a_failure() {
        let secs = |n| Some(Duration::from_secs(n));
        assert!(load_is_due(false, None), "at the start of the app");
        assert!(load_is_due(true, None));
        assert!(!load_is_due(false, secs(299)));
        assert!(load_is_due(false, secs(300)), "every five minutes");
        assert!(!load_is_due(true, secs(29)));
        assert!(load_is_due(true, secs(30)), "soon after a failure, so the news return with the network");
    }
}
