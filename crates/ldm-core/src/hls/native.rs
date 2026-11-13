use std::{
    collections::VecDeque,
    io::SeekFrom,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use ldm_types::download_job::CreateDownloadJobRequest;
use reqwest::{header::REFERER, StatusCode};
use tokio::{
    fs::OpenOptions,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    process::Command,
    sync::mpsc,
    task::JoinHandle,
};
use uuid::Uuid;

use crate::{
    download::{
        build_download_plan, CompletedDownload, DownloadCancellation, DownloadProgress, SpeedMeter,
    },
    error::CoreError,
    filename::{ensure_partials_dir_async, move_to_unique_path_async},
    process_tree::ProcessTree,
    provider::common::{
        filename_with_extension, first_non_empty, forwarded_headers, provider_temp_stem,
        source_referrer,
    },
    tool_discovery::{discover_tool, ToolKind},
};

use super::journal::HlsSegmentJournal;
use super::{
    fetch_hls_manifest_text, is_hls_media_playlist, is_live_hls_media_playlist,
    parse_hls_media_playlist, HlsMediaPlaylist,
};

const NATIVE_HLS_PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(500);
const NATIVE_HLS_SEGMENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const NATIVE_HLS_SEGMENT_READ_TIMEOUT: Duration = Duration::from_secs(30);
const NATIVE_HLS_SEGMENT_MAX_ATTEMPTS: usize = 3;
const NATIVE_HLS_SEGMENT_RETRY_BASE_DELAY: Duration = Duration::from_millis(350);
const NATIVE_HLS_SUPPORTS_RESUME: bool = true;
/// How many segments are fetched at once. Each one is held in memory until the segments before
/// it are written, so this also bounds the memory a download needs.
const NATIVE_HLS_PARALLEL_SEGMENTS: usize = 4;
/// How often the transport file is synced and the journal brought up to date. After a crash at
/// most this much is downloaded again.
const NATIVE_HLS_SYNC_INTERVAL: Duration = Duration::from_secs(2);
/// A segment's announced size reserves memory up front only up to this many bytes.
const NATIVE_HLS_SEGMENT_PREALLOCATION_LIMIT: u64 = 16 * 1024 * 1024;

pub fn can_run_native_hls(requests: &[CreateDownloadJobRequest]) -> bool {
    !requests.is_empty()
        && requests
            .iter()
            .all(|request| request.adapter_kind().is_hls())
}

pub async fn run_native_hls_download(
    requests: &[CreateDownloadJobRequest],
    target_dir: &Path,
    job_id: Uuid,
    on_progress: &mut impl FnMut(DownloadProgress),
    cancellation: DownloadCancellation,
) -> Result<CompletedDownload, CoreError> {
    let ffmpeg = discover_tool(ToolKind::Ffmpeg);
    let Some(ffmpeg_path) = ffmpeg.path else {
        return Err(CoreError::ProviderBackend(ffmpeg.message));
    };

    let mut last_error = None;
    for request in requests {
        match run_native_hls_download_attempt(
            request,
            target_dir,
            job_id,
            &ffmpeg_path,
            on_progress,
            cancellation.clone(),
        )
        .await
        {
            Ok(completed) => return Ok(completed),
            // Another rendition of a live stream is just as live.
            Err(error @ (CoreError::Canceled | CoreError::LiveStream)) => return Err(error),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        CoreError::ProviderBackend("native HLS download has no stream attempts".to_string())
    }))
}

async fn run_native_hls_download_attempt(
    request: &CreateDownloadJobRequest,
    target_dir: &Path,
    job_id: Uuid,
    ffmpeg_path: &str,
    on_progress: &mut impl FnMut(DownloadProgress),
    mut cancellation: DownloadCancellation,
) -> Result<CompletedDownload, CoreError> {
    let plan = build_download_plan(request, target_dir)?;
    tokio::fs::create_dir_all(&plan.target_dir).await?;

    let playlist = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(CoreError::Canceled),
        playlist = load_native_hls_playlist(request) => playlist?,
    };

    let work_dir = ensure_partials_dir_async(&plan.target_dir).await?;
    let files = NativeHlsWorkFiles::new(&work_dir, &provider_temp_stem(job_id));
    download_native_hls_segments(
        request,
        &playlist,
        &files,
        on_progress,
        cancellation.clone(),
    )
    .await?;

    remux_native_hls_to_mp4(
        ffmpeg_path,
        &files.transport_path,
        &files.remux_path,
        cancellation.clone(),
    )
    .await?;

    let final_filename = filename_with_extension(&plan.filename, "mp4");
    let final_path =
        move_to_unique_path_async(&files.remux_path, &plan.target_dir, &final_filename).await?;
    let _ = tokio::fs::remove_file(&files.transport_path).await;
    let _ = tokio::fs::remove_file(&files.journal_path).await;
    let downloaded_bytes = tokio::fs::metadata(&final_path).await?.len();
    let filename = final_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&final_filename)
        .to_string();

    Ok(CompletedDownload {
        final_path,
        final_url: Some(request.source_url.clone()),
        quality_report: request.quality_report.clone(),
        filename,
        downloaded_bytes,
        total_bytes: Some(downloaded_bytes),
        supports_resume: NATIVE_HLS_SUPPORTS_RESUME,
    })
}

/// Fetches the media playlist of `request` and checks that the native path can take it: a
/// finished playlist in a layout `parse_hls_media_playlist` supports.
///
/// A playlist that is still growing is `CoreError::LiveStream`, which ends the job instead of
/// falling back to yt-dlp: the native path would save the few segments listed right now and
/// report success, and yt-dlp would record until the broadcast ends.
pub async fn load_native_hls_playlist(
    request: &CreateDownloadJobRequest,
) -> Result<HlsMediaPlaylist, CoreError> {
    let manifest = fetch_hls_manifest_text(request, &request.source_url).await?;
    if !is_hls_media_playlist(&manifest) {
        return Err(CoreError::ProviderBackend(
            "native HLS download needs a media playlist".to_string(),
        ));
    }
    if is_live_hls_media_playlist(&manifest) {
        return Err(CoreError::LiveStream);
    }
    parse_hls_media_playlist(&request.source_url, &manifest)
}

/// The files one native HLS attempt keeps in the partials folder: the concatenated transport
/// stream, the journal of finished segments, and the remuxed MP4.
#[derive(Debug, Clone)]
pub struct NativeHlsWorkFiles {
    pub transport_path: PathBuf,
    pub journal_path: PathBuf,
    pub remux_path: PathBuf,
}

impl NativeHlsWorkFiles {
    pub fn new(work_dir: &Path, temp_stem: &str) -> Self {
        Self {
            transport_path: work_dir.join(format!("{temp_stem}.native.ts")),
            journal_path: work_dir.join(format!("{temp_stem}.native.hls.jsonl")),
            remux_path: work_dir.join(format!("{temp_stem}.native.mp4")),
        }
    }

    fn transport_filename(&self) -> &str {
        self.transport_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("ldm-native-hls.native.ts")
    }
}

/// Downloads every segment of `playlist` into `files.transport_path`, in playlist order.
///
/// Up to `NATIVE_HLS_PARALLEL_SEGMENTS` segments are fetched at once; each arrives whole in
/// memory, and they are written strictly in order. The journal at `files.journal_path` lists
/// the segments that are synced into the transport file, so a later call for the same playlist
/// continues after them (a transport file longer than that is cut back). Transient failures of
/// a segment are retried up to `NATIVE_HLS_SEGMENT_MAX_ATTEMPTS` times. Returns the transport
/// file size. FFmpeg is not involved; `run_native_hls_download` remuxes afterwards.
pub async fn download_native_hls_segments(
    request: &CreateDownloadJobRequest,
    playlist: &HlsMediaPlaylist,
    files: &NativeHlsWorkFiles,
    on_progress: &mut impl FnMut(DownloadProgress),
    mut cancellation: DownloadCancellation,
) -> Result<u64, CoreError> {
    let (mut journal, continued) = HlsSegmentJournal::open(
        &files.journal_path,
        &request.source_url,
        files.transport_filename(),
        &playlist.segments,
    )
    .await?;
    if continued && file_len(&files.transport_path).await? >= journal.completed_bytes() {
        let _ = tokio::fs::remove_file(&files.remux_path).await;
    } else {
        journal.reset().await?;
        cleanup_native_hls_output_artifacts(&files.transport_path, &files.remux_path).await?;
    }

    let segment_count = playlist.segments.len();
    let mut file = open_transport_file(&files.transport_path, journal.completed_bytes()).await?;
    let mut progress = NativeHlsProgress::new(
        segment_count,
        journal.completed_count(),
        journal.completed_bytes(),
        Instant::now(),
    );
    on_progress(progress.update(Instant::now()));

    let fetcher = Arc::new(SegmentFetcher::new(request, segment_count)?);
    let (events, mut received) = mpsc::unbounded_channel::<FetchEvent>();
    // The running fetches in playlist order. Each is a task of its own, so the ones behind
    // the first keep downloading while the first is awaited.
    let mut running = VecDeque::<AbortOnDrop<Vec<u8>>>::with_capacity(NATIVE_HLS_PARALLEL_SEGMENTS);
    let mut next_segment = journal.completed_count();

    // Sizes of the segments written since the journal was last brought up to date.
    let mut unrecorded = Vec::<u64>::new();
    let mut last_sync = Instant::now();
    let outcome = loop {
        while running.len() < NATIVE_HLS_PARALLEL_SEGMENTS {
            let Some(segment) = playlist.segments.get(next_segment) else {
                break;
            };
            running.push_back(AbortOnDrop(tokio::spawn(fetcher.clone().fetch(
                next_segment,
                segment.uri.clone(),
                events.clone(),
                cancellation.clone(),
            ))));
            next_segment += 1;
        }
        let Some(first) = running.front_mut() else {
            break Ok(());
        };

        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break Err(CoreError::Canceled),
            fetched = first.join() => {
                running.pop_front();
                let body = match fetched {
                    Ok(body) => body,
                    Err(error) => break Err(error),
                };
                // Everything the fetch reported is counted before its bytes move from
                // "in memory" to "written".
                while let Ok(event) = received.try_recv() {
                    progress.apply(event);
                }
                if let Err(error) = file.write_all(&body).await {
                    break Err(CoreError::from(error));
                }
                unrecorded.push(body.len() as u64);
                progress.segment_written(body.len() as u64);
                on_progress(progress.update(Instant::now()));

                if last_sync.elapsed() >= NATIVE_HLS_SYNC_INTERVAL {
                    if let Err(error) =
                        record_written_segments(&mut file, &mut journal, &mut unrecorded).await
                    {
                        break Err(error);
                    }
                    last_sync = Instant::now();
                }
            }
            Some(event) = received.recv() => {
                progress.apply(event);
                if let Some(update) = progress.throttled_update(Instant::now()) {
                    on_progress(update);
                }
            }
        }
    };
    // Ends the fetches that are still running.
    drop(running);
    // Whatever ended the loop: what was written is synced and recorded, so the next run
    // continues after it.
    let recorded = record_written_segments(&mut file, &mut journal, &mut unrecorded).await;
    outcome?;
    recorded?;

    let transport_bytes = file.metadata().await?.len();
    drop(file);
    Ok(transport_bytes)
}

/// Syncs the transport file and only then appends the written segments to the journal, which
/// therefore never claims more than the file holds.
async fn record_written_segments(
    file: &mut tokio::fs::File,
    journal: &mut HlsSegmentJournal,
    unrecorded: &mut Vec<u64>,
) -> Result<(), CoreError> {
    if unrecorded.is_empty() {
        return Ok(());
    }
    file.flush().await?;
    file.sync_data().await?;
    journal.append(unrecorded).await?;
    unrecorded.clear();
    Ok(())
}

/// A spawned task that ends when the handle is dropped, so abandoned fetches do not run on.
struct AbortOnDrop<T>(JoinHandle<Result<T, CoreError>>);

impl<T> AbortOnDrop<T> {
    /// Waits for the task. Giving up the wait leaves the task running; call it again later,
    /// but not once it has returned.
    async fn join(&mut self) -> Result<T, CoreError> {
        match (&mut self.0).await {
            Ok(result) => result,
            Err(error) => Err(CoreError::ProviderBackend(format!(
                "an HLS segment download stopped unexpectedly: {error}"
            ))),
        }
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug)]
enum NativeHlsSegmentError {
    Canceled,
    Empty,
    Request(reqwest::Error),
    Status(StatusCode),
}

impl NativeHlsSegmentError {
    fn is_retryable(&self) -> bool {
        match self {
            Self::Canceled => false,
            Self::Empty | Self::Request(_) => true,
            Self::Status(status) => native_hls_status_is_retryable(*status),
        }
    }

    fn into_core_error(self) -> CoreError {
        match self {
            Self::Canceled => CoreError::Canceled,
            Self::Empty => {
                CoreError::ProviderBackend("HLS segment downloaded no media bytes.".to_string())
            }
            Self::Request(error) => error.into(),
            Self::Status(status) => CoreError::HttpStatus(status),
        }
    }

    fn summary(&self) -> String {
        match self {
            Self::Canceled => "canceled".to_string(),
            Self::Empty => "empty segment".to_string(),
            Self::Request(error) if error.is_timeout() => "request timed out".to_string(),
            Self::Request(error) if error.is_connect() => "connection failed".to_string(),
            Self::Request(error) => error.to_string(),
            Self::Status(status) => format!("HTTP {}", status.as_u16()),
        }
    }
}

/// What a running fetch tells the progress display.
#[derive(Debug, Clone, Copy)]
enum FetchEvent {
    /// Bytes of a segment arrived in memory.
    Received(u64),
    /// An attempt failed and its bytes were thrown away.
    Discarded(u64),
}

/// What every segment request of one download shares.
struct SegmentFetcher {
    client: reqwest::Client,
    referrer: Option<String>,
    headers: Vec<(String, String)>,
    segment_count: usize,
}

impl SegmentFetcher {
    fn new(request: &CreateDownloadJobRequest, segment_count: usize) -> Result<Self, CoreError> {
        Ok(Self {
            client: native_hls_segment_client()?,
            referrer: source_referrer(request).map(str::to_string),
            headers: forwarded_headers(request)
                .map(|(name, value)| (name, value.to_string()))
                .collect(),
            segment_count,
        })
    }

    /// Fetches one segment into memory and tries again after transient failures.
    async fn fetch(
        self: Arc<Self>,
        index: usize,
        uri: String,
        events: mpsc::UnboundedSender<FetchEvent>,
        mut cancellation: DownloadCancellation,
    ) -> Result<Vec<u8>, CoreError> {
        for attempt in 1..=NATIVE_HLS_SEGMENT_MAX_ATTEMPTS {
            if attempt > 1 {
                wait_for_native_hls_segment_retry(&mut cancellation, attempt).await?;
            }

            let mut attempt_bytes = 0_u64;
            let error = match self
                .fetch_once(&uri, &events, &mut cancellation, &mut attempt_bytes)
                .await
            {
                Ok(body) => return Ok(body),
                Err(NativeHlsSegmentError::Canceled) => return Err(CoreError::Canceled),
                Err(error) => error,
            };
            let _ = events.send(FetchEvent::Discarded(attempt_bytes));
            if !(error.is_retryable() && attempt < NATIVE_HLS_SEGMENT_MAX_ATTEMPTS) {
                let summary = error.summary();
                return Err(native_hls_segment_failure(
                    index,
                    self.segment_count,
                    attempt,
                    error,
                    &summary,
                ));
            }
        }

        Err(CoreError::ProviderBackend(format!(
            "HLS segment {}/{} failed after {} attempts.",
            index + 1,
            self.segment_count,
            NATIVE_HLS_SEGMENT_MAX_ATTEMPTS
        )))
    }

    async fn fetch_once(
        &self,
        uri: &str,
        events: &mpsc::UnboundedSender<FetchEvent>,
        cancellation: &mut DownloadCancellation,
        attempt_bytes: &mut u64,
    ) -> Result<Vec<u8>, NativeHlsSegmentError> {
        let mut request = self.client.get(uri);
        if let Some(referrer) = &self.referrer {
            request = request.header(REFERER, referrer);
        }
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }

        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(NativeHlsSegmentError::Canceled),
            response = request.send() => response.map_err(NativeHlsSegmentError::Request)?,
        };
        if !response.status().is_success() {
            return Err(NativeHlsSegmentError::Status(response.status()));
        }

        let announced = response
            .content_length()
            .unwrap_or(0)
            .min(NATIVE_HLS_SEGMENT_PREALLOCATION_LIMIT);
        let mut body = Vec::with_capacity(announced as usize);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(NativeHlsSegmentError::Canceled),
            chunk = stream.next() => chunk,
        } {
            let chunk = chunk.map_err(NativeHlsSegmentError::Request)?;
            body.extend_from_slice(&chunk);
            *attempt_bytes += chunk.len() as u64;
            let _ = events.send(FetchEvent::Received(chunk.len() as u64));
        }

        if body.is_empty() {
            Err(NativeHlsSegmentError::Empty)
        } else {
            Ok(body)
        }
    }
}

/// Progress of a native HLS download: the segments in the transport file plus what the
/// parallel fetches hold in memory.
#[derive(Debug)]
struct NativeHlsProgress {
    segment_count: usize,
    written_segments: usize,
    written_bytes: u64,
    /// Received by fetches whose segment is not written yet.
    buffered_bytes: u64,
    meter: SpeedMeter,
    last_update: Instant,
    /// The highest percentage handed out so far.
    reported_percent: f64,
}

impl NativeHlsProgress {
    /// Starts with the segments an earlier run left in the transport file.
    fn new(
        segment_count: usize,
        written_segments: usize,
        written_bytes: u64,
        now: Instant,
    ) -> Self {
        Self {
            segment_count,
            written_segments: written_segments.min(segment_count),
            written_bytes,
            buffered_bytes: 0,
            meter: SpeedMeter::new(now, written_bytes),
            last_update: now,
            reported_percent: 0.0,
        }
    }

    fn apply(&mut self, event: FetchEvent) {
        match event {
            FetchEvent::Received(bytes) => {
                self.buffered_bytes = self.buffered_bytes.saturating_add(bytes);
            }
            FetchEvent::Discarded(bytes) => {
                self.buffered_bytes = self.buffered_bytes.saturating_sub(bytes);
            }
        }
    }

    fn segment_written(&mut self, bytes: u64) {
        self.written_segments = (self.written_segments + 1).min(self.segment_count);
        self.written_bytes = self.written_bytes.saturating_add(bytes);
        self.buffered_bytes = self.buffered_bytes.saturating_sub(bytes);
    }

    /// An update for bytes that arrived, at most every `NATIVE_HLS_PROGRESS_EMIT_INTERVAL`.
    fn throttled_update(&mut self, now: Instant) -> Option<DownloadProgress> {
        (now.duration_since(self.last_update) >= NATIVE_HLS_PROGRESS_EMIT_INTERVAL)
            .then(|| self.update(now))
    }

    fn update(&mut self, now: Instant) -> DownloadProgress {
        self.last_update = now;
        let downloaded_bytes = self.written_bytes + self.buffered_bytes;
        let speed = self.meter.record(now, downloaded_bytes);
        DownloadProgress {
            downloaded_bytes,
            total_bytes: None,
            progress_percent: self.rising_percent(),
            speed_bytes_per_second: speed,
            eta_seconds: self.eta_seconds(speed),
            supports_resume: NATIVE_HLS_SUPPORTS_RESUME,
        }
    }

    /// The typical size of a segment, known once one is written.
    fn average_segment_bytes(&self) -> Option<f64> {
        (self.written_segments > 0 && self.written_bytes > 0)
            .then(|| self.written_bytes as f64 / self.written_segments as f64)
    }

    /// `percent`, but never lower than what was handed out before. The share of the bytes in
    /// memory is an estimate that shifts with every written segment, and a failed attempt
    /// takes its bytes back; the display should not step backwards for either.
    fn rising_percent(&mut self) -> Option<f64> {
        let percent = self.percent()?.max(self.reported_percent);
        self.reported_percent = percent;
        Some(percent)
    }

    /// Written segments plus what the bytes in memory amount to in segments.
    fn percent(&self) -> Option<f64> {
        if self.segment_count == 0 {
            return None;
        }
        let remaining_segments = (self.segment_count - self.written_segments) as f64;
        // The bytes in memory belong to the segments being fetched, however large they are.
        let buffered_segments = self
            .average_segment_bytes()
            .map_or(0.0, |average| self.buffered_bytes as f64 / average)
            .min(remaining_segments)
            .min(NATIVE_HLS_PARALLEL_SEGMENTS as f64);
        let fraction =
            (self.written_segments as f64 + buffered_segments) / self.segment_count as f64;
        Some((fraction * 100.0).clamp(0.0, 100.0))
    }

    fn eta_seconds(&self, speed_bytes_per_second: Option<u64>) -> Option<u64> {
        let speed = speed_bytes_per_second.filter(|speed| *speed > 0)?;
        let average = self.average_segment_bytes()?;
        let remaining_segments = (self.segment_count - self.written_segments) as f64;
        let remaining_bytes = (remaining_segments * average - self.buffered_bytes as f64).max(0.0);
        Some((remaining_bytes / speed as f64).ceil() as u64)
    }
}

fn native_hls_segment_client() -> Result<reqwest::Client, CoreError> {
    reqwest::Client::builder()
        .connect_timeout(NATIVE_HLS_SEGMENT_CONNECT_TIMEOUT)
        .read_timeout(NATIVE_HLS_SEGMENT_READ_TIMEOUT)
        .build()
        .map_err(CoreError::from)
}

async fn wait_for_native_hls_segment_retry(
    cancellation: &mut DownloadCancellation,
    attempt: usize,
) -> Result<(), CoreError> {
    let multiplier = attempt.saturating_sub(1) as u64;
    let delay = Duration::from_millis(
        NATIVE_HLS_SEGMENT_RETRY_BASE_DELAY
            .as_millis()
            .try_into()
            .unwrap_or(350_u64)
            * multiplier.max(1),
    );

    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(CoreError::Canceled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

fn native_hls_segment_failure(
    segment_index: usize,
    segment_count: usize,
    attempt_count: usize,
    error: NativeHlsSegmentError,
    error_summary: &str,
) -> CoreError {
    match error {
        NativeHlsSegmentError::Status(status) => {
            if status == StatusCode::GONE {
                return CoreError::HttpStatus(status);
            }
            CoreError::ProviderBackend(format!(
                "HLS segment {}/{} failed after {} attempt(s): {error_summary}",
                segment_index + 1,
                segment_count,
                attempt_count
            ))
        }
        _ => {
            let core_error = error.into_core_error();
            match core_error {
                CoreError::Canceled => CoreError::Canceled,
                _ => CoreError::ProviderBackend(format!(
                    "HLS segment {}/{} failed after {} attempt(s): {error_summary}",
                    segment_index + 1,
                    segment_count,
                    attempt_count
                )),
            }
        }
    }
}

fn native_hls_status_is_retryable(status: StatusCode) -> bool {
    crate::http::is_transient_status(status)
}

async fn file_len(path: &Path) -> Result<u64, CoreError> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

/// Opens the transport file truncated to `keep_bytes` with the cursor at its end.
///
/// Not opened in append mode: on Windows an append handle lacks `FILE_WRITE_DATA`, so the
/// `set_len` in `truncate_transport_file` would fail with "access denied".
async fn open_transport_file(path: &Path, keep_bytes: u64) -> Result<tokio::fs::File, CoreError> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .await?;
    truncate_transport_file(&mut file, keep_bytes).await?;
    Ok(file)
}

/// Cuts the transport file back to `len` bytes and moves the cursor there, so the next write
/// continues at the end instead of leaving a zero-filled gap.
async fn truncate_transport_file(file: &mut tokio::fs::File, len: u64) -> Result<(), CoreError> {
    file.set_len(len).await?;
    file.seek(SeekFrom::Start(len)).await?;
    Ok(())
}

/// Lets FFmpeg repackage the transport stream as MP4 without re-encoding. A cancel ends FFmpeg
/// and returns once it is gone.
async fn remux_native_hls_to_mp4(
    ffmpeg_path: &str,
    input_path: &Path,
    output_path: &Path,
    mut cancellation: DownloadCancellation,
) -> Result<(), CoreError> {
    if cancellation.is_cancelled() {
        return Err(CoreError::Canceled);
    }

    let (mut child, tree) = ProcessTree::spawn_async(
        Command::new(ffmpeg_path)
            .arg("-y")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-i")
            .arg(input_path)
            .arg("-c")
            .arg("copy")
            .arg("-movflags")
            .arg("+faststart")
            .arg(output_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )
    .map_err(|error| {
        CoreError::ProviderBackend(format!("could not start FFmpeg for HLS remux: {error}"))
    })?;

    let mut stderr_pipe = child.stderr.take();
    // Read while FFmpeg runs: once the pipe is full, FFmpeg would wait for a reader forever.
    let read_stderr = async {
        let mut text = Vec::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut text).await;
        }
        text
    };
    let finished = tokio::select! {
        biased;
        _ = cancellation.cancelled() => None,
        finished = async { tokio::join!(child.wait(), read_stderr) } => Some(finished),
    };
    let Some((status, stderr)) = finished else {
        tree.terminate();
        let _ = child.wait().await;
        return Err(CoreError::Canceled);
    };

    if !status?.success() {
        let _ = tokio::fs::remove_file(output_path).await;
        return Err(CoreError::ProviderBackend(format!(
            "HLS remux failed: {}",
            first_non_empty(
                String::from_utf8_lossy(&stderr).trim(),
                "",
                "FFmpeg exited with an error"
            )
        )));
    }

    Ok(())
}

async fn cleanup_native_hls_output_artifacts(
    transport_path: &Path,
    remux_path: &Path,
) -> Result<(), CoreError> {
    remove_file_if_exists(transport_path).await?;
    remove_file_if_exists(remux_path).await?;
    Ok(())
}

async fn remove_file_if_exists(path: &Path) -> Result<(), CoreError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(segment_count: usize, written: usize, bytes: u64) -> (NativeHlsProgress, Instant) {
        let now = Instant::now();
        (
            NativeHlsProgress::new(segment_count, written, bytes, now),
            now,
        )
    }

    #[test]
    fn progress_starts_from_the_segments_an_earlier_run_wrote() {
        let (mut progress, now) = progress(4, 2, 2048);

        let update = progress.update(now);

        assert!(update.supports_resume);
        assert_eq!(update.downloaded_bytes, 2048);
        assert_eq!(update.progress_percent, Some(50.0));
        assert_eq!(update.total_bytes, None);
    }

    #[test]
    fn bytes_in_memory_count_as_part_of_a_segment() {
        let (mut progress, now) = progress(4, 2, 2000);

        // Half of a typical segment has arrived for the third one.
        progress.apply(FetchEvent::Received(500));
        let update = progress.update(now);

        assert_eq!(update.downloaded_bytes, 2500);
        assert_eq!(update.progress_percent, Some(62.5));
    }

    #[test]
    fn a_failed_attempt_takes_its_bytes_back() {
        let (mut progress, now) = progress(4, 2, 2000);
        progress.apply(FetchEvent::Received(500));

        progress.apply(FetchEvent::Discarded(500));
        let update = progress.update(now);

        assert_eq!(update.downloaded_bytes, 2000);
        assert_eq!(update.progress_percent, Some(50.0));
    }

    #[test]
    fn the_reported_percentage_never_steps_back() {
        let (mut progress, now) = progress(4, 2, 2000);
        progress.apply(FetchEvent::Received(500));
        assert_eq!(progress.update(now).progress_percent, Some(62.5));

        // The attempt fails: the estimate drops, the display holds.
        progress.apply(FetchEvent::Discarded(500));
        assert_eq!(progress.percent(), Some(50.0));
        let held = progress.update(now);
        assert_eq!(held.downloaded_bytes, 2000);
        assert_eq!(held.progress_percent, Some(62.5));

        progress.apply(FetchEvent::Received(1000));
        progress.segment_written(1000);
        assert_eq!(progress.update(now).progress_percent, Some(75.0));
    }

    #[test]
    fn bytes_in_memory_never_count_for_more_segments_than_are_fetched_at_once() {
        let (mut progress, now) = progress(100, 1, 10);

        // Far more than four typical segments, because the first one was tiny.
        progress.apply(FetchEvent::Received(1_000_000));

        assert_eq!(progress.update(now).progress_percent, Some(5.0));
    }

    #[test]
    fn a_written_segment_moves_its_bytes_out_of_memory() {
        let (mut progress, now) = progress(3, 0, 0);
        progress.apply(FetchEvent::Received(1000));
        progress.apply(FetchEvent::Received(400));

        progress.segment_written(1000);
        let update = progress.update(now);

        assert_eq!(update.downloaded_bytes, 1400);
        assert_eq!(progress.written_segments, 1);
        assert_eq!(progress.buffered_bytes, 400);

        progress.segment_written(1200);
        progress.segment_written(900);
        let finished = progress.update(now);
        assert_eq!(finished.progress_percent, Some(100.0));
        assert_eq!(finished.downloaded_bytes, 3100);
    }

    #[test]
    fn the_first_bytes_have_no_percentage_to_stand_on() {
        let (mut progress, now) = progress(4, 0, 0);
        progress.apply(FetchEvent::Received(700));

        let update = progress.update(now);

        assert_eq!(update.progress_percent, Some(0.0));
        assert_eq!(update.eta_seconds, None);
    }

    #[test]
    fn remaining_time_comes_from_the_typical_segment_size() {
        let (progress, _) = progress(10, 4, 4000);

        // Six segments of about 1000 bytes are left.
        assert_eq!(progress.eta_seconds(Some(1000)), Some(6));
        assert_eq!(progress.eta_seconds(Some(0)), None);
        assert_eq!(progress.eta_seconds(None), None);
    }

    #[test]
    fn updates_for_arriving_bytes_are_throttled() {
        let (mut progress, now) = progress(4, 0, 0);

        assert!(progress
            .throttled_update(now + Duration::from_millis(100))
            .is_none());
        assert!(progress
            .throttled_update(now + NATIVE_HLS_PROGRESS_EMIT_INTERVAL)
            .is_some());
        assert!(progress
            .throttled_update(now + NATIVE_HLS_PROGRESS_EMIT_INTERVAL + Duration::from_millis(100))
            .is_none());
    }

    #[test]
    fn native_hls_retries_transient_segment_statuses() {
        assert!(native_hls_status_is_retryable(
            StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(native_hls_status_is_retryable(
            StatusCode::TOO_MANY_REQUESTS
        ));
        assert!(!native_hls_status_is_retryable(StatusCode::FORBIDDEN));
        assert!(!native_hls_status_is_retryable(StatusCode::GONE));
    }
}
