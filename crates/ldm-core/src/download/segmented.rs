//! Parallel download of one file over several connections.
//!
//! The partial file is created sparse at its full size and every segment writes its byte range
//! in place: nothing is merged at the end, and the download never needs more disk space than
//! the file itself. The file's length therefore says nothing about how much of it is there.
//! That is recorded per segment in a sidecar (`<part>.segments`), and only as far as the bytes
//! have been flushed and synced, so the record never runs ahead of the data.
//!
//! One rule keeps a half-written file from ever passing as a complete one: a full-size partial
//! file with holes never exists without its sidecar. The sidecar is written before the file is
//! created and removed only once every segment is complete; discarding goes file first.

use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use ldm_types::download_job::CreateDownloadJobRequest;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::{JoinError, JoinSet};

use crate::error::CoreError;
use crate::filename::move_to_unique_path_async;

use super::{
    download_cancellation_pair, existing_file_len, final_file_name, remove_partial_metadata,
    retry_delay, send_download_request, sparse::create_sparse_file, CompletedDownload,
    DownloadCancellation, DownloadOptions, DownloadProgress, SpeedMeter,
};

const STATE_SUFFIX: &str = ".segments";
const STATE_TEMP_SUFFIX: &str = ".segments.tmp";
const STATE_VERSION: u8 = 1;
/// How often a running segment syncs its bytes and records how far it is. After a crash, at
/// most this much of each segment is downloaded again.
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(2);
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Where a segmented download writes and where the finished file goes.
pub(super) struct SegmentedDownloadTarget<'a> {
    pub(super) target_dir: &'a Path,
    pub(super) filename: &'a str,
    pub(super) temp_path: &'a Path,
    pub(super) total_bytes: u64,
}

/// How a segmented download ended when it did not complete.
#[derive(Debug)]
pub(super) enum SegmentedFailure {
    /// Segmenting does not work for this download; the caller uses one connection instead.
    UseSingleConnection,
    /// The download failed or was canceled. The partial file and its sidecar stay, and the
    /// next attempt continues every segment where it stopped.
    Failed(CoreError),
}

impl From<CoreError> for SegmentedFailure {
    fn from(error: CoreError) -> Self {
        Self::Failed(error)
    }
}

/// How far each segment of a parallel download is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SegmentState {
    version: u8,
    total_bytes: u64,
    segments: Vec<SegmentProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct SegmentProgress {
    /// First and last byte of the segment within the file.
    start: u64,
    end: u64,
    /// Bytes from `start` on that are safely on disk.
    done: u64,
}

impl SegmentProgress {
    fn size(&self) -> u64 {
        self.end - self.start + 1
    }

    fn is_complete(&self) -> bool {
        self.done >= self.size()
    }
}

impl SegmentState {
    fn fresh(total_bytes: u64, ranges: &[(u64, u64)]) -> Self {
        Self {
            version: STATE_VERSION,
            total_bytes,
            segments: ranges
                .iter()
                .map(|(start, end)| SegmentProgress {
                    start: *start,
                    end: *end,
                    done: 0,
                })
                .collect(),
        }
    }

    fn done_bytes(&self) -> u64 {
        self.segments
            .iter()
            .map(|segment| segment.done.min(segment.size()))
            .sum()
    }

    /// Whether this is the record of a download of `total_bytes` split into `ranges`.
    fn matches(&self, total_bytes: u64, ranges: &[(u64, u64)]) -> bool {
        self.version == STATE_VERSION
            && self.total_bytes == total_bytes
            && self.segments.len() == ranges.len()
            && self.segments.iter().zip(ranges).all(|(segment, range)| {
                (segment.start, segment.end) == *range && segment.done <= segment.size()
            })
    }
}

pub(super) fn segment_state_path(temp_path: &Path) -> PathBuf {
    path_with_suffix(temp_path, STATE_SUFFIX)
}

pub(super) fn segment_state_temp_path(temp_path: &Path) -> PathBuf {
    path_with_suffix(temp_path, STATE_TEMP_SUFFIX)
}

fn path_with_suffix(temp_path: &Path, suffix: &str) -> PathBuf {
    let mut filename = temp_path
        .file_name()
        .map(|value| value.to_os_string())
        .unwrap_or_default();
    filename.push(suffix);
    temp_path.with_file_name(filename)
}

/// Whether `temp_path` belongs to a parallel download. Its length is then the size of the whole
/// file, not the number of bytes received.
pub(super) async fn has_segment_state(temp_path: &Path) -> bool {
    tokio::fs::try_exists(segment_state_path(temp_path))
        .await
        .unwrap_or(false)
}

/// Gives up a parallel download: the partial file goes first and its sidecar second, so no
/// crash in between can leave a file with holes that looks complete.
pub(super) async fn discard_segmented_partial(temp_path: &Path) -> Result<(), CoreError> {
    remove_if_exists(temp_path).await?;
    remove_state(temp_path).await
}

async fn remove_state(temp_path: &Path) -> Result<(), CoreError> {
    remove_if_exists(&segment_state_temp_path(temp_path)).await?;
    remove_if_exists(&segment_state_path(temp_path)).await
}

async fn remove_if_exists(path: &Path) -> Result<(), CoreError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn save_state(temp_path: &Path, state: &SegmentState) -> Result<(), CoreError> {
    let payload = serde_json::to_vec(state).map_err(|error| {
        CoreError::Io(std::io::Error::other(format!(
            "could not encode the segment state: {error}"
        )))
    })?;
    let staging = segment_state_temp_path(temp_path);
    tokio::fs::write(&staging, payload).await?;
    tokio::fs::rename(&staging, segment_state_path(temp_path)).await?;
    Ok(())
}

/// The record of an earlier attempt, when it fits this download and its partial file is there.
async fn load_resumable_state(
    temp_path: &Path,
    total_bytes: u64,
    ranges: &[(u64, u64)],
) -> Option<SegmentState> {
    let payload = tokio::fs::read(segment_state_path(temp_path)).await.ok()?;
    let state: SegmentState = serde_json::from_slice(&payload).ok()?;
    let part_len = existing_file_len(temp_path).await.ok()?;
    (state.matches(total_bytes, ranges) && part_len == total_bytes).then_some(state)
}

async fn create_sparse_part(temp_path: &Path, total_bytes: u64) -> std::io::Result<()> {
    let path = temp_path.to_path_buf();
    tokio::task::spawn_blocking(move || create_sparse_file(&path, total_bytes))
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?
}

pub(super) async fn run_segmented_http_download(
    client: &Client,
    request: &CreateDownloadJobRequest,
    options: DownloadOptions,
    if_range: Option<String>,
    target: SegmentedDownloadTarget<'_>,
    on_progress: &mut impl FnMut(DownloadProgress),
    mut cancellation: DownloadCancellation,
) -> Result<CompletedDownload, SegmentedFailure> {
    let SegmentedDownloadTarget {
        target_dir,
        filename,
        temp_path,
        total_bytes,
    } = target;
    let ranges = segment_ranges(total_bytes, options.segment_count);
    if ranges.len() <= 1 {
        return Err(SegmentedFailure::UseSingleConnection);
    }

    let mut state = match load_resumable_state(temp_path, total_bytes, &ranges).await {
        Some(state) => state,
        None => {
            // Sidecar first: from the moment the full-size file exists, it is known to be
            // a parallel download in progress.
            let state = SegmentState::fresh(total_bytes, &ranges);
            save_state(temp_path, &state).await?;
            if let Err(error) = create_sparse_part(temp_path, total_bytes).await {
                // Without sparse files (FAT, exFAT) the volume would fill the gaps between
                // the segments with zeroes before the first far write returns.
                tracing::info!("no sparse partial file here ({error}), using one connection");
                return Err(SegmentedFailure::UseSingleConnection);
            }
            state
        }
    };

    let mut downloaded_bytes = state.done_bytes();
    let mut meter = SpeedMeter::new(Instant::now(), downloaded_bytes);
    on_progress(DownloadProgress::byte_progress(
        downloaded_bytes,
        Some(total_bytes),
        true,
    ));

    // The segments stop on this signal, not on the caller's: whether the user cancels or one
    // segment gives up, the others sync and record their position before they end.
    let (stop, stopped) = download_cancellation_pair();
    let (events, mut received) = mpsc::channel::<SegmentEvent>(EVENT_CHANNEL_CAPACITY);
    let mut tasks = JoinSet::new();
    for (index, segment) in state.segments.iter().enumerate() {
        if segment.is_complete() {
            continue;
        }
        tasks.spawn(download_segment(SegmentJob {
            client: client.clone(),
            request: request.clone(),
            if_range: if_range.clone(),
            part_path: temp_path.to_path_buf(),
            index,
            segment: *segment,
            events: events.clone(),
            stop: stopped.clone(),
            attempts: options.attempts,
            retry_delay: options.retry_delay,
        }));
    }
    drop(events);

    let mut failure: Option<SegmentedFailure> = None;
    let mut cancel_seen = false;
    loop {
        // Deliberately unbiased: with cancel-first ordering this loop stalls (the segment
        // tasks and the event channel never get a fair turn).
        tokio::select! {
            _ = cancellation.cancelled(), if !cancel_seen => {
                cancel_seen = true;
                failure = Some(SegmentedFailure::Failed(CoreError::Canceled));
                stop.cancel();
            }
            event = received.recv() => match event {
                Some(SegmentEvent::Bytes(count)) => {
                    downloaded_bytes += count;
                    on_progress(meter.progress(
                        Instant::now(),
                        downloaded_bytes,
                        Some(total_bytes),
                        true,
                    ));
                }
                Some(SegmentEvent::Checkpoint { index, done }) => {
                    state.segments[index].done = done;
                    if let Err(error) = save_state(temp_path, &state).await {
                        failure.get_or_insert(SegmentedFailure::Failed(error));
                        stop.cancel();
                    }
                }
                // Every segment has ended and dropped its sender.
                None => break,
            },
            Some(ended) = tasks.join_next() => {
                if let Some(reason) = segment_failure(ended) {
                    failure.get_or_insert(reason);
                    stop.cancel();
                }
            }
        }
    }
    while let Some(ended) = tasks.join_next().await {
        if let Some(reason) = segment_failure(ended) {
            failure.get_or_insert(reason);
        }
    }

    if let Some(failure) = failure {
        return Err(failure);
    }
    if !state.segments.iter().all(SegmentProgress::is_complete) {
        return Err(SegmentedFailure::Failed(CoreError::Io(
            std::io::Error::other("a segment ended before its range was complete"),
        )));
    }

    // The file is whole now, so it may exist without its sidecar.
    remove_state(temp_path).await?;
    let final_path = move_to_unique_path_async(temp_path, target_dir, filename).await?;
    remove_partial_metadata(temp_path).await?;

    Ok(CompletedDownload {
        filename: final_file_name(&final_path, filename),
        final_path,
        final_url: Some(request.source_url.clone()),
        quality_report: None,
        downloaded_bytes: total_bytes,
        total_bytes: Some(total_bytes),
        supports_resume: true,
    })
}

/// What a segment task that ended means for the whole download; `None` when it completed or
/// merely obeyed the stop signal.
fn segment_failure(ended: Result<Result<(), SegmentError>, JoinError>) -> Option<SegmentedFailure> {
    match ended {
        Ok(Ok(()) | Err(SegmentError::Stopped)) => None,
        Ok(Err(SegmentError::RangeRefused)) => Some(SegmentedFailure::UseSingleConnection),
        Ok(Err(SegmentError::Transient(error) | SegmentError::Fatal(error))) => {
            Some(SegmentedFailure::Failed(error))
        }
        Err(error) => Some(SegmentedFailure::Failed(CoreError::Io(
            std::io::Error::other(error.to_string()),
        ))),
    }
}

/// What a segment tells the coordinator.
enum SegmentEvent {
    /// Bytes written, for the progress display.
    Bytes(u64),
    /// The segment's first `done` bytes are synced to disk.
    Checkpoint { index: usize, done: u64 },
}

/// One byte range of a segmented download, owned by the task that fetches it.
struct SegmentJob {
    client: Client,
    request: CreateDownloadJobRequest,
    /// Validator of the response the download started from; a mismatch makes the server
    /// answer `200`, which counts as a refused range so the caller starts over.
    if_range: Option<String>,
    part_path: PathBuf,
    index: usize,
    /// `done` moves forward only after a sync, so it is always safe to continue from.
    segment: SegmentProgress,
    events: mpsc::Sender<SegmentEvent>,
    stop: DownloadCancellation,
    attempts: u32,
    retry_delay: Duration,
}

/// Why one segment stopped before it was complete.
#[derive(Debug)]
enum SegmentError {
    /// The coordinator asked every segment to stop.
    Stopped,
    /// The server did not serve the range as a range: it answered with something other than
    /// `206` (for example `200` after a validator mismatch, or `403` for a second connection)
    /// or sent more than was asked for.
    RangeRefused,
    /// The connection or the server failed in a way another attempt can get past.
    Transient(CoreError),
    /// Anything else, for example a full disk.
    Fatal(CoreError),
}

/// Downloads one segment and tries again after transient failures.
///
/// Every attempt continues after the last synced byte, and only attempts that brought no new
/// bytes count against `SegmentJob::attempts`, so a long segment survives any number of short
/// interruptions while a dead server still fails the download after a few tries.
async fn download_segment(mut job: SegmentJob) -> Result<(), SegmentError> {
    let mut stop = job.stop.clone();
    let mut fruitless_attempts = 0_u32;
    loop {
        let before = job.segment.done;
        let attempt = download_segment_once(&mut job).await;
        let _ = job
            .events
            .send(SegmentEvent::Checkpoint {
                index: job.index,
                done: job.segment.done,
            })
            .await;
        let error = match attempt {
            Ok(()) => return Ok(()),
            Err(SegmentError::Transient(error)) => error,
            Err(other) => return Err(other),
        };

        fruitless_attempts = if job.segment.done > before {
            1
        } else {
            fruitless_attempts + 1
        };
        if fruitless_attempts >= job.attempts {
            return Err(SegmentError::Transient(error));
        }
        let delay = retry_delay(job.retry_delay, fruitless_attempts);
        tokio::select! {
            biased;
            _ = stop.cancelled() => return Err(SegmentError::Stopped),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// One request for the part of the segment that is not on disk yet, written in place.
async fn download_segment_once(job: &mut SegmentJob) -> Result<(), SegmentError> {
    let mut stop = job.stop.clone();
    if stop.is_cancelled() {
        return Err(SegmentError::Stopped);
    }
    let expected_bytes = job.segment.size();
    if job.segment.is_complete() {
        return Ok(());
    }

    let response = tokio::select! {
        biased;
        _ = stop.cancelled() => return Err(SegmentError::Stopped),
        response = send_download_request(
            &job.client,
            &job.request,
            Some(segment_range_header(&job.segment)),
            job.if_range.clone(),
        ) => response.map_err(|error| SegmentError::Transient(error.into()))?,
    };
    let status = response.status();
    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(if crate::http::is_transient_status(status) {
            SegmentError::Transient(CoreError::HttpStatus(status))
        } else {
            SegmentError::RangeRefused
        });
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&job.part_path)
        .await
        .map_err(|error| SegmentError::Fatal(error.into()))?;
    file.seek(SeekFrom::Start(job.segment.start + job.segment.done))
        .await
        .map_err(|error| SegmentError::Fatal(error.into()))?;

    let mut stream = response.bytes_stream();
    let mut written = job.segment.done;
    let mut last_checkpoint = Instant::now();
    let outcome = loop {
        let chunk = tokio::select! {
            biased;
            _ = stop.cancelled() => break Err(SegmentError::Stopped),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break Ok(());
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => break Err(SegmentError::Transient(error.into())),
        };
        // A body longer than the range would run into the next segment's bytes.
        if written + chunk.len() as u64 > expected_bytes {
            break Err(SegmentError::RangeRefused);
        }
        if let Err(error) = file.write_all(&chunk).await {
            break Err(SegmentError::Fatal(error.into()));
        }
        written += chunk.len() as u64;
        let _ = job
            .events
            .send(SegmentEvent::Bytes(chunk.len() as u64))
            .await;

        if last_checkpoint.elapsed() >= CHECKPOINT_INTERVAL {
            if let Err(error) = sync_to_disk(&mut file).await {
                break Err(SegmentError::Fatal(error.into()));
            }
            job.segment.done = written;
            last_checkpoint = Instant::now();
            let _ = job
                .events
                .send(SegmentEvent::Checkpoint {
                    index: job.index,
                    done: written,
                })
                .await;
        }
    };

    // Whatever ended the stream: what was written counts only once it is synced.
    match sync_to_disk(&mut file).await {
        Ok(()) => job.segment.done = written,
        Err(error) if outcome.is_ok() => return Err(SegmentError::Fatal(error.into())),
        Err(_) => {}
    }
    drop(file);
    outcome?;

    if job.segment.is_complete() {
        Ok(())
    } else {
        // The body ended early without an error: a dropped connection in disguise.
        Err(SegmentError::Transient(CoreError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "a segment ended before its range was complete",
        ))))
    }
}

async fn sync_to_disk(file: &mut tokio::fs::File) -> std::io::Result<()> {
    file.flush().await?;
    file.sync_data().await
}

/// The `Range` value for what is left of a segment.
fn segment_range_header(segment: &SegmentProgress) -> String {
    let resume_start = (segment.start + segment.done).min(segment.end);
    format!("bytes={resume_start}-{}", segment.end)
}

/// Splits `total_bytes` into up to `requested_parts` ranges of first and last byte.
fn segment_ranges(total_bytes: u64, requested_parts: u64) -> Vec<(u64, u64)> {
    if total_bytes == 0 || requested_parts == 0 {
        return Vec::new();
    }

    let part_count = requested_parts.min(total_bytes);
    let part_size = total_bytes.div_ceil(part_count);
    (0..part_count)
        .filter_map(|index| {
            let start = index * part_size;
            if start >= total_bytes {
                return None;
            }
            let end = ((index + 1) * part_size)
                .saturating_sub(1)
                .min(total_bytes - 1);
            Some((start, end))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_file_into_bounded_byte_ranges() {
        assert_eq!(segment_ranges(10, 4), vec![(0, 2), (3, 5), (6, 8), (9, 9)]);
        assert_eq!(segment_ranges(3, 8), vec![(0, 0), (1, 1), (2, 2)]);
        assert!(segment_ranges(0, 4).is_empty());
    }

    #[test]
    fn asks_for_what_is_left_of_a_segment() {
        let mut segment = SegmentProgress {
            start: 10,
            end: 19,
            done: 0,
        };
        assert_eq!(segment_range_header(&segment), "bytes=10-19");

        segment.done = 5;
        assert_eq!(segment_range_header(&segment), "bytes=15-19");
    }

    #[test]
    fn a_state_only_fits_the_download_it_was_written_for() {
        let ranges = segment_ranges(1000, 4);
        let mut state = SegmentState::fresh(1000, &ranges);
        state.segments[1].done = 250;
        state.segments[2].done = 100;

        assert!(state.matches(1000, &ranges));
        assert_eq!(state.done_bytes(), 350);
        assert!(state.segments[1].is_complete());
        assert!(!state.segments[2].is_complete());

        assert!(!state.matches(1001, &segment_ranges(1001, 4)));
        assert!(!state.matches(1000, &segment_ranges(1000, 2)));
        state.segments[0].done = 251;
        assert!(
            !state.matches(1000, &ranges),
            "more done than the segment holds"
        );
    }

    #[tokio::test]
    async fn the_state_round_trips_and_needs_its_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("ldm-a.part");
        let ranges = segment_ranges(4096, 4);
        let mut state = SegmentState::fresh(4096, &ranges);
        state.segments[3].done = 1024;

        save_state(&temp_path, &state).await.unwrap();
        assert!(has_segment_state(&temp_path).await);
        assert_eq!(
            load_resumable_state(&temp_path, 4096, &ranges).await,
            None,
            "a state without its partial file is not resumable"
        );

        create_sparse_part(&temp_path, 4096).await.unwrap();
        assert_eq!(
            load_resumable_state(&temp_path, 4096, &ranges).await,
            Some(state)
        );
        assert_eq!(load_resumable_state(&temp_path, 8192, &ranges).await, None);

        discard_segmented_partial(&temp_path).await.unwrap();
        assert!(!temp_path.exists());
        assert!(!has_segment_state(&temp_path).await);
    }
}
