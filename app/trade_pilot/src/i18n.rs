use serde::{Deserialize, Serialize};

#[derive(PartialEq, Clone, Copy, Serialize, Deserialize)]
pub enum Lang {
    En,
    De,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Page {
    Dashboard,
    LiveData,
    Quotes,
    InstantOrder,
    Templates,
    Schedules,
    Webhooks,
    Conditions,
    Reports,
    News,
    Logs,
    Settings,
}

pub const PAGES: [Page; 12] = [
    Page::Dashboard,
    Page::LiveData,
    Page::Quotes,
    Page::InstantOrder,
    Page::Templates,
    Page::Schedules,
    Page::Webhooks,
    Page::Conditions,
    Page::Reports,
    Page::News,
    Page::Logs,
    Page::Settings,
];

/// The pages of the Light mode: the most important ones. Advanced shows all of `PAGES`.
pub const LIGHT_PAGES: [Page; 7] = [Page::Dashboard, Page::InstantOrder, Page::Templates, Page::Schedules, Page::News, Page::Logs, Page::Settings];

/// Whether `page` is in the sidebar in the given mode.
#[must_use]
pub fn page_is_shown(page: Page, light: bool) -> bool {
    !light || LIGHT_PAGES.contains(&page)
}

/// The name of the interface mode on the screens (the same in both languages).
#[must_use]
pub fn ui_mode_name(light: bool) -> &'static str {
    if light {
        "Light"
    } else {
        "Advanced"
    }
}

pub fn page_name(lang: Lang, page: Page) -> &'static str {
    match (page, lang) {
        (Page::Dashboard, Lang::En) => "Dashboard",
        (Page::Dashboard, Lang::De) => "Übersicht",
        (Page::LiveData, Lang::En) => "Live Data",
        (Page::LiveData, Lang::De) => "Live-Daten",
        (Page::Quotes, Lang::En) => "Quotes",
        (Page::Quotes, Lang::De) => "Kurse",
        (Page::InstantOrder, Lang::En) => "Instant Order",
        (Page::InstantOrder, Lang::De) => "Sofortorder",
        (Page::Templates, Lang::En) => "Trade Templates",
        (Page::Templates, Lang::De) => "Trade-Vorlagen",
        (Page::Schedules, Lang::En) => "Schedules",
        (Page::Schedules, Lang::De) => "Zeitpläne",
        (Page::Webhooks, Lang::En) => "Webhooks / Triggers",
        (Page::Webhooks, Lang::De) => "Webhooks / Trigger",
        (Page::Conditions, Lang::En) => "Conditions",
        (Page::Conditions, Lang::De) => "Bedingungen",
        (Page::Reports, Lang::En) => "Reports",
        (Page::Reports, Lang::De) => "Berichte",
        (Page::News, Lang::En) => "News",
        (Page::News, Lang::De) => "Nachrichten",
        (Page::Logs, Lang::En) => "Logs",
        (Page::Logs, Lang::De) => "Protokolle",
        (Page::Settings, Lang::En) => "Settings",
        (Page::Settings, Lang::De) => "Einstellungen",
    }
}

/// Makes the dates of all screens follow `lang`: English YYYY-MM-DD, German DD-MM-YYYY.
pub fn set_date_language(lang: Lang) {
    strategy::dates::set_german(lang == Lang::De);
}

/// The `strftime` pattern of a date in `lang`: YYYY-MM-DD in English, DD-MM-YYYY in German.
#[must_use]
pub fn date_pattern_in(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "%Y-%m-%d",
        Lang::De => "%d-%m-%Y",
    }
}

/// The `strftime` pattern of a date in the language of the screens.
#[must_use]
pub fn date_pattern() -> &'static str {
    strategy::dates::date_pattern()
}

/// A date as the screens show it: YYYY-MM-DD in English, DD-MM-YYYY in German.
#[must_use]
pub fn show_date(date: chrono::NaiveDate) -> String {
    date.format(date_pattern()).to_string()
}

/// A date and time as the screens show them (the date as `show_date`, then `HH:MM:SS`).
#[must_use]
pub fn show_date_time(moment: chrono::NaiveDateTime) -> String {
    format!("{} {}", show_date(moment.date()), moment.format("%H:%M:%S"))
}

/// A stored day (`YYYY-MM-DD`) as the screens show it; text that is no such day stays as it is.
#[must_use]
pub fn show_stored_day(text: &str) -> String {
    chrono::NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").map_or_else(|_| text.to_string(), show_date)
}

#[cfg(test)]
mod date_tests {
    use super::*;

    /// The global style is not touched by tests (they run in parallel and expect the English default).
    #[test]
    fn dates_follow_the_language() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 13).unwrap();
        assert_eq!(day.format(date_pattern_in(Lang::En)).to_string(), "2026-11-13");
        assert_eq!(day.format(date_pattern_in(Lang::De)).to_string(), "13-11-2026");
        assert_eq!(show_date_time(day.and_hms_opt(9, 5, 7).unwrap()), "2026-11-13 09:05:07");
        assert_eq!(show_stored_day("2026-11-13"), "2026-11-13");
        assert_eq!(show_stored_day("not a day"), "not a day");
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn the_light_mode_shows_only_the_seven_important_pages_and_advanced_shows_all() {
        let light: Vec<Page> = PAGES.into_iter().filter(|page| page_is_shown(*page, true)).collect();
        assert!(light == LIGHT_PAGES, "the pages keep the order of the sidebar");
        assert!(PAGES.into_iter().all(|page| page_is_shown(page, false)));
        for page in LIGHT_PAGES {
            assert!(PAGES.contains(&page), "every page of the Light mode is a page of the sidebar");
        }
        assert!(page_is_shown(Page::Logs, true), "the log is in the Light mode too");
        assert!(!page_is_shown(Page::Reports, true) && !page_is_shown(Page::Conditions, true));
    }
}
