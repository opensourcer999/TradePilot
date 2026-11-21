//! Export and import of schedules as a JSON file. A schedule
//! points at templates by id, and ids differ from one installation to the next, so the file also carries
//! the template names; on import the names find the templates again. Imported schedules arrive switched
//! off: a file must not start trading by itself.

use crate::schedule::{next_id, Schedule};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The text that marks the file as an export of schedules.
const FORMAT: &str = "TradePilot schedules";

/// The version of the file layout.
const VERSION: u32 = 1;

/// One schedule in the file: the schedule itself and the names of the templates it points at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedSchedule {
    /// The schedule as saved.
    pub schedule: Schedule,
    /// The name of its template (empty if it was not found).
    pub template_name: String,
    /// The name of its alternate template (empty if there is none).
    pub alternate_template_name: String,
}

/// The whole file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleExport {
    /// Always `TradePilot schedules`.
    pub format: String,
    /// The version of the layout.
    pub version: u32,
    /// The schedules.
    pub schedules: Vec<ExportedSchedule>,
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// How many schedules were added.
    pub imported: usize,
    /// How many of them have no template yet because none of this name exists here (they need one before they can be switched on).
    pub without_template: usize,
}

/// The export of `schedules`; `template_name` gives the name of a template id (`None` if it does not exist).
#[must_use]
pub fn build_export(schedules: &[Schedule], template_name: impl Fn(i32) -> Option<String>) -> ScheduleExport {
    let schedules = schedules
        .iter()
        .map(|schedule| ExportedSchedule {
            schedule: schedule.clone(),
            template_name: template_name(schedule.template_id).unwrap_or_default(),
            alternate_template_name: template_name(schedule.alternate_template_id).unwrap_or_default(),
        })
        .collect();
    ScheduleExport { format: FORMAT.to_string(), version: VERSION, schedules }
}

/// The export in a file in `dir` (made if needed); the name has `stamp` (a time of day and date) in it.
///
/// # Errors
///
/// The folder or the file cannot be written.
pub fn write_export(export: &ScheduleExport, dir: &Path, stamp: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("TradePilot-Schedules-{stamp}.json"));
    let json = serde_json::to_string_pretty(export).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(&path, json)?;
    Ok(path)
}

/// Reads an export from `text`.
///
/// # Errors
///
/// The text is no JSON, is not an export of schedules, or has a newer layout than this version knows.
pub fn parse_export(text: &str) -> Result<ScheduleExport, String> {
    let export: ScheduleExport = serde_json::from_str(text).map_err(|error| format!("this is not a schedules export ({error})"))?;
    if export.format != FORMAT {
        return Err(format!("this is not a schedules export (format \"{}\")", export.format));
    }
    if export.version > VERSION {
        return Err(format!("the export has version {}, this version of TradePilot reads up to {VERSION}", export.version));
    }
    Ok(export)
}

/// Reads an export from a file.
///
/// # Errors
///
/// As `parse_export`, or the file cannot be read.
pub fn read_export(path: &Path) -> Result<ScheduleExport, String> {
    parse_export(&fs::read_to_string(path).map_err(|error| format!("{} cannot be read ({error})", path.display()))?)
}

/// The export files in the export folder (newest first).
#[must_use]
pub fn import_candidates(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            name.starts_with("tradepilot-schedules-") && std::path::Path::new(&name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        })
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    files.into_iter().map(|(_, path)| path).collect()
}

/// Adds the schedules of `export` to `list` as new ones in `group`, all switched off and without a last
/// run, and without a condition. Templates are found again by name in `templates` (id and name); a schedule whose template is not
/// found gets template 0. A link to another schedule of the file is moved to the new id of that schedule.
pub fn import_schedules(list: &mut Vec<Schedule>, export: ScheduleExport, templates: &[(i32, String)], group: u8) -> ImportReport {
    let find = |name: &str| templates.iter().find(|(_, template)| !name.is_empty() && template == name).map_or(0, |(id, _)| *id);
    let mut report = ImportReport::default();
    let mut new_ids: Vec<(u32, u32)> = Vec::new();
    for exported in export.schedules {
        let mut schedule = exported.schedule;
        let old_id = schedule.id;
        schedule.id = next_id(list);
        schedule.group = group;
        schedule.active = false;
        schedule.last_run = None;
        // Conditions are not in the file: they have other ids here and are chosen again.
        schedule.condition_id = 0;
        schedule.condition_failure_template_id = 0;
        schedule.template_id = find(&exported.template_name);
        schedule.alternate_template_id = find(&exported.alternate_template_name);
        if schedule.template_id == 0 {
            report.without_template += 1;
        }
        new_ids.push((old_id, schedule.id));
        list.push(schedule);
        report.imported += 1;
    }
    // Links point at the old ids of the file; they go to the new ones, or are dropped if the target is not in the file.
    let imported_from = list.len().saturating_sub(report.imported);
    for schedule in list.iter_mut().skip(imported_from) {
        schedule.linked_schedule_id = new_ids.iter().find(|(old, _)| *old == schedule.linked_schedule_id && *old != 0).map_or(0, |(_, new)| *new);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::{Days, QuantityRule};

    fn schedule(id: u32, template_id: i32) -> Schedule {
        Schedule {
            id,
            name: format!("S{id}"),
            template_id,
            account: "DU1".into(),
            hour: 9,
            minute: 35,
            quantity: QuantityRule::Fixed { count: 2 },
            active: true,
            days: Days::WEEKDAYS,
            last_run: Some("2026-11-16".into()),
            ..Schedule::default()
        }
    }

    fn names(id: i32) -> Option<String> {
        match id {
            4 => Some("PCS".to_string()),
            5 => Some("IC".to_string()),
            _ => None,
        }
    }

    #[test]
    fn an_export_carries_the_template_names_and_reads_back() {
        let list = vec![Schedule { alternate_template_id: 5, linked_schedule_id: 0, ..schedule(1, 4) }, schedule(2, 99)];
        let export = build_export(&list, names);
        assert_eq!((export.schedules[0].template_name.as_str(), export.schedules[0].alternate_template_name.as_str()), ("PCS", "IC"));
        assert_eq!(export.schedules[1].template_name, "", "an unknown template has no name");
        let text = serde_json::to_string(&export).unwrap();
        assert_eq!(parse_export(&text).unwrap(), export);
    }

    #[test]
    fn a_file_that_is_no_schedules_export_or_too_new_is_refused() {
        assert!(parse_export("not json").is_err());
        assert!(parse_export("[]").is_err());
        let other = r#"{"format":"something else","version":1,"schedules":[]}"#;
        assert!(parse_export(other).unwrap_err().contains("not a schedules export"));
        let newer = format!(r#"{{"format":"{FORMAT}","version":{},"schedules":[]}}"#, VERSION + 1);
        assert!(parse_export(&newer).unwrap_err().contains("version"));
        assert!(parse_export(&format!(r#"{{"format":"{FORMAT}","version":1,"schedules":[]}}"#)).is_ok());
    }

    #[test]
    fn imported_schedules_are_new_switched_off_in_the_chosen_set_and_find_their_templates_by_name() {
        let export = build_export(
            &[
                Schedule { alternate_template_id: 5, ..schedule(7, 4) },
                Schedule { linked_schedule_id: 7, ..schedule(8, 99) },
                Schedule { linked_schedule_id: 55, ..schedule(9, 4) },
            ],
            names,
        );
        let mut list = vec![schedule(1, 4)];
        // Here the templates have other ids than where the file was made.
        let templates = vec![(40, "PCS".to_string()), (50, "IC".to_string())];
        let report = import_schedules(&mut list, export, &templates, 2);
        assert_eq!(report, ImportReport { imported: 3, without_template: 1 });
        assert_eq!(list.iter().map(|s| s.id).collect::<Vec<_>>(), [1, 2, 3, 4], "new ids, the existing schedule is untouched");
        assert!(list[0].active, "the existing schedule is untouched");
        for imported in &list[1..] {
            assert!(!imported.active && imported.last_run.is_none() && imported.group == 2, "{imported:?}");
        }
        assert_eq!((list[1].template_id, list[1].alternate_template_id), (40, 50));
        assert_eq!((list[1].condition_id, list[1].condition_failure_template_id), (0, 0), "conditions are chosen again");
        assert_eq!(list[2].template_id, 0, "no template of that name here");
        assert_eq!(list[2].linked_schedule_id, 2, "the link follows the new id");
        assert_eq!(list[3].linked_schedule_id, 0, "a link to a schedule that is not in the file is dropped");
    }

    #[test]
    fn export_files_are_written_and_found_newest_first() {
        let guard = crate::test_support::TempDir::new("schedule-export");
        let dir = guard.to_path_buf();
        let export = build_export(&[schedule(1, 4)], names);
        let first = write_export(&export, &dir, "a").unwrap();
        assert_eq!(read_export(&first).unwrap(), export);
        fs::write(dir.join("notes.txt"), "x").unwrap();
        fs::write(dir.join("TradePilot-Templates-1.tpt"), "x").unwrap();
        assert_eq!(import_candidates(&dir), [first]);
        assert!(read_export(&dir.join("missing.json")).unwrap_err().contains("cannot be read"));
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(import_candidates(&dir), Vec::<PathBuf>::new());
    }
}
