//! HTTP download core against a wiremock server: fresh downloads, resume with `If-Range`,
//! changed resources, the segmented path, cancellation, HTTP errors, and name collisions.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use ldm_core::{
    download::{
        download_cancellation_pair, partial_metadata_path, run_http_download,
        run_http_download_with, write_partial_metadata, DownloadOptions, DownloadProgress,
        PartialDownloadMetadata,
    },
    error::CoreError,
    filename::job_partial_path,
};
use ldm_types::download_job::{CreateDownloadJobRequest, DownloadKind};
use uuid::Uuid;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, Request, Respond, ResponseTemplate,
};

const FILENAME: &str = "clip.bin";

#[tokio::test]
async fn downloads_a_file_into_the_target_folder() {
    let server = MockServer::start().await;
    let body = pattern(10 * 1024, 1);
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_bytes(body.clone()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    let progress = ProgressLog::default();

    let completed = run_http_download(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        progress.recorder(),
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(completed.filename, FILENAME);
    assert_eq!(completed.final_path, target.path().join(FILENAME));
    assert_eq!(completed.downloaded_bytes, body.len() as u64);
    assert_eq!(completed.total_bytes, Some(body.len() as u64));
    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    let updates = progress.updates();
    assert!(!updates.is_empty(), "progress callbacks were expected");
    assert_eq!(updates[0].downloaded_bytes, 0);
    assert_eq!(updates.last().unwrap().downloaded_bytes, body.len() as u64);
    assert!(updates
        .iter()
        .all(|update| update.total_bytes == Some(body.len() as u64)));
    assert!(updates
        .windows(2)
        .all(|pair| pair[0].downloaded_bytes <= pair[1].downloaded_bytes));
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn resumes_a_partial_file_with_range_and_if_range() {
    let server = MockServer::start().await;
    let body = pattern(16 * 1024, 2);
    let existing = 4096_usize;
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .and(header("range", "bytes=4096-"))
        .and(header("if-range", "\"v1\""))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header(
                    "content-range",
                    format!("bytes {existing}-{}/{}", body.len() - 1, body.len()),
                )
                .insert_header("etag", "\"v1\"")
                .set_body_bytes(body[existing..].to_vec()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    seed_partial(target.path(), &body[..existing], "\"v1\"").await;
    let progress = ProgressLog::default();

    let completed = run_http_download(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        progress.recorder(),
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    assert_eq!(completed.downloaded_bytes, body.len() as u64);
    assert!(completed.supports_resume);
    assert_eq!(progress.updates()[0].downloaded_bytes, existing as u64);
    assert_no_partial_files(target.path());
    server.verify().await;
}

#[tokio::test]
async fn restarts_from_scratch_when_the_resource_changed() {
    let server = MockServer::start().await;
    let stale = pattern(4096, 3);
    let current = pattern(12 * 1024, 4);
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .and(header("if-range", "\"v1\""))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v2\"")
                .set_body_bytes(current.clone()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    seed_partial(target.path(), &stale, "\"v1\"").await;
    let progress = ProgressLog::default();

    let completed = run_http_download(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        progress.recorder(),
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), current);
    assert_eq!(completed.downloaded_bytes, current.len() as u64);
    assert!(!completed.supports_resume);
    assert_eq!(progress.updates()[0].downloaded_bytes, 0);
    let resume_request = server.received_requests().await.unwrap().remove(0);
    assert_eq!(header_text(&resume_request, "range"), Some("bytes=4096-"));
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn cancel_keeps_the_part_and_validator_and_a_later_run_resumes() {
    let server = MockServer::start().await;
    let body = pattern(4 * 1024 * 1024, 5);
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(RangeServer::new(body.clone(), "\"v1\""))
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    let (cancel, cancellation) = download_cancellation_pair();
    let temp_path = part_path(target.path());

    let first = run_http_download(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        move |update: DownloadProgress| {
            if update.downloaded_bytes > 0 {
                cancel.cancel();
            }
        },
        cancellation,
    )
    .await;

    assert!(matches!(first, Err(CoreError::Canceled)), "{first:?}");
    let kept_bytes = std::fs::metadata(&temp_path).unwrap().len();
    assert!(kept_bytes > 0 && kept_bytes < body.len() as u64);
    let metadata: PartialDownloadMetadata =
        serde_json::from_slice(&std::fs::read(partial_metadata_path(&temp_path)).unwrap()).unwrap();
    assert_eq!(metadata.etag.as_deref(), Some("\"v1\""));
    assert!(!target.path().join(FILENAME).exists());

    let completed = run_http_download(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    let resume_request = server.received_requests().await.unwrap().pop().unwrap();
    assert_eq!(
        header_text(&resume_request, "range"),
        Some(format!("bytes={kept_bytes}-").as_str())
    );
    assert_eq!(header_text(&resume_request, "if-range"), Some("\"v1\""));
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn downloads_large_range_capable_files_in_segments() {
    let server = MockServer::start().await;
    let body = pattern(8 * 1024 + 123, 6);
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(RangeServer::new(body.clone(), "\"seg-v1\""))
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    let mut request = request(&server, "/clip.bin");
    request.prefer_range_request = true;
    let progress = ProgressLog::default();

    let completed = run_http_download_with(
        &request,
        target.path(),
        &part_path(target.path()),
        segmented_options(4, Duration::from_millis(20)),
        progress.recorder(),
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    assert_eq!(completed.downloaded_bytes, body.len() as u64);
    assert_eq!(completed.total_bytes, Some(body.len() as u64));
    assert_eq!(
        progress.updates().last().unwrap().downloaded_bytes,
        body.len() as u64
    );
    let requests = server.received_requests().await.unwrap();
    let segment_requests = requests
        .iter()
        .filter(|request| header_text(request, "range").is_some_and(|range| !range.ends_with('-')))
        .collect::<Vec<_>>();
    assert_eq!(segment_requests.len(), 4, "{requests:?}");
    assert!(segment_requests
        .iter()
        .all(|request| header_text(request, "if-range") == Some("\"seg-v1\"")));
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn cancel_while_the_server_stalls_reports_canceled_and_leaves_no_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(30))
                .set_body_bytes(pattern(1024, 7)),
        )
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    let (cancel, cancellation) = download_cancellation_pair();
    let request = request(&server, "/clip.bin");
    let target_dir = target.path().to_path_buf();
    let download = tokio::spawn(async move {
        run_http_download(
            &request,
            &target_dir,
            &part_path(&target_dir),
            |_| {},
            cancellation,
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), download)
        .await
        .expect("cancel must end the download promptly")
        .unwrap();

    assert!(matches!(result, Err(CoreError::Canceled)), "{result:?}");
    assert!(!target.path().join(FILENAME).exists());
}

#[tokio::test]
async fn http_error_status_fails_without_creating_a_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();

    let result = run_http_download(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        |_| {},
        download_cancellation_pair().1,
    )
    .await;

    assert!(
        matches!(result, Err(CoreError::HttpStatus(status)) if status == reqwest::StatusCode::NOT_FOUND),
        "{result:?}"
    );
    assert!(!target.path().join(FILENAME).exists());
    assert!(!part_path(target.path()).exists());
}

#[tokio::test]
async fn name_collision_adds_a_counter() {
    let server = MockServer::start().await;
    let body = pattern(2048, 8);
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    std::fs::write(target.path().join(FILENAME), b"already here").unwrap();

    let completed = run_http_download(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(completed.filename, "clip (1).bin");
    assert_eq!(completed.final_path, target.path().join("clip (1).bin"));
    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    assert_eq!(
        std::fs::read(target.path().join(FILENAME)).unwrap(),
        b"already here"
    );
}

#[tokio::test]
async fn a_partial_file_that_is_already_complete_is_finished_without_downloading_again() {
    let server = MockServer::start().await;
    let body = pattern(12 * 1024, 15);
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(RangeServer::new(body.clone(), "\"v1\""))
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    seed_partial(target.path(), &body, "\"v1\"").await;
    let progress = ProgressLog::default();

    let completed = run_http_download(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        progress.recorder(),
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    assert_eq!(completed.downloaded_bytes, body.len() as u64);
    assert_eq!(completed.total_bytes, Some(body.len() as u64));
    assert_eq!(
        progress.updates().last().unwrap().downloaded_bytes,
        body.len() as u64
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        header_text(&requests[0], "range"),
        Some(format!("bytes={}-", body.len()).as_str())
    );
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn a_partial_file_longer_than_the_resource_is_dropped_and_downloaded_again() {
    let server = MockServer::start().await;
    let body = pattern(12 * 1024, 16);
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(RangeServer::new(body.clone(), "\"v1\""))
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    let mut too_long = body.clone();
    too_long.extend_from_slice(&[0xAA; 100]);
    seed_partial(target.path(), &too_long, "\"v1\"").await;

    let completed = run_http_download(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert_eq!(header_text(&requests[1], "range"), None);
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn two_jobs_with_the_same_filename_keep_both_files() {
    let server = MockServer::start().await;
    let first_body = pattern(256 * 1024, 9);
    let second_body = pattern(300 * 1024, 10);
    for (route, body) in [("/a/clip.bin", &first_body), ("/b/clip.bin", &second_body)] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
    }
    let target = tempfile::tempdir().unwrap();
    let first_request = request(&server, "/a/clip.bin");
    let second_request = request(&server, "/b/clip.bin");
    let first_part = job_partial_path(target.path(), Uuid::from_u128(11));
    let second_part = job_partial_path(target.path(), Uuid::from_u128(12));

    let (first, second) = tokio::join!(
        run_http_download(
            &first_request,
            target.path(),
            &first_part,
            |_| {},
            download_cancellation_pair().1,
        ),
        run_http_download(
            &second_request,
            target.path(),
            &second_part,
            |_| {},
            download_cancellation_pair().1,
        ),
    );
    let (first, second) = (first.unwrap(), second.unwrap());

    assert_ne!(first.final_path, second.final_path);
    assert_eq!(std::fs::read(&first.final_path).unwrap(), first_body);
    assert_eq!(std::fs::read(&second.final_path).unwrap(), second_body);
    let mut names = [first.filename, second.filename];
    names.sort();
    assert_eq!(names, ["clip (1).bin", "clip.bin"]);
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn a_connection_that_drops_is_continued_without_starting_over() {
    let server = MockServer::start().await;
    let body = pattern(64 * 1024, 17);
    let cut_at = body.len() / 2;
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(CutShortOnce {
            inner: RangeServer::new(body.clone(), "\"v1\""),
            cut_at,
            hits: AtomicUsize::new(0),
        })
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();
    let mut request = request(&server, "/clip.bin");
    request.prefer_range_request = true;

    let completed = run_http_download_with(
        &request,
        target.path(),
        &part_path(target.path()),
        retrying_options(3),
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert_eq!(
        header_text(&requests[1], "range"),
        Some(format!("bytes={cut_at}-").as_str())
    );
    assert_eq!(header_text(&requests[1], "if-range"), Some("\"v1\""));
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn a_busy_server_is_asked_again_and_a_missing_file_is_not() {
    let server = MockServer::start().await;
    let body = pattern(8 * 1024, 18);
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gone.bin"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();

    let completed = run_http_download_with(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        retrying_options(4),
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();
    let missing = run_http_download_with(
        &request(&server, "/gone.bin"),
        target.path(),
        &part_path(target.path()),
        retrying_options(4),
        |_| {},
        download_cancellation_pair().1,
    )
    .await;

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    assert!(
        matches!(missing, Err(CoreError::HttpStatus(status)) if status == reqwest::StatusCode::NOT_FOUND),
        "{missing:?}"
    );
    let requests = server.received_requests().await.unwrap();
    let asked = |route: &str| {
        requests
            .iter()
            .filter(|request| request.url.path() == route)
            .count()
    };
    assert_eq!(asked("/clip.bin"), 3);
    assert_eq!(asked("/gone.bin"), 1);
}

#[tokio::test]
async fn a_server_that_stays_busy_fails_the_download_after_the_last_attempt() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let target = tempfile::tempdir().unwrap();

    let failed = run_http_download_with(
        &request(&server, "/clip.bin"),
        target.path(),
        &part_path(target.path()),
        retrying_options(3),
        |_| {},
        download_cancellation_pair().1,
    )
    .await;

    assert!(
        matches!(failed, Err(CoreError::HttpStatus(status)) if status == reqwest::StatusCode::SERVICE_UNAVAILABLE),
        "{failed:?}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    assert!(!target.path().join(FILENAME).exists());
}

#[tokio::test]
async fn a_segment_that_fails_twice_is_retried_and_the_download_completes() {
    let server = MockServer::start().await;
    let body = pattern(8 * 1024 + 123, 11);
    let fault_start = segment_start(&body, 2);
    mount_faulty(&server, &body, fault_start, Fault::UnavailableTimes(2)).await;
    let target = tempfile::tempdir().unwrap();
    let mut request = request(&server, "/clip.bin");
    request.prefer_range_request = true;

    let completed = run_http_download_with(
        &request,
        target.path(),
        &part_path(target.path()),
        segmented_options(4, Duration::from_millis(20)),
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    let attempts = closed_ranges(&server, 0)
        .await
        .iter()
        .filter(|range| range.starts_with(&format!("bytes={fault_start}-")))
        .count();
    assert_eq!(attempts, 3);
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn a_segment_that_is_cut_short_continues_where_it_stopped() {
    let server = MockServer::start().await;
    let body = pattern(8 * 1024 + 123, 12);
    let segment_len = body.len().div_ceil(4);
    let fault_start = segment_start(&body, 2);
    mount_faulty(&server, &body, fault_start, Fault::CutShortOnce).await;
    let target = tempfile::tempdir().unwrap();
    let mut request = request(&server, "/clip.bin");
    request.prefer_range_request = true;

    let completed = run_http_download_with(
        &request,
        target.path(),
        &part_path(target.path()),
        segmented_options(3, Duration::from_millis(20)),
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    let continued = format!(
        "bytes={}-{}",
        fault_start + segment_len / 2,
        fault_start + segment_len - 1
    );
    let ranges = closed_ranges(&server, 0).await;
    assert!(ranges.contains(&continued), "{continued} not in {ranges:?}");
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn a_segment_that_keeps_failing_fails_the_download_and_keeps_the_other_segments() {
    let server = MockServer::start().await;
    let body = pattern(8 * 1024 + 123, 13);
    let segment_len = body.len().div_ceil(4);
    let fault_start = segment_start(&body, 2);
    let healthy = mount_faulty(&server, &body, fault_start, Fault::AlwaysFails).await;
    let target = tempfile::tempdir().unwrap();
    let mut request = request(&server, "/clip.bin");
    request.prefer_range_request = true;
    let part = part_path(target.path());
    // The pause before the second attempt gives the three healthy segments time to finish.
    let options = segmented_options(2, Duration::from_millis(300));

    let failed = run_http_download_with(
        &request,
        target.path(),
        &part,
        options,
        |_| {},
        download_cancellation_pair().1,
    )
    .await;

    assert!(
        matches!(failed, Err(CoreError::HttpStatus(status)) if status == reqwest::StatusCode::INTERNAL_SERVER_ERROR),
        "{failed:?}"
    );
    assert!(!target.path().join(FILENAME).exists());
    assert_eq!(
        segments_done(&part),
        [segment_len, segment_len, 0, body.len() - 3 * segment_len]
    );
    assert_eq!(
        std::fs::metadata(&part).unwrap().len(),
        body.len() as u64,
        "the partial file is full-size from the start"
    );

    healthy.store(true, Ordering::SeqCst);
    let requests_before = server.received_requests().await.unwrap().len();
    let completed = run_http_download_with(
        &request,
        target.path(),
        &part,
        options,
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    assert_eq!(
        closed_ranges(&server, requests_before).await,
        [format!(
            "bytes={fault_start}-{}",
            fault_start + segment_len - 1
        )]
    );
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn a_server_that_ignores_parallel_ranges_gets_one_connection_instead() {
    let server = MockServer::start().await;
    let body = pattern(8 * 1024 + 123, 14);
    mount_faulty(&server, &body, 0, Fault::IgnoresRanges).await;
    let target = tempfile::tempdir().unwrap();
    let mut request = request(&server, "/clip.bin");
    request.prefer_range_request = true;

    let completed = run_http_download_with(
        &request,
        target.path(),
        &part_path(target.path()),
        segmented_options(3, Duration::from_millis(20)),
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    let whole_file_requests = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| header_text(request, "range") == Some("bytes=0-"))
        .count();
    assert_eq!(whole_file_requests, 2);
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn a_paused_parallel_download_continues_every_segment_where_it_stopped() {
    let server = MockServer::start().await;
    let body = pattern(8 * 1024 + 123, 19);
    let segment_len = body.len().div_ceil(4);
    let fault_start = segment_start(&body, 2);
    let healthy = mount_faulty(&server, &body, fault_start, Fault::Stalls).await;
    let target = tempfile::tempdir().unwrap();
    let mut request = request(&server, "/clip.bin");
    request.prefer_range_request = true;
    let part = part_path(target.path());
    let options = segmented_options(3, Duration::from_millis(20));
    let (cancel, cancellation) = download_cancellation_pair();
    // Everything but the stalled segment has arrived: time to pause.
    let pause_at = (body.len() - segment_len) as u64;

    let paused = run_http_download_with(
        &request,
        target.path(),
        &part,
        options,
        move |update: DownloadProgress| {
            if update.downloaded_bytes >= pause_at {
                cancel.cancel();
            }
        },
        cancellation,
    )
    .await;

    assert!(matches!(paused, Err(CoreError::Canceled)), "{paused:?}");
    assert_eq!(
        segments_done(&part),
        [segment_len, segment_len, 0, body.len() - 3 * segment_len]
    );
    assert!(!target.path().join(FILENAME).exists());

    healthy.store(true, Ordering::SeqCst);
    let requests_before = server.received_requests().await.unwrap().len();
    let progress = ProgressLog::default();
    let completed = run_http_download_with(
        &request,
        target.path(),
        &part,
        options,
        progress.recorder(),
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    assert_eq!(progress.updates()[0].downloaded_bytes, pause_at);
    assert_eq!(
        closed_ranges(&server, requests_before).await,
        [format!(
            "bytes={fault_start}-{}",
            fault_start + segment_len - 1
        )]
    );
    assert_no_partial_files(target.path());
}

#[tokio::test]
async fn a_parallel_partial_file_is_never_mistaken_for_a_complete_one() {
    // The full-size file of an interrupted parallel download, mostly holes, and a server
    // that no longer offers ranges for it: the download has to start over, not "finish".
    let server = MockServer::start().await;
    let body = pattern(8 * 1024 + 123, 20);
    let fault_start = segment_start(&body, 2);
    let healthy = mount_faulty(&server, &body, fault_start, Fault::Stalls).await;
    let target = tempfile::tempdir().unwrap();
    let mut request = request(&server, "/clip.bin");
    request.prefer_range_request = true;
    let part = part_path(target.path());
    let (cancel, cancellation) = download_cancellation_pair();
    let pause_at = (body.len() - body.len().div_ceil(4)) as u64;
    let paused = run_http_download_with(
        &request,
        target.path(),
        &part,
        segmented_options(3, Duration::from_millis(20)),
        move |update: DownloadProgress| {
            if update.downloaded_bytes >= pause_at {
                cancel.cancel();
            }
        },
        cancellation,
    )
    .await;
    assert!(matches!(paused, Err(CoreError::Canceled)), "{paused:?}");
    assert_eq!(std::fs::metadata(&part).unwrap().len(), body.len() as u64);

    // The same job continues with the production thresholds: far too small to be split.
    healthy.store(true, Ordering::SeqCst);
    let completed = run_http_download_with(
        &request,
        target.path(),
        &part,
        retrying_options(3),
        |_| {},
        download_cancellation_pair().1,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&completed.final_path).unwrap(), body);
    assert_no_partial_files(target.path());
}

/// How many bytes of each segment the state file next to `part` records as done.
fn segments_done(part: &Path) -> Vec<usize> {
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{}.segments", part.display())).unwrap())
            .unwrap();
    state["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|segment| segment["done"].as_u64().unwrap() as usize)
        .collect()
}

fn segmented_options(attempts: u32, retry_delay: Duration) -> DownloadOptions {
    DownloadOptions {
        segmented_min_bytes: 1024,
        segment_count: 4,
        attempts,
        retry_delay,
    }
}

/// The production thresholds (one connection for these small bodies) with a short pause.
fn retrying_options(attempts: u32) -> DownloadOptions {
    DownloadOptions {
        attempts,
        retry_delay: Duration::from_millis(20),
        ..DownloadOptions::default()
    }
}

/// A `RangeServer` whose first answer carries only the bytes up to `cut_at`, although it
/// announces the whole file: what a connection that drops mid-download looks like.
struct CutShortOnce {
    inner: RangeServer,
    cut_at: usize,
    hits: AtomicUsize,
}

impl Respond for CutShortOnce {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        if self.hits.fetch_add(1, Ordering::SeqCst) > 0 {
            return self.inner.respond(request);
        }
        let len = self.inner.body.len();
        ResponseTemplate::new(206)
            .insert_header("content-range", format!("bytes 0-{}/{len}", len - 1))
            .insert_header("etag", self.inner.etag)
            .set_body_bytes(self.inner.body[..self.cut_at].to_vec())
    }
}

/// Where segment `index` of a four-part download of `body` begins.
fn segment_start(body: &[u8], index: usize) -> usize {
    index * body.len().div_ceil(4)
}

/// The `Range` values of the segment requests (closed ranges) the server received after the
/// first `skip` requests, in the order they arrived.
async fn closed_ranges(server: &MockServer, skip: usize) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .skip(skip)
        .filter_map(|request| header_text(request, "range"))
        .filter(|range| !range.ends_with('-'))
        .map(str::to_string)
        .collect()
}

/// How a `FaultyRangeServer` misbehaves.
enum Fault {
    /// `503` for the first `n` requests of the segment at `fault_start`.
    UnavailableTimes(usize),
    /// `500` for that segment until the server is marked healthy.
    AlwaysFails,
    /// The first answer for that segment carries only the first half of its range.
    CutShortOnce,
    /// Every segment request is answered with `200` and the whole body.
    IgnoresRanges,
    /// That segment's answer does not arrive until the server is marked healthy.
    Stalls,
}

/// A `RangeServer` whose segment requests go wrong in one specific way.
struct FaultyRangeServer {
    inner: RangeServer,
    fault_start: usize,
    fault: Fault,
    hits: AtomicUsize,
    healthy: Arc<AtomicBool>,
}

impl Respond for FaultyRangeServer {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let range_header = header_text(request, "range").unwrap_or_default();
        let is_segment_request = !range_header.is_empty() && !range_header.ends_with('-');
        if !is_segment_request || self.healthy.load(Ordering::SeqCst) {
            return self.inner.respond(request);
        }
        if matches!(self.fault, Fault::IgnoresRanges) {
            return ResponseTemplate::new(200)
                .insert_header("etag", self.inner.etag)
                .set_body_bytes(self.inner.body.clone());
        }

        let faulty_range = parse_range(range_header, self.inner.body.len())
            .filter(|(start, _)| *start == self.fault_start);
        if let Some((start, end)) = faulty_range {
            let hit = self.hits.fetch_add(1, Ordering::SeqCst);
            match self.fault {
                Fault::UnavailableTimes(times) if hit < times => return ResponseTemplate::new(503),
                Fault::AlwaysFails => return ResponseTemplate::new(500),
                Fault::Stalls => {
                    return self
                        .inner
                        .respond(request)
                        .set_delay(Duration::from_secs(30));
                }
                Fault::CutShortOnce if hit == 0 => {
                    let range_len = end - start + 1;
                    let half = start + range_len / 2;
                    return ResponseTemplate::new(206)
                        .insert_header(
                            "content-range",
                            format!("bytes {start}-{end}/{}", self.inner.body.len()),
                        )
                        .insert_header("etag", self.inner.etag)
                        .set_body_bytes(self.inner.body[start..half].to_vec());
                }
                _ => {}
            }
        }
        self.inner.respond(request)
    }
}

/// Mounts a `FaultyRangeServer` for `/clip.bin` and returns the switch that makes it healthy.
async fn mount_faulty(
    server: &MockServer,
    body: &[u8],
    fault_start: usize,
    fault: Fault,
) -> Arc<AtomicBool> {
    let healthy = Arc::new(AtomicBool::new(false));
    Mock::given(method("GET"))
        .and(path("/clip.bin"))
        .respond_with(FaultyRangeServer {
            inner: RangeServer::new(body.to_vec(), "\"faulty-v1\""),
            fault_start,
            fault,
            hits: AtomicUsize::new(0),
            healthy: healthy.clone(),
        })
        .mount(server)
        .await;
    healthy
}

/// Answers `Range` requests with `206` and the requested slice, everything else with the whole
/// body, and stamps every response with `etag`.
struct RangeServer {
    body: Vec<u8>,
    etag: &'static str,
}

impl RangeServer {
    fn new(body: Vec<u8>, etag: &'static str) -> Self {
        Self { body, etag }
    }
}

impl Respond for RangeServer {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let starts_past_the_end = header_text(request, "range")
            .and_then(range_start)
            .is_some_and(|start| start >= self.body.len());
        if starts_past_the_end {
            return ResponseTemplate::new(416)
                .insert_header("content-range", format!("bytes */{}", self.body.len()))
                .insert_header("etag", self.etag);
        }
        let range =
            header_text(request, "range").and_then(|value| parse_range(value, self.body.len()));
        match range {
            Some((start, end)) => ResponseTemplate::new(206)
                .insert_header(
                    "content-range",
                    format!("bytes {start}-{end}/{}", self.body.len()),
                )
                .insert_header("etag", self.etag)
                .set_body_bytes(self.body[start..=end].to_vec()),
            None => ResponseTemplate::new(200)
                .insert_header("etag", self.etag)
                .set_body_bytes(self.body.clone()),
        }
    }
}

fn range_start(value: &str) -> Option<usize> {
    value
        .strip_prefix("bytes=")?
        .split_once('-')?
        .0
        .parse()
        .ok()
}

fn parse_range(value: &str, len: usize) -> Option<(usize, usize)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    let start = start.parse::<usize>().ok()?;
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<usize>().ok()?.min(len - 1)
    };
    (start <= end).then_some((start, end))
}

fn header_text<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
}

#[derive(Clone, Default)]
struct ProgressLog(Arc<Mutex<Vec<DownloadProgress>>>);

impl ProgressLog {
    fn recorder(&self) -> impl FnMut(DownloadProgress) + Send {
        let log = self.0.clone();
        move |update| log.lock().unwrap().push(update)
    }

    fn updates(&self) -> Vec<DownloadProgress> {
        self.0.lock().unwrap().clone()
    }
}

fn request(server: &MockServer, url_path: &str) -> CreateDownloadJobRequest {
    CreateDownloadJobRequest {
        source_url: format!("{}{url_path}", server.uri()),
        source_fallback_urls: Vec::new(),
        page_url: None,
        page_title: None,
        referrer: None,
        suggested_filename: Some(FILENAME.to_string()),
        content_type: Some("application/octet-stream".to_string()),
        content_length: None,
        kind: DownloadKind::Other,
        adapter_id: "generic-download-links".to_string(),
        provider_id: None,
        selected_option_id: None,
        headers: None,
        prefer_range_request: false,
        quality_report: None,
    }
}

/// Deterministic, non-repeating-looking bytes so misplaced slices are detected.
fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|index| (index as u64 * 31 + seed as u64 * 7 + (index / 251) as u64) as u8)
        .collect()
}

/// The partial file of the one job most tests run.
fn part_path(target_dir: &Path) -> PathBuf {
    job_partial_path(target_dir, Uuid::from_u128(1))
}

async fn seed_partial(target_dir: &Path, bytes: &[u8], etag: &str) -> PathBuf {
    let temp_path = part_path(target_dir);
    std::fs::create_dir_all(temp_path.parent().unwrap()).unwrap();
    std::fs::write(&temp_path, bytes).unwrap();
    write_partial_metadata(
        &temp_path,
        &PartialDownloadMetadata {
            etag: Some(etag.to_string()),
            last_modified: None,
        },
    )
    .await
    .unwrap();
    temp_path
}

fn assert_no_partial_files(target_dir: &Path) {
    let temp_path = part_path(target_dir);
    let leftovers = std::fs::read_dir(temp_path.parent().unwrap())
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "partial files left behind: {leftovers:?}"
    );
}
