//! The values the number fields of the editors offer in their lists. A field stays a text field: the person may pick a value or type another one.

/// The seconds of a schedule's time.
pub const SECONDS: [&str; 4] = ["0", "15", "30", "45"];
/// Minutes of a time window.
pub const WINDOW_MINUTES: [&str; 8] = ["0", "1", "2", "5", "10", "15", "30", "60"];
/// Number of attempts (0 = no limit).
pub const ATTEMPTS: [&str; 5] = ["0", "1", "3", "5", "10"];
/// Limits in counts (0 = no limit).
pub const COUNTS: [&str; 6] = ["0", "1", "2", "3", "5", "10"];
/// Numbers of contracts.
pub const CONTRACTS: [&str; 6] = ["1", "2", "3", "5", "10", "20"];
/// Quantity as a percentage of the net liquidation value.
pub const NET_LIQ_PERCENT: [&str; 7] = ["1", "2", "5", "10", "20", "50", "100"];
/// Premium budget in dollars.
pub const PREMIUM_BUDGET: [&str; 6] = ["100", "250", "500", "1000", "2500", "5000"];
/// Ratio of a custom leg.
pub const RATIO: [&str; 3] = ["1", "2", "3"];
/// Days to expiration.
pub const DTES: [&str; 8] = ["0", "1", "2", "3", "5", "7", "14", "30"];
/// A delta target in percent.
pub const DELTAS: [&str; 8] = ["5", "10", "15", "20", "25", "30", "40", "50"];
/// A premium target.
pub const PREMIUMS: [&str; 7] = ["0.25", "0.5", "1", "1.5", "2", "3", "5"];
/// A strike offset in points.
pub const STRIKE_OFFSETS: [&str; 21] =
    ["-50", "-45", "-40", "-35", "-30", "-25", "-20", "-15", "-10", "-5", "0", "5", "10", "15", "20", "25", "30", "35", "40", "45", "50"];
/// Minimum distance out of the money.
pub const MIN_OTM: [&str; 5] = ["0", "5", "10", "20", "50"];
/// Widths of a spread.
pub const WIDTHS: [&str; 6] = ["5", "10", "15", "20", "25", "50"];
/// Premium limits of the long options.
pub const LONG_PREMIUMS: [&str; 5] = ["0.05", "0.1", "0.25", "0.5", "1"];
/// Limits of the entry price.
pub const ENTRY_PRICES: [&str; 6] = ["0.5", "1", "1.5", "2", "3", "5"];
/// Limits of the short/long ratio.
pub const SHORT_LONG_RATIOS: [&str; 5] = ["0.5", "1", "1.5", "2", "3"];
/// Seconds to wait for a fill.
pub const WAIT_SECONDS: [&str; 6] = ["3", "5", "7", "10", "15", "30"];
/// The step an unfilled order is moved by.
pub const FILL_STEPS: [&str; 3] = ["0.05", "0.1", "0.25"];
/// Multiples of a stop.
pub const STOP_MULTIPLES: [&str; 7] = ["1", "1.5", "2", "2.5", "3", "4", "5"];
/// Amounts of a stop.
pub const STOP_AMOUNTS: [&str; 4] = ["1", "2", "3", "5"];
/// Offsets of orders.
pub const OFFSETS: [&str; 6] = ["0", "0.05", "0.1", "0.25", "0.5", "1"];
/// The limit of a REL order.
pub const REL_LIMITS: [&str; 4] = ["1", "2", "5", "8"];
/// The distance of the ITM protection.
pub const ITM_DISTANCES: [&str; 5] = ["0", "5", "10", "20", "25"];
/// Minutes before the close.
pub const MINUTES_BEFORE_CLOSE: [&str; 5] = ["5", "10", "15", "30", "60"];
/// The offsets of the guardrail.
pub const GUARDRAIL_OFFSETS: [&str; 5] = ["0", "5", "10", "15", "20"];
/// The trigger of a stop adjustment in percent of the profit.
pub const ADJUSTMENT_PERCENTS: [&str; 3] = ["25", "50", "75"];
/// The new stop of an adjustment as a multiple.
pub const NEW_STOP_MULTIPLES: [&str; 5] = ["0", "0.5", "1", "1.5", "2"];
/// A profit target in percent.
pub const TARGET_PERCENTS: [&str; 3] = ["25", "50", "75"];
/// The share of the trade a target closes.
pub const SHARES_TO_CLOSE: [&str; 4] = ["25", "50", "75", "100"];
/// Minutes in a trade.
pub const MINUTES_IN_TRADE: [&str; 6] = ["5", "15", "30", "60", "120", "240"];
/// The delay before a re-entry in minutes.
pub const DELAY_MINUTES: [&str; 6] = ["0", "1", "5", "10", "15", "30"];
/// The group of a rule.
pub const GROUPS: [&str; 4] = ["1", "2", "3", "4"];
/// Numbers of bars.
pub const BAR_COUNTS: [&str; 8] = ["5", "10", "14", "20", "40", "50", "100", "200"];

/// The list of a target of a short strike by its type; none for a type without a list.
#[must_use]
pub fn for_short_target(target_type: &str) -> &'static [&'static str] {
    match target_type {
        "Delta" => &DELTAS,
        "Premium" | "Short premium" => &PREMIUMS,
        _ => &[],
    }
}

/// The list of the target of a custom leg by its type.
#[must_use]
pub fn for_custom_target(target_type: &str) -> &'static [&'static str] {
    match target_type {
        "Delta" => &DELTAS,
        "Fixed premium" => &PREMIUMS,
        "Offset from price" | "Offset from open" | "Offset from leg 1" | "Offset from leg 2" | "Offset from leg 3" => &STRIKE_OFFSETS,
        _ => &[],
    }
}

// ----------------------------------------------------------------------------------------------------------------------
// The values a person added to a list.
// ----------------------------------------------------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Mutex;

/// The values added by the person, by list, loaded from `number_choices.json` on first use.
static CUSTOM: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);

/// The name a list is stored under: its own values, so every field with the same list shares what was added.
#[must_use]
pub fn list_key(list: &[&str]) -> String {
    list.join(",")
}

fn custom_path() -> Option<std::path::PathBuf> {
    crate::storage::data_file("number_choices.json")
}

/// What went wrong with `number_choices.json` and is not logged yet; the widgets that use the lists have no log of their own.
static PROBLEMS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn report(problem: String) {
    PROBLEMS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(problem);
}

/// The problems with the file of the added values since the last call, for the log.
#[must_use]
pub fn take_problems() -> Vec<String> {
    std::mem::take(&mut *PROBLEMS.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

/// The added values in the file; a missing file is an empty one, an unreadable one is reported and taken as empty.
fn read_custom() -> HashMap<String, Vec<String>> {
    let Some(path) = custom_path() else { return HashMap::new() };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return HashMap::new(),
        Err(error) => {
            report(format!("{} could not be read: {error}", path.display()));
            return HashMap::new();
        }
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        report(format!("{} could not be read: {error}", path.display()));
        HashMap::new()
    })
}

fn with_custom<R>(use_map: impl FnOnce(&mut HashMap<String, Vec<String>>) -> R) -> R {
    let mut guard = CUSTOM.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = guard.get_or_insert_with(read_custom);
    use_map(map)
}

/// The values the person added to the list `key`, smallest first.
#[must_use]
pub fn custom_values(key: &str) -> Vec<String> {
    with_custom(|map| map.get(key).cloned().unwrap_or_default())
}

/// A number as it is kept: trimmed, with a point as the decimal sign; `None` if it is not a number.
#[must_use]
pub fn clean_number(text: &str) -> Option<String> {
    let text = text.trim().replace(',', ".");
    text.parse::<f64>().ok().filter(|value| value.is_finite()).map(|_| text)
}

/// Adds `text` to the values of the list `key` (and to its file) unless it is not a number or the list has it already.
/// `list` holds the values the list offers anyway. Returns the value as it is kept.
pub fn remember(key: &str, list: &[&str], text: &str) -> Option<String> {
    let value = clean_number(text)?;
    if list.contains(&value.as_str()) {
        return Some(value);
    }
    with_custom(|map| {
        let values = map.entry(key.to_string()).or_default();
        if !values.contains(&value) {
            values.push(value.clone());
            values.sort_by(|a, b| a.parse::<f64>().unwrap_or(0.0).total_cmp(&b.parse::<f64>().unwrap_or(0.0)));
            if let (Some(path), Ok(json)) = (custom_path(), serde_json::to_string_pretty(&*map)) {
                if let Err(error) = std::fs::write(&path, json) {
                    report(format!("{} could not be saved: {error}", path.display()));
                }
            }
        }
    });
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_with_the_file_is_kept_for_the_log_and_handed_over_once() {
        report("number_choices.json could not be saved: denied".to_string());
        let problems = take_problems();
        assert!(problems.iter().any(|p| p.contains("could not be saved: denied")), "{problems:?}");
        assert!(!take_problems().iter().any(|p| p.contains("denied")), "handed over once");
    }

    #[test]
    fn every_value_of_a_list_is_a_number_and_lists_have_no_repeats() {
        let lists: [&[&str]; 8] = [&SECONDS, &WINDOW_MINUTES, &STRIKE_OFFSETS, &OFFSETS, &STOP_MULTIPLES, &PREMIUMS, &DELTAS, &BAR_COUNTS];
        for list in lists {
            assert!(list.iter().all(|value| value.parse::<f64>().is_ok()), "{list:?}");
            assert_eq!(list.iter().collect::<std::collections::HashSet<_>>().len(), list.len(), "{list:?}");
        }
    }

    #[test]
    fn a_value_the_person_adds_is_kept_in_the_list_and_only_numbers_are_added() {
        let key = "unit-test-list";
        let list = ["1", "5"];
        assert_eq!(remember(key, &list, " 7,5 ").as_deref(), Some("7.5"));
        assert_eq!(remember(key, &list, "2").as_deref(), Some("2"));
        assert_eq!(remember(key, &list, "5").as_deref(), Some("5"), "a value the list has already is not added again");
        assert_eq!(remember(key, &list, "7.5").as_deref(), Some("7.5"));
        assert_eq!(remember(key, &list, "abc"), None);
        assert_eq!(custom_values(key), ["2", "7.5"], "smallest first, no repeats");
        assert_eq!(list_key(&list), "1,5");
    }

    #[test]
    fn a_target_type_without_a_list_offers_none() {
        assert_eq!(for_short_target("Delta"), DELTAS);
        assert_eq!(for_short_target("Percent OTM"), [] as [&str; 0]);
        assert_eq!(for_custom_target("Offset from leg 2").len(), 21);
        assert_eq!(for_custom_target("Percent OTM"), [] as [&str; 0]);
    }
}
