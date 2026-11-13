//! Tells the user when a newer LDM has been released.
//!
//! LDM does not update itself. It asks GitHub for the latest release of its own repository,
//! compares that with the running version, and offers to open the release page, where the
//! user downloads the installer as the first time. The repository is the one named in
//! `Cargo.toml`, so a fork looks at its own releases.

use std::time::Duration;

use reqwest::{header::ACCEPT, StatusCode};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// `https://github.com/<owner>/<name>`, from `[workspace.package]` in the root `Cargo.toml`.
const REPOSITORY_URL: &str = env!("CARGO_PKG_REPOSITORY");
const GITHUB_PREFIX: &str = "https://github.com/";
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("LDM/", env!("CARGO_PKG_VERSION"), " (update check)");

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The version that is running.
    pub current_version: String,
    /// The latest released version; the running one when nothing has been released yet.
    pub latest_version: String,
    pub update_available: bool,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

/// Asks GitHub for the latest release and compares it with the running version.
#[tauri::command]
pub async fn check_for_update() -> Result<UpdateStatus, AppError> {
    let slug = repository_slug(REPOSITORY_URL).ok_or_else(|| {
        AppError::internal("This build does not name a GitHub repository to check for updates.")
    })?;
    let client = reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| AppError::internal(format!("Could not build the HTTP client: {error}")))?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{slug}/releases/latest"
        ))
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| AppError::network(format!("Could not reach GitHub: {error}")))?;

    // Drafts and pre-releases do not count as "latest"; a repository without a published
    // release answers 404.
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(update_status(CURRENT_VERSION, None));
    }
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

    Ok(update_status(CURRENT_VERSION, Some(&release.tag_name)))
}

/// Opens the page of the latest release in the default browser. The address is fixed by the
/// build; nothing the webview or the network supplied goes into it.
#[tauri::command]
pub fn open_release_page() -> Result<(), AppError> {
    let page = release_page_url(REPOSITORY_URL).ok_or_else(|| {
        AppError::internal("This build does not name a GitHub repository to open.")
    })?;
    std::process::Command::new("explorer.exe")
        .arg(page)
        .spawn()
        .map(|_| ())
        .map_err(|error| AppError::internal(format!("Could not open the release page: {error}")))
}

fn update_status(current: &str, latest_tag: Option<&str>) -> UpdateStatus {
    let latest = latest_tag
        .map(|tag| tag.trim().trim_start_matches('v').to_string())
        .filter(|version| !version.is_empty());
    let update_available = latest
        .as_deref()
        .is_some_and(|latest| is_newer(latest, current));
    UpdateStatus {
        current_version: current.to_string(),
        latest_version: latest.unwrap_or_else(|| current.to_string()),
        update_available,
    }
}

/// `owner/name` of a `https://github.com/owner/name` address.
fn repository_slug(repository: &str) -> Option<&str> {
    let slug = repository
        .strip_prefix(GITHUB_PREFIX)?
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let (owner, name) = slug.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(name)).then_some(slug)
}

fn release_page_url(repository: &str) -> Option<String> {
    repository_slug(repository).map(|slug| format!("{GITHUB_PREFIX}{slug}/releases/latest"))
}

/// Whether `candidate` is a later version than `current`. Versions are `major.minor.patch`
/// with an optional `-suffix` for a pre-release, which sorts before the release itself.
/// Anything that does not read as a version is not newer.
fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// The numbers of a version, followed by whether it is a release (`true`) or a pre-release.
fn parse_version(version: &str) -> Option<([u64; 3], bool)> {
    let version = version.trim().trim_start_matches('v');
    let without_build = version.split('+').next()?;
    let (numbers, pre_release) = match without_build.split_once('-') {
        Some((numbers, _)) => (numbers, true),
        None => (without_build, false),
    };
    let mut parts = numbers.split('.').map(|part| part.parse::<u64>().ok());
    let parsed = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some((parsed, !pre_release))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_version_is_an_update_and_the_same_or_an_older_one_is_not() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
    }

    #[test]
    fn a_pre_release_comes_before_its_release() {
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
        assert!(is_newer("0.2.0-beta.1", "0.1.0"));
    }

    #[test]
    fn what_is_not_a_version_is_never_an_update() {
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("1.2", "0.1.0"));
        assert!(!is_newer("1.2.3.4", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
    }

    #[test]
    fn the_release_tag_is_compared_without_its_v() {
        let status = update_status("0.1.0", Some("v0.2.0"));

        assert_eq!(
            status,
            UpdateStatus {
                current_version: "0.1.0".to_string(),
                latest_version: "0.2.0".to_string(),
                update_available: true,
            }
        );
    }

    #[test]
    fn no_release_yet_means_no_update() {
        let status = update_status("0.1.0", None);

        assert!(!status.update_available);
        assert_eq!(status.latest_version, "0.1.0");
    }

    #[test]
    fn the_repository_address_gives_the_api_slug_and_the_release_page() {
        assert_eq!(
            repository_slug("https://github.com/heyluftt/LDM"),
            Some("heyluftt/LDM")
        );
        assert_eq!(
            repository_slug("https://github.com/heyluftt/LDM.git"),
            Some("heyluftt/LDM")
        );
        assert_eq!(
            release_page_url("https://github.com/heyluftt/LDM/").as_deref(),
            Some("https://github.com/heyluftt/LDM/releases/latest")
        );
        assert_eq!(repository_slug("https://example.com/heyluftt/LDM"), None);
        assert_eq!(repository_slug("https://github.com/heyluftt"), None);
        assert_eq!(
            repository_slug("https://github.com/heyluftt/LDM/../../evil"),
            None
        );
        assert_eq!(repository_slug(""), None);
    }

    #[test]
    fn this_build_names_a_repository_and_a_version() {
        assert!(
            repository_slug(REPOSITORY_URL).is_some(),
            "{REPOSITORY_URL}"
        );
        assert!(
            parse_version(CURRENT_VERSION).is_some(),
            "{CURRENT_VERSION}"
        );
    }
}
