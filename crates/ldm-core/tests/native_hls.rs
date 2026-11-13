//! Native HLS segment download against a wiremock server: complete download with journal,
//! resume after a cancel, retries on 500, and (only when FFmpeg is installed) the remux.

use std::{path::Path, process::Command, time::Duration};

use ldm_core::{
    download::{download_cancellation_pair, DownloadProgress},
    error::CoreError,
    filename::partials_dir,
    hls::{
        fetch_hls_manifest_text,
        native::{
            download_native_hls_segments, load_native_hls_playlist, run_native_hls_download,
            NativeHlsWorkFiles,
        },
        parse_hls_media_playlist, HlsMediaPlaylist, HlsSegmentJournal,
    },
    tool_discovery::{discover_tool, ToolKind},
};
use ldm_types::download_job::{CreateDownloadJobRequest, DownloadKind};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

const PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:4.0,\nseg0.ts\n#EXTINF:4.0,\nseg1.ts\n#EXTINF:4.0,\nseg2.ts\n#EXT-X-ENDLIST\n";
const TEMP_STEM: &str = "ldm-test";
/// The sliding window of a broadcast in progress: a media sequence number, no end marker.
const LIVE_PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:812\n#EXTINF:4.0,\nseg0.ts\n#EXTINF:4.0,\nseg1.ts\n#EXTINF:4.0,\nseg2.ts\n";

#[tokio::test]
async fn a_finished_playlist_loads_and_a_live_one_is_refused() {
    let server = MockServer::start().await;
    mount_playlist(&server).await;
    Mock::given(method("GET"))
        .and(path("/live/index.m3u8"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.apple.mpegurl")
                .set_body_string(LIVE_PLAYLIST),
        )
        .mount(&server)
        .await;
    let finished = request(&server);
    let mut live = request(&server);
    live.source_url = format!("{}/live/index.m3u8", server.uri());

    let playlist = load_native_hls_playlist(&finished).await.unwrap();
    let refused = load_native_hls_playlist(&live).await;

    assert_eq!(playlist.segments.len(), 3);
    assert!(matches!(refused, Err(CoreError::LiveStream)), "{refused:?}");
    // No segment of the live stream was requested: nothing was saved as a "finished" clip.
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|request| request.url.path().ends_with(".m3u8")),
        "{requests:?}"
    );
}

#[tokio::test]
async fn downloads_every_segment_into_the_transport_file_and_journal() {
    let server = MockServer::start().await;
    let segments = segments();
    mount_playlist(&server).await;
    for (index, body) in segments.iter().enumerate() {
        mount_segment(&server, index, body.clone()).await;
    }
    let (request, playlist) = playlist_for(&server).await;
    let work = tempfile::tempdir().unwrap();
    let files = NativeHlsWorkFiles::new(work.path(), TEMP_STEM);
    let mut updates = Vec::<DownloadProgress>::new();

    let transport_bytes = download_native_hls_segments(
        &request,
        &playlist,
        &files,
        &mut |update| updates.push(update),
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    let expected = segments.concat();
    assert_eq!(transport_bytes, expected.len() as u64);
    assert_eq!(std::fs::read(&files.transport_path).unwrap(), expected);
    let journal = journal_of(&files, &request, &playlist).await;
    assert_eq!(journal.completed_count(), 3);
    assert_eq!(journal.completed_bytes(), expected.len() as u64);
    assert_eq!(updates.first().unwrap().downloaded_bytes, 0);
    assert_eq!(updates.last().unwrap().progress_percent, Some(100.0));
    assert_eq!(
        updates.last().unwrap().downloaded_bytes,
        expected.len() as u64
    );
    assert!(updates.iter().all(|update| update.supports_resume));
    assert!(!files.remux_path.exists());
}

#[tokio::test]
async fn resumes_after_the_last_journaled_segment() {
    let server = MockServer::start().await;
    let segments = segments();
    mount_playlist(&server).await;
    mount_segment(&server, 0, segments[0].clone()).await;
    Mock::given(method("GET"))
        .and(path("/seg1.ts"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(30))
                .set_body_bytes(segments[1].clone()),
        )
        .mount(&server)
        .await;
    mount_segment(&server, 2, segments[2].clone()).await;
    let (request, playlist) = playlist_for(&server).await;
    let work = tempfile::tempdir().unwrap();
    let files = NativeHlsWorkFiles::new(work.path(), TEMP_STEM);
    let (cancel, cancellation) = download_cancellation_pair();

    // Cancel once the first segment is written. The second one is stuck at the stalled
    // server, so the third, although fetched in parallel, cannot be written either.
    let first = download_native_hls_segments(
        &request,
        &playlist,
        &files,
        &mut |update| {
            if update
                .progress_percent
                .is_some_and(|percent| percent >= 33.0)
            {
                cancel.cancel();
            }
        },
        cancellation,
    )
    .await;

    assert!(matches!(first, Err(CoreError::Canceled)), "{first:?}");
    let journal = journal_of(&files, &request, &playlist).await;
    assert_eq!(journal.completed_count(), 1);
    assert_eq!(journal.completed_bytes(), segments[0].len() as u64);
    assert_eq!(
        std::fs::metadata(&files.transport_path).unwrap().len(),
        segments[0].len() as u64
    );

    // Fresh mocks and a clean request log for the second run.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/seg0.ts"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    mount_segment(&server, 1, segments[1].clone()).await;
    mount_segment(&server, 2, segments[2].clone()).await;
    let mut updates = Vec::<DownloadProgress>::new();

    let transport_bytes = download_native_hls_segments(
        &request,
        &playlist,
        &files,
        &mut |update| updates.push(update),
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    let expected = segments.concat();
    assert_eq!(transport_bytes, expected.len() as u64);
    assert_eq!(std::fs::read(&files.transport_path).unwrap(), expected);
    // The two missing segments are fetched side by side, so their order is not fixed.
    let mut requested_paths = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_string())
        .collect::<Vec<_>>();
    requested_paths.sort();
    assert_eq!(requested_paths, ["/seg1.ts", "/seg2.ts"]);
    assert_eq!(
        updates.first().unwrap().downloaded_bytes,
        segments[0].len() as u64
    );
    assert_eq!(
        journal_of(&files, &request, &playlist)
            .await
            .completed_count(),
        3
    );
    server.verify().await;
}

#[tokio::test]
async fn retries_a_failing_segment_three_times_then_fails() {
    let server = MockServer::start().await;
    let segments = segments();
    mount_playlist(&server).await;
    mount_segment(&server, 0, segments[0].clone()).await;
    Mock::given(method("GET"))
        .and(path("/seg1.ts"))
        .respond_with(ResponseTemplate::new(500))
        .expect(3)
        .mount(&server)
        .await;
    mount_segment(&server, 2, segments[2].clone()).await;
    let (request, playlist) = playlist_for(&server).await;
    let work = tempfile::tempdir().unwrap();
    let files = NativeHlsWorkFiles::new(work.path(), TEMP_STEM);

    let result = download_native_hls_segments(
        &request,
        &playlist,
        &files,
        &mut |_| {},
        download_cancellation_pair().1,
    )
    .await;

    let message = match result {
        Err(CoreError::ProviderBackend(message)) => message,
        other => panic!("expected a provider backend error, got {other:?}"),
    };
    assert!(
        message.contains("HLS segment 2/3 failed after 3 attempt(s)")
            && message.contains("HTTP 500"),
        "{message}"
    );
    // The segment before the failing one is written and recorded; the one after it was
    // fetched in parallel but must not be written out of order.
    let journal = journal_of(&files, &request, &playlist).await;
    assert_eq!(journal.completed_count(), 1);
    assert_eq!(
        std::fs::read(&files.transport_path).unwrap(),
        segments[0],
        "only the first segment may be in the transport file"
    );
    server.verify().await;
}

/// The whole native path including the FFmpeg remux. Runs only when FFmpeg is discoverable;
/// otherwise it reports the skip and passes, so machines without FFmpeg stay green.
#[tokio::test]
async fn remuxes_the_transport_stream_with_ffmpeg_when_available() {
    let ffmpeg = discover_tool(ToolKind::Ffmpeg);
    let Some(ffmpeg_path) = ffmpeg.path else {
        eprintln!("skipping FFmpeg remux test: {}", ffmpeg.message);
        return;
    };

    let fixture = tempfile::tempdir().unwrap();
    let transport = generate_transport_stream(&ffmpeg_path, fixture.path());
    let segments = split_transport_stream(&transport, 3);
    let server = MockServer::start().await;
    mount_playlist(&server).await;
    for (index, body) in segments.iter().enumerate() {
        mount_segment(&server, index, body.clone()).await;
    }
    let request = request(&server);
    let target = tempfile::tempdir().unwrap();

    let completed = run_native_hls_download(
        &[request],
        target.path(),
        uuid::Uuid::new_v4(),
        &mut |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(completed.filename, "Owned Stream.mp4");
    assert_eq!(completed.final_path, target.path().join("Owned Stream.mp4"));
    assert!(completed.downloaded_bytes > 0);
    assert_eq!(
        std::fs::metadata(&completed.final_path).unwrap().len(),
        completed.downloaded_bytes
    );
    assert!(completed.supports_resume);
    let leftovers = std::fs::read_dir(partials_dir(target.path()))
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "work files must be removed after the remux");
}

fn generate_transport_stream(ffmpeg_path: &str, dir: &Path) -> Vec<u8> {
    let output = dir.join("source.ts");
    let result = Command::new(ffmpeg_path)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=64x64:rate=10",
            "-c:v",
            "mpeg4",
            "-f",
            "mpegts",
        ])
        .arg(&output)
        .output()
        .expect("FFmpeg should start");
    assert!(
        result.status.success(),
        "FFmpeg could not create the fixture: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    std::fs::read(output).unwrap()
}

/// Splits a transport stream on 188-byte packet boundaries, so each piece is valid TS and the
/// concatenation equals the original.
fn split_transport_stream(transport: &[u8], pieces: usize) -> Vec<Vec<u8>> {
    const PACKET: usize = 188;
    assert_eq!(transport.len() % PACKET, 0, "fixture is not packet aligned");
    let packets = transport.len() / PACKET;
    let per_piece = packets.div_ceil(pieces);
    transport
        .chunks(per_piece * PACKET)
        .map(<[u8]>::to_vec)
        .collect()
}

async fn mount_playlist(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/index.m3u8"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.apple.mpegurl")
                .set_body_string(PLAYLIST),
        )
        .mount(server)
        .await;
}

async fn mount_segment(server: &MockServer, index: usize, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path(format!("/seg{index}.ts")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "video/MP2T")
                .set_body_bytes(body),
        )
        .mount(server)
        .await;
}

async fn playlist_for(server: &MockServer) -> (CreateDownloadJobRequest, HlsMediaPlaylist) {
    let request = request(server);
    let manifest = fetch_hls_manifest_text(&request, &request.source_url)
        .await
        .unwrap();
    let playlist = parse_hls_media_playlist(&request.source_url, &manifest).unwrap();
    assert_eq!(playlist.segments.len(), 3);
    (request, playlist)
}

fn request(server: &MockServer) -> CreateDownloadJobRequest {
    CreateDownloadJobRequest {
        source_url: format!("{}/index.m3u8", server.uri()),
        source_fallback_urls: Vec::new(),
        page_url: Some("https://example.test/watch".to_string()),
        page_title: Some("Owned Stream".to_string()),
        referrer: None,
        suggested_filename: Some("Owned Stream".to_string()),
        content_type: Some("application/vnd.apple.mpegurl".to_string()),
        content_length: None,
        kind: DownloadKind::Video,
        adapter_id: "hls".to_string(),
        provider_id: None,
        selected_option_id: Some("manifest".to_string()),
        headers: None,
        prefer_range_request: false,
        quality_report: None,
    }
}

fn segments() -> Vec<Vec<u8>> {
    vec![pattern(3000, 1), pattern(2500, 2), pattern(4100, 3)]
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|index| (index as u64 * 37 + seed as u64 * 11) as u8)
        .collect()
}

/// The journal the download left behind, opened the way the next run would open it.
async fn journal_of(
    files: &NativeHlsWorkFiles,
    request: &CreateDownloadJobRequest,
    playlist: &HlsMediaPlaylist,
) -> HlsSegmentJournal {
    let transport_filename = files
        .transport_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap();
    let (journal, continued) = HlsSegmentJournal::open(
        &files.journal_path,
        &request.source_url,
        transport_filename,
        &playlist.segments,
    )
    .await
    .unwrap();
    assert!(continued, "the journal should belong to this playlist");
    journal
}

#[tokio::test]
async fn a_long_playlist_is_written_in_order_although_segments_arrive_out_of_order() {
    let server = MockServer::start().await;
    let bodies = (0..12_u8)
        .map(|index| pattern(700 + usize::from(index) * 53, index + 20))
        .collect::<Vec<_>>();
    let mut playlist_text = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n");
    for index in 0..bodies.len() {
        playlist_text.push_str(&format!("#EXTINF:4.0,\nlong{index}.ts\n"));
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    Mock::given(method("GET"))
        .and(path("/index.m3u8"))
        .respond_with(ResponseTemplate::new(200).set_body_string(playlist_text))
        .mount(&server)
        .await;
    for (index, body) in bodies.iter().enumerate() {
        // Every fourth segment answers late, so its successors finish before it does.
        let delay = if index % 4 == 0 { 150 } else { 0 };
        Mock::given(method("GET"))
            .and(path(format!("/long{index}.ts")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(delay))
                    .set_body_bytes(body.clone()),
            )
            .mount(&server)
            .await;
    }
    let request = request(&server);
    let manifest = fetch_hls_manifest_text(&request, &request.source_url)
        .await
        .unwrap();
    let playlist = parse_hls_media_playlist(&request.source_url, &manifest).unwrap();
    let work = tempfile::tempdir().unwrap();
    let files = NativeHlsWorkFiles::new(work.path(), TEMP_STEM);
    let mut updates = Vec::<DownloadProgress>::new();

    let transport_bytes = download_native_hls_segments(
        &request,
        &playlist,
        &files,
        &mut |update| updates.push(update),
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    let expected = bodies.concat();
    assert_eq!(transport_bytes, expected.len() as u64);
    assert_eq!(std::fs::read(&files.transport_path).unwrap(), expected);
    assert_eq!(
        journal_of(&files, &request, &playlist)
            .await
            .completed_count(),
        12
    );
    assert_eq!(updates.last().unwrap().progress_percent, Some(100.0));
    assert!(updates
        .windows(2)
        .all(|pair| pair[0].progress_percent <= pair[1].progress_percent));
}
