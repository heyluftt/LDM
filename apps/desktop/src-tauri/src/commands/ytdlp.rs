//! yt-dlp version check for the settings view: the installed version from `yt-dlp --version`
//! and the latest release tag from GitHub. Updating stays manual; the UI shows the command.

use std::{path::Path, process::Stdio, time::Duration};

use ldm_core::tool_discovery::{discover_tool, ToolKind};
use reqwest::header::ACCEPT;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
const USER_AGENT: &str = concat!("LDM/", env!("CARGO_PKG_VERSION"), " (yt-dlp update check)");
/// Keeps the console window of the child process hidden, as the provider backend does.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YtDlpVersion {
    /// Output of `yt-dlp --version`, for example `2026.03.14`.
    pub version: Option<String>,
    pub path: Option<String>,
    /// How this installation is updated by hand.
    pub update_command: String,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn ytdlp_version() -> YtDlpVersion {
    let tool = discover_tool(ToolKind::YtDlp);
    let Some(path) = tool.path else {
        return YtDlpVersion {
            version: None,
            path: None,
            update_command: update_command_for(None).to_string(),
            error: Some(tool.message),
        };
    };
    let update_command = update_command_for(Some(Path::new(&path))).to_string();
    match run_version_command(&path).await {
        Ok(version) => YtDlpVersion {
            version: Some(version),
            path: Some(path),
            update_command,
            error: None,
        },
        Err(error) => YtDlpVersion {
            version: None,
            path: Some(path),
            update_command,
            error: Some(error),
        },
    }
}

async fn run_version_command(path: &str) -> Result<String, String> {
    let mut command = tokio::process::Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            format!(
                "yt-dlp --version did not finish within {} s.",
                VERSION_TIMEOUT.as_secs()
            )
        })?
        .map_err(|error| format!("Could not run yt-dlp: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "yt-dlp --version failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        Err("yt-dlp --version printed nothing.".to_string())
    } else {
        Ok(version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestYtDlpVersion {
    /// The latest release tag, for example `2026.04.01`.
    pub version: String,
    /// Whether `installed` is older than `version`; `None` without an installed version or
    /// when the two are not comparable (a non-numeric part).
    pub update_available: Option<bool>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

/// Looks up the latest yt-dlp release on GitHub and compares it with `installed`.
#[tauri::command]
pub async fn latest_ytdlp_version(
    installed: Option<String>,
) -> Result<LatestYtDlpVersion, AppError> {
    let client = reqwest::Client::builder()
        .timeout(VERSION_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| AppError::internal(format!("Could not build the HTTP client: {error}")))?;
    let response = client
        .get(LATEST_RELEASE_URL)
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| AppError::network(format!("Could not reach GitHub: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::network(format!(
            "GitHub answered with {}.",
            response.status()
        )));
    }
    let release: GithubRelease = response
        .json()
        .await
        .map_err(|error| AppError::network(format!("Unexpected GitHub response: {error}")))?;

    let version = release.tag_name.trim().trim_start_matches('v').to_string();
    let update_available = installed
        .as_deref()
        .and_then(|installed| is_older_version(installed, &version));
    Ok(LatestYtDlpVersion {
        version,
        update_available,
    })
}

/// `pip install -U yt-dlp` for a pip layout (`...\Scripts\yt-dlp.exe`), `yt-dlp -U` otherwise.
/// A standalone executable updates itself; a pip-installed one would be overwritten by the next
/// `pip install`, so pip has to do it.
pub fn update_command_for(path: Option<&Path>) -> &'static str {
    let is_pip_layout = path
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .is_some_and(|dir| dir.eq_ignore_ascii_case("Scripts"));
    if is_pip_layout {
        "pip install -U yt-dlp"
    } else {
        "yt-dlp -U"
    }
}

/// Compares dotted numeric versions such as `2026.03.14` or `2026.03.14.232958` (nightly).
/// `None` when either side has a part that is not a number.
pub fn is_older_version(installed: &str, latest: &str) -> Option<bool> {
    let parse = |value: &str| {
        value
            .trim()
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    Some(parse(installed)? < parse(latest)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_layout_updates_through_pip() {
        assert_eq!(
            update_command_for(Some(Path::new(r"C:\Coding\Python\Scripts\yt-dlp.exe"))),
            "pip install -U yt-dlp"
        );
        assert_eq!(
            update_command_for(Some(Path::new(r"C:\project\.venv\scripts\yt-dlp.exe"))),
            "pip install -U yt-dlp"
        );
    }

    #[test]
    fn standalone_executable_updates_itself() {
        assert_eq!(
            update_command_for(Some(Path::new(r"C:\Tools\yt-dlp\yt-dlp.exe"))),
            "yt-dlp -U"
        );
        assert_eq!(
            update_command_for(Some(Path::new(
                r"C:\Users\x\AppData\Local\LDM\resources\tools\yt-dlp.exe"
            ))),
            "yt-dlp -U"
        );
        assert_eq!(update_command_for(None), "yt-dlp -U");
    }

    #[test]
    fn compares_release_versions_numerically() {
        assert_eq!(is_older_version("2026.03.14", "2026.04.01"), Some(true));
        assert_eq!(is_older_version("2026.04.01", "2026.04.01"), Some(false));
        assert_eq!(is_older_version("2026.10.02", "2026.09.30"), Some(false));
        assert_eq!(is_older_version("2025.12.31", "2026.01.01"), Some(true));
        assert_eq!(is_older_version("2026.3.14", "2026.03.14"), Some(false));
    }

    #[test]
    fn nightly_builds_are_not_older_than_the_release_of_their_day() {
        assert_eq!(
            is_older_version("2026.03.14.232958", "2026.03.14"),
            Some(false)
        );
        assert_eq!(
            is_older_version("2026.03.13.232958", "2026.03.14"),
            Some(true)
        );
    }

    #[test]
    fn non_numeric_versions_are_not_compared() {
        assert_eq!(is_older_version("unknown", "2026.03.14"), None);
        assert_eq!(is_older_version("2026.03.14", "latest"), None);
        assert_eq!(is_older_version("", "2026.03.14"), None);
    }
}
