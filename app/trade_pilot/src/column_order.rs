//! The arrangement of the tables whose columns can be moved by drag and drop and sized with the mouse: per table the order of
//! its columns and the widths the person gave them, remembered in `table_columns.json`. A column is kept by its key (not its
//! title, which changes with the language), so a column added in a later version simply comes at the end with its own width and
//! a column that is gone is dropped.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// The narrowest a column can be made, as a share of the table (the weights of a table add up to about 10 to 15).
pub const MIN_WEIGHT: f32 = 0.3;

/// How one table is arranged.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TableLayout {
    /// The column keys in the order the person arranged them.
    #[serde(default)]
    pub order: Vec<String>,
    /// The widths the person gave columns, by key, as weights like those of `TableColumn`.
    #[serde(default)]
    pub widths: HashMap<String, f32>,
}

/// The file as it is read: the layout of each table, or only its order as the first version of the file kept it.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredLayout {
    Layout(TableLayout),
    Order(Vec<String>),
}

/// The layouts of the tables, loaded from the file on first use.
static LAYOUTS: Mutex<Option<HashMap<String, TableLayout>>> = Mutex::new(None);

/// What went wrong with the file and is not logged yet.
static PROBLEMS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn path() -> Option<std::path::PathBuf> {
    crate::storage::data_file("table_columns.json")
}

fn report(problem: String) {
    PROBLEMS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(problem);
}

/// The problems with the file since the last call, for the log.
#[must_use]
pub fn take_problems() -> Vec<String> {
    std::mem::take(&mut *PROBLEMS.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

/// The layouts in `text`, a file of either form.
fn parse(text: &str) -> Result<HashMap<String, TableLayout>, serde_json::Error> {
    let stored: HashMap<String, StoredLayout> = serde_json::from_str(text)?;
    Ok(stored
        .into_iter()
        .map(|(table, layout)| {
            let layout = match layout {
                StoredLayout::Layout(layout) => layout,
                StoredLayout::Order(order) => TableLayout { order, widths: HashMap::new() },
            };
            (table, layout)
        })
        .collect())
}

/// The layouts in the file; a missing file is an empty one, an unreadable one is reported and taken as empty.
fn read() -> HashMap<String, TableLayout> {
    let Some(path) = path() else { return HashMap::new() };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).unwrap_or_else(|error| {
            report(format!("{} could not be read: {error}", path.display()));
            HashMap::new()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(error) => {
            report(format!("{} could not be read: {error}", path.display()));
            HashMap::new()
        }
    }
}

fn with_layouts<R>(use_map: impl FnOnce(&mut HashMap<String, TableLayout>) -> R) -> R {
    let mut guard = LAYOUTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    use_map(guard.get_or_insert_with(read))
}

/// Writes all layouts to the file.
fn save(map: &HashMap<String, TableLayout>) {
    if let (Some(path), Ok(json)) = (path(), serde_json::to_string_pretty(map)) {
        if let Err(error) = std::fs::write(&path, json) {
            report(format!("{} could not be saved: {error}", path.display()));
        }
    }
}

/// The columns of a table with the `keys` in the order of `stored`, as indexes into `keys`: the keys stored first, then those
/// not stored yet in their own order; a stored key the table no longer has is left out.
#[must_use]
pub fn arranged(stored: &[String], keys: &[&str]) -> Vec<usize> {
    let mut order: Vec<usize> = stored.iter().filter_map(|key| keys.iter().position(|k| k == key)).collect();
    order.dedup();
    for index in 0..keys.len() {
        if !order.contains(&index) {
            order.push(index);
        }
    }
    order
}

/// `order` with the column at place `from` moved to place `to` (the others close up); unchanged for a place out of range.
#[must_use]
pub fn moved(order: &[usize], from: usize, to: usize) -> Vec<usize> {
    let mut order = order.to_vec();
    if from < order.len() && to < order.len() && from != to {
        let column = order.remove(from);
        order.insert(to, column);
    }
    order
}

/// The widths of two neighbouring columns after the border between them was dragged by `delta` (in weights, positive to the
/// right): the one grows by what the other gives up, neither below `MIN_WEIGHT`, so the table keeps its width.
#[must_use]
pub fn resized(left: f32, right: f32, delta: f32) -> (f32, f32) {
    let (lowest, highest) = (MIN_WEIGHT - left, right - MIN_WEIGHT);
    // Two columns that are both too narrow already leave nothing to move (and `clamp` needs a range that is not empty).
    if !delta.is_finite() || lowest > highest {
        return (left, right);
    }
    let delta = delta.clamp(lowest, highest);
    (left + delta, right - delta)
}

/// The order of the columns of `table` (indexes into its `keys`) and the weight of each column (by index into `keys`): the one
/// the person set, else its own from `defaults`.
#[must_use]
pub fn layout(table: &str, keys: &[&str], defaults: &[f32]) -> (Vec<usize>, Vec<f32>) {
    with_layouts(|map| {
        let stored = map.get(table);
        let order = arranged(stored.map(|l| l.order.as_slice()).unwrap_or_default(), keys);
        let widths = keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                let own = defaults.get(i).copied().unwrap_or(1.0);
                stored.and_then(|l| l.widths.get(*key)).copied().filter(|w| w.is_finite() && *w >= MIN_WEIGHT).unwrap_or(own)
            })
            .collect();
        (order, widths)
    })
}

/// Moves the column at place `from` of `table` to place `to` and keeps the new order in the file.
pub fn move_column(table: &str, keys: &[&str], from: usize, to: usize) {
    with_layouts(|map| {
        let entry = map.entry(table.to_string()).or_default();
        let order = moved(&arranged(&entry.order, keys), from, to);
        entry.order = order.iter().filter_map(|index| keys.get(*index)).map(|key| (*key).to_string()).collect();
        save(map);
    });
}

/// Sets the widths of the columns `widths` (key, weight) of `table`; written to the file only when `keep` (once the mouse is let
/// go), so a drag does not write the file at every frame.
pub fn set_widths(table: &str, widths: &[(&str, f32)], keep: bool) {
    with_layouts(|map| {
        let entry = map.entry(table.to_string()).or_default();
        for (key, weight) in widths {
            entry.widths.insert((*key).to_string(), weight.max(MIN_WEIGHT));
        }
        if keep {
            save(map);
        }
    });
}

/// Forgets the widths the person gave the columns of `table`: its own widths again (the order stays).
pub fn reset_widths(table: &str) {
    with_layouts(|map| {
        if let Some(layout) = map.get_mut(table).filter(|layout| !layout.widths.is_empty()) {
            layout.widths.clear();
            save(map);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: [&str; 4] = ["account", "symbol", "value", "profit"];

    #[test]
    fn a_stored_order_is_applied_and_new_or_gone_columns_do_not_break_it() {
        assert_eq!(arranged(&[], &KEYS), [0, 1, 2, 3], "nothing stored: the table's own order");
        let stored: Vec<String> = ["profit", "account"].map(String::from).to_vec();
        assert_eq!(arranged(&stored, &KEYS), [3, 0, 1, 2], "the stored ones first, the others after them");
        let old: Vec<String> = ["gone", "value", "value"].map(String::from).to_vec();
        assert_eq!(arranged(&old, &KEYS), [2, 0, 1, 3], "an unknown key is dropped, a repeat counts once");
    }

    #[test]
    fn a_column_moved_by_drag_and_drop_lands_on_the_place_it_was_dropped_on() {
        assert_eq!(moved(&[0, 1, 2, 3], 3, 0), [3, 0, 1, 2]);
        assert_eq!(moved(&[0, 1, 2, 3], 0, 2), [1, 2, 0, 3]);
        assert_eq!(moved(&[0, 1, 2, 3], 1, 1), [0, 1, 2, 3]);
        assert_eq!(moved(&[0, 1, 2, 3], 7, 0), [0, 1, 2, 3], "out of range: unchanged");
    }

    #[test]
    fn dragging_a_border_gives_one_column_what_the_other_gives_up_but_never_below_the_narrowest() {
        assert_eq!(resized(1.0, 2.0, 0.5), (1.5, 1.5));
        assert_eq!(resized(1.0, 2.0, -0.5), (0.5, 2.5));
        let (left, right) = resized(1.0, 2.0, -5.0);
        assert!((left - MIN_WEIGHT).abs() < 1e-6 && (left + right - 3.0).abs() < 1e-6, "the left one stops at the narrowest: {left} {right}");
        let (left, right) = resized(1.0, 2.0, 5.0);
        assert!((right - MIN_WEIGHT).abs() < 1e-6 && (left + right - 3.0).abs() < 1e-6, "the right one stops at the narrowest: {left} {right}");
        assert_eq!(resized(0.3, 0.3, 1.0), (0.3, 0.3), "two columns at their narrowest stay as they are");
        assert_eq!(resized(0.2, 0.2, 1.0), (0.2, 0.2), "two columns narrower than that (their own widths) are not touched");
    }

    #[test]
    fn the_order_and_the_widths_of_a_table_are_kept_and_the_widths_can_be_reset() {
        // A table name of its own: the remembered layouts are shared by the whole test run.
        let table = "test_table_layout";
        let defaults = [1.0, 1.0, 2.0, 2.0];
        assert_eq!(layout(table, &KEYS, &defaults), (vec![0, 1, 2, 3], defaults.to_vec()));
        move_column(table, &KEYS, 3, 0);
        set_widths(table, &[("value", 3.0), ("profit", 1.0)], true);
        assert_eq!(layout(table, &KEYS, &defaults), (vec![3, 0, 1, 2], vec![1.0, 1.0, 3.0, 1.0]));
        reset_widths(table);
        assert_eq!(layout(table, &KEYS, &defaults), (vec![3, 0, 1, 2], defaults.to_vec()), "the order stays, the widths are its own again");
    }

    #[test]
    fn the_file_of_the_first_version_with_only_the_order_is_still_read() {
        let old = parse(r#"{"live_positions": ["profit", "account"]}"#).unwrap();
        assert_eq!(old.get("live_positions").map(|l| l.order.clone()), Some(vec!["profit".to_string(), "account".to_string()]));
        let new = parse(r#"{"trades": {"order": ["a"], "widths": {"a": 2.5}}}"#).unwrap();
        assert_eq!(new.get("trades").and_then(|l| l.widths.get("a")).copied(), Some(2.5));
    }
}
