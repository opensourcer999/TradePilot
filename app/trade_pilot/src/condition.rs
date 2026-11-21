//! The conditions of the app (`strategy::condition`) and where they are stored.

use crate::list_file::Loaded;
use crate::storage::mode_dir;
use std::io;
use std::path::PathBuf;

pub use strategy::condition::*;

/// The conditions of the Live or the Paper Trading mode (see `storage::mode_dir`).
///
/// # Errors
///
/// The folder of the mode cannot be found.
pub fn conditions_path(live: bool) -> io::Result<PathBuf> {
    Ok(mode_dir(live)?.join("conditions.json"))
}

/// Every saved condition of the mode. Not in tests: they must not touch the real data folder.
#[must_use]
pub fn load_conditions(live: bool, set_aside_suffix: &str) -> Loaded<Condition> {
    if cfg!(test) {
        return Loaded::default();
    }
    match conditions_path(live) {
        Ok(path) => crate::list_file::load_upgraded(&path, set_aside_suffix, "conditions", crate::legacy_names::upgrade_conditions),
        Err(error) => Loaded {
            problem: Some(format!("the conditions folder cannot be found ({error}) - no conditions loaded, saving is blocked")),
            save_blocked: true,
            ..Loaded::default()
        },
    }
}
