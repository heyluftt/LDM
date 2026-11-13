//! Fixtures shared by the provider module tests.

use ldm_types::download_job::{CreateDownloadJobRequest, DownloadKind};

use crate::tool_discovery::{ToolKind, ToolStatus};

pub(super) fn download_request(adapter_id: &str) -> CreateDownloadJobRequest {
    CreateDownloadJobRequest {
        source_url: "https://example.invalid/watch?v=owned".to_string(),
        source_fallback_urls: Vec::new(),
        page_url: None,
        page_title: None,
        referrer: None,
        suggested_filename: Some("Owned Video".to_string()),
        content_type: None,
        content_length: None,
        kind: DownloadKind::Video,
        adapter_id: adapter_id.to_string(),
        provider_id: None,
        selected_option_id: None,
        headers: None,
        prefer_range_request: false,
        quality_report: None,
    }
}

pub(super) fn failed_output(stderr: &str) -> std::process::Output {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::Output {
            status: std::process::ExitStatus::from_raw(1),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::Output {
            status: std::process::ExitStatus::from_raw(1),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }
}

pub(super) fn found_tool(kind: ToolKind, path: &str) -> ToolStatus {
    ToolStatus {
        kind,
        name: kind.name(),
        env_var: kind.env_var(),
        required: kind.required(),
        found: true,
        source: Some(crate::tool_discovery::ToolSource::Bundled),
        path: Some(path.to_string()),
        message: String::new(),
    }
}
