use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

use crate::error::AppError;

const STARTUP_VALUE_NAME: &str = "LDM";
const BACKGROUND_ARG: &str = "--background";
const AUTOSTART_EXE_ENV: &str = "LDM_AUTOSTART_EXE";
const DESKTOP_EXE_ENV: &str = "LDM_DESKTOP_EXE";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupStatus {
    pub available: bool,
    pub enabled: bool,
    pub reason: Option<String>,
}

#[tauri::command]
pub fn startup_status(_app: AppHandle) -> StartupStatus {
    startup_status_inner()
}

#[tauri::command]
pub fn set_startup_enabled(_app: AppHandle, enabled: bool) -> Result<StartupStatus, AppError> {
    if enabled {
        // Why autostart is not possible here is something the user can fix (a wrong variable,
        // a development build); a registry failure is not.
        let command = startup_command().map_err(AppError::invalid_request)?;
        write_startup_value(&command).map_err(AppError::internal)?;
    } else {
        delete_startup_value().map_err(AppError::internal)?;
    }

    Ok(startup_status_inner())
}

pub fn is_background_launch() -> bool {
    std::env::args().any(|arg| arg == BACKGROUND_ARG)
}

fn startup_status_inner() -> StartupStatus {
    let command = startup_command();
    let current_value = read_startup_value();
    let reason = command.as_ref().err().cloned();
    let enabled = match (command.as_ref(), current_value.as_ref()) {
        (Ok(command), Ok(Some(current))) => current == command,
        _ => false,
    };

    StartupStatus {
        available: command.is_ok(),
        enabled,
        reason,
    }
}

fn startup_command() -> Result<String, String> {
    let exe = startup_exe_path()?;
    Ok(format!("\"{}\" {BACKGROUND_ARG}", exe.display()))
}

fn startup_exe_path() -> Result<PathBuf, String> {
    for name in [AUTOSTART_EXE_ENV, DESKTOP_EXE_ENV] {
        if let Some(path) = std::env::var_os(name).map(PathBuf::from) {
            if path.is_file() {
                return Ok(path);
            }
            return Err(format!("{name} points to a file that does not exist."));
        }
    }

    let current_exe =
        std::env::current_exe().map_err(|error| format!("Could not resolve LDM.exe: {error}"))?;
    if is_debug_target_exe(&current_exe) {
        return Err(
            "Autostart needs a bundled LDM.exe. The current development build depends on the dev server."
                .to_string(),
        );
    }

    Ok(current_exe)
}

fn is_debug_target_exe(path: &Path) -> bool {
    let mut saw_target = false;
    for component in path.components() {
        let value = component.as_os_str().to_string_lossy();
        if value.eq_ignore_ascii_case("target") {
            saw_target = true;
            continue;
        }
        if saw_target && value.eq_ignore_ascii_case("debug") {
            return true;
        }
    }

    false
}

#[cfg(windows)]
fn read_startup_value() -> Result<Option<String>, String> {
    crate::windows_registry::read_string_value(RUN_KEY, STARTUP_VALUE_NAME)
        .map_err(|error| format!("Could not read Windows startup setting: {error}"))
}

#[cfg(not(windows))]
fn read_startup_value() -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(windows)]
fn write_startup_value(command: &str) -> Result<(), String> {
    crate::windows_registry::write_string_value(RUN_KEY, STARTUP_VALUE_NAME, command)
        .map_err(|error| format!("Could not save Windows startup setting: {error}"))
}

#[cfg(not(windows))]
fn write_startup_value(_command: &str) -> Result<(), String> {
    Err("Autostart is only available on Windows.".to_string())
}

#[cfg(windows)]
fn delete_startup_value() -> Result<(), String> {
    crate::windows_registry::delete_value(RUN_KEY, STARTUP_VALUE_NAME)
        .map_err(|error| format!("Could not remove Windows startup setting: {error}"))
}

#[cfg(not(windows))]
fn delete_startup_value() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::is_debug_target_exe;

    #[test]
    fn detects_debug_target_exe() {
        assert!(is_debug_target_exe(Path::new(
            r"C:\repo\target\debug\ldm-desktop.exe"
        )));
        assert!(!is_debug_target_exe(Path::new(
            r"C:\Program Files\LDM\LDM.exe"
        )));
    }
}
