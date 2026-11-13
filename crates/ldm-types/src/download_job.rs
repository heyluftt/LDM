use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::adapter::AdapterKind;

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
#[derive(TS)]
#[ts(export)]
pub enum DownloadKind {
    Video,
    Audio,
    Program,
    Document,
    Archive,
    Image,
    Other,
}

impl DownloadKind {
    /// Every kind, in the order the settings UI lists them.
    pub const ALL: [Self; 7] = [
        Self::Video,
        Self::Audio,
        Self::Image,
        Self::Program,
        Self::Archive,
        Self::Document,
        Self::Other,
    ];
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[derive(TS)]
#[ts(export)]
pub enum DownloadStatus {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export, optional_fields)]
pub struct CreateDownloadJobRequest {
    pub source_url: String,
    #[serde(default)]
    #[ts(as = "Option<_>")]
    pub source_fallback_urls: Vec<String>,
    pub page_url: Option<String>,
    pub page_title: Option<String>,
    pub referrer: Option<String>,
    pub suggested_filename: Option<String>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub kind: DownloadKind,
    pub adapter_id: String,
    pub provider_id: Option<String>,
    pub selected_option_id: Option<String>,
    #[ts(optional, type = "Record<string, string>")]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    #[ts(as = "Option<_>")]
    pub prefer_range_request: bool,
    pub quality_report: Option<String>,
}

impl CreateDownloadJobRequest {
    pub fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::parse(&self.adapter_id)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(TS)]
#[ts(export, optional_fields)]
pub struct DownloadJob {
    pub id: Uuid,
    pub source_url: String,
    pub final_url: Option<String>,
    pub filename: String,
    pub target_directory: String,
    pub temp_path: String,
    pub kind: DownloadKind,
    pub adapter_id: String,
    pub provider_id: Option<String>,
    pub selected_option_id: Option<String>,
    pub request_content_type: Option<String>,
    pub request_content_length: Option<u64>,
    pub source_fallback_count: usize,
    pub forwarded_header_names: Vec<String>,
    pub prefer_range_request: bool,
    pub referrer_present: bool,
    pub page_url: Option<String>,
    pub status: DownloadStatus,
    pub progress_bytes: u64,
    pub total_bytes: Option<u64>,
    pub progress_percent: Option<f64>,
    pub speed_bytes_per_second: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub supports_resume: bool,
    pub retry_count: u32,
    pub error: Option<String>,
    pub error_report: Option<String>,
    pub quality_report: Option<String>,
    /// Unix seconds. Older state files stored these as strings; see `timestamp`.
    #[serde(deserialize_with = "crate::timestamp::deserialize_unix_seconds")]
    pub created_at: u64,
    #[serde(deserialize_with = "crate::timestamp::deserialize_unix_seconds")]
    pub updated_at: u64,
}

impl DownloadJob {
    pub fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::parse(&self.adapter_id)
    }
}
