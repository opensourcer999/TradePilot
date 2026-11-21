//! Lists of records kept in a JSON file (the schedules and their daily runs): read so that a file
//! that cannot be read is never replaced by an empty list, and written as a whole.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What reading a list file found.
#[derive(Debug, PartialEq)]
pub struct Loaded<T> {
    /// Every record; empty for a missing file and for one that could not be read.
    pub items: Vec<T>,
    /// Why the records are not loaded, for the log and the page; `None` when the file was read (or
    /// there is none yet).
    pub problem: Option<String>,
    /// The file is still in place but could not be read: saving must not overwrite it.
    pub save_blocked: bool,
}

impl<T> Default for Loaded<T> {
    fn default() -> Self {
        Self { items: Vec::new(), problem: None, save_blocked: false }
    }
}

/// The records in the file at `path` (`what` names them in the messages, e.g. "schedules"). A
/// missing file is no problem. A file that is not valid JSON of records is set aside as
/// `<file>.unreadable-<set_aside_suffix>` (its content is kept) and the app starts without
/// records; if that is not possible, or the file cannot be read at all, saving is blocked so the
/// file is never overwritten.
#[must_use]
pub fn load_from<T: DeserializeOwned>(path: &Path, set_aside_suffix: &str, what: &str) -> Loaded<T> {
    load_upgraded(path, set_aside_suffix, what, |_| {})
}

/// `load_from` for a file that earlier versions wrote differently: `upgrade` brings its JSON to the current form before the records are read.
#[must_use]
pub fn load_upgraded<T: DeserializeOwned>(path: &Path, set_aside_suffix: &str, what: &str, upgrade: fn(&mut serde_json::Value)) -> Loaded<T> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Loaded::default(),
        Err(error) => {
            return Loaded {
                problem: Some(format!("{} cannot be read ({error}) - no {what} loaded, saving is blocked so the file is not overwritten", path.display())),
                save_blocked: true,
                ..Loaded::default()
            }
        }
    };
    let parsed = serde_json::from_str::<serde_json::Value>(&text).map(|mut value| {
        upgrade(&mut value);
        value
    });
    let error = match parsed.and_then(serde_json::from_value::<Vec<T>>) {
        Ok(items) => return Loaded { items, ..Loaded::default() },
        Err(error) => error,
    };
    let mut aside = path.as_os_str().to_owned();
    aside.push(format!(".unreadable-{set_aside_suffix}"));
    let aside = PathBuf::from(aside);
    match fs::rename(path, &aside) {
        Ok(()) => Loaded {
            problem: Some(format!("{} is not a valid {what} file ({error}) - it was set aside as {} and TradePilot starts without {what}", path.display(), aside.display())),
            ..Loaded::default()
        },
        Err(rename_error) => Loaded {
            problem: Some(format!(
                "{} is not a valid {what} file ({error}) and could not be set aside ({rename_error}) - no {what} loaded, saving is blocked so the file is not overwritten",
                path.display()
            )),
            save_blocked: true,
            ..Loaded::default()
        },
    }
}

/// Writes `items` to `path` as a whole: first to `<file>.tmp`, then renamed over the file, so an
/// interrupted write never leaves a half-written file behind.
pub fn write_to<T: Serialize>(path: &Path, items: &[T]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(items).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, json)?;
    fs::rename(&temporary, path)
}
