//! The jobs of `App` that run once a day on this computer: cleaning up old log lines and files, and the daily backup of the settings.

use super::App;
use crate::storage;

/// How many days of log lines are kept when the interval in the settings is empty or unusable.
const DEFAULT_LOG_DAYS: u32 = 90;

/// The days of log lines kept for the interval typed in the settings: a whole number from 1 up, else `DEFAULT_LOG_DAYS`.
pub(super) fn log_days(text: &str) -> u32 {
    text.trim().parse::<u32>().ok().filter(|days| *days > 0).unwrap_or(DEFAULT_LOG_DAYS)
}

impl App {
    /// Runs the daily jobs once on every local day the app is running, the first time at the start.
    pub(super) fn housekeeping_tick(&mut self) {
        let today = self.clock.local().date_naive();
        if self.housekeeping_day == Some(today) {
            return;
        }
        self.housekeeping_day = Some(today);
        if cfg!(test) {
            return;
        }
        self.clean_up_logs(today);
        self.daily_backup(today);
    }

    /// Settings > Logs > "Clean up old log files": removes the log lines older than the interval, and the other files of the log folder
    /// that were last changed before it.
    fn clean_up_logs(&mut self, today: chrono::NaiveDate) {
        if !self.settings.view_flags.settings_clean_logs {
            return;
        }
        let Some(path) = self.log_path.clone() else { return };
        let days = log_days(&self.settings.settings_cleanup_interval);
        let Some(keep_from) = today.checked_sub_days(chrono::Days::new(u64::from(days))) else { return };
        match storage::prune_logs(&path, keep_from) {
            Ok((0, 0)) => {}
            Ok((lines, files)) => self.log(&format!("Log cleanup: {lines} line(s) and {files} file(s) older than {days} days removed")),
            Err(error) => self.log(&format!("WARNING: the old log lines could not be removed: {error}")),
        }
    }

    /// Settings > Database Backup > "Daily Backups Enabled": writes a backup of the settings once a day, unless this computer wrote one
    /// today already (also in an earlier run).
    fn daily_backup(&mut self, today: chrono::NaiveDate) {
        if !self.settings.misc_flags.daily_backup_enabled {
            return;
        }
        let settings = self.current_settings();
        if storage::backup_written_on(&today.format("%Y%m%d").to_string(), &settings.backup_machine_name) {
            return;
        }
        match storage::write_backup(&settings) {
            Ok(path) => self.log(&format!("Daily backup written to {}", path.display())),
            Err(error) => self.log(&format!("WARNING: the daily backup could not be written: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::log_days;

    #[test]
    fn the_log_interval_is_a_whole_number_of_days_from_one_up_else_the_default() {
        assert_eq!(log_days(" 30 "), 30);
        assert_eq!(log_days("1"), 1);
        assert_eq!(log_days("0"), 90);
        assert_eq!(log_days(""), 90);
        assert_eq!(log_days("-5"), 90);
        assert_eq!(log_days("abc"), 90);
    }
}
