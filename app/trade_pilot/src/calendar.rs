//! A calendar in a pop-up for a list of dates kept as text (the blackout dates of a schedule): a click on a day adds it to the text
//! or removes it again, the days of the list are marked, and a day with a note (an exchange holiday, an early close) shows it as a tooltip.

use crate::schedule_form::{day_note, has_date, set_date};
use chrono::{Datelike, NaiveDate};
use eframe::egui;

const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

/// The first day of the month after `first`, or before it with `forward` false.
fn neighbour_month(first: NaiveDate, forward: bool) -> NaiveDate {
    let (year, month) = (first.year(), first.month());
    let (year, month) = match (forward, month) {
        (true, 12) => (year + 1, 1),
        (true, month) => (year, month + 1),
        (false, 1) => (year - 1, 12),
        (false, month) => (year, month - 1),
    };
    NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(first)
}

/// The days a month shows as weeks of seven (a Monday first); `None` where a week has days of the month before or after.
fn month_weeks(first: NaiveDate) -> Vec<[Option<NaiveDate>; 7]> {
    let leading = usize::try_from(first.weekday().num_days_from_monday()).unwrap_or(0);
    let days_in_month = (1..=31).take_while(|day| first.with_day(*day).is_some()).count();
    let mut cells: Vec<Option<NaiveDate>> = vec![None; leading];
    cells.extend((1..=31).filter_map(|day| first.with_day(day)).take(days_in_month).map(Some));
    cells.chunks(7).map(|week| std::array::from_fn(|index| week.get(index).copied().flatten())).collect()
}

/// The tooltip of a day: that it is in the list, and its note.
fn day_tooltip(date: NaiveDate, listed: bool) -> Option<String> {
    match (listed, day_note(date)) {
        (true, Some(note)) => Some(format!("Blackout day. {note}")),
        (true, None) => Some("Blackout day".to_string()),
        (false, note) => note,
    }
}

/// A button that opens the calendar for `dates`; `today` decides the month it opens at.
pub fn calendar_button(ui: &mut egui::Ui, id: impl std::hash::Hash, dates: &mut String, today: NaiveDate) {
    let popup_id = ui.make_persistent_id(id);
    let button = ui.add(crate::widgets::MetalButton::new("Calendar..."));
    if button.clicked() {
        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
    }
    egui::popup::popup_below_widget(ui, popup_id, &button, egui::popup::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        let month_id = popup_id.with("month");
        let mut first = ui.data(|data| data.get_temp::<NaiveDate>(month_id)).unwrap_or_else(|| today.with_day(1).unwrap_or(today));
        ui.horizontal(|ui| {
            if ui.add(crate::widgets::MetalButton::new("<")).clicked() {
                first = neighbour_month(first, false);
            }
            ui.label(first.format("%B %Y").to_string());
            if ui.add(crate::widgets::MetalButton::new(">")).clicked() {
                first = neighbour_month(first, true);
            }
        });
        egui::Grid::new(popup_id.with("grid")).spacing([4.0, 4.0]).show(ui, |ui| {
            for name in WEEKDAYS {
                ui.label(name);
            }
            ui.end_row();
            for week in month_weeks(first) {
                for cell in week {
                    let Some(date) = cell else {
                        ui.label("");
                        continue;
                    };
                    let listed = has_date(dates, date);
                    let mut button = egui::Button::new(date.day().to_string()).min_size(egui::vec2(30.0, 24.0));
                    if listed {
                        button = button.fill(ui.visuals().selection.bg_fill);
                    } else if day_note(date).is_some() {
                        button = button.stroke(egui::Stroke::new(1.5_f32, ui.visuals().warn_fg_color));
                    }
                    let mut response = ui.add(button);
                    if let Some(tip) = day_tooltip(date, listed) {
                        response = response.on_hover_text(tip);
                    }
                    if response.clicked() {
                        set_date(dates, date, !listed);
                    }
                }
                ui.end_row();
            }
        });
        ui.data_mut(|data| data.insert_temp(month_id, first));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_month_is_laid_out_in_weeks_that_start_on_monday() {
        // November 2026 starts on a Sunday and has 30 days.
        let weeks = month_weeks(day("2026-11-01"));
        assert_eq!(weeks.len(), 6);
        assert_eq!(weeks[0], [None, None, None, None, None, None, Some(day("2026-11-01"))]);
        assert_eq!(weeks[1][0], Some(day("2026-11-02")));
        assert_eq!(weeks[5][0], Some(day("2026-11-30")));
        assert_eq!(weeks[5][1], None);
        assert_eq!(weeks.iter().flatten().flatten().count(), 30);
        // February 2026 starts on a Sunday too, and has 28 days.
        assert_eq!(month_weeks(day("2026-02-01")).iter().flatten().flatten().count(), 28);
        assert_eq!(month_weeks(day("2024-02-01")).iter().flatten().flatten().count(), 29, "a leap year");
    }

    #[test]
    fn the_months_before_and_after_wrap_around_the_year() {
        assert_eq!(neighbour_month(day("2026-12-01"), true), day("2027-01-01"));
        assert_eq!(neighbour_month(day("2026-01-01"), false), day("2025-12-01"));
        assert_eq!(neighbour_month(day("2026-06-01"), true), day("2026-07-01"));
    }

    #[test]
    fn a_day_tells_that_it_is_listed_and_what_it_is() {
        assert_eq!(day_tooltip(day("2026-11-17"), false), None);
        assert_eq!(day_tooltip(day("2026-11-17"), true).as_deref(), Some("Blackout day"));
        assert_eq!(day_tooltip(day("2026-11-27"), false).as_deref(), Some("Early close at 13:00"));
        assert!(day_tooltip(day("2026-11-26"), true).is_some_and(|tip| tip.starts_with("Blackout day. ") && tip.ends_with("the exchange is closed")));
    }
}
