//! Helpers shared by the provider backend and the native HLS path: temp-file naming, request
//! classification, and the request headers that may be forwarded to a tool.

use std::path::Path;

use ldm_types::{adapter::AdapterKind, download_job::CreateDownloadJobRequest};
use uuid::Uuid;

/// Browser request headers forwarded to yt-dlp, FFmpeg, and manifest inspection. Cookies and
/// authorization never leave the browser.
pub(crate) const FORWARDED_HEADER_NAMES: &[&str] =
    &["accept", "accept-language", "origin", "user-agent"];

pub(crate) fn provider_temp_stem(job_id: Uuid) -> String {
    format!("ldm-{}", job_id.simple())
}

pub(crate) fn filename_with_extension(filename: &str, extension: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("video");

    format!("{stem}.{extension}")
}

pub(crate) fn first_non_empty<'a>(first: &'a str, second: &'a str, fallback: &'a str) -> &'a str {
    if !first.is_empty() {
        first
    } else if !second.is_empty() {
        second
    } else {
        fallback
    }
}

pub(crate) fn source_referrer(request: &CreateDownloadJobRequest) -> Option<&str> {
    request
        .referrer
        .as_deref()
        .or(request.page_url.as_deref())
        .filter(|value| !value.trim().is_empty())
}

/// The request headers in `FORWARDED_HEADER_NAMES`, names lower-cased.
pub(crate) fn forwarded_headers(
    request: &CreateDownloadJobRequest,
) -> impl Iterator<Item = (String, &str)> {
    request
        .headers
        .iter()
        .flat_map(|headers| headers.iter())
        .filter_map(|(name, value)| {
            let normalized = name.trim().to_ascii_lowercase();
            FORWARDED_HEADER_NAMES
                .contains(&normalized.as_str())
                .then_some((normalized, value.as_str()))
        })
}

pub(super) fn requires_stream_merge_tool(request: &CreateDownloadJobRequest) -> bool {
    request.adapter_kind().is_stream()
}

pub(super) fn is_stream_request(request: &CreateDownloadJobRequest) -> bool {
    request.adapter_kind().is_stream()
        || request.source_url.contains(".m3u8")
        || request.source_url.contains(".mpd")
}

pub(super) fn is_youtube_request(request: &CreateDownloadJobRequest) -> bool {
    request.provider_id.as_deref() == Some("youtube")
        || matches!(request.adapter_kind(), AdapterKind::Provider(provider) if provider == "youtube")
        || host_matches(&request.source_url, &["youtube.com", "youtu.be"])
}

/// Whether the URL's host is one of `hosts` or a subdomain of one (`www.` is ignored).
pub(super) fn host_matches(value: &str, hosts: &[&str]) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let normalized = host.trim_start_matches("www.");
    hosts
        .iter()
        .any(|candidate| normalized == *candidate || normalized.ends_with(&format!(".{candidate}")))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::download_request;
    use super::*;

    #[test]
    fn applies_provider_output_extension_to_safe_filename() {
        assert_eq!(
            filename_with_extension("Owned Video", "webm"),
            "Owned Video.webm"
        );
        assert_eq!(
            filename_with_extension("Owned Video.mp4", "mkv"),
            "Owned Video.mkv"
        );
    }

    #[test]
    fn hls_and_dash_require_stream_merge_tool() {
        assert!(requires_stream_merge_tool(&download_request("hls")));
        assert!(requires_stream_merge_tool(&download_request("dash")));
        assert!(!requires_stream_merge_tool(&download_request("provider-x")));
        assert!(!requires_stream_merge_tool(&download_request(
            "generic-html5-video"
        )));
    }

    #[test]
    fn forwards_only_safe_browser_headers() {
        let mut request = download_request("hls");
        request.headers = Some(std::collections::BTreeMap::from([
            ("Accept-Language".to_string(), "de-DE".to_string()),
            ("Cookie".to_string(), "blocked=true".to_string()),
        ]));

        let headers = forwarded_headers(&request).collect::<Vec<_>>();

        assert_eq!(headers, vec![("accept-language".to_string(), "de-DE")]);
    }
}
