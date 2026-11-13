use url::Url;

use crate::error::CoreError;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HlsVariant {
    pub uri: String,
    pub bandwidth: Option<u64>,
    pub average_bandwidth: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HlsMediaSegment {
    pub uri: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HlsMediaPlaylist {
    pub segments: Vec<HlsMediaSegment>,
}

pub fn has_alternate_audio_group(manifest: &str) -> bool {
    manifest.lines().map(str::trim).any(|line| {
        let Some(attributes) = line.strip_prefix("#EXT-X-MEDIA:") else {
            return false;
        };
        parse_hls_attribute_list(attributes)
            .get("TYPE")
            .is_some_and(|value| value.eq_ignore_ascii_case("AUDIO"))
    })
}

pub fn parse_hls_master_variants(
    manifest_url: &str,
    manifest: &str,
) -> Result<Vec<HlsVariant>, CoreError> {
    let base_url =
        Url::parse(manifest_url).map_err(|_| CoreError::InvalidUrl("sourceUrl".to_string()))?;
    let lines = manifest.lines().map(str::trim).collect::<Vec<_>>();
    let mut variants = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let Some(attributes) = line.strip_prefix("#EXT-X-STREAM-INF:") else {
            continue;
        };
        let Some(uri) = next_playlist_uri(&lines[index + 1..]) else {
            continue;
        };

        let attributes = parse_hls_attribute_list(attributes);
        let resolution = attributes
            .get("RESOLUTION")
            .and_then(|value| parse_resolution(value));
        let variant_url = base_url
            .join(uri)
            .map_err(|_| CoreError::InvalidUrl("hls variant".to_string()))?;

        variants.push(HlsVariant {
            uri: variant_url.to_string(),
            bandwidth: attributes
                .get("BANDWIDTH")
                .and_then(|value| value.parse().ok()),
            average_bandwidth: attributes
                .get("AVERAGE-BANDWIDTH")
                .and_then(|value| value.parse().ok()),
            width: resolution.map(|item| item.0),
            height: resolution.map(|item| item.1),
        });
    }

    Ok(variants)
}

pub fn parse_hls_media_playlist(
    manifest_url: &str,
    manifest: &str,
) -> Result<HlsMediaPlaylist, CoreError> {
    if has_unsupported_native_hls_feature(manifest) {
        return Err(CoreError::ProviderBackend(
            "native HLS download does not support this playlist layout yet".to_string(),
        ));
    }

    let base_url =
        Url::parse(manifest_url).map_err(|_| CoreError::InvalidUrl("sourceUrl".to_string()))?;
    let lines = manifest.lines().map(str::trim).collect::<Vec<_>>();
    let mut segments = Vec::new();
    let mut expect_segment_uri = false;

    for line in lines {
        if line.is_empty() {
            continue;
        }

        if line.starts_with("#EXTINF:") {
            expect_segment_uri = true;
            continue;
        }

        if line.starts_with('#') {
            continue;
        }

        if !expect_segment_uri {
            continue;
        }

        let segment_url = base_url
            .join(line)
            .map_err(|_| CoreError::InvalidUrl("hls segment".to_string()))?;
        segments.push(HlsMediaSegment {
            uri: segment_url.to_string(),
        });
        expect_segment_uri = false;
    }

    if segments.is_empty() {
        return Err(CoreError::ProviderBackend(
            "HLS media playlist has no downloadable segments.".to_string(),
        ));
    }

    Ok(HlsMediaPlaylist { segments })
}

pub fn best_hls_variant(variants: &[HlsVariant]) -> Option<&HlsVariant> {
    variants.iter().max_by_key(|variant| {
        (
            variant.height.unwrap_or(0),
            variant.width.unwrap_or(0),
            variant.average_bandwidth.or(variant.bandwidth).unwrap_or(0),
        )
    })
}

pub fn is_hls_media_playlist(manifest: &str) -> bool {
    manifest
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("#EXTINF:") || line.starts_with("#EXT-X-TARGETDURATION:"))
}

/// Whether a media playlist is still growing: it has no `#EXT-X-ENDLIST` and does not declare
/// itself `VOD`. Such a playlist lists only the segments available right now (for a live
/// broadcast the last few seconds), so downloading it would save a short clip and call the job
/// complete.
pub fn is_live_hls_media_playlist(manifest: &str) -> bool {
    let mut declared_vod = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with("#EXT-X-ENDLIST") {
            return false;
        }
        if let Some(playlist_type) = line.strip_prefix("#EXT-X-PLAYLIST-TYPE:") {
            declared_vod = playlist_type.trim().eq_ignore_ascii_case("VOD");
        }
    }
    !declared_vod
}

pub(super) fn parse_hls_attribute_list(value: &str) -> std::collections::BTreeMap<String, String> {
    let mut attributes = std::collections::BTreeMap::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for character in value.chars() {
        match character {
            '"' => {
                in_quotes = !in_quotes;
                current.push(character);
            }
            ',' if !in_quotes => {
                insert_hls_attribute(&mut attributes, &current);
                current.clear();
            }
            _ => current.push(character),
        }
    }
    insert_hls_attribute(&mut attributes, &current);

    attributes
}

fn has_unsupported_native_hls_feature(manifest: &str) -> bool {
    manifest.lines().map(str::trim).any(|line| {
        if line.starts_with("#EXT-X-BYTERANGE:")
            || line.starts_with("#EXT-X-MAP:")
            || line.starts_with("#EXT-X-DISCONTINUITY")
        {
            return true;
        }

        if let Some(attributes) = line.strip_prefix("#EXT-X-KEY:") {
            return parse_hls_attribute_list(attributes)
                .get("METHOD")
                .is_some_and(|method| !method.eq_ignore_ascii_case("NONE"));
        }

        false
    })
}

fn insert_hls_attribute(attributes: &mut std::collections::BTreeMap<String, String>, value: &str) {
    let Some((key, raw_value)) = value.split_once('=') else {
        return;
    };
    let normalized_value = raw_value.trim().trim_matches('"').to_string();
    attributes.insert(key.trim().to_ascii_uppercase(), normalized_value);
}

fn next_playlist_uri<'a>(lines: &'a [&'a str]) -> Option<&'a str> {
    lines
        .iter()
        .copied()
        .find(|line| !line.is_empty() && !line.starts_with('#'))
}

fn parse_resolution(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::is_live_hls_media_playlist;

    const SEGMENTS: &str = "#EXTINF:4.0,\nseg0.ts\n#EXTINF:4.0,\nseg1.ts\n";

    #[test]
    fn a_playlist_with_an_end_marker_is_finished() {
        let manifest = format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n{SEGMENTS}#EXT-X-ENDLIST\n");

        assert!(!is_live_hls_media_playlist(&manifest));
    }

    #[test]
    fn a_playlist_without_an_end_marker_is_live() {
        let sliding_window =
            format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:812\n{SEGMENTS}");
        let growing_event =
            format!("#EXTM3U\n#EXT-X-PLAYLIST-TYPE:EVENT\n#EXT-X-TARGETDURATION:4\n{SEGMENTS}");

        assert!(is_live_hls_media_playlist(&sliding_window));
        assert!(is_live_hls_media_playlist(&growing_event));
    }

    #[test]
    fn a_playlist_declared_vod_is_finished_even_without_the_end_marker() {
        let manifest =
            format!("#EXTM3U\n#EXT-X-PLAYLIST-TYPE: vod \n#EXT-X-TARGETDURATION:4\n{SEGMENTS}");

        assert!(!is_live_hls_media_playlist(&manifest));
    }
}
