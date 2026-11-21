//! Trade templates: data model, persistence and export/import (`.tpt` files). Field names are the
//! template file format's property names (`PascalCase`, as the JSON stores them), so exported
//! files import unchanged and vice versa.

use crate::storage::{app_data_dir, mode_dir};
use serde_json::Value;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
pub use strategy::templates::*;

/// An unset value (e.g. a string) may be JSON `null`; dropping those keys lets
/// `#[serde(default)]` fill in the default instead of failing on `null` for a `String`.
fn strip_nulls(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            map.values_mut().for_each(strip_nulls);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_nulls),
        _ => {}
    }
}

fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> io::Result<T> {
    let mut value: Value = serde_json::from_str(text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    strip_nulls(&mut value);
    crate::legacy_names::upgrade_templates(&mut value);
    serde_json::from_value(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// The templates of the Live or the Paper Trading mode (see `storage::mode_dir`).
pub fn templates_path(live: bool) -> io::Result<PathBuf> {
    Ok(mode_dir(live)?.join("templates.json"))
}

/// What reading the templates file found.
#[derive(Debug, Default, PartialEq)]
pub struct LoadedTemplates {
    /// Every saved template, including soft-deleted ones (they are kept so re-entry links to them
    /// stay valid). Empty for a missing file, and for one that could not be read.
    pub templates: Vec<TradeTemplate>,
    /// Why the file's templates are not loaded, for the log and the Templates page; `None` when
    /// the file was read (or there is none yet).
    pub problem: Option<String>,
    /// The file is still in place but could not be read: saving must not overwrite it, or the
    /// templates in it would be lost.
    pub save_blocked: bool,
}

/// Every saved template of the Live or the Paper Trading mode (see `load_templates_from`). Not in
/// tests: they must not touch the real data folder.
pub fn load_templates(live: bool, set_aside_suffix: &str) -> LoadedTemplates {
    if cfg!(test) {
        return LoadedTemplates::default();
    }
    match templates_path(live) {
        Ok(path) => load_templates_from(&path, set_aside_suffix),
        Err(error) => LoadedTemplates {
            problem: Some(format!("the templates folder cannot be found ({error}) - no templates loaded, saving is blocked")),
            save_blocked: true,
            ..LoadedTemplates::default()
        },
    }
}

/// The templates in the file at `path`. A missing file is no problem (no templates yet). A file
/// that is not valid JSON of templates is never silently replaced by an empty list that the next
/// save would write over it: it is set aside as `<file>.unreadable-<set_aside_suffix>` (e.g. a
/// time stamp), so its content is kept, and the app starts without templates; if it cannot be
/// set aside, saving is blocked instead. A file that cannot be read at all blocks saving too.
pub fn load_templates_from(path: &Path, set_aside_suffix: &str) -> LoadedTemplates {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return LoadedTemplates::default(),
        Err(error) => {
            return LoadedTemplates {
                problem: Some(format!("{} cannot be read ({error}) - no templates loaded, saving is blocked so the file is not overwritten", path.display())),
                save_blocked: true,
                ..LoadedTemplates::default()
            }
        }
    };
    let error = match from_json::<Vec<TradeTemplate>>(&text) {
        Ok(templates) => return LoadedTemplates { templates, ..LoadedTemplates::default() },
        Err(error) => error,
    };
    let mut aside = path.as_os_str().to_owned();
    aside.push(format!(".unreadable-{set_aside_suffix}"));
    let aside = PathBuf::from(aside);
    match fs::rename(path, &aside) {
        Ok(()) => LoadedTemplates {
            problem: Some(format!("{} is not a valid templates file ({error}) - it was set aside as {} and TradePilot starts without templates", path.display(), aside.display())),
            ..LoadedTemplates::default()
        },
        Err(rename_error) => LoadedTemplates {
            problem: Some(format!(
                "{} is not a valid templates file ({error}) and could not be set aside ({rename_error}) - no templates loaded, saving is blocked so the file is not overwritten",
                path.display()
            )),
            save_blocked: true,
            ..LoadedTemplates::default()
        },
    }
}

pub fn write_templates(live: bool, templates: &[TradeTemplate]) -> io::Result<()> {
    write_templates_to(&templates_path(live)?, templates)
}

/// Writes `templates` to `path` as a whole: first to `<file>.tmp`, then renamed over the file, so
/// an interrupted write (a crash, a full disk) never leaves a half-written templates file behind.
pub fn write_templates_to(path: &Path, templates: &[TradeTemplate]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(templates).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, json)?;
    fs::rename(&temporary, path)
}

/// Where imports are looked for and the export dialog starts (the user picks the export file;
/// one fixed, discoverable folder).
pub fn exports_dir() -> io::Result<PathBuf> {
    match std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        Some(home) => Ok(PathBuf::from(home).join("Documents").join("TradePilot")),
        None => Ok(app_data_dir()?.join("exports")),
    }
}

/// The suggested file name in the save dialog.
pub fn default_export_file_name() -> String {
    format!("TradePilot-Templates-{}.tpt", chrono::Local::now().format("%m%d%Y-%H%M"))
}

/// Writes `export` to the file the user picked (`.tpt` is appended when the name has no extension).
pub fn write_export_to(export: &ConfigExport, path: &Path) -> io::Result<PathBuf> {
    let mut path = path.to_path_buf();
    if path.extension().is_none() {
        path.set_extension("tpt");
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(export).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(&path, json)?;
    Ok(path)
}

/// `.tpt` files in the export folder and in Downloads, newest first.
pub fn import_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(dir) = exports_dir() {
        dirs.push(dir);
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        dirs.push(PathBuf::from(home).join("Downloads"));
    }
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = dirs
        .iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("tpt")))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    files.into_iter().map(|(_, path)| path).collect()
}

pub fn read_export(path: &Path) -> io::Result<ConfigExport> {
    from_json(&fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The switches are grouped in the program, but a template keeps the flat keys of the file (`.tpt` import and export, and
    /// `templates.json`).
    #[test]
    fn a_template_keeps_its_flat_keys_with_grouped_switches() {
        let mut template = TradeTemplate::default();
        template.stop_flags.stop_rel = true;
        template.list_flags.is_deleted = true;
        template.leg_dte_b.long_put_dte_adjust = true;
        let json = serde_json::to_string(&template).unwrap();
        for key in ["\"stop_rel\":true", "\"is_deleted\":true", "\"long_put_dte_adjust\":true", "\"all_or_none\":false", "\"reentry_close\":false"] {
            assert!(json.contains(key), "{key} missing in {json}");
        }
        assert!(!json.contains("stop_flags") && !json.contains("list_flags") && !json.contains("leg_dte"), "{json}");
        // A file without any switch (an older or foreign file) reads with all of them off.
        let bare: TradeTemplate = serde_json::from_str(r#"{"name":"x"}"#).unwrap();
        assert!(!bare.stop_flags.stop_rel && !bare.exit_flags.allow_strike_conflict);
        let back: TradeTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back, template);
    }

    /// A file that an earlier version wrote (other key names) is read with its values and written back with the current names.
    #[test]
    fn old_template_files_are_upgraded() {
        let old = r#"[{"TradeTemplateID":3,"Name":"Old","StopMultiple":2.5,"CLeg1Action":"Sell","ReEnterStop":true,"LowerTarget":true}]"#;
        let templates: Vec<TradeTemplate> = from_json(old).unwrap();
        let [template] = templates.as_slice() else { panic!("one template expected") };
        assert_eq!((template.id, template.name.as_str(), template.stop_multiple), (3, "Old", 2.5));
        assert_eq!(template.custom_leg1_action, "Sell");
        assert!(template.reenter.re_enter_stop && template.list_flags.lower_target);
        let written = serde_json::to_string(&templates).unwrap();
        assert!(written.contains("\"stop_multiple\":2.5") && !written.contains("StopMultiple"), "{written}");
        let export = from_json::<ConfigExport>(r#"{"Templates":[{"TradeTemplateID":4,"Name":"X"}],"MarketCloseHour":16}"#).unwrap();
        assert_eq!((export.templates.len(), export.market_close_hour), (1, 16));
    }

    /// Shape of a real `.tpt` file (`PascalCase` properties,
    /// `null` for unset strings/nullables, dictionaries as `null` when unused).
    const SAMPLE_EXPORT: &str = r#"{"templates":[
        {"id":7,"name":"IWM ShortStrangle","trade_type":"ShortStrangle","target_type":"Delta",
         "target_type_call":"Delta","target_min":10.0,"target_min_call":10.0,"target_max":15.0,"target_max_call":15.0,
         "long_type":null,"long_type_call":null,"long_width":null,"long_max_premium":null,"quantity_default":1,"fill_attempts":3,
         "fill_wait":7,"fill_adjustment":0.05,"stop_type":"Short","stop_multiple":2.0,"stop_trigger":4,"stop_order_type":"Market stop",
         "profit_target_type":"Percentage","profit_target":50.0,"exit_hour":15,"exit_minute":0,"reentry_stop":true,
         "reentry_stop_template_id":9,"underlying_symbol":"IWM","strategy":"Strangles","exit_condition_id":3,
         "profit_target_expiration_hour":15,"upgrade_flag":6.0,"stop_guardrail_offset_open":10,"stop_guardrail_offset_exp":10},
        {"id":9,"name":"IWM Re-Entry","trade_type":"ShortPut","underlying_symbol":"IWM","upgrade_flag":6.0}
      ],"conditions":[],"condition_details":[],"schedules":[],"condition_id_map":null,"template_id_map":null,
      "schedule_master_id_map":null,"market_close_hour":16}"#;

    /// A fresh, empty folder of its own for one test.
    fn temp_folder(name: &str) -> (crate::test_support::TempDir, PathBuf) {
        let guard = crate::test_support::TempDir::new(&format!("templates-{name}"));
        fs::create_dir_all(&guard).unwrap();
        let dir = guard.to_path_buf();
        (guard, dir)
    }

    fn named(id: i32, name: &str) -> TradeTemplate {
        TradeTemplate { id, name: name.into(), trade_type: "PutSpread".into(), ..TradeTemplate::default() }
    }

    #[test]
    fn templates_are_written_as_a_whole_and_read_back() {
        let (_guard, dir) = temp_folder("roundtrip");
        let path = dir.join("templates.json");
        // No file yet: no templates, and nothing wrong.
        assert_eq!(load_templates_from(&path, "x"), LoadedTemplates::default());
        let templates = vec![named(1, "PCS"), named(2, "IC")];
        write_templates_to(&path, &templates).unwrap();
        assert_eq!(load_templates_from(&path, "x"), LoadedTemplates { templates: templates.clone(), ..LoadedTemplates::default() });
        assert!(!dir.join("templates.json.tmp").exists(), "the temporary file is renamed over the real one");
        // Writing again replaces the file.
        write_templates_to(&path, &templates[..1]).unwrap();
        assert_eq!(load_templates_from(&path, "x").templates, templates[..1]);
    }

    #[test]
    fn an_unreadable_templates_file_is_set_aside_instead_of_being_overwritten() {
        let (_guard, dir) = temp_folder("broken");
        let path = dir.join("templates.json");
        fs::write(&path, "[{\"id\": 1, \"name\": \"PCS\"").unwrap();
        let loaded = load_templates_from(&path, "20261114-230000");
        assert!(loaded.templates.is_empty() && !loaded.save_blocked);
        let problem = loaded.problem.unwrap();
        assert!(problem.contains("not a valid templates file") && problem.contains("set aside"), "{problem}");
        // The content is kept under the new name; the next save writes a fresh file next to it.
        let aside = dir.join("templates.json.unreadable-20261114-230000");
        assert_eq!(fs::read_to_string(&aside).unwrap(), "[{\"id\": 1, \"name\": \"PCS\"");
        assert!(!path.exists());
        write_templates_to(&path, &[named(3, "new")]).unwrap();
        assert_eq!(fs::read_to_string(&aside).unwrap(), "[{\"id\": 1, \"name\": \"PCS\"", "the set-aside file is untouched");
    }

    #[test]
    fn a_templates_file_that_can_neither_be_read_nor_set_aside_blocks_saving() {
        let (_guard, dir) = temp_folder("blocked");
        let path = dir.join("templates.json");
        fs::write(&path, "not json").unwrap();
        // The name it would be set aside under is taken by a folder: the rename fails.
        fs::create_dir_all(dir.join("templates.json.unreadable-x")).unwrap();
        let loaded = load_templates_from(&path, "x");
        assert!(loaded.save_blocked, "{loaded:?}");
        assert!(loaded.problem.is_some_and(|p| p.contains("saving is blocked")));
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json", "the file stays as it was");
        // A templates "file" that is a folder cannot be read at all: saving is blocked as well.
        let folder = dir.join("folder.json");
        fs::create_dir_all(&folder).unwrap();
        assert!(load_templates_from(&folder, "x").save_blocked);
    }

    #[test]
    fn a_tpt_export_parses_including_its_nulls() {
        let export: ConfigExport = from_json(SAMPLE_EXPORT).unwrap();
        assert_eq!(export.templates.len(), 2);
        let t = &export.templates[0];
        assert_eq!((t.name.as_str(), t.trade_type.as_str(), t.underlying_symbol.as_str()), ("IWM ShortStrangle", "ShortStrangle", "IWM"));
        assert_eq!(t.long_width, "", "a null string becomes the default (empty)");
        assert_eq!(t.long_max_premium, None);
        assert_eq!((t.fill_attempts, t.fill_wait, t.fill_adjustment), (3, 7, 0.05));
        assert_eq!(export.market_close_hour, 16);
    }

    #[test]
    fn import_assigns_new_ids_relinks_re_entries_and_shifts_hours_to_this_market_close() {
        let mut all = vec![TradeTemplate { id: 9, name: "Already here".into(), ..TradeTemplate::default() }];
        let export: ConfigExport = from_json(SAMPLE_EXPORT).unwrap();
        let report = import_templates(&mut all, export, 22);
        assert_eq!(report, ImportReport { imported: 2, dropped_exit_conditions: 1, skipped_duplicates: 0, renamed: Vec::new() });
        let strangle = all.iter().find(|t| t.name == "IWM ShortStrangle").unwrap();
        let re_entry = all.iter().find(|t| t.name == "IWM Re-Entry").unwrap();
        assert_eq!(strangle.id, 10);
        assert_eq!(re_entry.id, 11);
        assert_eq!(strangle.reentry_stop_template_id, 11, "re-entry link follows the re-entry template's new id");
        assert_eq!((strangle.exit_hour, strangle.profit_target_expiration_hour), (21, 21), "16:00 close -> 22:00 close");
        assert_eq!(strangle.exit_condition_id, 0);
        assert_eq!(all.iter().find(|t| t.id == 9).unwrap().name, "Already here", "existing template untouched");
    }

    #[test]
    fn importing_the_same_export_twice_adds_nothing_the_second_time() {
        let mut all = Vec::new();
        import_templates(&mut all, from_json(SAMPLE_EXPORT).unwrap(), 16);
        let before = all.clone();
        let report = import_templates(&mut all, from_json(SAMPLE_EXPORT).unwrap(), 16);
        assert_eq!((report.imported, report.skipped_duplicates), (0, 2));
        assert_eq!(all, before);
    }

    #[test]
    fn a_taken_name_with_different_settings_gets_a_numbered_suffix() {
        let mut all = Vec::new();
        import_templates(&mut all, from_json(SAMPLE_EXPORT).unwrap(), 16);
        let mut changed: ConfigExport = from_json(SAMPLE_EXPORT).unwrap();
        changed.templates[0].quantity_default += 1;
        let name = changed.templates[0].name.clone();
        let report = import_templates(&mut all, changed, 16);
        assert_eq!(report.renamed, [format!("{name} (2)")]);
        assert!(all.iter().any(|t| t.name == format!("{name} (2)")));
    }

    #[test]
    fn export_includes_re_entry_templates_once_and_reads_back() {
        let all = vec![
            TradeTemplate { id: 1, name: "A".into(), reentry_stop_template_id: 2, reentry_close_template_id: 2, ..TradeTemplate::default() },
            TradeTemplate { id: 2, name: "B".into(), reentry_stop_template_id: 1, ..TradeTemplate::default() },
            TradeTemplate { id: 3, name: "C".into(), ..TradeTemplate::default() },
        ];
        let export = build_export(&all, &[1], 16);
        let names: Vec<&str> = export.templates.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
        let dir = crate::test_support::TempDir::new("templates-export");
        let path = write_export_to(&export, &dir.join(default_export_file_name())).unwrap();
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("TradePilot-Templates-"));
        let back = read_export(&path).unwrap();
        assert_eq!(back.templates, export.templates);
    }

    #[test]
    fn upgrade_renames_legacy_values_and_fills_call_side_defaults() {
        let mut all = vec![TradeTemplate {
            target_type: "Credit".into(),
            target_type_call: "PointsFromOpen".into(),
            preference: "Highest Credit/Delta".into(),
            long_type: "Width".into(),
            long_width: "45,50".into(),
            long_max_premium: Some(0.1),
            ..TradeTemplate::default()
        }];
        upgrade_templates(&mut all);
        let t = &all[0];
        assert_eq!((t.target_type.as_str(), t.target_type_call.as_str()), ("Premium", "Offset from open"));
        assert_eq!(t.preference, "Highest premium");
        assert_eq!((t.long_type_call.as_str(), t.long_width_call.as_str(), t.long_max_premium_call), ("Width", "45,50", Some(0.1)));
        assert_eq!((t.stop_guardrail_offset_open, t.stop_guardrail_offset_exp, t.upgrade_flag), (10, 10, 6.0));
        assert_eq!(t.underlying_symbol, "SPX");
    }

    #[test]
    fn upsert_gives_new_templates_the_next_id_and_replaces_edited_ones() {
        let mut all = vec![TradeTemplate { id: 4, name: "Old".into(), ..TradeTemplate::default() }];
        let id = upsert_template(&mut all, TradeTemplate { name: "New".into(), ..TradeTemplate::default() });
        assert_eq!(id, 5);
        upsert_template(&mut all, TradeTemplate { id: 4, name: "Renamed".into(), ..TradeTemplate::default() });
        let names: Vec<&str> = visible_templates(&all).iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["New", "Renamed"]);
    }
}
