use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer};

const LOG_FILE_PREFIX: &str = "ldm.log";
const KEPT_LOG_FILES: usize = 7;

/// Keeps the background log writer alive; drop it only when the process exits.
pub struct LogGuard(#[allow(dead_code)] WorkerGuard);

/// Logs to `<app data>/logs/ldm.log.<date>` (one file per day, the last seven kept) and, in
/// debug builds, to stdout as well. Release builds have no console, so without the file the
/// logs would be lost.
pub fn init(app_data_dir: &Path) -> Result<LogGuard, String> {
    let log_dir = app_data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).map_err(|error| {
        format!(
            "could not create log directory '{}': {error}",
            log_dir.display()
        )
    })?;

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .max_log_files(KEPT_LOG_FILES)
        .build(&log_dir)
        .map_err(|error| {
            format!(
                "could not open log file in '{}': {error}",
                log_dir.display()
            )
        })?;
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(false)
        .with_writer(file_writer)
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);
    let stdout_layer = cfg!(debug_assertions).then(|| {
        fmt::layer()
            .with_target(false)
            .with_filter(tracing_subscriber::filter::LevelFilter::DEBUG)
    });

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stdout_layer)
        .try_init()
        .map_err(|error| format!("could not initialize logging: {error}"))?;

    tracing::info!(
        "LDM {} started, logs in '{}'",
        env!("CARGO_PKG_VERSION"),
        log_dir.display()
    );
    Ok(LogGuard(guard))
}
