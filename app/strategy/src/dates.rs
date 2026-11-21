//! How dates are shown: YYYY-MM-DD in English, DD-MM-YYYY in German. The screens set the language once
//! per frame, so every text built here shows dates the same way.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether dates are shown the German way.
static GERMAN: AtomicBool = AtomicBool::new(false);

/// Makes all dates follow the language: `true` = German DD-MM-YYYY, else YYYY-MM-DD.
pub fn set_german(german: bool) {
    GERMAN.store(german, Ordering::Relaxed);
}

/// The `strftime` pattern of a date in the language that is set.
#[must_use]
pub fn date_pattern() -> &'static str {
    if GERMAN.load(Ordering::Relaxed) {
        "%d-%m-%Y"
    } else {
        "%Y-%m-%d"
    }
}

/// A date as the screens show it.
#[must_use]
pub fn show_date(date: chrono::NaiveDate) -> String {
    date.format(date_pattern()).to_string()
}
