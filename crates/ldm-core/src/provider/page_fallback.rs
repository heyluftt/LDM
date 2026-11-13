//! The page URL as the last attempt of a stream job whose captured manifest link no longer
//! works. Signed HLS and DASH URLs are often bound to the IP address and time window of the
//! browser session; yt-dlp's site extractors fetch the page and receive a link signed for their
//! own request, so the download can still succeed where the captured link cannot.

use ldm_types::download_job::CreateDownloadJobRequest;
use serde_json::{json, Value};
use url::Url;

use super::common::host_matches;
use super::diagnostics::looks_like_stream_link_rejected;

/// Pages with a provider adapter in the extension. A stream job from one of these does not fall
/// back to its page: the provider adapter is the right path there, with its own retries.
const PROVIDER_PAGE_HOSTS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "x.com",
    "twitter.com",
    "tiktok.com",
];

/// Whether a failed stream job should run yt-dlp on the page the stream was found on. True only
/// for HLS and DASH jobs whose yt-dlp output shows a rejected link (404, 403, 410) or a manifest
/// yt-dlp could not use ("Unsupported URL"), when the page is an http(s) URL other than the
/// stream itself and on no provider host. Network and tool errors are reported as they are.
pub(super) fn should_try_page_url_fallback(
    request: &CreateDownloadJobRequest,
    output_text: &str,
) -> bool {
    page_url_fallback_target(request, output_text).is_some()
}

/// The request for that attempt: the page as the source, no stream fallbacks, no Referer (yt-dlp
/// fetches the page itself), the forwarded browser headers kept. Adapter and kind stay, so the
/// stream remux and the artifact lookup work as they do for the stream URL.
pub(super) fn page_url_fallback_request(
    request: &CreateDownloadJobRequest,
    output_text: &str,
) -> Option<CreateDownloadJobRequest> {
    if !should_try_page_url_fallback(request, output_text) {
        return None;
    }

    let page_url = request.page_url.as_deref()?.trim().to_string();
    let mut fallback = request.clone();
    fallback.source_url = page_url;
    fallback.source_fallback_urls.clear();
    fallback.referrer = None;
    fallback.page_url = None;
    Some(fallback)
}

fn page_url_fallback_target<'a>(
    request: &'a CreateDownloadJobRequest,
    output_text: &str,
) -> Option<&'a str> {
    if !request.adapter_kind().is_stream() {
        return None;
    }
    if looks_like_stream_link_rejected(output_text).is_none()
        && !looks_like_unusable_manifest(output_text)
    {
        return None;
    }

    let page_url = request
        .page_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let parsed = Url::parse(page_url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || page_url == request.source_url
        || host_matches(page_url, PROVIDER_PAGE_HOSTS)
    {
        return None;
    }

    Some(page_url)
}

/// yt-dlp got an answer for the manifest URL but found no stream in it, for example an HTML
/// error page served with status 200.
fn looks_like_unusable_manifest(output_text: &str) -> bool {
    output_text.to_ascii_lowercase().contains("unsupported url")
}

/// Marks the quality report of a download that yt-dlp fetched from the page URL. A report that
/// is not a JSON object is kept under `originalReport`.
pub(super) fn with_page_url_fallback_report(quality_report: Option<&str>) -> String {
    let parsed = quality_report.and_then(|value| serde_json::from_str::<Value>(value).ok());
    let mut report = match parsed {
        Some(report @ Value::Object(_)) => report,
        _ => {
            let mut report = json!({ "version": 1 });
            if let Some(original) = quality_report {
                report["originalReport"] = Value::String(original.to_string());
            }
            report
        }
    };
    report["pageUrlFallback"] = Value::Bool(true);
    report.to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::test_support::download_request;
    use super::*;

    const REJECTED_404: &str =
        "ERROR: master.m3u8: Unable to download webpage: HTTP Error 404: Not Found";
    const REJECTED_403: &str =
        "ERROR: [generic] Unable to download webpage: HTTP Error 403: Forbidden";
    const UNSUPPORTED_URL: &str =
        "ERROR: Unsupported URL: https://hv-h.cdn.example.test/videos/master.m3u8";
    const NETWORK_ERROR: &str =
        "ERROR: Unable to download webpage: <urlopen error [Errno 11001] getaddrinfo failed>";
    const TOOL_ERROR: &str = "ERROR: ffmpeg exited with code 1";
    const PAGE: &str = "https://videos.example.test/view_video.php?viewkey=abc";

    fn stream_request(adapter_id: &str, page_url: Option<&str>) -> CreateDownloadJobRequest {
        let mut request = download_request(adapter_id);
        request.source_url = "https://hv-h.cdn.example.test/videos/master.m3u8?sig=abc".to_string();
        request.page_url = page_url.map(str::to_string);
        request
    }

    #[test]
    fn page_url_fallback_decision_table() {
        let cases: &[(&str, Option<&str>, &str, bool)] = &[
            ("hls", Some(PAGE), REJECTED_404, true),
            ("hls", Some(PAGE), REJECTED_403, true),
            (
                "dash",
                Some("https://videos.example.test/watch/123"),
                REJECTED_404,
                true,
            ),
            ("hls", Some(PAGE), UNSUPPORTED_URL, true),
            (
                "hls",
                Some("https://www.youtube.com/watch?v=owned"),
                REJECTED_404,
                false,
            ),
            ("hls", Some("https://youtu.be/owned"), REJECTED_404, false),
            (
                "hls",
                Some("https://x.com/user/status/1"),
                REJECTED_404,
                false,
            ),
            (
                "hls",
                Some("https://www.tiktok.com/@user/video/1"),
                REJECTED_404,
                false,
            ),
            (
                "provider-youtube",
                Some("https://www.youtube.com/watch?v=owned"),
                REJECTED_404,
                false,
            ),
            ("provider-x", Some(PAGE), REJECTED_404, false),
            ("generic-html5-video", Some(PAGE), REJECTED_404, false),
            ("hls", Some(PAGE), NETWORK_ERROR, false),
            ("hls", Some(PAGE), TOOL_ERROR, false),
            ("hls", Some(PAGE), "", false),
            ("hls", None, REJECTED_404, false),
            ("hls", Some("   "), REJECTED_404, false),
            (
                "hls",
                Some("chrome-extension://abc/page.html"),
                REJECTED_404,
                false,
            ),
            (
                "hls",
                Some("https://hv-h.cdn.example.test/videos/master.m3u8?sig=abc"),
                REJECTED_404,
                false,
            ),
        ];

        for (adapter_id, page_url, output, expected) in cases {
            assert_eq!(
                should_try_page_url_fallback(&stream_request(adapter_id, *page_url), output),
                *expected,
                "{adapter_id} {page_url:?} {output:?}"
            );
        }
    }

    #[test]
    fn page_url_fallback_request_targets_the_page_without_a_referer() {
        let mut request = stream_request("hls", Some(PAGE));
        request.source_fallback_urls =
            vec!["https://hv-h.cdn.example.test/videos/720/index.m3u8".to_string()];
        request.referrer = Some("https://videos.example.test/".to_string());
        request.headers = Some(BTreeMap::from([(
            "user-agent".to_string(),
            "Browser".to_string(),
        )]));

        let fallback = page_url_fallback_request(&request, REJECTED_404).unwrap();

        assert_eq!(fallback.source_url, PAGE);
        assert!(fallback.source_fallback_urls.is_empty());
        assert_eq!(fallback.referrer, None);
        assert_eq!(fallback.page_url, None);
        assert_eq!(fallback.adapter_id, "hls");
        assert_eq!(fallback.headers, request.headers);
        assert_eq!(fallback.suggested_filename, request.suggested_filename);
        assert!(page_url_fallback_request(&request, NETWORK_ERROR).is_none());
    }

    #[test]
    fn page_url_fallback_marks_the_quality_report() {
        let report: Value = serde_json::from_str(&with_page_url_fallback_report(Some(
            r#"{"version":1,"selectedAfterPrepare":{"quality":"1080p"}}"#,
        )))
        .unwrap();
        assert_eq!(report["pageUrlFallback"], true);
        assert_eq!(report["selectedAfterPrepare"]["quality"], "1080p");

        let report: Value = serde_json::from_str(&with_page_url_fallback_report(None)).unwrap();
        assert_eq!(report, json!({ "version": 1, "pageUrlFallback": true }));

        let report: Value =
            serde_json::from_str(&with_page_url_fallback_report(Some("not json"))).unwrap();
        assert_eq!(report["pageUrlFallback"], true);
        assert_eq!(report["originalReport"], "not json");
    }
}
