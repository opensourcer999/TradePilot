use crate::database;
use crate::i18n::Lang;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
pub use strategy::records::{
    CustomLegStored, ExitLevels, RelSettings, StopAdjustment, StopAdjustments, StoredExecution, StoredLeg, StoredTrade, SymbolConfig, TimedExit, TradeOrigin,
};

/// The file in the default data folder that names the folder the user chose for the database, the
/// logs and everything else (`app_data_dir`). It is read once at start-up.
const LOCATION_FILE: &str = "storage_location.txt";

/// Where the data lives in this run and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataLocation {
    /// The folder in use.
    pub dir: PathBuf,
    /// The default folder of the operating system.
    pub default_dir: PathBuf,
    /// The chosen folder that could not be used, and why - the default folder is used instead.
    pub fallback: Option<String>,
}

/// The folder in use: the one the user chose (Settings > storage location) if it can hold the data,
/// else the default. Decided once per run, so a change takes effect at the next start.
pub fn data_location() -> io::Result<DataLocation> {
    static LOCATION: std::sync::OnceLock<Result<DataLocation, String>> = std::sync::OnceLock::new();
    LOCATION.get_or_init(|| default_data_dir().map(resolve_location).map_err(|e| e.to_string())).clone().map_err(io::Error::other)
}

/// Where the data lives, given the default folder: the folder the location file there names if it
/// can hold the data, else the default folder (and why).
fn resolve_location(default_dir: PathBuf) -> DataLocation {
    let Some(chosen) = chosen_data_dir(&default_dir) else {
        return DataLocation { dir: default_dir.clone(), default_dir, fallback: None };
    };
    match check_data_dir(&chosen) {
        Ok(()) => DataLocation { dir: chosen, default_dir, fallback: None },
        Err(error) => {
            let reason = format!("the chosen storage folder {} cannot be used ({error}) - the default folder is used", chosen.display());
            DataLocation { dir: default_dir.clone(), default_dir, fallback: Some(reason) }
        }
    }
}

/// The folder named in the location file next to the default data folder, if there is one.
fn chosen_data_dir(default_dir: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(default_dir.join(LOCATION_FILE)).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

/// Checks that `dir` can hold the data: it exists or can be created, and a file can be written
/// into it.
pub fn check_data_dir(dir: &Path) -> io::Result<()> {
    if !dir.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the folder must be a full path"));
    }
    fs::create_dir_all(dir)?;
    let probe = dir.join(".tradepilot-write-test");
    fs::write(&probe, b"ok")?;
    fs::remove_file(&probe)
}

/// Remembers `dir` as the storage folder for the next start (`None` = the default folder again).
/// The location file always stays in the default folder.
pub fn choose_data_dir(dir: Option<&Path>) -> io::Result<()> {
    choose_data_dir_in(&default_data_dir()?, dir)
}

/// `choose_data_dir` for the default folder `default_dir`.
fn choose_data_dir_in(default_dir: &Path, dir: Option<&Path>) -> io::Result<()> {
    fs::create_dir_all(default_dir)?;
    let file = default_dir.join(LOCATION_FILE);
    match dir {
        Some(dir) if dir != default_dir => {
            check_data_dir(dir)?;
            fs::write(file, dir.display().to_string())
        }
        _ => match fs::remove_file(file) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

/// Copies the files of the data folder `from` into `to`, subfolders included. A file that
/// already exists in `to` is kept, the location file and `to` itself are left out. Returns how
/// many files were copied.
pub fn copy_data(from: &Path, to: &Path) -> io::Result<usize> {
    fs::create_dir_all(to)?;
    let mut copied = 0;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path == to || entry.file_name() == LOCATION_FILE {
            continue;
        }
        if entry.file_type()?.is_dir() {
            copied += copy_data(&path, &target)?;
        } else if !target.exists() {
            fs::copy(&path, &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Opens the system's folder dialog (Windows only) on a thread of its own and returns the channel
/// that delivers the chosen folder, or `None` when the dialog was cancelled or is not available.
/// `start` is the folder the dialog opens in.
pub fn pick_folder(start: &str) -> std::sync::mpsc::Receiver<Option<String>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let start = start.to_string();
    std::thread::spawn(move || {
        let _ = sender.send(run_folder_dialog(&start));
    });
    receiver
}

#[cfg(target_os = "windows")]
fn run_folder_dialog(start: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // The start folder travels in an environment variable, never inside the script text.
    let script = "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
        Add-Type -AssemblyName System.Windows.Forms; \
        $d = New-Object System.Windows.Forms.FolderBrowserDialog; \
        if ($env:TRADEPILOT_START -and (Test-Path -LiteralPath $env:TRADEPILOT_START)) { $d.SelectedPath = $env:TRADEPILOT_START }; \
        if ($d.ShowDialog() -eq 'OK') { [Console]::Out.Write($d.SelectedPath) }";
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", script])
        .env("TRADEPILOT_START", start)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(target_os = "macos")]
fn run_folder_dialog(start: &str) -> Option<String> {
    // The start folder travels as a script argument, never inside the script text.
    let script = "on run argv\n\
        set startFolder to item 1 of argv\n\
        try\n\
        set chosen to choose folder default location (POSIX file startFolder)\n\
        on error\n\
        try\n\
        set chosen to choose folder\n\
        on error\n\
        return \"\"\n\
        end try\n\
        end try\n\
        return POSIX path of chosen\n\
        end run";
    let output = std::process::Command::new("osascript").args(["-e", script, start]).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn run_folder_dialog(_start: &str) -> Option<String> {
    None
}

/// The name of the file that switches the portable mode on when it lies next to the program.
pub const PORTABLE_MARKER: &str = "portable.txt";

/// The data folder of the portable mode for the program `exe`: `Data` next to it, if a file `portable.txt` lies there and the
/// folder can hold the data. `None` for a program without the file (the normal case) or in a folder that cannot be written to
/// (the program then keeps its data in the folder of the user, as always).
#[must_use]
pub fn portable_data_dir(exe: &Path) -> Option<PathBuf> {
    let folder = exe.parent()?;
    if !folder.join(PORTABLE_MARKER).is_file() {
        return None;
    }
    let data = folder.join("Data");
    check_data_dir(&data).ok().map(|()| data)
}

/// The data folder of the portable mode of this run, if it is on (decided once).
#[must_use]
pub fn portable_dir() -> Option<PathBuf> {
    static PORTABLE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    PORTABLE.get_or_init(|| std::env::current_exe().ok().and_then(|exe| portable_data_dir(&exe))).clone()
}

/// The default data folder: `Data` next to the program in the portable mode, else the one of the operating system (`app_data_dir`
/// unless the user chose another).
pub fn default_data_dir() -> io::Result<PathBuf> {
    if let Some(portable) = portable_dir() {
        return Ok(portable);
    }
    let no_dir_err = || io::Error::new(io::ErrorKind::NotFound, "No writable user data directory");
    // Each OS's own convention for per-user app data, not just the Windows one reused everywhere.
    #[cfg(target_os = "windows")]
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or_else(no_dir_err)?);
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(std::env::var_os("HOME").ok_or_else(no_dir_err)?).join("Library").join("Application Support");
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share")))
        .ok_or_else(no_dir_err)?;
    Ok(base.join("TradeWorkspace"))
}

/// The folder for the database, the logs, the settings and everything else: the one the user
/// chose, else the default (see `data_location`).
pub fn app_data_dir() -> io::Result<PathBuf> {
    data_location().map(|location| location.dir)
}

/// The path of a file of the app's own in the data folder (`news_seen.json`, ...); `None` in tests, which write no files.
#[must_use]
pub fn data_file(name: &str) -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    app_data_dir().ok().map(|dir| dir.join(name))
}

/// Reads the JSON file at `path`. A file that does not exist gives the default; any other failure is the error, in words with the file name.
pub fn read_json_file<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| format!("{} could not be read: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(format!("{} could not be read: {error}", path.display())),
    }
}

/// Writes `value` as JSON to the file at `path`: to a file beside it first, then renamed, so a crash leaves the old file and not half of the new.
pub fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let json = serde_json::to_string(value).map_err(|error| error.to_string())?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, json).and_then(|()| fs::rename(&temporary, path)).map_err(|error| format!("{} could not be saved: {error}", path.display()))
}

pub fn log_path() -> io::Result<PathBuf> {
    Ok(app_data_dir()?.join("logs").join("app.log"))
}

/// Separate data per mode: settings
/// and templates of the Live mode stay in the app data folder itself, Paper Trading has its own
/// `paper` subfolder. Logs and backups are shared.
pub fn mode_dir(live: bool) -> io::Result<PathBuf> {
    let base = app_data_dir()?;
    Ok(if live { base } else { base.join("paper") })
}

pub fn settings_path(live: bool) -> io::Result<PathBuf> {
    Ok(mode_dir(live)?.join("settings.json"))
}

#[derive(Serialize, Deserialize)]
struct TradingMode {
    live: bool,
}

/// The Live / Paper Trading switch as last set; Paper Trading when never set.
pub fn load_trading_mode() -> bool {
    app_data_dir()
        .ok()
        .and_then(|dir| fs::read_to_string(dir.join("mode.json")).ok())
        .and_then(|text| serde_json::from_str::<TradingMode>(&text).ok())
        .is_some_and(|mode| mode.live)
}

pub fn write_trading_mode(live: bool) -> io::Result<()> {
    let dir = app_data_dir()?;
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string(&TradingMode { live }).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(dir.join("mode.json"), json)
}

/// The first time Paper Trading is used, it starts from a copy of the existing settings and
/// templates (everything made before the switch existed), so nothing has to be set up twice.
/// Returns whether anything was copied.
pub fn seed_mode_dir(from: &Path, to: &Path) -> io::Result<bool> {
    let names = ["settings.json", "templates.json"];
    if names.iter().any(|name| to.join(name).exists()) {
        return Ok(false);
    }
    let mut copied = false;
    for name in names {
        if from.join(name).is_file() {
            fs::create_dir_all(to)?;
            fs::copy(from.join(name), to.join(name))?;
            copied = true;
        }
    }
    Ok(copied)
}

/// Reads and parses `settings.json`, if it exists and is valid. Any failure (missing file, bad
/// JSON from a future/older format) falls back to `None` so startup always proceeds with the
/// hardcoded defaults instead of failing to launch over a settings file.
/// The settings of the mode, laid over `defaults` (see `restore_from_text`): a file of an older version keeps every value it has, the
/// settings it lacks take their default. `None` without a readable settings file.
pub fn load_settings(live: bool, defaults: &PersistedSettings) -> Option<Restored> {
    let path = settings_path(live).ok()?;
    let text = fs::read_to_string(path).ok()?;
    restore_from_text(&text, defaults).ok()
}

pub fn write_settings(path: &Path, settings: &PersistedSettings) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid settings file path"))?;
    fs::create_dir_all(parent)?;
    let json = serde_json::to_string_pretty(settings).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(path, json)
}

pub fn append_log(path: &Path, line: &str) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid log file path"))?;
    fs::create_dir_all(parent)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

/// Whether a line of the log file stays when the lines before `keep_from` are removed: a line that does not start with its date
/// (`dd-mm-yyyy`) is kept, it cannot be told apart from a newer one.
fn keeps_log_line(line: &str, keep_from: chrono::NaiveDate) -> bool {
    line.get(..10).and_then(|date| chrono::NaiveDate::parse_from_str(date, "%d-%m-%Y").ok()).is_none_or(|date| date >= keep_from)
}

/// Removes the lines written before `keep_from` from the log file at `path`, and every other file in its folder last changed before
/// that day. Returns how many lines and files were removed. The log file is rewritten through a temporary file, so a failure leaves
/// it whole.
pub fn prune_logs(path: &Path, keep_from: chrono::NaiveDate) -> io::Result<(usize, usize)> {
    let mut lines_removed = 0;
    if path.is_file() {
        let text = fs::read_to_string(path)?;
        let kept: Vec<&str> = text.lines().filter(|line| keeps_log_line(line, keep_from)).collect();
        lines_removed = text.lines().count().saturating_sub(kept.len());
        if lines_removed > 0 {
            let temporary = path.with_extension("log.tmp");
            let mut joined = kept.join("\n");
            if !joined.is_empty() {
                joined.push('\n');
            }
            fs::write(&temporary, joined)?;
            fs::rename(&temporary, path)?;
        }
    }
    let mut files_removed = 0;
    let Some(dir) = path.parent().filter(|dir| dir.is_dir()) else {
        return Ok((lines_removed, files_removed));
    };
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_file() || entry.path() == path {
            continue;
        }
        let changed: chrono::DateTime<chrono::Local> = metadata.modified()?.into();
        if changed.date_naive() < keep_from {
            fs::remove_file(entry.path())?;
            files_removed += 1;
        }
    }
    Ok((lines_removed, files_removed))
}

fn backups_dir() -> io::Result<PathBuf> {
    Ok(app_data_dir()?.join("backups"))
}

/// The computer name as a part of a file name: letters, digits, `-` and `_` only, at most 40 characters; empty when nothing is left.
fn file_name_part(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// The file name of a backup written at `timestamp` on the computer `machine` (left out when empty).
fn backup_file_name(machine: &str, timestamp: &str) -> String {
    match file_name_part(machine) {
        part if part.is_empty() => format!("backup-{timestamp}.json"),
        part => format!("backup-{part}-{timestamp}.json"),
    }
}

/// Whether a backup of the computer `machine` was written on `day` (`yyyymmdd`), told by the file names in the backup folder.
pub fn backup_written_on(day: &str, machine: &str) -> bool {
    let prefix = backup_file_name(machine, day);
    let prefix = prefix.trim_end_matches(".json");
    list_backups().is_ok_and(|backups| backups.iter().any(|b| b.file_name.starts_with(prefix)))
}

/// Writes the given settings as a real local backup file - a genuine, restorable file on
/// disk. The file name carries the computer name of the settings, so backups of several computers in one folder stay apart.
/// Returns the path written to.
pub fn write_backup(settings: &PersistedSettings) -> io::Result<PathBuf> {
    let dir = backups_dir()?;
    fs::create_dir_all(&dir)?;
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let path = dir.join(backup_file_name(&settings.backup_machine_name, &timestamp));
    let json = serde_json::to_string_pretty(settings).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(&path, json)?;
    Ok(path)
}

/// One row of Settings > Database Backup > "Available Backups" - real file metadata read from
/// disk, not invented.
pub struct BackupInfo {
    pub path: PathBuf,
    pub file_name: String,
    pub modified: String,
    pub size_bytes: u64,
}

/// Lists real backup files under this app's backups directory, newest first. Returns an empty
/// list (not an error) if the directory doesn't exist yet - nothing has been backed up, which is
/// not itself a failure.
pub fn list_backups() -> io::Result<Vec<BackupInfo>> {
    let dir = backups_dir()?;
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut backups = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        let modified: chrono::DateTime<chrono::Local> = metadata.modified()?.into();
        backups.push(BackupInfo {
            path: entry.path(),
            file_name: entry.file_name().to_string_lossy().into_owned(),
            modified: crate::i18n::show_date_time(modified.naive_local()),
            size_bytes: metadata.len(),
        });
    }
    backups.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    Ok(backups)
}

/// A backup read back over the settings in use: the settings to apply, and where the backup does not fit the settings of this version.
#[derive(Clone, PartialEq)]
pub struct Restored {
    /// The settings in use, with every value of the backup that fits.
    pub settings: PersistedSettings,
    /// Settings of this version the backup does not have (written by an older version): they keep their current value.
    pub missing: Vec<String>,
    /// Entries of the backup this version does not know (written by another version): they are left out.
    pub unknown: Vec<String>,
    /// Entries of the backup whose value does not fit the setting of the same name any more: they keep their current value.
    pub unusable: Vec<String>,
}

impl Restored {
    /// Whether the backup fitted the settings of this version exactly.
    #[must_use]
    pub fn exact(&self) -> bool {
        self.missing.is_empty() && self.unknown.is_empty() && self.unusable.is_empty()
    }
}

/// Lays the backup `text` over the settings `current`, entry by entry, so that a backup of another version is restored as far as it
/// fits instead of being refused as a whole.
///
/// # Errors
///
/// The reason as text when the text is no settings backup at all (no JSON object).
pub fn restore_from_text(text: &str, current: &PersistedSettings) -> Result<Restored, String> {
    let backup: serde_json::Value = serde_json::from_str(text).map_err(|error| format!("the file is no settings backup ({error})"))?;
    let serde_json::Value::Object(backup) = backup else { return Err("the file is no settings backup (no JSON object)".to_string()) };
    let serde_json::Value::Object(base) = serde_json::to_value(current).map_err(|error| error.to_string())? else {
        return Err("the current settings cannot be read".to_string());
    };
    let missing: Vec<String> = base.keys().filter(|key| !backup.contains_key(*key)).cloned().collect();
    let unknown: Vec<String> = backup.keys().filter(|key| !base.contains_key(*key)).cloned().collect();
    let mut merged = base.clone();
    let mut unusable = Vec::new();
    for (key, value) in backup.iter().filter(|(key, _)| base.contains_key(*key)) {
        let previous = merged.insert(key.clone(), value.clone());
        if serde_json::from_value::<PersistedSettings>(serde_json::Value::Object(merged.clone())).is_err() {
            if let Some(previous) = previous {
                merged.insert(key.clone(), previous);
            }
            unusable.push(key.clone());
        }
    }
    let settings = serde_json::from_value(serde_json::Value::Object(merged)).map_err(|error| error.to_string())?;
    Ok(Restored { settings, missing, unknown, unusable })
}

/// Reads the backup file at `path` over the settings `current` (see `restore_from_text`).
///
/// # Errors
///
/// When the file cannot be read or is no settings backup.
pub fn restore_backup(path: &Path, current: &PersistedSettings) -> io::Result<Restored> {
    let text = fs::read_to_string(path)?;
    restore_from_text(&text, current).map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason))
}

/// Deletes the backup file at `path`; only a file in the backup folder.
///
/// # Errors
///
/// When the file is not in the backup folder or cannot be deleted.
pub fn delete_backup(path: &Path) -> io::Result<()> {
    let dir = backups_dir()?;
    if path.parent() != Some(dir.as_path()) || !path.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file in the backup folder"));
    }
    fs::remove_file(path)
}

/// The switches of the 0-DTE conversion and the market close time (PM or not).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeFlags {
    /// The `convert_0dte_enabled` switch.
    pub convert_0dte_enabled: bool,
    /// The `convert_0dte_pm` switch.
    pub convert_0dte_pm: bool,
    /// The `market_close_pm` switch.
    pub market_close_pm: bool,
}

/// The switches of the logs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewFlags {
    /// The Debug logging level is on (else Info): the log also shows what TWS answered and how each answer was read.
    pub settings_logging_debug: bool,
    /// The `settings_clean_logs` switch.
    pub settings_clean_logs: bool,
}

/// The switches of the grid, the backup and the SPX estimate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiscFlags {
    /// The `settings_mobile_grid` switch.
    pub settings_mobile_grid: bool,
    /// The `daily_backup_enabled` switch.
    pub daily_backup_enabled: bool,
    /// The `estimate_spx_extended_hours` switch.
    pub estimate_spx_extended_hours: bool,
}

/// Which events send an e-mail, and the opened event as webhook.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyFlags {
    /// The `notify_trade_opened_email` switch.
    pub notify_trade_opened_email: bool,
    /// The `notify_trade_stopped_email` switch.
    pub notify_trade_stopped_email: bool,
    /// The `notify_trade_opened_webhook` switch.
    pub notify_trade_opened_webhook: bool,
}

/// The stopped event as webhook, TLS for SMTP and the webhook triggers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailFlags {
    /// The `notify_trade_stopped_webhook` switch.
    pub notify_trade_stopped_webhook: bool,
    /// The `smtp_use_tls` switch.
    pub smtp_use_tls: bool,
    /// The `webhook_triggers_enabled` switch.
    pub webhook_triggers_enabled: bool,
}

/// The switches of the stop conversions of the Trading Features tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertFlags {
    /// The `iron_fly_stop_split_enabled` switch.
    pub iron_fly_stop_split_enabled: bool,
    /// The `convert_short_to_vertical_enabled` switch.
    pub convert_short_to_vertical_enabled: bool,
    /// The `convert_vertical_to_short_enabled` switch.
    pub convert_vertical_to_short_enabled: bool,
}

/// The switches of the stop safety features.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopSafetyFlags {
    /// The `close_on_stop_margin_rejection` switch.
    pub close_on_stop_margin_rejection: bool,
    /// The `close_beyond_stop_enabled` switch.
    pub close_beyond_stop_enabled: bool,
    /// The `close_beyond_stop_market_order` switch.
    pub close_beyond_stop_market_order: bool,
}

/// The switches of the missing-stop and slippage alerts and the retry on margin rejection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertFlags {
    /// The `missing_stop_notifications_enabled` switch.
    pub missing_stop_notifications_enabled: bool,
    /// The `high_slippage_notifications_enabled` switch.
    pub high_slippage_notifications_enabled: bool,
    /// The `retry_on_margin_rejection` switch.
    pub retry_on_margin_rejection: bool,
}

/// Everything on the Settings page that is actually persisted to `settings.json` (see
/// `settings_path`). Comparing a freshly-built one against `App::settings_saved` is how the UI
/// knows whether there are unsaved changes, instead of hand-flagging "dirty" on every widget.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedSettings {
    pub tws_host: String,
    pub tws_port: String,
    pub settings_client_id: String,
    pub settings_maintain_connection: String,
    pub convert_0dte_hour: String,
    pub convert_0dte_minute: String,
    pub market_close_hour: String,
    pub market_close_minute: String,
    pub settings_ignore_accounts: String,
    pub settings_cleanup_interval: String,
    pub settings_grid_font: String,
    pub notification_email: String,
    pub notification_webhook_url: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_username: String,
    pub smtp_password: String,
    pub long_strike_reuse: String,
    pub iron_fly_stop_split_price: String,
    pub convert_vertical_to_short_price: String,
    pub daily_profit_target: String,
    pub daily_profit_target_delay_seconds: String,
    pub net_liquidity_override: String,
    /// An address of its own that is given to the free translation service of the news; with it the service allows more per day (empty: none).
    #[serde(default)]
    pub translation_contact_email: String,
    /// The Light mode of the interface: only the most important pages are in the sidebar (else Advanced: all of them).
    #[serde(default)]
    pub light_mode: bool,
    /// After the market close, the trades of the day are written to a CSV file in the `exports` folder (see `daily_csv_tick`).
    #[serde(default)]
    pub daily_csv_export: bool,
    /// The most contracts (or combos) of one symbol open at the same time before a new position is refused; empty or 0 = no limit.
    #[serde(default)]
    pub risk_max_contracts_per_symbol: String,
    /// The loss of today in dollars from which on no new position is opened today; empty or 0 = no limit.
    #[serde(default)]
    pub risk_max_daily_loss: String,
    /// The most new opening orders within one minute; empty or 0 = no limit.
    #[serde(default)]
    pub risk_max_orders_per_minute: String,
    // Market Data
    pub market_data_provider: String,
    pub symbol_configs: Vec<SymbolConfig>,
    pub ticker_price_symbol: String,
    pub ticker_delta_symbol: String,
    pub close_beyond_stop_pct: String,
    pub close_beyond_stop_wait_seconds: String,
    pub high_slippage_pct: String,
    pub backup_machine_name: String,
    pub lang: Lang,
    /// The switches of the 0-DTE conversion and the market close time (PM or not).
    #[serde(flatten)]
    pub time_flags: TimeFlags,
    /// The switches of the logs.
    #[serde(flatten)]
    pub view_flags: ViewFlags,
    /// The switches of the grid, the backup and the SPX estimate.
    #[serde(flatten)]
    pub misc_flags: MiscFlags,
    /// Which events send an e-mail, and the opened event as webhook.
    #[serde(flatten)]
    pub notify_flags: NotifyFlags,
    /// The stopped event as webhook, TLS for SMTP and the webhook triggers.
    #[serde(flatten)]
    pub mail_flags: MailFlags,
    /// The switches of the stop conversions of the Trading Features tab.
    #[serde(flatten)]
    pub convert_flags: ConvertFlags,
    /// The switches of the stop safety features.
    #[serde(flatten)]
    pub stop_flags: StopSafetyFlags,
    /// The switches of the missing-stop and slippage alerts and the retry on margin rejection.
    #[serde(flatten)]
    pub alert_flags: AlertFlags,
}

/// Replaces the stored copy of `trade` (same day, order, trade type and strikes) in the database
/// at `path`.
pub fn update_trade(path: &Path, trade: &StoredTrade) -> io::Result<()> {
    database::update_trade(path, trade)
}

/// The database of the Live or the Paper Trading mode (`tradepilot.db`): trades and executions.
pub fn trades_path(live: bool) -> io::Result<PathBuf> {
    Ok(database::db_path(&mode_dir(live)?))
}

/// Every stored trade; a missing or unreadable file means none, never a startup failure.
pub fn load_trades(path: &Path) -> Vec<StoredTrade> {
    database::load_trades(path).unwrap_or_default()
}

/// Appends one trade to the list at `path`.
pub fn append_trade(path: &Path, trade: &StoredTrade) -> io::Result<()> {
    database::append_trade(path, trade)
}

/// The stored form of an execution TWS reported.
#[must_use]
pub fn stored_execution(row: &broker::ExecutionRow) -> StoredExecution {
    StoredExecution {
        exec_id: row.exec_id.clone(),
        time: row.time.clone(),
        account: row.account.clone(),
        symbol: row.symbol.clone(),
        con_id: row.con_id,
        order_id: row.order_id,
        order_ref: row.order_ref.clone(),
        sec_type: row.sec_type.clone(),
        expiry: row.expiry.clone(),
        right: row.right.clone(),
        strike: row.strike,
        side: row.side.clone(),
        quantity: row.quantity,
        price: row.price,
    }
}

/// The execution history of the Live or the Paper Trading mode: the same database as the trades.
pub fn executions_path(live: bool) -> io::Result<PathBuf> {
    trades_path(live)
}

/// Every stored execution; a missing file means none, a damaged line is skipped.
pub fn load_executions(path: &Path) -> Vec<StoredExecution> {
    database::load_executions(path).unwrap_or_default()
}

/// Every stored commission by execution id; a missing file means none.
pub fn load_commissions(path: &Path) -> std::collections::HashMap<String, rust_decimal::Decimal> {
    database::load_commissions(path).unwrap_or_default()
}

/// Stores the commission TWS charged for one execution.
pub fn save_commission(path: &Path, exec_id: &str, amount: rust_decimal::Decimal) -> io::Result<()> {
    database::save_commission(path, exec_id, amount)
}

/// Appends one execution as a line.
pub fn append_execution(path: &Path, execution: &StoredExecution) -> io::Result<()> {
    database::append_execution(path, execution)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_json_file_is_read_and_written_and_its_problems_are_reasons_not_silence() {
        let dir = std::env::temp_dir().join(format!("tradepilot-json-file-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let _ = fs::remove_file(&path);
        let empty: std::collections::HashMap<String, String> = read_json_file(&path).unwrap();
        assert!(empty.is_empty(), "a missing file gives the default");
        let value = std::collections::HashMap::from([("a".to_string(), "b".to_string())]);
        write_json_file(&path, &value).unwrap();
        assert_eq!(read_json_file::<std::collections::HashMap<String, String>>(&path), Ok(value.clone()));
        assert!(!dir.join("state.json.tmp").exists(), "the file beside it is renamed away");
        fs::write(&path, "not json").unwrap();
        assert!(read_json_file::<std::collections::HashMap<String, String>>(&path).unwrap_err().contains("could not be read"));
        // A folder in the place of the file cannot be written.
        assert!(write_json_file(&dir, &value).unwrap_err().contains("could not be saved"));
        let _ = fs::remove_dir_all(&dir);
    }
    #[test]
    fn the_portable_mode_needs_the_marker_next_to_the_program_and_keeps_the_data_beside_it() {
        let base =
            std::env::temp_dir().join(format!("tradepilot-portable-test-{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let exe = base.join("TradePilot.exe");
        assert_eq!(portable_data_dir(&exe), None, "no marker: the normal folder of the user");
        fs::write(base.join(PORTABLE_MARKER), "").unwrap();
        assert_eq!(portable_data_dir(&exe), Some(base.join("Data")));
        assert!(base.join("Data").is_dir(), "the folder is made");
        // A marker where the data folder cannot be made (a file is in the way): the mode is not switched on.
        let other = std::env::temp_dir().join(format!("tradepilot-portable-test-b-{}", std::process::id()));
        let _ = fs::remove_dir_all(&other);
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join(PORTABLE_MARKER), "").unwrap();
        fs::write(other.join("Data"), "a file").unwrap();
        assert_eq!(portable_data_dir(&other.join("TradePilot.exe")), None);
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&other);
    }

    use super::*;

    #[test]
    fn trades_are_appended_and_read_back_exactly() {
        let folder = crate::test_support::TempDir::new("trades");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("trades.json");
        assert!(load_trades(&path).is_empty(), "no file: no trades");
        let trade = StoredTrade {
            day: "2026-11-13".into(),
            opened: "13-11-2026 15:30:00".into(),
            account: "DU123".into(),
            symbol: "SPX".into(),
            trade_type: "PutSpread".into(),
            strikes: "6650/6640 P".into(),
            order_id: 7,
            qty: 1,
            multiplier: 100,
            price_open: rust_decimal::Decimal::from_str_exact("1.15").unwrap(),
            delta: 4.5,
            short_put: Some(StoredLeg { con_id: 11, strike: "6650".into() }),
            long_put: Some(StoredLeg { con_id: 12, strike: "6640".into() }),
            short_call: None,
            long_call: None,
            put_ratio: 1,
            call_ratio: 1,
            expiry: "20261113".into(),
            status: "Open".into(),
            price_close: None,
            stop_order_id: None,
            profit_order_id: None,
            stop_limit_order_id: None,
            exit_levels: None,
            adjustments: None,
            oca_group: None,
            close_order_id: None,
            stop_price: None,
            profit_price: None,
            entry_unknown: false,
            strategy: String::new(),
            timed_exit: None,
            profit_cancel: None,
            origin: None,
            custom_legs: None,
        };
        append_trade(&path, &trade).unwrap();
        append_trade(&path, &StoredTrade { order_id: 8, ..trade.clone() }).unwrap();
        let back = load_trades(&path);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0], trade);
        assert_eq!(back[1].order_id, 8);

        // An expired trade replaces its own stored copy only.
        let expired = StoredTrade { status: "Expired".into(), price_close: Some(rust_decimal::Decimal::from_str_exact("-5").unwrap()), ..trade.clone() };
        update_trade(&path, &expired).unwrap();
        let back = load_trades(&path);
        assert_eq!(back[0], expired);
        assert_eq!(back[1].status, "Open");

        // (Files from before V0.97 and the JSON files of earlier versions are covered by the
        // migration tests in `database`.)
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn writes_and_appends_log_messages_and_reports_io_errors() {
        let folder = crate::test_support::TempDir::new("log");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("app.log");
        append_log(&path, "first").unwrap();
        append_log(&path, "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().lines().collect::<Vec<_>>(), ["first", "second"]);
        assert!(append_log(&path.join("child"), "bad").is_err());
    }

    #[test]
    fn pruning_the_logs_removes_the_lines_before_the_day_and_keeps_lines_without_a_date() {
        let folder = crate::test_support::TempDir::new("prune-log");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("app.log");
        fs::write(&path, "01-03-2026 10:00:00  old\n  continued\n15-03-2026 09:00:00  kept\n20-03-2026 08:00:00  newer\n").unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        assert_eq!(prune_logs(&path, day).unwrap(), (1, 0));
        assert_eq!(fs::read_to_string(&path).unwrap(), "  continued\n15-03-2026 09:00:00  kept\n20-03-2026 08:00:00  newer\n");
        assert_eq!(prune_logs(&path, day).unwrap(), (0, 0), "nothing is left to remove");
        let tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap();
        fs::write(folder.join("app-old.log"), "x").unwrap();
        assert_eq!(prune_logs(&path, tomorrow).unwrap().1, 1, "another file changed before the day goes");
        assert!(!folder.join("app-old.log").exists());
        assert!(path.exists(), "the log file itself stays, even when empty");
    }

    #[test]
    fn a_backup_file_name_carries_the_computer_name_made_safe_for_a_file_name() {
        assert_eq!(backup_file_name("", "20260315-100000"), "backup-20260315-100000.json");
        assert_eq!(backup_file_name(" Office PC ", "20260315-100000"), "backup-Office_PC-20260315-100000.json");
        assert_eq!(backup_file_name("../..", "t"), "backup-t.json", "nothing but separators is no name");
    }

    #[test]
    fn settings_round_trip_through_disk_matches_what_was_written() {
        let folder = crate::test_support::TempDir::new("settings");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("settings.json");
        let settings = sample_settings();
        write_settings(&path, &settings).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let reloaded: PersistedSettings = serde_json::from_str(&text).unwrap();
        assert!(reloaded == settings);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_backup_of_this_version_is_restored_exactly() {
        let settings = sample_settings();
        let text = serde_json::to_string(&settings).unwrap();
        let restored = restore_from_text(&text, &PersistedSettings { tws_port: "1".into(), ..settings.clone() }).unwrap();
        assert!(restored.exact());
        assert!(restored.settings == settings, "every value comes from the backup");
    }

    #[test]
    fn an_older_backup_without_a_setting_is_restored_and_the_setting_keeps_its_current_value() {
        let current = PersistedSettings { view_flags: ViewFlags { settings_logging_debug: true, settings_clean_logs: true }, ..sample_settings() };
        let mut old = serde_json::to_value(PersistedSettings { tws_port: "4002".into(), ..sample_settings() }).unwrap();
        let object = old.as_object_mut().unwrap();
        object.remove("settings_logging_debug");
        object.insert("a_switch_of_an_old_version".into(), serde_json::Value::Bool(true));
        object.insert("settings_grid_font".into(), serde_json::json!([1, 2]));
        let restored = restore_from_text(&old.to_string(), &current).unwrap();
        assert_eq!(restored.missing, ["settings_logging_debug"]);
        assert_eq!(restored.unknown, ["a_switch_of_an_old_version"]);
        assert_eq!(restored.unusable, ["settings_grid_font"], "a value of another type");
        assert!(restored.settings.view_flags.settings_logging_debug, "the missing setting keeps its current value");
        assert_eq!(restored.settings.settings_grid_font, current.settings_grid_font);
        assert_eq!(restored.settings.tws_port, "4002", "the rest comes from the backup");
        assert!(!restored.exact());
    }

    #[test]
    fn a_file_that_is_no_settings_backup_is_refused_with_the_reason() {
        assert!(restore_from_text("not json", &sample_settings()).err().unwrap().contains("no settings backup"));
        assert!(restore_from_text("[1, 2]", &sample_settings()).err().unwrap().contains("no JSON object"));
    }

    #[test]
    fn only_a_file_in_the_backup_folder_can_be_deleted() {
        let folder = crate::test_support::TempDir::new("not-backups");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("backup-x.json");
        fs::write(&path, "{}").unwrap();
        assert!(delete_backup(&path).is_err());
        assert!(path.exists(), "a file elsewhere is never deleted");
    }

    /// Settings with a value of their own in every field.
    fn sample_settings() -> PersistedSettings {
        PersistedSettings {
            tws_host: "127.0.0.1".to_string(),
            tws_port: "7497".to_string(),
            settings_client_id: "9".to_string(),
            settings_maintain_connection: "Always".to_string(),
            time_flags: TimeFlags { convert_0dte_enabled: true, convert_0dte_pm: true, market_close_pm: true },
            convert_0dte_hour: "3".to_string(),
            convert_0dte_minute: "00".to_string(),
            market_close_hour: "4".to_string(),
            market_close_minute: "00".to_string(),
            settings_ignore_accounts: "DU000000".to_string(),
            view_flags: ViewFlags { settings_logging_debug: true, settings_clean_logs: false },
            settings_cleanup_interval: "30".to_string(),
            settings_grid_font: "16".to_string(),
            misc_flags: MiscFlags { settings_mobile_grid: true, estimate_spx_extended_hours: false, daily_backup_enabled: false },
            notification_email: "test-recipient".to_string(),
            notify_flags: NotifyFlags { notify_trade_opened_email: true, notify_trade_stopped_email: false, notify_trade_opened_webhook: false },
            notification_webhook_url: "https://example.com/hook".to_string(),
            mail_flags: MailFlags { notify_trade_stopped_webhook: true, smtp_use_tls: true, webhook_triggers_enabled: false },
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: "587".to_string(),
            smtp_username: "test-user".to_string(),
            smtp_password: "hunter2".to_string(),
            long_strike_reuse: "Ignore".to_string(),
            convert_flags: ConvertFlags {
                iron_fly_stop_split_enabled: true,
                convert_short_to_vertical_enabled: false,
                convert_vertical_to_short_enabled: true,
            },
            iron_fly_stop_split_price: "0".to_string(),
            convert_vertical_to_short_price: "0".to_string(),
            daily_profit_target: "0".to_string(),
            daily_profit_target_delay_seconds: "15".to_string(),
            net_liquidity_override: "0".to_string(),
            translation_contact_email: String::new(),
            light_mode: false,
            daily_csv_export: false,
            risk_max_contracts_per_symbol: "15".to_string(),
            risk_max_daily_loss: "750".to_string(),
            risk_max_orders_per_minute: "6".to_string(),
            market_data_provider: "TWS".to_string(),
            symbol_configs: vec![SymbolConfig {
                symbol: "SPX".to_string(),
                required_dtes: "0".to_string(),
                put_otm: "220".to_string(),
                call_otm: "200".to_string(),
                put_itm: "20".to_string(),
                call_itm: "20".to_string(),
                extended_quote_strikes: false,
            }],
            ticker_price_symbol: "SPX".to_string(),
            ticker_delta_symbol: "SPX".to_string(),
            stop_flags: StopSafetyFlags { close_on_stop_margin_rejection: false, close_beyond_stop_enabled: false, close_beyond_stop_market_order: true },
            close_beyond_stop_pct: "10".to_string(),
            close_beyond_stop_wait_seconds: "10".to_string(),
            alert_flags: AlertFlags { missing_stop_notifications_enabled: true, high_slippage_notifications_enabled: true, retry_on_margin_rejection: false },
            high_slippage_pct: "100".to_string(),
            backup_machine_name: "TESTHOST".to_string(),
            lang: Lang::De,
        }
    }

    /// A folder of its own (not made yet) and the guard that removes it when the test is over.
    fn temp_folder(name: &str) -> (crate::test_support::TempDir, PathBuf) {
        let guard = crate::test_support::TempDir::new(name);
        let dir = guard.to_path_buf();
        (guard, dir)
    }

    #[test]
    fn a_storage_folder_must_be_a_full_path_that_can_be_written() {
        assert!(check_data_dir(Path::new("relative/folder")).is_err());
        let (_guard, root) = temp_folder("check");
        let dir = root.join("new").join("deeper");
        check_data_dir(&dir).unwrap();
        assert!(dir.is_dir(), "the folder was created");
        assert!(!dir.join(".tradepilot-write-test").exists(), "the probe file is removed again");
    }

    #[test]
    fn copying_the_data_keeps_existing_files_and_skips_the_location_file() {
        let (_guard_from, from) = temp_folder("from");
        let to = from.join("moved");
        fs::create_dir_all(from.join("paper")).unwrap();
        fs::write(from.join("tradepilot.db"), "db").unwrap();
        fs::write(from.join("paper").join("settings.json"), "paper").unwrap();
        fs::write(from.join(LOCATION_FILE), "elsewhere").unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(to.join("tradepilot.db"), "newer").unwrap();
        // The target lies inside the source: it is not copied into itself.
        assert_eq!(copy_data(&from, &to).unwrap(), 1);
        assert_eq!(fs::read_to_string(to.join("tradepilot.db")).unwrap(), "newer", "an existing file is kept");
        assert_eq!(fs::read_to_string(to.join("paper").join("settings.json")).unwrap(), "paper");
        assert!(!to.join(LOCATION_FILE).exists());
    }

    #[test]
    fn the_storage_folder_is_the_default_until_the_user_chose_another() {
        let (_guard_default_dir, default_dir) = temp_folder("loc-default");
        fs::create_dir_all(&default_dir).unwrap();
        let none = resolve_location(default_dir.clone());
        assert_eq!((none.dir.clone(), none.fallback), (default_dir.clone(), None));
        // A chosen folder that can be written is used, and the default folder is still known.
        let (_guard_chosen, chosen) = temp_folder("loc-chosen");
        choose_data_dir_in(&default_dir, Some(&chosen)).unwrap();
        let used = resolve_location(default_dir.clone());
        assert_eq!((used.dir, used.default_dir, used.fallback), (chosen.clone(), default_dir.clone(), None));
        // Back to the default removes the file; so does naming the default itself.
        choose_data_dir_in(&default_dir, None).unwrap();
        assert!(!default_dir.join(LOCATION_FILE).exists());
        choose_data_dir_in(&default_dir, Some(&chosen)).unwrap();
        choose_data_dir_in(&default_dir, Some(&default_dir)).unwrap();
        assert!(!default_dir.join(LOCATION_FILE).exists());
        // Removing what is not there is no error.
        choose_data_dir_in(&default_dir, None).unwrap();
    }

    #[test]
    fn a_chosen_folder_that_cannot_be_used_falls_back_to_the_default_and_says_why() {
        let (_guard_default_dir, default_dir) = temp_folder("loc-fallback");
        fs::create_dir_all(&default_dir).unwrap();
        // A folder below a plain file cannot be created.
        let file = default_dir.join("plain-file");
        fs::write(&file, "x").unwrap();
        fs::write(default_dir.join(LOCATION_FILE), file.join("below").display().to_string()).unwrap();
        let location = resolve_location(default_dir.clone());
        assert_eq!(location.dir, default_dir);
        assert!(location.fallback.is_some_and(|reason| reason.contains("cannot be used") && reason.contains("default folder")));
        // A relative path in the file is refused as well, and so is it when choosing.
        fs::write(default_dir.join(LOCATION_FILE), "relative/path").unwrap();
        assert!(resolve_location(default_dir.clone()).fallback.is_some());
        assert!(choose_data_dir_in(&default_dir, Some(Path::new("relative/path"))).is_err());
        // An empty or blank location file means no choice at all.
        fs::write(default_dir.join(LOCATION_FILE), "  \n").unwrap();
        assert_eq!(resolve_location(default_dir.clone()).fallback, None);
        // A folder that cannot be written is not remembered.
        assert!(choose_data_dir_in(&default_dir, Some(&file.join("below"))).is_err());
    }

    #[test]
    fn prices_are_decimals_stored_as_text_and_the_files_of_earlier_versions_read_unchanged() {
        // As earlier versions wrote it: every price as decimal text.
        let old = r#"{"day":"2026-11-13","opened":"13-11-2026 10:00:00","account":"DU1","symbol":"SPX","trade_type":"PutSpread",
            "strikes":"6650/6640 P","order_id":7,"qty":2,"multiplier":100,"price_open":"1.15","delta":4.5,
            "status":"Stopped","price_close":"-3.45","stop_price":"3.45","profit_price":"0.55"}"#;
        let trade: StoredTrade = serde_json::from_str(old).unwrap();
        let d = |text: &str| text.parse::<rust_decimal::Decimal>().unwrap();
        assert_eq!(
            (trade.price_open, trade.price_close, trade.stop_price, trade.profit_price),
            (d("1.15"), Some(d("-3.45")), Some(d("3.45")), Some(d("0.55")))
        );
        // Written back, a price is text again - nothing becomes a float on the way to disk.
        let json = serde_json::to_value(&trade).unwrap();
        assert_eq!((json["price_open"].as_str(), json["price_close"].as_str()), (Some("1.15"), Some("-3.45")));
        assert_eq!(serde_json::from_value::<StoredTrade>(json).unwrap(), trade);
        // Text that is no price is an error of the file, not a price of 0.
        assert!(serde_json::from_str::<StoredTrade>(&old.replace("\"1.15\"", "\"x\"")).is_err());
    }

    fn trade_with_levels() -> StoredTrade {
        StoredTrade {
            day: "2026-11-16".into(),
            opened: "16-11-2026 10:00:00".into(),
            account: "DU1".into(),
            symbol: "SPX".into(),
            trade_type: "PutSpread".into(),
            strikes: "6650/6640 P".into(),
            order_id: 5,
            qty: 4,
            multiplier: 100,
            price_open: rust_decimal::Decimal::from_str_exact("0.3").unwrap(),
            delta: 0.0,
            short_put: None,
            long_put: None,
            short_call: None,
            long_call: None,
            put_ratio: 1,
            call_ratio: 1,
            expiry: "20261116".into(),
            status: "Open".into(),
            price_close: None,
            stop_order_id: Some(11),
            profit_order_id: Some(12),
            close_order_id: Some(20),
            stop_price: None,
            profit_price: None,
            entry_unknown: false,
            strategy: String::new(),
            timed_exit: None,
            profit_cancel: None,
            origin: None,
            custom_legs: None,
            stop_limit_order_id: None,
            exit_levels: Some(ExitLevels {
                qty: [2, 2, 0, 0],
                level: 1,
                order_ids: [Some(12), Some(14), None, None],
                closed: vec![(2, "-0.15".into())],
                ..ExitLevels::default()
            }),
            adjustments: None,
            oca_group: None,
        }
    }

    #[test]
    fn what_the_levels_closed_is_not_open_any_more_and_has_made_its_profit() {
        let trade = trade_with_levels();
        assert_eq!(trade.remaining_qty(), 2);
        // (0.30 - 0.15) * 2 * 100
        assert_eq!(trade.realized_profit(rust_decimal::Decimal::new(3, 1)), Some(rust_decimal::Decimal::from(30)));
        let plain = StoredTrade { exit_levels: None, ..trade.clone() };
        assert_eq!((plain.remaining_qty(), plain.realized_profit(rust_decimal::Decimal::ONE)), (4, Some(rust_decimal::Decimal::ZERO)));
        let unreadable = StoredTrade { exit_levels: Some(ExitLevels { closed: vec![(1, "x".into())], ..ExitLevels::default() }), ..trade };
        assert_eq!(unreadable.realized_profit(rust_decimal::Decimal::ONE), None);
    }

    #[test]
    fn every_exit_order_of_a_trade_is_listed_once() {
        let mut ids = trade_with_levels().exit_order_ids();
        ids.sort_unstable();
        assert_eq!(ids, [11, 12, 14, 20], "level 1's order is also the profit order, listed once");
        let plain = StoredTrade { exit_levels: None, stop_order_id: None, profit_order_id: None, close_order_id: None, ..trade_with_levels() };
        assert_eq!(plain.exit_order_ids(), [] as [i32; 0]);
    }

    #[test]
    fn a_trade_saved_before_the_levels_existed_still_loads() {
        let mut value = serde_json::to_value(trade_with_levels()).unwrap();
        if let Some(object) = value.as_object_mut() {
            object.remove("exit_levels");
            object.remove("stop_limit_order_id");
        }
        let trade: StoredTrade = serde_json::from_value(value).unwrap();
        assert_eq!((trade.exit_levels.clone(), trade.stop_limit_order_id, trade.remaining_qty()), (None, None, 4));
    }
}
