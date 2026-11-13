use std::time::Duration;

use ldm_types::download_job::CreateDownloadJobRequest;
use reqwest::header::REFERER;
use url::Url;

use crate::error::CoreError;
use crate::provider::common::forwarded_headers;

use super::{
    parser::{
        best_hls_variant, has_alternate_audio_group, is_hls_media_playlist,
        parse_hls_master_variants,
    },
    report::{
        hls_master_quality_report, hls_media_playlist_quality_report, with_core_hls_quality_report,
    },
};

const MAX_HLS_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const HLS_INSPECT_TIMEOUT_SECONDS: u64 = 2;
const HLS_INSPECT_CONNECT_TIMEOUT_SECONDS: u64 = 1;
const MAX_HLS_DISCOVERY_CANDIDATES: usize = 16;
const HLS_MASTER_FILENAMES: &[&str] = &[
    "master.m3u8",
    "playlist.m3u8",
    "manifest.m3u8",
    "index.m3u8",
];

pub async fn resolve_hls_request(
    request: &CreateDownloadJobRequest,
) -> Result<CreateDownloadJobRequest, CoreError> {
    if !request.adapter_kind().is_hls() {
        return Ok(request.clone());
    }

    let client = hls_inspection_client()?;
    let manifest = match fetch_hls_manifest(&client, request, &request.source_url).await {
        Ok(manifest) => manifest,
        Err(error) if is_transient_manifest_inspect_error(&error) => return Ok(request.clone()),
        Err(error) => return Err(error),
    };

    if let Some(resolved) = resolve_hls_master_request(request, &request.source_url, &manifest)? {
        return Ok(resolved);
    }

    if let Some(resolved) = discover_hls_master_request(&client, request).await? {
        return Ok(resolved);
    }

    Ok(request.clone())
}

pub async fn fetch_hls_manifest_text(
    request: &CreateDownloadJobRequest,
    manifest_url: &str,
) -> Result<String, CoreError> {
    let client = hls_inspection_client()?;
    fetch_hls_manifest(&client, request, manifest_url).await
}

pub fn hls_request_quality_score(request: &CreateDownloadJobRequest) -> u32 {
    hls_url_quality_score(&request.source_url)
}

/// Manifest inspection is optional: it picks the best variant and looks for a master playlist.
/// A CDN that rejects the link (404 or 403 for a signed URL bound to another IP address or an
/// elapsed time window, 410 for an expired one) and a request error do not fail the job here;
/// the download itself then fails with a clear message.
pub(super) fn is_transient_manifest_inspect_error(error: &CoreError) -> bool {
    match error {
        CoreError::HttpStatus(status) => matches!(status.as_u16(), 403 | 404 | 410),
        CoreError::Request(_) => true,
        _ => false,
    }
}

pub(super) fn resolve_hls_master_request(
    request: &CreateDownloadJobRequest,
    manifest_url: &str,
    manifest: &str,
) -> Result<Option<CreateDownloadJobRequest>, CoreError> {
    let variants = parse_hls_master_variants(manifest_url, manifest)?;
    let Some(best_variant) = best_hls_variant(&variants) else {
        return Ok(None);
    };

    let keeps_master_for_audio = has_alternate_audio_group(manifest);
    let resolved_source_url = if keeps_master_for_audio {
        manifest_url.to_string()
    } else {
        best_variant.uri.clone()
    };
    let report = hls_master_quality_report(
        manifest_url,
        &resolved_source_url,
        &variants,
        best_variant,
        keeps_master_for_audio,
    );
    let mut resolved = with_core_hls_quality_report(request, report);
    resolved.source_url = resolved_source_url;
    Ok(Some(resolved))
}

async fn discover_hls_master_request(
    client: &reqwest::Client,
    request: &CreateDownloadJobRequest,
) -> Result<Option<CreateDownloadJobRequest>, CoreError> {
    for candidate_url in candidate_hls_discovery_urls(&request.source_url) {
        if candidate_url == request.source_url {
            continue;
        }

        let manifest = match fetch_hls_manifest(client, request, &candidate_url).await {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        if let Some(resolved) = resolve_hls_master_request(request, &candidate_url, &manifest)? {
            return Ok(Some(resolved));
        }
        if hls_url_quality_score(&candidate_url) > hls_url_quality_score(&request.source_url)
            && is_hls_media_playlist(&manifest)
        {
            let report = hls_media_playlist_quality_report(&candidate_url);
            let mut resolved = with_core_hls_quality_report(request, report);
            resolved.source_url = candidate_url;
            return Ok(Some(resolved));
        }
    }

    Ok(None)
}

fn hls_inspection_client() -> Result<reqwest::Client, CoreError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(HLS_INSPECT_TIMEOUT_SECONDS))
        .connect_timeout(Duration::from_secs(HLS_INSPECT_CONNECT_TIMEOUT_SECONDS))
        .build()
        .map_err(CoreError::from)
}

async fn fetch_hls_manifest(
    client: &reqwest::Client,
    request: &CreateDownloadJobRequest,
    manifest_url: &str,
) -> Result<String, CoreError> {
    let mut request_builder = client.get(manifest_url);
    if let Some(referrer) = request.referrer.as_ref().or(request.page_url.as_ref()) {
        request_builder = request_builder.header(REFERER, referrer);
    }
    for (name, value) in forwarded_headers(request) {
        request_builder = request_builder.header(name, value);
    }

    let response = request_builder.send().await?;
    if !response.status().is_success() {
        return Err(CoreError::HttpStatus(response.status()));
    }

    let bytes = response.bytes().await?;
    if bytes.len() > MAX_HLS_MANIFEST_BYTES {
        return Err(CoreError::ProviderBackend(
            "HLS manifest is too large to inspect safely.".to_string(),
        ));
    }

    String::from_utf8(bytes.to_vec()).map_err(|_| {
        CoreError::ProviderBackend("HLS manifest is not valid UTF-8 text.".to_string())
    })
}

pub(super) fn candidate_hls_discovery_urls(media_playlist_url: &str) -> Vec<String> {
    let Ok(url) = Url::parse(media_playlist_url) else {
        return Vec::new();
    };

    let original_query = url.query().map(str::to_string);
    let mut candidates = Vec::new();
    push_quality_sibling_urls(&mut candidates, &url, original_query.as_deref());
    for prefix in ["../", "../../", "", "../../../"] {
        for filename in HLS_MASTER_FILENAMES {
            let Ok(candidate_url) = url.join(&format!("{prefix}{filename}")) else {
                continue;
            };
            push_candidate_url(&mut candidates, candidate_url, original_query.as_deref());
            if candidates.len() >= MAX_HLS_DISCOVERY_CANDIDATES {
                return candidates;
            }
        }
    }

    candidates
}

fn push_quality_sibling_urls(
    candidates: &mut Vec<String>,
    url: &Url,
    original_query: Option<&str>,
) {
    let path = decode_uri_component_lossy(url.path());
    let Some((current_height, current_bitrate, suffix)) = quality_segment_parts(&path) else {
        return;
    };

    let filenames = sibling_playlist_filenames(url);
    for (height, bitrates) in higher_quality_bitrate_candidates(current_height) {
        for bitrate in bitrates {
            let replacement = format!("{height}P_{bitrate}K{suffix}");
            let sibling_path = replace_quality_segment(
                &path,
                current_height,
                current_bitrate,
                suffix,
                &replacement,
            );
            for filename in &filenames {
                let mut candidate = url.clone();
                candidate.set_path(&replace_last_path_segment(&sibling_path, filename));
                push_candidate_url(candidates, candidate, original_query);
                if candidates.len() >= MAX_HLS_DISCOVERY_CANDIDATES {
                    return;
                }
            }
        }
    }
}

fn sibling_playlist_filenames(url: &Url) -> Vec<String> {
    let current = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|value| !value.is_empty())
        .unwrap_or("master.m3u8")
        .to_string();
    let mut filenames = vec![current];
    for filename in HLS_MASTER_FILENAMES {
        if !filenames.iter().any(|item| item == filename) {
            filenames.push((*filename).to_string());
        }
    }
    filenames
}

fn higher_quality_bitrate_candidates(current_height: u32) -> Vec<(u32, &'static [u32])> {
    const H2160: &[u32] = &[20000, 16000, 12000];
    const H1440: &[u32] = &[12000, 10000, 8000];
    const H1080: &[u32] = &[8000, 6000, 5000, 4000, 3000];
    const H720: &[u32] = &[4000, 3000, 2500, 2000];

    [(720, H720), (1080, H1080), (1440, H1440), (2160, H2160)]
        .into_iter()
        .filter(|(height, _)| *height > current_height)
        .collect()
}

pub(super) fn hls_url_quality_score(value: &str) -> u32 {
    let Ok(url) = Url::parse(value) else {
        return 0;
    };
    let haystack = decode_uri_component_lossy(url.path());
    quality_height_from_text(&haystack).unwrap_or(0)
}

fn quality_segment_parts(path: &str) -> Option<(u32, u32, &str)> {
    for segment in path.split('/') {
        let upper = segment.to_ascii_uppercase();
        let Some(p_index) = upper.find("P_") else {
            continue;
        };
        let height_start = upper[..p_index]
            .rfind(|ch: char| !ch.is_ascii_digit())
            .map_or(0, |index| index + 1);
        let height = upper[height_start..p_index].parse().ok()?;
        let bitrate_start = p_index + 2;
        let Some(k_index) = upper[bitrate_start..].find('K') else {
            continue;
        };
        let bitrate_end = bitrate_start + k_index;
        let bitrate = upper[bitrate_start..bitrate_end].parse().ok()?;
        let suffix = &segment[bitrate_end + 1..];
        return Some((height, bitrate, suffix));
    }

    None
}

fn replace_quality_segment(
    path: &str,
    current_height: u32,
    current_bitrate: u32,
    suffix: &str,
    replacement: &str,
) -> String {
    let current = format!("{current_height}P_{current_bitrate}K{suffix}");
    path.replacen(&current, replacement, 1)
}

fn replace_last_path_segment(path: &str, filename: &str) -> String {
    let Some((prefix, _)) = path.rsplit_once('/') else {
        return filename.to_string();
    };
    format!("{prefix}/{filename}")
}

fn quality_height_from_text(value: &str) -> Option<u32> {
    let lower = value.to_ascii_lowercase();
    let resolution_match = lower
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, 'x' | '_' | '-')))
        .find_map(|part| {
            let (_, height) = part.split_once('x')?;
            height.parse::<u32>().ok()
        });
    if resolution_match.is_some() {
        return resolution_match;
    }

    lower
        .split(|ch: char| !(ch.is_ascii_alphanumeric()))
        .find_map(|part| part.strip_suffix('p')?.parse::<u32>().ok())
}

fn decode_uri_component_lossy(value: &str) -> String {
    url::form_urlencoded::parse(value.as_bytes())
        .map(|(key, value)| format!("{key}{value}"))
        .collect::<String>()
}

fn push_candidate_url(candidates: &mut Vec<String>, mut url: Url, original_query: Option<&str>) {
    if let Some(query) = original_query {
        url.set_query(Some(query));
        push_unique_candidate_url(candidates, url.to_string());
        url.set_query(None);
    }
    push_unique_candidate_url(candidates, url.to_string());
}

fn push_unique_candidate_url(candidates: &mut Vec<String>, url: String) {
    if !candidates.iter().any(|candidate| candidate == &url) {
        candidates.push(url);
    }
}
