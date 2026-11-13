//! Validator sidecar for resumable HTTP downloads.
//!
//! A `.part` file alone does not say which version of a resource it belongs to. The first
//! successful response's `ETag` and `Last-Modified` are stored in `<part>.meta` next to it;
//! a resume sends the stored validator as `If-Range`, so a server that changed the resource
//! answers `200` with the whole file instead of `206` with bytes that no longer fit.

use std::path::{Path, PathBuf};

use reqwest::header::{HeaderMap, ETAG, LAST_MODIFIED};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Appended to the `.part` file name: `video.mp4.part` keeps `video.mp4.part.meta`.
pub const PARTIAL_METADATA_SUFFIX: &str = ".meta";

/// The validators of the response a `.part` file was started from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialDownloadMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

impl PartialDownloadMetadata {
    pub fn from_response(response: &reqwest::Response) -> Self {
        Self::from_headers(response.headers())
    }

    pub fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            etag: header_text(headers, &ETAG),
            last_modified: header_text(headers, &LAST_MODIFIED),
        }
    }

    /// The `If-Range` value for a resume: a strong `ETag`, otherwise `Last-Modified`.
    ///
    /// Weak entity tags (`W/"..."`) are not allowed in `If-Range` (RFC 9110 section 13.1.5),
    /// so they fall through to the date.
    pub fn if_range_value(&self) -> Option<&str> {
        self.etag
            .as_deref()
            .filter(|etag| !etag.starts_with("W/"))
            .or(self.last_modified.as_deref())
    }
}

fn header_text(headers: &HeaderMap, name: &reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// The sidecar path for a `.part` file (same folder, `.meta` appended).
pub fn partial_metadata_path(temp_path: &Path) -> PathBuf {
    let mut filename = temp_path
        .file_name()
        .map(|value| value.to_os_string())
        .unwrap_or_default();
    filename.push(PARTIAL_METADATA_SUFFIX);
    temp_path.with_file_name(filename)
}

/// Reads the sidecar of `temp_path`. A missing or unreadable sidecar is `None`: the resume then
/// runs without `If-Range`, exactly as it did before validators were stored.
pub async fn read_partial_metadata(
    temp_path: &Path,
) -> Result<Option<PartialDownloadMetadata>, CoreError> {
    let bytes = match tokio::fs::read(partial_metadata_path(temp_path)).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(serde_json::from_slice(&bytes).ok())
}

/// Stores `metadata` next to `temp_path`, or removes the sidecar when the response carried no
/// validator at all (nothing to compare against on the next resume).
pub async fn write_partial_metadata(
    temp_path: &Path,
    metadata: &PartialDownloadMetadata,
) -> Result<(), CoreError> {
    if metadata.if_range_value().is_none() {
        return remove_partial_metadata(temp_path).await;
    }
    let text = serde_json::to_vec(metadata).map_err(|error| {
        CoreError::Io(std::io::Error::other(format!(
            "could not encode partial download metadata: {error}"
        )))
    })?;
    tokio::fs::write(partial_metadata_path(temp_path), text).await?;
    Ok(())
}

pub async fn remove_partial_metadata(temp_path: &Path) -> Result<(), CoreError> {
    match tokio::fs::remove_file(partial_metadata_path(temp_path)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    #[test]
    fn sidecar_sits_next_to_the_part_file() {
        let path = partial_metadata_path(Path::new(r"C:\Videos\.ldm-partials\clip.mp4.part"));

        assert_eq!(
            path,
            Path::new(r"C:\Videos\.ldm-partials\clip.mp4.part.meta")
        );
    }

    #[test]
    fn strong_etag_wins_over_last_modified() {
        let metadata = PartialDownloadMetadata {
            etag: Some("\"abc\"".to_string()),
            last_modified: Some("Tue, 20 Oct 2026 10:00:00 GMT".to_string()),
        };

        assert_eq!(metadata.if_range_value(), Some("\"abc\""));
    }

    #[test]
    fn weak_etag_falls_back_to_last_modified() {
        let metadata = PartialDownloadMetadata {
            etag: Some("W/\"abc\"".to_string()),
            last_modified: Some("Tue, 20 Oct 2026 10:00:00 GMT".to_string()),
        };

        assert_eq!(
            metadata.if_range_value(),
            Some("Tue, 20 Oct 2026 10:00:00 GMT")
        );
        assert_eq!(
            PartialDownloadMetadata {
                etag: Some("W/\"abc\"".to_string()),
                last_modified: None,
            }
            .if_range_value(),
            None
        );
    }

    #[test]
    fn reads_validators_from_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(ETAG, HeaderValue::from_static("\"v1\""));
        headers.insert(
            LAST_MODIFIED,
            HeaderValue::from_static(" Mon, 01 Jan 2024 00:00:00 GMT "),
        );

        let metadata = PartialDownloadMetadata::from_headers(&headers);

        assert_eq!(metadata.etag.as_deref(), Some("\"v1\""));
        assert_eq!(
            metadata.last_modified.as_deref(),
            Some("Mon, 01 Jan 2024 00:00:00 GMT")
        );
        assert_eq!(
            PartialDownloadMetadata::from_headers(&HeaderMap::new()),
            PartialDownloadMetadata::default()
        );
    }

    #[tokio::test]
    async fn round_trips_through_the_sidecar_file() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("clip.mp4.part");
        let metadata = PartialDownloadMetadata {
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
        };

        write_partial_metadata(&temp_path, &metadata).await.unwrap();
        assert_eq!(
            read_partial_metadata(&temp_path).await.unwrap(),
            Some(metadata)
        );

        remove_partial_metadata(&temp_path).await.unwrap();
        assert_eq!(read_partial_metadata(&temp_path).await.unwrap(), None);
        remove_partial_metadata(&temp_path).await.unwrap();
    }

    #[tokio::test]
    async fn response_without_validators_leaves_no_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("clip.mp4.part");
        write_partial_metadata(
            &temp_path,
            &PartialDownloadMetadata {
                etag: Some("\"old\"".to_string()),
                last_modified: None,
            },
        )
        .await
        .unwrap();

        write_partial_metadata(&temp_path, &PartialDownloadMetadata::default())
            .await
            .unwrap();

        assert!(!partial_metadata_path(&temp_path).exists());
    }

    #[tokio::test]
    async fn corrupt_sidecar_reads_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("clip.mp4.part");
        std::fs::write(partial_metadata_path(&temp_path), b"not json").unwrap();

        assert_eq!(read_partial_metadata(&temp_path).await.unwrap(), None);
    }
}
