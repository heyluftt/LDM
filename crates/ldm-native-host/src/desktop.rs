use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{bail, Context, Result};
use ldm_types::messaging::{DesktopToExtensionMessage, ExtensionToDesktopMessage};
use ldm_types::SCHEMA_VERSION;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ClientOptions;

use crate::host_log;
use crate::native_protocol::MAX_NATIVE_MESSAGE_BYTES;

const PIPE_NAME: &str = r"\\.\pipe\ldm-native-host";
const DESKTOP_EXE_ENV: &str = "LDM_DESKTOP_EXE";
const DESKTOP_COMMAND_ENV: &str = "LDM_DESKTOP_COMMAND";
const DESKTOP_ARGS_ENV: &str = "LDM_DESKTOP_ARGS";
const DESKTOP_CWD_ENV: &str = "LDM_DESKTOP_CWD";
const START_LOCK_NAME: &str = "ldm-desktop-start.lock";
/// How long the host keeps retrying the pipe after launching the desktop app. A packaged
/// build needs a few seconds; the extension waits longer than this once it has been told
/// that the desktop is starting.
const START_WAIT_TOTAL: Duration = Duration::from_secs(25);
const START_LOCK_STALE_AFTER: Duration = Duration::from_secs(30);
const START_RETRY_FAST_WINDOW: Duration = Duration::from_millis(750);
const START_RETRY_FAST_DELAY: Duration = Duration::from_millis(25);
const START_RETRY_SLOW_DELAY: Duration = Duration::from_millis(150);
const PRODUCTION_DESKTOP_EXE_NAMES: &[&str] = &["LDM.exe", "ldm-desktop.exe"];

#[derive(Debug)]
enum DesktopLaunch {
    Command {
        command: String,
        args: Vec<String>,
        cwd: PathBuf,
    },
    Exe {
        path: PathBuf,
        cwd: Option<PathBuf>,
    },
}

pub fn desktop_start_help() -> String {
    format!("Start LDM manually, set {DESKTOP_EXE_ENV} to the installed LDM.exe, or set {DESKTOP_COMMAND_ENV} for a development build.")
}

/// Forwards `message` to the running desktop app, or launches the app first.
///
/// `notify` receives a `DesktopStarting` message before a launch so the extension can
/// extend its wait; the final reply is returned.
pub async fn forward_or_start_desktop(
    message: &ExtensionToDesktopMessage,
    notify: &mut dyn FnMut(DesktopToExtensionMessage),
) -> Result<DesktopToExtensionMessage> {
    let first_error = match forward_to_desktop(message).await {
        Ok(response) => return Ok(response),
        Err(error) => error,
    };
    host_log::line(format!("desktop pipe unavailable: {first_error:#}"));

    let launch = resolve_desktop_launch()
        .with_context(|| format!("Initial desktop connection failed: {first_error:#}"))?;
    notify(DesktopToExtensionMessage::DesktopStarting {
        schema_version: SCHEMA_VERSION,
        detail: launch.description(),
    });

    let owns_start_lock = ensure_desktop_started(&launch)?;
    host_log::line(format!(
        "waiting for desktop pipe (launched here: {owns_start_lock}, via {})",
        launch.description()
    ));
    let started_at = Instant::now();
    let result = retry_desktop_connection(message, owns_start_lock).await;
    match &result {
        Ok(_) => host_log::line(format!(
            "desktop answered after {} ms",
            started_at.elapsed().as_millis()
        )),
        Err(error) => host_log::line(format!(
            "desktop did not answer within {} ms: {error:#}",
            started_at.elapsed().as_millis()
        )),
    }
    result
}

async fn retry_desktop_connection(
    message: &ExtensionToDesktopMessage,
    owns_start_lock: bool,
) -> Result<DesktopToExtensionMessage> {
    let started_at = Instant::now();
    let mut last_error = None;
    while started_at.elapsed() < START_WAIT_TOTAL {
        match forward_to_desktop(message).await {
            Ok(response) => {
                if owns_start_lock {
                    cleanup_start_lock();
                }
                return Ok(response);
            }
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(start_retry_delay(started_at.elapsed())).await;
    }

    if owns_start_lock {
        cleanup_start_lock();
    }

    match last_error {
        Some(error) => Err(error).context("desktop did not open its local pipe in time"),
        None => bail!("desktop did not open its local pipe in time"),
    }
}

async fn forward_to_desktop(
    message: &ExtensionToDesktopMessage,
) -> Result<DesktopToExtensionMessage> {
    let mut pipe = ClientOptions::new()
        .open(PIPE_NAME)
        .context("failed to open local desktop pipe")?;

    let mut payload = serde_json::to_vec(message).context("failed to serialize desktop request")?;
    payload.push(b'\n');
    pipe.write_all(&payload)
        .await
        .context("failed to write desktop request")?;
    pipe.flush()
        .await
        .context("failed to flush desktop request")?;

    let mut response = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = pipe
            .read(&mut buffer)
            .await
            .context("failed to read desktop response")?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        match chunk.iter().position(|byte| *byte == b'\n') {
            Some(newline) => {
                response.extend_from_slice(&chunk[..newline]);
                break;
            }
            None => response.extend_from_slice(chunk),
        }
        if response.len() > MAX_NATIVE_MESSAGE_BYTES {
            bail!("desktop response exceeded message size limit");
        }
    }

    serde_json::from_slice(&response).context("failed to parse desktop response")
}

/// Launches the desktop app unless another host instance is already doing so.
/// Returns whether this process owns the start lock.
fn ensure_desktop_started(launch: &DesktopLaunch) -> Result<bool> {
    let lock_path = std::env::temp_dir().join(START_LOCK_NAME);
    let lock = acquire_start_lock(&lock_path)?;

    if lock.is_none() {
        host_log::line("another host instance is already starting the desktop");
        return Ok(false);
    }

    let spawn_result = spawn_desktop(launch);
    drop(lock);

    match spawn_result {
        Ok(child) => {
            host_log::line(format!(
                "launched desktop pid={} via {}",
                child.id(),
                launch.description()
            ));
            Ok(true)
        }
        Err(error) => {
            let _ = std::fs::remove_file(lock_path);
            Err(error).with_context(|| format!("failed to launch {}", launch.description()))
        }
    }
}

fn cleanup_start_lock() {
    let _ = std::fs::remove_file(std::env::temp_dir().join(START_LOCK_NAME));
}

fn start_retry_delay(elapsed: Duration) -> Duration {
    if elapsed < START_RETRY_FAST_WINDOW {
        START_RETRY_FAST_DELAY
    } else {
        START_RETRY_SLOW_DELAY
    }
}

fn acquire_start_lock(lock_path: &Path) -> Result<Option<std::fs::File>> {
    if let Ok(metadata) = std::fs::metadata(lock_path) {
        if is_stale_lock(&metadata) {
            let _ = std::fs::remove_file(lock_path);
        }
    }

    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(lock_path)
    {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(None),
        Err(error) => Err(error).context("failed to create desktop start lock"),
    }
}

fn is_stale_lock(metadata: &std::fs::Metadata) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > START_LOCK_STALE_AFTER)
}

/// Picks how to start the desktop app, in this order:
/// 1. `LDM_DESKTOP_EXE`, when it points at an existing file (packaged build, also for
///    development machines that keep a packaged build installed),
/// 2. `LDM.exe` or `ldm-desktop.exe` next to a packaged native host,
/// 3. `LDM_DESKTOP_COMMAND` with `LDM_DESKTOP_ARGS` and `LDM_DESKTOP_CWD` (development).
///
/// A development command such as `pnpm tauri dev` takes far longer than any extension
/// waits, so it is the last resort rather than the first.
fn resolve_desktop_launch() -> Result<DesktopLaunch> {
    let mut skipped = Vec::new();

    match std::env::var_os(DESKTOP_EXE_ENV).map(PathBuf::from) {
        Some(path) if path.is_file() => {
            return Ok(DesktopLaunch::Exe {
                cwd: resolve_optional_cwd(&path)?,
                path,
            });
        }
        Some(path) => skipped.push(format!(
            "{DESKTOP_EXE_ENV} points to a file that does not exist ({})",
            path.display()
        )),
        None => {}
    }

    if let Some(path) = resolve_installed_desktop_exe()? {
        return Ok(DesktopLaunch::Exe {
            cwd: path.parent().map(PathBuf::from),
            path,
        });
    }

    if let Some(command) = env_var_trimmed(DESKTOP_COMMAND_ENV) {
        let cwd = resolve_required_cwd()?;
        let args = match env_var_trimmed(DESKTOP_ARGS_ENV) {
            Some(value) => split_command_args(&value)?,
            None => Vec::new(),
        };

        return Ok(DesktopLaunch::Command { command, args, cwd });
    }

    let skipped = if skipped.is_empty() {
        String::new()
    } else {
        format!(" ({})", skipped.join("; "))
    };
    bail!(
        "No desktop start configuration is set{skipped}. Install LDM or set {DESKTOP_EXE_ENV} to LDM.exe. For development set {DESKTOP_COMMAND_ENV}, {DESKTOP_ARGS_ENV}, and {DESKTOP_CWD_ENV}."
    )
}

fn resolve_installed_desktop_exe() -> Result<Option<PathBuf>> {
    let current_exe =
        std::env::current_exe().context("failed to resolve native host executable path")?;
    if is_development_binary(&current_exe) {
        return Ok(None);
    }

    let Some(exe_dir) = current_exe.parent() else {
        return Ok(None);
    };

    let mut dirs = vec![exe_dir.to_path_buf()];
    if let Some(parent) = exe_dir.parent() {
        dirs.push(parent.to_path_buf());
    }

    Ok(dirs
        .iter()
        .flat_map(|dir| {
            PRODUCTION_DESKTOP_EXE_NAMES
                .iter()
                .map(move |name| dir.join(name))
        })
        .find(|path| path.is_file()))
}

fn is_development_binary(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("target")
    })
}

fn spawn_desktop(launch: &DesktopLaunch) -> io::Result<std::process::Child> {
    let mut command = match launch {
        DesktopLaunch::Command { command, args, cwd } => {
            let mut child = Command::new(command);
            child.args(args).current_dir(cwd);
            child
        }
        DesktopLaunch::Exe { path, cwd } => {
            let mut child = Command::new(path);
            if let Some(cwd) = cwd {
                child.current_dir(cwd);
            }
            child
        }
    };

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

impl DesktopLaunch {
    fn description(&self) -> String {
        match self {
            DesktopLaunch::Command { command, args, .. } => {
                format!("{DESKTOP_COMMAND_ENV}={command} {}", args.join(" "))
                    .trim()
                    .to_string()
            }
            DesktopLaunch::Exe { path, .. } => path.display().to_string(),
        }
    }
}

fn env_var_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_required_cwd() -> Result<PathBuf> {
    let Some(cwd) = std::env::var_os(DESKTOP_CWD_ENV).map(PathBuf::from) else {
        bail!("{DESKTOP_CWD_ENV} is required when {DESKTOP_COMMAND_ENV} is used");
    };

    if cwd.is_dir() {
        Ok(cwd)
    } else {
        bail!("{DESKTOP_CWD_ENV} points to a directory that does not exist")
    }
}

fn resolve_optional_cwd(exe_path: &Path) -> Result<Option<PathBuf>> {
    if let Some(cwd) = std::env::var_os(DESKTOP_CWD_ENV).map(PathBuf::from) {
        if cwd.is_dir() {
            return Ok(Some(cwd));
        }
        bail!("{DESKTOP_CWD_ENV} points to a directory that does not exist");
    }

    Ok(exe_path.parent().map(PathBuf::from))
}

fn split_command_args(value: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '\\' if matches!(chars.peek(), Some('"')) => {
                current.push('"');
                chars.next();
            }
            ch if ch.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }

    if in_quotes {
        bail!("{DESKTOP_ARGS_ENV} contains an unterminated quote");
    }

    if !current.is_empty() {
        args.push(current);
    }

    Ok(args)
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use super::{is_development_binary, split_command_args, start_retry_delay};

    #[test]
    fn splits_desktop_command_args() {
        let args = split_command_args("pnpm --filter @ldm/desktop tauri dev").unwrap();

        assert_eq!(args, ["pnpm", "--filter", "@ldm/desktop", "tauri", "dev"]);
    }

    #[test]
    fn preserves_quoted_args() {
        let args = split_command_args(r#"pnpm --filter "@ldm/desktop" "tauri dev""#).unwrap();

        assert_eq!(args, ["pnpm", "--filter", "@ldm/desktop", "tauri dev"]);
    }

    #[test]
    fn detects_target_directory_as_development_binary() {
        assert!(is_development_binary(Path::new(
            r"C:\repo\target\debug\ldm-native-host.exe"
        )));
        assert!(!is_development_binary(Path::new(
            r"C:\Program Files\LDM\ldm-native-host.exe"
        )));
    }

    #[test]
    fn retries_more_aggressively_during_desktop_boot() {
        assert_eq!(start_retry_delay(Duration::ZERO), Duration::from_millis(25));
        assert_eq!(
            start_retry_delay(Duration::from_millis(700)),
            Duration::from_millis(25)
        );
        assert_eq!(
            start_retry_delay(Duration::from_millis(800)),
            Duration::from_millis(150)
        );
    }
}
