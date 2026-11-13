use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::download_job::DownloadKind;

pub const DOWNLOAD_SETTINGS_FILENAME: &str = "download-settings.json";

/// Downloads that run at the same time when the user has not chosen a limit.
pub const DEFAULT_MAX_CONCURRENT_DOWNLOADS: u8 = 3;
/// Smallest and largest limit the settings accept; other values are clamped.
pub const MIN_MAX_CONCURRENT_DOWNLOADS: u8 = 1;
pub const MAX_MAX_CONCURRENT_DOWNLOADS: u8 = 8;

/// User-editable download settings, persisted by the desktop app.
///
/// `target_folders` only holds kinds the user overrode; every other kind falls back to the
/// built-in default folder for that kind.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSettings {
    #[serde(default)]
    pub target_folders: BTreeMap<DownloadKind, String>,
    /// How many downloads run at once; the rest wait as `queued`. See
    /// `effective_max_concurrent_downloads` for the clamped value.
    #[serde(default = "default_max_concurrent_downloads")]
    pub max_concurrent_downloads: u8,
}

impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            target_folders: BTreeMap::new(),
            max_concurrent_downloads: DEFAULT_MAX_CONCURRENT_DOWNLOADS,
        }
    }
}

fn default_max_concurrent_downloads() -> u8 {
    DEFAULT_MAX_CONCURRENT_DOWNLOADS
}

impl DownloadSettings {
    pub fn target_folder_override(&self, kind: DownloadKind) -> Option<&str> {
        self.target_folders
            .get(&kind)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
    }

    /// `max_concurrent_downloads` clamped to the supported range.
    pub fn effective_max_concurrent_downloads(&self) -> usize {
        clamp_max_concurrent_downloads(self.max_concurrent_downloads) as usize
    }
}

pub fn clamp_max_concurrent_downloads(value: u8) -> u8 {
    value.clamp(MIN_MAX_CONCURRENT_DOWNLOADS, MAX_MAX_CONCURRENT_DOWNLOADS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_kinds_as_lowercase_object_keys() {
        let settings = DownloadSettings {
            target_folders: BTreeMap::from([(DownloadKind::Video, r"D:\Media\Video".to_string())]),
            max_concurrent_downloads: 2,
        };

        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            json,
            r#"{"targetFolders":{"video":"D:\\Media\\Video"},"maxConcurrentDownloads":2}"#
        );

        let parsed: DownloadSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn blank_overrides_count_as_unset() {
        let settings = DownloadSettings {
            target_folders: BTreeMap::from([(DownloadKind::Audio, "   ".to_string())]),
            ..Default::default()
        };

        assert_eq!(settings.target_folder_override(DownloadKind::Audio), None);
        assert_eq!(settings.target_folder_override(DownloadKind::Video), None);
    }

    #[test]
    fn missing_fields_deserialize_to_defaults() {
        let parsed: DownloadSettings = serde_json::from_str("{}").unwrap();
        assert!(parsed.target_folders.is_empty());
        assert_eq!(
            parsed.max_concurrent_downloads,
            DEFAULT_MAX_CONCURRENT_DOWNLOADS
        );

        let legacy: DownloadSettings =
            serde_json::from_str(r#"{"targetFolders":{"video":"D:\\V"}}"#).unwrap();
        assert_eq!(legacy.effective_max_concurrent_downloads(), 3);
    }

    #[test]
    fn clamps_concurrency_to_supported_range() {
        let zero = DownloadSettings {
            max_concurrent_downloads: 0,
            ..Default::default()
        };
        let huge = DownloadSettings {
            max_concurrent_downloads: 200,
            ..Default::default()
        };
        assert_eq!(zero.effective_max_concurrent_downloads(), 1);
        assert_eq!(huge.effective_max_concurrent_downloads(), 8);
        assert_eq!(clamp_max_concurrent_downloads(5), 5);
    }
}
