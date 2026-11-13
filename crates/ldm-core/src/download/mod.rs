use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use ldm_types::download_job::CreateDownloadJobRequest;
use reqwest::{
    header::{CONTENT_LENGTH, CONTENT_RANGE, IF_RANGE, RANGE, REFERER},
    Client, RequestBuilder,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use crate::diagnostics::sanitize_diagnostic_message;
use crate::error::CoreError;
use crate::filename::{
    ensure_partials_dir_async, move_to_unique_path_async, partials_dir, safe_filename,
};
use crate::request::validate_download_request;

mod resume;
mod segmented;
mod sparse;
mod speed;

pub use speed::{eta_seconds, SpeedMeter};

pub use resume::{
    partial_metadata_path, read_partial_metadata, remove_partial_metadata, write_partial_metadata,
    PartialDownloadMetadata,
};

use segmented::{run_segmented_http_download, SegmentedDownloadTarget, SegmentedFailure};

/// Tunables of the HTTP download path.
///
/// Production uses `Default`; tests shrink the segmented thresholds so a few kilobytes
/// exercise the parallel path against a mock server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadOptions {
    /// A fresh, resumable download of at least this many bytes is split into
    /// `segment_count` ranges that download in parallel (`segmented`).
    pub segmented_min_bytes: u64,
    /// How many ranges a segmented download requests at once.
    pub segment_count: u64,
    /// How many attempts in a row may bring no new bytes before the download fails. An attempt
    /// that made progress starts the count again. Applies to a download over one connection
    /// and to each segment of a parallel one.
    pub attempts: u32,
    /// Pause before the next attempt; doubled for every further fruitless one.
    pub retry_delay: Duration,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            segmented_min_bytes: 32 * 1024 * 1024,
            segment_count: 4,
            attempts: 4,
            retry_delay: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadPlan {
    pub filename: String,
    pub target_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub progress_percent: Option<f64>,
    pub speed_bytes_per_second: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub supports_resume: bool,
}

#[derive(Debug, Clone)]
pub struct CompletedDownload {
    pub final_path: PathBuf,
    pub final_url: Option<String>,
    pub quality_report: Option<String>,
    pub filename: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub supports_resume: bool,
}

impl DownloadProgress {
    pub fn byte_progress(
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        supports_resume: bool,
    ) -> Self {
        Self {
            downloaded_bytes,
            total_bytes,
            progress_percent: None,
            speed_bytes_per_second: None,
            eta_seconds: None,
            supports_resume,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadCancelHandle {
    sender: watch::Sender<bool>,
}

#[derive(Debug, Clone)]
pub struct DownloadCancellation {
    receiver: watch::Receiver<bool>,
}

pub fn download_cancellation_pair() -> (DownloadCancelHandle, DownloadCancellation) {
    let (sender, receiver) = watch::channel(false);
    (
        DownloadCancelHandle { sender },
        DownloadCancellation { receiver },
    )
}

impl DownloadCancelHandle {
    pub fn cancel(&self) {
        let _ = self.sender.send(true);
    }
}

impl DownloadCancellation {
    /// Resolves once the handle canceled. A handle that is dropped without canceling leaves
    /// the download running: nobody can cancel it any more, which is not the same as a cancel,
    /// and treating it as one would end the task while the job still reads "running".
    pub async fn cancelled(&mut self) {
        while !*self.receiver.borrow() {
            if self.receiver.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        *self.receiver.borrow()
    }
}

/// Validates the request and decides the filename inside `target_dir`.
///
/// The target folder is chosen by the caller (the desktop app resolves it from the user's
/// settings when the job is created), so the core never guesses where files should go.
pub fn build_download_plan(
    request: &CreateDownloadJobRequest,
    target_dir: &Path,
) -> Result<DownloadPlan, CoreError> {
    validate_download_request(request)?;
    let filename = safe_filename(request.suggested_filename.as_deref(), &request.source_url)?;

    Ok(DownloadPlan {
        filename,
        target_dir: target_dir.to_path_buf(),
    })
}

/// Downloads `request` into `target_dir`.
///
/// `temp_path` is where the partial file lives until the download is complete. The caller owns
/// that choice (the queue gives every job its own, see `job_partial_path`) and passes the same
/// path again to resume. The finished file gets a free name in `target_dir` only at the end,
/// so a file that appears there meanwhile is never overwritten.
pub async fn run_http_download(
    request: &CreateDownloadJobRequest,
    target_dir: &Path,
    temp_path: &Path,
    on_progress: impl FnMut(DownloadProgress) + Send,
    cancellation: DownloadCancellation,
) -> Result<CompletedDownload, CoreError> {
    run_http_download_with(
        request,
        target_dir,
        temp_path,
        DownloadOptions::default(),
        on_progress,
        cancellation,
    )
    .await
}

/// `run_http_download` with explicit `options`; see `DownloadOptions`.
///
/// A connection that drops, a timeout, or a "try again later" status does not fail the
/// download: after a pause the next attempt continues the partial file. Only attempts that
/// leave the partial file no longer than they found it count against `options.attempts`.
pub async fn run_http_download_with(
    request: &CreateDownloadJobRequest,
    target_dir: &Path,
    temp_path: &Path,
    options: DownloadOptions,
    mut on_progress: impl FnMut(DownloadProgress) + Send,
    mut cancellation: DownloadCancellation,
) -> Result<CompletedDownload, CoreError> {
    let plan = build_download_plan(request, target_dir)?;
    tokio::fs::create_dir_all(&plan.target_dir).await?;
    prepare_partial_dir(&plan.target_dir, temp_path).await?;

    let mut fruitless_attempts = 0_u32;
    loop {
        let before = existing_file_len(temp_path).await?;
        let attempt = download_attempt(
            request,
            &plan,
            temp_path,
            options,
            &mut on_progress,
            cancellation.clone(),
        )
        .await;
        let error = match attempt {
            Ok(completed) => return Ok(completed),
            Err(AttemptError::Transient(error)) => error,
            Err(AttemptError::Final(error)) => return Err(error),
        };

        let after = existing_file_len(temp_path).await.unwrap_or(before);
        fruitless_attempts = if after > before {
            1
        } else {
            fruitless_attempts + 1
        };
        if fruitless_attempts >= options.attempts {
            return Err(error);
        }
        let delay = retry_delay(options.retry_delay, fruitless_attempts);
        tracing::info!(
            "download interrupted, next attempt in {delay:?}: {}",
            sanitize_diagnostic_message(&error.to_string())
        );
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(CoreError::Canceled),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// The pause before the next attempt: `base`, doubled for every fruitless attempt before it.
fn retry_delay(base: Duration, fruitless_attempts: u32) -> Duration {
    base.saturating_mul(1 << fruitless_attempts.saturating_sub(1).min(6))
}

/// How one attempt of a download ended when it did not complete.
enum AttemptError {
    /// The connection or the server failed in a way another attempt can get past.
    Transient(CoreError),
    /// Another attempt would end the same way, or the download was canceled.
    Final(CoreError),
}

impl AttemptError {
    fn from_request(error: reqwest::Error) -> Self {
        if crate::http::is_transient_request_error(&error) {
            Self::Transient(error.into())
        } else {
            Self::Final(error.into())
        }
    }

    fn from_status(status: reqwest::StatusCode) -> Self {
        if crate::http::is_transient_status(status) {
            Self::Transient(CoreError::HttpStatus(status))
        } else {
            Self::Final(CoreError::HttpStatus(status))
        }
    }
}

impl From<CoreError> for AttemptError {
    fn from(error: CoreError) -> Self {
        Self::Final(error)
    }
}

/// One attempt: a request that continues the partial file when there is one, and the stream of
/// its body into that file.
async fn download_attempt(
    request: &CreateDownloadJobRequest,
    plan: &DownloadPlan,
    temp_path: &Path,
    options: DownloadOptions,
    on_progress: &mut (impl FnMut(DownloadProgress) + Send),
    mut cancellation: DownloadCancellation,
) -> Result<CompletedDownload, AttemptError> {
    // The partial file of a parallel download is as long as the whole file from the start.
    // Its length is not progress, so it must never be continued or finished as if it were.
    let mut segmented_partial = segmented::has_segment_state(temp_path).await;
    let mut existing_bytes = if segmented_partial {
        0
    } else {
        existing_file_len(temp_path).await?
    };
    // The validators of the response the partial data came from, when a previous attempt
    // stored them. They go out as `If-Range` next to every `Range` header.
    let mut metadata = read_partial_metadata(temp_path).await?;

    let client = crate::http::download_client();
    let mut allow_segmented_download = true;

    let (response, supports_resume, append_to_part, starting_bytes, total_bytes, response_total) = loop {
        let range_header = range_header_value(request, existing_bytes);
        let if_range_header = range_header
            .as_ref()
            .and(metadata.as_ref())
            .and_then(PartialDownloadMetadata::if_range_value)
            .map(str::to_string);
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(CoreError::Canceled.into()),
            response = send_download_request(
                client,
                request,
                range_header,
                if_range_header,
            ) => response.map_err(AttemptError::from_request)?,
        };
        let status = response.status();
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && existing_bytes > 0 {
            if unsatisfied_range_total(&response) == Some(existing_bytes) {
                // The partial file already holds every byte: the app stopped between the
                // last write and the rename. Nothing is left to download.
                let final_url = response.url().to_string();
                on_progress(DownloadProgress::byte_progress(
                    existing_bytes,
                    Some(existing_bytes),
                    true,
                ));
                remove_legacy_segment_files(temp_path).await?;
                let final_path =
                    move_to_unique_path_async(temp_path, &plan.target_dir, &plan.filename).await?;
                remove_partial_metadata(temp_path).await?;
                return Ok(CompletedDownload {
                    filename: final_file_name(&final_path, &plan.filename),
                    final_path,
                    final_url: Some(final_url),
                    quality_report: None,
                    downloaded_bytes: existing_bytes,
                    total_bytes: Some(existing_bytes),
                    supports_resume: true,
                });
            }

            // The partial file does not fit the resource (any more): start over without it.
            discard_partial_download(temp_path).await?;
            existing_bytes = 0;
            metadata = None;
            continue;
        }
        if !status.is_success() {
            return Err(AttemptError::from_status(status));
        }

        // A `200` to a resume means the `If-Range` validator no longer matched (or the server
        // does not do ranges): the partial data is stale and the file starts over.
        let supports_resume = status == reqwest::StatusCode::PARTIAL_CONTENT;
        let append_to_part = supports_resume && existing_bytes > 0;
        let starting_bytes = if append_to_part { existing_bytes } else { 0 };
        // What this response says about the size; the request's own guess is only for display.
        let response_total = total_bytes_from_response(&response, starting_bytes);
        let total_bytes = response_total.or(request.content_length);

        if !append_to_part || metadata.is_none() {
            let fresh = PartialDownloadMetadata::from_response(&response);
            write_partial_metadata(temp_path, &fresh).await?;
            metadata = Some(fresh);
        }
        let if_range = metadata
            .as_ref()
            .and_then(PartialDownloadMetadata::if_range_value)
            .map(str::to_string);

        if allow_segmented_download
            && should_use_segmented_download(
                request,
                supports_resume,
                existing_bytes,
                total_bytes,
                options,
            )
        {
            drop(response);
            let total_bytes = total_bytes.expect("checked by should_use_segmented_download");
            match run_segmented_http_download(
                client,
                request,
                options,
                if_range.clone(),
                SegmentedDownloadTarget {
                    target_dir: &plan.target_dir,
                    filename: &plan.filename,
                    temp_path,
                    total_bytes,
                },
                on_progress,
                cancellation.clone(),
            )
            .await
            {
                Ok(completed) => return Ok(completed),
                // The partial file and its segment state stay: the next attempt of this
                // job continues every segment where it stopped. The segments have made
                // their own attempts, so this failure is final.
                Err(SegmentedFailure::Failed(error)) => return Err(AttemptError::Final(error)),
                Err(SegmentedFailure::UseSingleConnection) => {
                    tracing::info!("the server refused parallel ranges, using one connection");
                    segmented::discard_segmented_partial(temp_path).await?;
                    segmented_partial = false;
                    allow_segmented_download = false;
                    continue;
                }
            }
        }

        break (
            response,
            supports_resume,
            append_to_part,
            starting_bytes,
            total_bytes,
            response_total,
        );
    };
    let final_url = response.url().to_string();

    if segmented_partial {
        // An earlier parallel attempt that this one does not continue.
        segmented::discard_segmented_partial(temp_path).await?;
    }
    let mut file = open_part_file(temp_path, append_to_part).await?;
    let mut downloaded_bytes = starting_bytes;
    let mut meter = SpeedMeter::new(Instant::now(), starting_bytes);
    let mut stream = response.bytes_stream();

    on_progress(DownloadProgress::byte_progress(
        downloaded_bytes,
        total_bytes,
        supports_resume,
    ));

    let outcome = loop {
        let chunk = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break Err(AttemptError::Final(CoreError::Canceled)),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break Ok(());
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => break Err(AttemptError::from_request(error)),
        };
        if let Err(error) = file.write_all(&chunk).await {
            break Err(AttemptError::Final(error.into()));
        }
        downloaded_bytes += chunk.len() as u64;
        on_progress(meter.progress(
            Instant::now(),
            downloaded_bytes,
            total_bytes,
            supports_resume,
        ));
    };
    // Whatever ended the stream, the bytes written so far must be on disk before a resume
    // measures the partial file.
    let flushed = file.flush().await;
    drop(file);
    outcome?;
    flushed.map_err(CoreError::from)?;
    if response_total.is_some_and(|total| downloaded_bytes < total) {
        // The body ended early without an error: a dropped connection in disguise.
        return Err(AttemptError::Transient(CoreError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "the connection closed before the file was complete",
        ))));
    }
    remove_legacy_segment_files(temp_path).await?;
    let final_path = move_to_unique_path_async(temp_path, &plan.target_dir, &plan.filename).await?;
    remove_partial_metadata(temp_path).await?;

    Ok(CompletedDownload {
        filename: final_file_name(&final_path, &plan.filename),
        final_path,
        final_url: Some(final_url),
        quality_report: None,
        downloaded_bytes,
        total_bytes,
        supports_resume,
    })
}

/// Creates the folder `temp_path` lives in. The partials folder of the target is hidden as
/// well; a job from an older version may keep its partial file somewhere else.
async fn prepare_partial_dir(target_dir: &Path, temp_path: &Path) -> Result<(), CoreError> {
    if temp_path.starts_with(partials_dir(target_dir)) {
        ensure_partials_dir_async(target_dir).await?;
    } else if let Some(parent) = temp_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    Ok(())
}

fn final_file_name(final_path: &Path, fallback: &str) -> String {
    final_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(fallback)
        .to_string()
}

/// Removes everything a direct download keeps for `temp_path`: the partial file, its validator
/// sidecar, and the segment state of a parallel download. Missing files are not an error.
///
/// The partial file goes first: a parallel download's file is full-size from the start, and
/// without its segment state it would look complete.
pub fn remove_partial_download(temp_path: &Path) -> std::io::Result<()> {
    remove_file_if_exists(temp_path)?;
    remove_file_if_exists(&partial_metadata_path(temp_path))?;
    remove_file_if_exists(&segmented::segment_state_temp_path(temp_path))?;
    remove_file_if_exists(&segmented::segment_state_path(temp_path))?;
    remove_legacy_segment_files_blocking(temp_path)
}

/// Versions before the in-place parallel download kept one `<part>.seg<n>` file per segment.
fn remove_legacy_segment_files_blocking(temp_path: &Path) -> std::io::Result<()> {
    let (Some(dir), Some(name)) = (
        temp_path.parent(),
        temp_path.file_name().and_then(|value| value.to_str()),
    ) else {
        return Ok(());
    };
    let segment_prefix = format!("{name}.seg");
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries.filter_map(Result::ok) {
        let is_segment = entry
            .file_name()
            .to_str()
            .and_then(|filename| filename.strip_prefix(&segment_prefix))
            .is_some_and(|index| {
                !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
            });
        if is_segment {
            remove_file_if_exists(&entry.path())?;
        }
    }

    Ok(())
}

async fn remove_legacy_segment_files(temp_path: &Path) -> Result<(), CoreError> {
    run_blocking_cleanup(temp_path, remove_legacy_segment_files_blocking).await
}

/// Removes the partial file and everything kept next to it.
async fn discard_partial_download(temp_path: &Path) -> Result<(), CoreError> {
    run_blocking_cleanup(temp_path, remove_partial_download).await
}

async fn run_blocking_cleanup(
    temp_path: &Path,
    cleanup: fn(&Path) -> std::io::Result<()>,
) -> Result<(), CoreError> {
    let path = temp_path.to_path_buf();
    tokio::task::spawn_blocking(move || cleanup(&path))
        .await
        .map_err(|error| CoreError::Io(std::io::Error::other(error.to_string())))?
        .map_err(CoreError::from)
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn send_download_request(
    client: &Client,
    request: &CreateDownloadJobRequest,
    range_header: Option<String>,
    if_range_header: Option<String>,
) -> impl std::future::Future<Output = Result<reqwest::Response, reqwest::Error>> {
    build_download_request(client, request, range_header, if_range_header).send()
}

/// Builds the GET with the forwarded browser headers. `Range` and `If-Range` are owned by the
/// download core, so forwarded headers of those names are dropped.
fn build_download_request(
    client: &Client,
    request: &CreateDownloadJobRequest,
    range_header: Option<String>,
    if_range_header: Option<String>,
) -> RequestBuilder {
    let mut request_builder = client.get(&request.source_url);

    if let Some(referrer) = request.referrer.as_ref().or(request.page_url.as_ref()) {
        request_builder = request_builder.header(REFERER, referrer);
    }

    if let Some(headers) = &request.headers {
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("range") || name.eq_ignore_ascii_case("if-range") {
                continue;
            }
            if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(name.as_bytes()) {
                request_builder = request_builder.header(header_name, value);
            }
        }
    }

    if let Some(range_header) = range_header {
        request_builder = request_builder.header(RANGE, range_header);
        if let Some(if_range_header) = if_range_header {
            request_builder = request_builder.header(IF_RANGE, if_range_header);
        }
    }

    request_builder
}

fn should_use_segmented_download(
    request: &CreateDownloadJobRequest,
    supports_resume: bool,
    existing_bytes: u64,
    total_bytes: Option<u64>,
    options: DownloadOptions,
) -> bool {
    request.prefer_range_request
        && supports_resume
        && existing_bytes == 0
        && total_bytes.is_some_and(|bytes| bytes >= options.segmented_min_bytes)
}

fn range_header_value(request: &CreateDownloadJobRequest, existing_bytes: u64) -> Option<String> {
    if existing_bytes > 0 {
        return Some(format!("bytes={existing_bytes}-"));
    }

    request.prefer_range_request.then(|| "bytes=0-".to_string())
}

fn total_bytes_from_response(response: &reqwest::Response, starting_bytes: u64) -> Option<u64> {
    response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(content_range_total_bytes)
        .or_else(|| {
            response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|length| length + starting_bytes)
        })
}

/// The size a `416` reports for the resource: `Content-Range: bytes */<size>`.
fn unsatisfied_range_total(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(content_range_total_bytes)
}

fn content_range_total_bytes(value: &str) -> Option<u64> {
    value
        .rsplit_once('/')
        .and_then(|(_, total)| total.parse::<u64>().ok())
}

async fn existing_file_len(path: &Path) -> Result<u64, CoreError> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

async fn open_part_file(path: &Path, append: bool) -> Result<tokio::fs::File, CoreError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).write(true);

    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }

    options.open(path).await.map_err(CoreError::from)
}

#[cfg(test)]
mod tests {
    use ldm_types::download_job::{CreateDownloadJobRequest, DownloadKind};

    use super::*;

    #[test]
    fn starts_with_zero_range_for_browser_media_when_requested() {
        let mut request = request();
        request.prefer_range_request = true;

        assert_eq!(
            range_header_value(&request, 0),
            Some("bytes=0-".to_string())
        );
    }

    #[test]
    fn resume_range_wins_over_initial_range_preference() {
        let mut request = request();
        request.prefer_range_request = true;

        assert_eq!(
            range_header_value(&request, 1024),
            Some("bytes=1024-".to_string())
        );
    }

    #[test]
    fn sends_if_range_only_next_to_a_range_header() {
        let client = Client::new();
        let request = request();

        let resumed = build_download_request(
            &client,
            &request,
            Some("bytes=10-".to_string()),
            Some("\"v1\"".to_string()),
        )
        .build()
        .unwrap();
        assert_eq!(header_text(resumed.headers(), RANGE), Some("bytes=10-"));
        assert_eq!(header_text(resumed.headers(), IF_RANGE), Some("\"v1\""));

        let validator_without_range =
            build_download_request(&client, &request, None, Some("\"v1\"".to_string()))
                .build()
                .unwrap();
        assert_eq!(header_text(validator_without_range.headers(), RANGE), None);
        assert_eq!(
            header_text(validator_without_range.headers(), IF_RANGE),
            None
        );
    }

    #[test]
    fn forwarded_headers_cannot_override_range_or_if_range() {
        let client = Client::new();
        let mut request = request();
        request.headers = Some(std::collections::BTreeMap::from([
            ("Range".to_string(), "bytes=0-1".to_string()),
            ("If-Range".to_string(), "\"forged\"".to_string()),
            ("Accept".to_string(), "*/*".to_string()),
        ]));

        let built = build_download_request(&client, &request, Some("bytes=5-".to_string()), None)
            .build()
            .unwrap();

        assert_eq!(header_text(built.headers(), RANGE), Some("bytes=5-"));
        assert_eq!(header_text(built.headers(), IF_RANGE), None);
        assert_eq!(
            header_text(built.headers(), reqwest::header::ACCEPT),
            Some("*/*")
        );
    }

    fn header_text(
        headers: &reqwest::header::HeaderMap,
        name: reqwest::header::HeaderName,
    ) -> Option<&str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }

    #[test]
    fn parses_content_range_total_size() {
        assert_eq!(content_range_total_bytes("bytes 0-1023/4096"), Some(4096));
        assert_eq!(content_range_total_bytes("bytes 0-1023/*"), None);
        assert_eq!(content_range_total_bytes("bytes */4096"), Some(4096));
    }

    #[test]
    fn opts_into_segmented_download_for_large_fresh_range_jobs() {
        let mut request = request();
        request.prefer_range_request = true;
        let options = DownloadOptions::default();
        let min_bytes = Some(options.segmented_min_bytes);

        assert!(should_use_segmented_download(
            &request, true, 0, min_bytes, options
        ));
        assert!(!should_use_segmented_download(
            &request, true, 1024, min_bytes, options
        ));
        assert!(!should_use_segmented_download(
            &request, false, 0, min_bytes, options
        ));
        assert!(!should_use_segmented_download(
            &request,
            true,
            0,
            Some(options.segmented_min_bytes - 1),
            options
        ));
    }

    #[test]
    fn default_options_keep_the_production_thresholds() {
        let options = DownloadOptions::default();

        assert_eq!(options.segmented_min_bytes, 32 * 1024 * 1024);
        assert_eq!(options.segment_count, 4);
        assert_eq!(options.attempts, 4);
        assert_eq!(options.retry_delay, Duration::from_secs(1));
    }

    #[test]
    fn the_pause_doubles_with_every_fruitless_attempt_up_to_a_limit() {
        let base = Duration::from_secs(1);

        assert_eq!(retry_delay(base, 1), Duration::from_secs(1));
        assert_eq!(retry_delay(base, 2), Duration::from_secs(2));
        assert_eq!(retry_delay(base, 3), Duration::from_secs(4));
        assert_eq!(retry_delay(base, 40), Duration::from_secs(64));
        assert_eq!(retry_delay(base, 0), Duration::from_secs(1));
    }

    #[test]
    fn removes_the_partial_file_its_sidecar_and_its_segments_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("ldm-a.part");
        let other = dir.path().join("ldm-b.part");
        for path in [
            temp_path.clone(),
            partial_metadata_path(&temp_path),
            segmented::segment_state_path(&temp_path),
            segmented::segment_state_temp_path(&temp_path),
            dir.path().join("ldm-a.part.seg0"),
            dir.path().join("ldm-a.part.seg11"),
            other.clone(),
            dir.path().join("ldm-b.part.seg0"),
            dir.path().join("ldm-a.part.segments-note"),
        ] {
            std::fs::write(path, b"x").unwrap();
        }

        remove_partial_download(&temp_path).unwrap();
        // Nothing left to remove is fine too.
        remove_partial_download(&temp_path).unwrap();

        let mut left = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        left.sort();
        assert_eq!(
            left,
            ["ldm-a.part.segments-note", "ldm-b.part", "ldm-b.part.seg0"]
        );
    }

    #[tokio::test]
    async fn a_dropped_cancel_handle_is_not_a_cancel() {
        let (handle, mut cancellation) = download_cancellation_pair();

        drop(handle);

        let waited = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            cancellation.cancelled(),
        )
        .await;
        assert!(waited.is_err(), "a dropped handle must not resolve");
        assert!(!cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn cancellation_signal_stops_download_work() {
        let (handle, mut cancellation) = download_cancellation_pair();

        handle.cancel();

        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            cancellation.cancelled(),
        )
        .await
        .expect("cancellation should resolve promptly");
        assert!(cancellation.is_cancelled());
    }

    fn request() -> CreateDownloadJobRequest {
        CreateDownloadJobRequest {
            source_url: "https://example.invalid/video.mp4".to_string(),
            source_fallback_urls: Vec::new(),
            page_url: None,
            page_title: None,
            referrer: None,
            suggested_filename: Some("video.mp4".to_string()),
            content_type: Some("video/mp4".to_string()),
            content_length: None,
            kind: DownloadKind::Video,
            adapter_id: "generic-html5-video".to_string(),
            provider_id: None,
            selected_option_id: None,
            headers: None,
            prefer_range_request: false,
            quality_report: None,
        }
    }
}
