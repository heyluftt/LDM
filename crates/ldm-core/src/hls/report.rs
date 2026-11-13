use ldm_types::download_job::CreateDownloadJobRequest;
use serde_json::{json, Value};
use url::Url;

use super::parser::HlsVariant;

pub(super) fn hls_master_quality_report(
    manifest_url: &str,
    resolved_source_url: &str,
    variants: &[HlsVariant],
    selected_variant: &HlsVariant,
    keeps_master_for_audio: bool,
) -> Value {
    json!({
        "version": 1,
        "mode": "master",
        "inspectedManifest": safe_url_report(manifest_url),
        "resolvedSource": safe_url_report(resolved_source_url),
        "variantCount": variants.len(),
        "keepsMasterForAlternateAudio": keeps_master_for_audio,
        "selectedVariant": hls_variant_report(selected_variant),
        "variants": variants.iter().map(hls_variant_report).take(12).collect::<Vec<_>>(),
        "note": "Core HLS report omits query strings, header values, cookies, authorization, and local file paths.",
    })
}

pub(super) fn hls_media_playlist_quality_report(candidate_url: &str) -> Value {
    json!({
        "version": 1,
        "mode": "discovered-media-playlist",
        "resolvedSource": safe_url_report(candidate_url),
        "variantCount": 0,
        "variants": [],
        "note": "Core HLS report omits query strings, header values, cookies, authorization, and local file paths.",
    })
}

pub(super) fn with_core_hls_quality_report(
    request: &CreateDownloadJobRequest,
    core_hls_report: Value,
) -> CreateDownloadJobRequest {
    let mut resolved = request.clone();
    let mut report = request
        .quality_report
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "version": 1 }));

    if let Some(object) = report.as_object_mut() {
        object.insert("coreHls".to_string(), core_hls_report);
    }

    resolved.quality_report = Some(
        serde_json::to_string_pretty(&report)
            .unwrap_or_else(|_| r#"{"version":1,"coreHls":{"version":1}}"#.to_string()),
    );
    resolved
}

fn hls_variant_report(variant: &HlsVariant) -> Value {
    json!({
        "url": safe_url_report(&variant.uri),
        "width": variant.width,
        "height": variant.height,
        "bandwidth": variant.bandwidth,
        "averageBandwidth": variant.average_bandwidth,
        "score": [
            variant.height.unwrap_or(0) as u64,
            variant.width.unwrap_or(0) as u64,
            variant.average_bandwidth.or(variant.bandwidth).unwrap_or(0),
        ],
    })
}

fn safe_url_report(value: &str) -> Value {
    match Url::parse(value) {
        Ok(url) => json!({
            "host": url.host_str().unwrap_or(""),
            "filename": url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .unwrap_or(""),
            "path": decode_uri_component_lossy(url.path()),
            "hasQuery": url.query().is_some(),
        }),
        Err(_) => json!({
            "host": "",
            "filename": "",
            "path": "",
            "hasQuery": value.contains('?'),
        }),
    }
}

fn decode_uri_component_lossy(value: &str) -> String {
    url::form_urlencoded::parse(value.as_bytes())
        .map(|(key, value)| format!("{key}{value}"))
        .collect::<String>()
}
