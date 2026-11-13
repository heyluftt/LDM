use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::download_job::DownloadKind;

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[derive(TS)]
#[ts(export)]
pub enum DetectionSource {
    Html5Video,
    DownloadLink,
    Hls,
    Dash,
    Provider,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export, optional_fields)]
pub struct DetectionContext {
    pub page_url: String,
    pub page_title: Option<String>,
    pub referrer: Option<String>,
    pub document_content_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export, optional_fields)]
pub struct DetectedDownloadCandidate {
    pub id: String,
    pub url: String,
    #[serde(default)]
    #[ts(as = "Option<_>")]
    pub source_fallback_urls: Vec<String>,
    pub source: DetectionSource,
    pub quality_height: Option<u32>,
    pub bitrate: Option<u64>,
    pub quality_label: Option<String>,
    #[ts(optional, type = "Record<string, string>")]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    pub title: Option<String>,
    pub page_url: Option<String>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub suggested_filename: Option<String>,
    pub adapter_id: String,
    pub provider_id: Option<String>,
    pub requires_usage_notice: Option<bool>,
    pub kind: DownloadKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export, optional_fields)]
pub struct DownloadOption {
    pub id: String,
    pub label: String,
    pub url: String,
    pub kind: DownloadKind,
    pub content_type: Option<String>,
    pub estimated_bytes: Option<u64>,
    pub quality_label: Option<String>,
    pub requires_external_tool: Option<bool>,
}

/// Classification of an adapter id (`DownloadJob::adapter_id`,
/// `CreateDownloadJobRequest::adapter_id`). Adapter ids are opaque strings chosen by the
/// extension; everything that is not a stream manifest or a provider page is downloaded over
/// plain HTTP by the core.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum AdapterKind {
    /// `generic-html5-video`, `generic-download-links`, `browser-download`, and any id the
    /// desktop app does not know.
    DirectHttp,
    /// `hls`: an HLS manifest, downloaded natively or through the external backend.
    Hls,
    /// `dash`: a DASH manifest, always downloaded through the external backend.
    Dash,
    /// `provider-<name>`: a provider page handled by the external backend. Holds `<name>`.
    Provider(String),
}

impl AdapterKind {
    pub const HLS_ID: &'static str = "hls";
    pub const DASH_ID: &'static str = "dash";
    pub const PROVIDER_ID_PREFIX: &'static str = "provider-";

    pub fn parse(adapter_id: &str) -> Self {
        if adapter_id == Self::HLS_ID {
            Self::Hls
        } else if adapter_id == Self::DASH_ID {
            Self::Dash
        } else if let Some(provider) = adapter_id.strip_prefix(Self::PROVIDER_ID_PREFIX) {
            Self::Provider(provider.to_string())
        } else {
            Self::DirectHttp
        }
    }

    /// Plain HTTP download run by the core itself.
    pub fn is_direct_http(&self) -> bool {
        matches!(self, Self::DirectHttp)
    }

    /// Streams and provider pages go through the external provider backend (yt-dlp), unless
    /// the native HLS path can take the job.
    pub fn uses_external_backend(&self) -> bool {
        !self.is_direct_http()
    }

    /// Direct downloads and native HLS keep a resumable temp file; provider and DASH jobs
    /// can only be canceled and retried.
    pub fn supports_pause_resume(&self) -> bool {
        matches!(self, Self::DirectHttp | Self::Hls)
    }

    /// HLS or DASH manifest.
    pub fn is_stream(&self) -> bool {
        matches!(self, Self::Hls | Self::Dash)
    }

    pub fn is_hls(&self) -> bool {
        matches!(self, Self::Hls)
    }

    pub fn is_provider(&self) -> bool {
        matches!(self, Self::Provider(_))
    }
}

#[cfg(test)]
mod tests {
    use super::AdapterKind;

    #[test]
    fn parses_known_adapter_ids() {
        assert_eq!(AdapterKind::parse("hls"), AdapterKind::Hls);
        assert_eq!(AdapterKind::parse("dash"), AdapterKind::Dash);
        assert_eq!(
            AdapterKind::parse("provider-youtube"),
            AdapterKind::Provider("youtube".to_string())
        );
        assert_eq!(
            AdapterKind::parse("provider-x"),
            AdapterKind::Provider("x".to_string())
        );
    }

    #[test]
    fn treats_direct_and_unknown_adapter_ids_as_direct_http() {
        for id in [
            "generic-html5-video",
            "generic-download-links",
            "browser-download",
            "something-new",
            "",
            "HLS",
        ] {
            assert_eq!(AdapterKind::parse(id), AdapterKind::DirectHttp, "{id}");
        }
    }

    #[test]
    fn classifies_backend_and_controls() {
        let direct = AdapterKind::DirectHttp;
        let hls = AdapterKind::Hls;
        let dash = AdapterKind::Dash;
        let provider = AdapterKind::Provider("tiktok".to_string());

        assert!(direct.is_direct_http());
        assert!(!hls.is_direct_http());
        assert!(!dash.is_direct_http());
        assert!(!provider.is_direct_http());

        assert!(!direct.uses_external_backend());
        assert!(hls.uses_external_backend());
        assert!(dash.uses_external_backend());
        assert!(provider.uses_external_backend());

        assert!(direct.supports_pause_resume());
        assert!(hls.supports_pause_resume());
        assert!(!dash.supports_pause_resume());
        assert!(!provider.supports_pause_resume());

        assert!(!direct.is_stream());
        assert!(hls.is_stream());
        assert!(dash.is_stream());
        assert!(!provider.is_stream());
    }
}
