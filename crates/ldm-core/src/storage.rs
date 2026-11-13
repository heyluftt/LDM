use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use directories::UserDirs;
use ldm_types::download_job::DownloadKind;
use ldm_types::download_settings::DownloadSettings;

use crate::error::CoreError;

/// Where downloads of a kind land when the user has not chosen a folder.
///
/// Media kinds go into the matching Windows library, everything else into the Downloads folder.
pub fn default_target_folder(kind: DownloadKind) -> PathBuf {
    let dirs = UserDirs::new();
    let downloads = library_dir(dirs.as_ref().and_then(UserDirs::download_dir), "Downloads");

    match kind {
        DownloadKind::Video => {
            library_dir(dirs.as_ref().and_then(UserDirs::video_dir), "Videos").join("Downloads")
        }
        DownloadKind::Audio => {
            library_dir(dirs.as_ref().and_then(UserDirs::audio_dir), "Music").join("Downloads")
        }
        DownloadKind::Image => {
            library_dir(dirs.as_ref().and_then(UserDirs::picture_dir), "Pictures").join("Downloads")
        }
        DownloadKind::Program => downloads.join("Programs"),
        DownloadKind::Archive => downloads.join("Compressed"),
        DownloadKind::Document => downloads.join("Documents"),
        DownloadKind::Other => downloads.join("Other"),
    }
}

pub fn default_target_folders() -> BTreeMap<DownloadKind, PathBuf> {
    DownloadKind::ALL
        .iter()
        .map(|kind| (*kind, default_target_folder(*kind)))
        .collect()
}

/// The folder a new download of `kind` goes to: the user's override when set, else the default.
pub fn target_folder(kind: DownloadKind, settings: &DownloadSettings) -> PathBuf {
    settings
        .target_folder_override(kind)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_target_folder(kind))
}

/// Checks a folder the user typed or picked before it is stored as an override.
pub fn validate_target_folder(value: &str) -> Result<PathBuf, CoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidRequest(
            "target folder must not be empty".to_string(),
        ));
    }

    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(CoreError::InvalidRequest(
            "target folder must be an absolute path".to_string(),
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(CoreError::InvalidRequest(
            "target folder must not contain '..'".to_string(),
        ));
    }
    if path.exists() && !path.is_dir() {
        return Err(CoreError::InvalidRequest(
            "target folder points to a file".to_string(),
        ));
    }

    Ok(path)
}

fn library_dir(known: Option<&Path>, fallback_name: &str) -> PathBuf {
    known.map(Path::to_path_buf).unwrap_or_else(|| {
        UserDirs::new()
            .map(|dirs| dirs.home_dir().join(fallback_name))
            .unwrap_or_else(|| PathBuf::from(fallback_name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins_over_default() {
        let settings = DownloadSettings {
            target_folders: BTreeMap::from([(DownloadKind::Video, r"D:\Media\Video".to_string())]),
            ..Default::default()
        };

        assert_eq!(
            target_folder(DownloadKind::Video, &settings),
            PathBuf::from(r"D:\Media\Video")
        );
        assert_eq!(
            target_folder(DownloadKind::Audio, &settings),
            default_target_folder(DownloadKind::Audio)
        );
    }

    #[test]
    fn defaults_cover_every_kind_with_distinct_folders() {
        let defaults = default_target_folders();

        assert_eq!(defaults.len(), DownloadKind::ALL.len());
        let unique = defaults.values().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), defaults.len());
        assert!(defaults.values().all(|path| path.is_absolute()));
    }

    #[test]
    fn rejects_relative_empty_and_parent_paths() {
        assert!(validate_target_folder("").is_err());
        assert!(validate_target_folder("Downloads").is_err());
        assert!(validate_target_folder(r"C:\Users\Example\..\Other").is_err());
    }

    #[test]
    fn accepts_absolute_folder_that_does_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("new-folder");

        assert_eq!(
            validate_target_folder(&missing.to_string_lossy()).unwrap(),
            missing
        );
    }

    #[test]
    fn rejects_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"x").unwrap();

        assert!(validate_target_folder(&file.to_string_lossy()).is_err());
    }
}
