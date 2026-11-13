pub mod native;

mod journal;
mod parser;
mod report;
mod resolver;

pub use journal::HlsSegmentJournal;
pub use parser::{
    best_hls_variant, has_alternate_audio_group, is_hls_media_playlist, is_live_hls_media_playlist,
    parse_hls_master_variants, parse_hls_media_playlist, HlsMediaPlaylist, HlsMediaSegment,
    HlsVariant,
};
pub use resolver::{fetch_hls_manifest_text, hls_request_quality_score, resolve_hls_request};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ldm_types::download_job::CreateDownloadJobRequest;
    use serde_json::Value;

    use super::*;

    #[test]
    fn parses_master_variants_and_resolves_relative_urls() {
        let variants = parse_hls_master_variants(
            "https://cdn.example.test/path/master.m3u8",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\n720/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=4500000,RESOLUTION=1920x1080\n1080/index.m3u8\n",
        )
        .unwrap();

        assert_eq!(variants.len(), 2);
        assert_eq!(
            variants[0].uri,
            "https://cdn.example.test/path/720/index.m3u8"
        );
        assert_eq!(variants[1].height, Some(1080));
    }

    #[test]
    fn selects_highest_resolution_before_bandwidth() {
        let variants = vec![
            HlsVariant {
                uri: "https://example.test/720.m3u8".to_string(),
                bandwidth: Some(8_000_000),
                average_bandwidth: None,
                width: Some(1280),
                height: Some(720),
            },
            HlsVariant {
                uri: "https://example.test/1080.m3u8".to_string(),
                bandwidth: Some(4_000_000),
                average_bandwidth: None,
                width: Some(1920),
                height: Some(1080),
            },
        ];

        assert_eq!(best_hls_variant(&variants).unwrap().height, Some(1080));
    }

    #[test]
    fn treats_media_playlists_as_unresolved() {
        let variants = parse_hls_master_variants(
            "https://cdn.example.test/720/index.m3u8",
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nsegment0.ts\n",
        )
        .unwrap();

        assert!(variants.is_empty());
    }

    #[test]
    fn parses_media_playlist_segments() {
        let playlist = parse_hls_media_playlist(
            "https://cdn.example.test/video/720/index.m3u8",
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nsegment0.ts\n#EXTINF:4,\n../shared/segment1.ts?sig=1\n",
        )
        .unwrap();

        assert_eq!(
            playlist.segments,
            vec![
                HlsMediaSegment {
                    uri: "https://cdn.example.test/video/720/segment0.ts".to_string(),
                },
                HlsMediaSegment {
                    uri: "https://cdn.example.test/video/shared/segment1.ts?sig=1".to_string(),
                },
            ]
        );
    }

    #[test]
    fn native_media_playlist_rejects_encrypted_segments() {
        let error = parse_hls_media_playlist(
            "https://cdn.example.test/video/720/index.m3u8",
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:4,\nsegment0.ts\n",
        )
        .unwrap_err();

        assert!(error.to_string().contains("native HLS download"));
    }

    #[test]
    fn derives_nearby_master_candidates_from_rendition_playlist() {
        let candidates = resolver::candidate_hls_discovery_urls(
            "https://cdn.example.test/video/1280x720/index-v1-a1.m3u8?sig=abc",
        );

        assert!(
            candidates.contains(&"https://cdn.example.test/video/master.m3u8?sig=abc".to_string())
        );
        assert!(candidates.contains(&"https://cdn.example.test/video/master.m3u8".to_string()));
    }

    #[test]
    fn derives_higher_quality_sibling_candidates() {
        let candidates = resolver::candidate_hls_discovery_urls(
            "https://cdn.example.test/hls/videos/720P_4000K_49874985.mp4/master.m3u8?sig=abc",
        );

        assert!(candidates.contains(
            &"https://cdn.example.test/hls/videos/1080P_8000K_49874985.mp4/master.m3u8?sig=abc"
                .to_string()
        ));
    }

    #[test]
    fn scores_hls_urls_by_visible_quality() {
        assert_eq!(
            resolver::hls_url_quality_score(
                "https://cdn.example.test/hls/videos/720P_4000K_49874985.mp4/master.m3u8"
            ),
            720
        );
        assert_eq!(
            resolver::hls_url_quality_score(
                "https://cdn.example.test/hls/videos/1080P_6000K_49874985.mp4/master.m3u8"
            ),
            1080
        );
    }

    #[test]
    fn resolves_discovered_master_to_highest_variant() {
        let request = hls_request();
        let resolved = resolver::resolve_hls_master_request(
            &request,
            "https://cdn.example.test/video/master.m3u8",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\n1280x720/index-v1-a1.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=4000000,RESOLUTION=1920x1080\n1920x1080/index-v1-a1.m3u8\n",
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            resolved.source_url,
            "https://cdn.example.test/video/1920x1080/index-v1-a1.m3u8"
        );
        let report: Value =
            serde_json::from_str(resolved.quality_report.as_deref().unwrap()).unwrap();
        assert_eq!(report["coreHls"]["mode"], "master");
        assert_eq!(report["coreHls"]["variantCount"], 2);
        assert_eq!(report["coreHls"]["selectedVariant"]["height"], 1080);
        assert_eq!(
            report["coreHls"]["selectedVariant"]["url"]["path"],
            "/video/1920x1080/index-v1-a1.m3u8"
        );
    }

    #[test]
    fn detects_alternate_audio_groups() {
        assert!(has_alternate_audio_group(
            "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",URI=\"audio.m3u8\"\n"
        ));
        assert!(!has_alternate_audio_group(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=640x360\n360.m3u8\n"
        ));
    }

    #[test]
    fn treats_rejected_manifest_links_as_non_fatal_inspection_errors() {
        use crate::error::CoreError;
        use reqwest::StatusCode;

        for status in [
            StatusCode::GONE,
            StatusCode::NOT_FOUND,
            StatusCode::FORBIDDEN,
        ] {
            assert!(
                resolver::is_transient_manifest_inspect_error(&CoreError::HttpStatus(status)),
                "{status}"
            );
        }
        assert!(!resolver::is_transient_manifest_inspect_error(
            &CoreError::HttpStatus(StatusCode::INTERNAL_SERVER_ERROR)
        ));
        assert!(!resolver::is_transient_manifest_inspect_error(
            &CoreError::ProviderBackend("HLS manifest is too large to inspect safely.".to_string())
        ));
    }

    #[test]
    fn manifest_inspection_forwards_only_safe_browser_headers() {
        let mut request = hls_request();
        request.headers = Some(BTreeMap::from([
            ("Accept-Language".to_string(), "de-DE".to_string()),
            ("Cookie".to_string(), "blocked=true".to_string()),
        ]));

        let headers = crate::provider::common::forwarded_headers(&request).collect::<Vec<_>>();

        assert_eq!(headers, vec![("accept-language".to_string(), "de-DE")]);
    }

    fn hls_request() -> CreateDownloadJobRequest {
        CreateDownloadJobRequest {
            source_url: "https://cdn.example.test/video/master.m3u8".to_string(),
            source_fallback_urls: Vec::new(),
            page_url: Some("https://example.test/watch".to_string()),
            page_title: Some("Owned Stream".to_string()),
            referrer: None,
            suggested_filename: Some("Owned Stream".to_string()),
            content_type: Some("application/vnd.apple.mpegurl".to_string()),
            content_length: None,
            kind: ldm_types::download_job::DownloadKind::Video,
            adapter_id: "hls".to_string(),
            provider_id: None,
            selected_option_id: Some("manifest".to_string()),
            headers: None,
            prefer_range_request: false,
            quality_report: None,
        }
    }
}
