use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use directories::BaseDirs;

/// The host runs headless inside the browser, so this file is the only place its decisions
/// (which desktop launch it chose, why a forward failed) can be seen afterwards.
const LOG_DIRECTORY: &str = "LDM/native-host";
const LOG_FILENAME: &str = "host.log";
const TRUNCATE_ABOVE_BYTES: u64 = 512 * 1024;

pub fn line(message: impl AsRef<str>) {
    static FILE: OnceLock<Option<Mutex<fs::File>>> = OnceLock::new();
    let Some(file) = FILE.get_or_init(open_log_file) else {
        return;
    };
    let Ok(mut file) = file.lock() else {
        return;
    };
    let _ = writeln!(
        file,
        "{} pid={} {}",
        timestamp(),
        std::process::id(),
        message.as_ref()
    );
}

pub fn log_path() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| dirs.data_local_dir().join(LOG_DIRECTORY).join(LOG_FILENAME))
}

fn open_log_file() -> Option<Mutex<fs::File>> {
    let path = log_path()?;
    fs::create_dir_all(path.parent()?).ok()?;
    let oversized = fs::metadata(&path).is_ok_and(|metadata| metadata.len() > TRUNCATE_ABOVE_BYTES);
    let file = OpenOptions::new()
        .create(true)
        .append(!oversized)
        .write(true)
        .truncate(oversized)
        .open(path)
        .ok()?;
    Some(Mutex::new(file))
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}
