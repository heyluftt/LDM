use std::path::Path;

use ldm_types::download_job::{CreateDownloadJobRequest, DownloadJob, DownloadStatus};
use ldm_types::timestamp::unix_seconds_now;
use uuid::Uuid;

use crate::diagnostics::{sanitize_diagnostic_message, sanitize_diagnostic_report};
use crate::download::{build_download_plan, CompletedDownload, DownloadProgress};
use crate::error::CoreError;
use crate::filename::job_partial_path;

#[derive(Debug, Default)]
pub struct DownloadQueue {
    jobs: Vec<DownloadJob>,
}

impl DownloadQueue {
    pub fn restore(mut jobs: Vec<DownloadJob>) -> Self {
        let now = unix_seconds_now();
        for job in &mut jobs {
            job.error = job
                .error
                .take()
                .map(|error| sanitize_diagnostic_message(&error));
            job.error_report = job
                .error_report
                .take()
                .map(|report| sanitize_diagnostic_report(&report));
            job.quality_report = job
                .quality_report
                .take()
                .map(|report| sanitize_diagnostic_report(&report));
            if matches!(job.status, DownloadStatus::Queued | DownloadStatus::Running) {
                if job.adapter_kind().supports_pause_resume() {
                    job.status = DownloadStatus::Paused;
                    job.error = Some(
                        "LDM closed before this download finished. Resume to continue.".to_string(),
                    );
                } else {
                    job.status = DownloadStatus::Failed;
                    job.error = Some(
                        "LDM closed before this provider or stream download finished. Retry to start again."
                            .to_string(),
                    );
                }
                job.speed_bytes_per_second = None;
                job.updated_at = now;
            }
        }

        Self { jobs }
    }

    /// Adds a job that will download into `target_dir`, which the caller resolves from the
    /// user's settings for `request.kind`.
    pub fn enqueue(
        &mut self,
        request: CreateDownloadJobRequest,
        target_dir: &Path,
    ) -> Result<DownloadJob, CoreError> {
        let plan = build_download_plan(&request, target_dir)?;
        let id = Uuid::new_v4();
        let now = unix_seconds_now();
        let target_directory = plan.target_dir.to_string_lossy().to_string();
        // Named after the job so that two jobs with the same filename never share partial data.
        let temp_path = job_partial_path(&plan.target_dir, id)
            .to_string_lossy()
            .to_string();
        let mut forwarded_header_names = request
            .headers
            .as_ref()
            .map(|headers| headers.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        forwarded_header_names.sort();
        let initial_total_bytes = known_total_bytes_from_request(&request);
        let quality_report = request
            .quality_report
            .map(|report| sanitize_diagnostic_report(&report));

        let job = DownloadJob {
            id,
            source_url: request.source_url,
            final_url: None,
            filename: plan.filename,
            target_directory,
            temp_path,
            kind: request.kind,
            adapter_id: request.adapter_id,
            provider_id: request.provider_id,
            selected_option_id: request.selected_option_id,
            request_content_type: request.content_type,
            request_content_length: request.content_length,
            source_fallback_count: request.source_fallback_urls.len(),
            forwarded_header_names,
            prefer_range_request: request.prefer_range_request,
            referrer_present: request
                .referrer
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty()),
            page_url: request.page_url,
            status: DownloadStatus::Queued,
            progress_bytes: 0,
            total_bytes: initial_total_bytes,
            progress_percent: None,
            speed_bytes_per_second: None,
            eta_seconds: None,
            supports_resume: false,
            retry_count: 0,
            error: None,
            error_report: None,
            quality_report,
            created_at: now,
            updated_at: now,
        };

        self.jobs.push(job.clone());
        Ok(job)
    }

    pub fn jobs(&self) -> &[DownloadJob] {
        &self.jobs
    }

    pub fn history_jobs(&self) -> Vec<DownloadJob> {
        self.jobs
            .iter()
            .filter(|job| is_history_status(job.status))
            .cloned()
            .collect()
    }

    pub fn job(&self, id: Uuid) -> Option<DownloadJob> {
        self.jobs.iter().find(|job| job.id == id).cloned()
    }

    pub fn remove_history_job(&mut self, id: Uuid) -> Option<DownloadJob> {
        let index = self
            .jobs
            .iter()
            .position(|job| job.id == id && is_history_status(job.status))?;
        Some(self.jobs.remove(index))
    }

    pub fn clear_history(&mut self) -> Vec<DownloadJob> {
        let mut removed = Vec::new();
        let mut active = Vec::with_capacity(self.jobs.len());

        for job in self.jobs.drain(..) {
            if is_history_status(job.status) {
                removed.push(job);
            } else {
                active.push(job);
            }
        }

        self.jobs = active;
        removed
    }

    /// Keeps the `keep` history jobs that changed last and removes the older ones, which it
    /// returns. Jobs that are queued, running, or paused are never touched.
    pub fn prune_history(&mut self, keep: usize) -> Vec<DownloadJob> {
        let mut history = self
            .jobs
            .iter()
            .filter(|job| is_history_status(job.status))
            .map(|job| (job.updated_at, job.id))
            .collect::<Vec<_>>();
        if history.len() <= keep {
            return Vec::new();
        }
        // Newest first; the sort is stable, so of two jobs of the same second the one further
        // back in the queue counts as newer.
        history.reverse();
        history.sort_by_key(|(updated_at, _)| std::cmp::Reverse(*updated_at));
        let outdated = history
            .split_off(keep)
            .into_iter()
            .map(|(_, id)| id)
            .collect::<std::collections::HashSet<_>>();

        let mut removed = Vec::with_capacity(outdated.len());
        let mut kept = Vec::with_capacity(self.jobs.len() - outdated.len());
        for job in self.jobs.drain(..) {
            if outdated.contains(&job.id) {
                removed.push(job);
            } else {
                kept.push(job);
            }
        }
        self.jobs = kept;
        removed
    }

    pub fn mark_running(&mut self, id: Uuid) {
        if let Some(job) = self.find_mut(id) {
            job.status = DownloadStatus::Running;
            job.error = None;
            job.error_report = None;
            job.updated_at = unix_seconds_now();
        }
    }

    pub fn mark_queued(&mut self, id: Uuid) {
        if let Some(job) = self.find_mut(id) {
            job.status = DownloadStatus::Queued;
            job.error = None;
            job.error_report = None;
            job.speed_bytes_per_second = None;
            job.eta_seconds = None;
            job.updated_at = unix_seconds_now();
        }
    }

    pub fn mark_retry_queued(&mut self, id: Uuid, total_bytes: Option<u64>) {
        if let Some(job) = self.find_mut(id) {
            job.status = DownloadStatus::Queued;
            job.progress_bytes = 0;
            job.total_bytes = total_bytes;
            job.progress_percent = None;
            job.speed_bytes_per_second = None;
            job.eta_seconds = None;
            job.supports_resume = false;
            job.retry_count = job.retry_count.saturating_add(1);
            job.error = None;
            job.error_report = None;
            job.updated_at = unix_seconds_now();
        }
    }

    pub fn mark_paused(&mut self, id: Uuid) {
        if let Some(job) = self.find_mut(id) {
            job.status = DownloadStatus::Paused;
            job.speed_bytes_per_second = None;
            job.eta_seconds = None;
            job.updated_at = unix_seconds_now();
        }
    }

    pub fn mark_canceled(&mut self, id: Uuid) {
        if let Some(job) = self.find_mut(id) {
            job.status = DownloadStatus::Canceled;
            job.speed_bytes_per_second = None;
            job.eta_seconds = None;
            job.updated_at = unix_seconds_now();
        }
    }

    /// Stores a progress update. Speed and remaining time come from whoever measures the
    /// transfer (`SpeedMeter`, the native HLS tracker, yt-dlp's own output); an update without
    /// a speed keeps the last known one.
    pub fn update_progress(&mut self, id: Uuid, progress: &DownloadProgress) {
        if let Some(job) = self.find_mut(id) {
            let progress_bytes = progress.downloaded_bytes;
            job.progress_bytes = progress_bytes;
            job.total_bytes = progress.total_bytes.or(job.total_bytes);
            job.progress_percent = progress
                .progress_percent
                .map(|value| value.clamp(0.0, 100.0))
                .or_else(|| calculated_progress_percent(progress_bytes, job.total_bytes))
                .or(job.progress_percent);
            job.eta_seconds = progress.eta_seconds;
            job.supports_resume = progress.supports_resume;
            job.speed_bytes_per_second = progress
                .speed_bytes_per_second
                .or(job.speed_bytes_per_second);
            job.updated_at = unix_seconds_now();
        }
    }

    pub fn mark_completed(&mut self, id: Uuid, completed: CompletedDownload) {
        if let Some(job) = self.find_mut(id) {
            job.status = DownloadStatus::Completed;
            job.final_url = completed.final_url;
            if let Some(report) = completed.quality_report {
                job.quality_report = Some(sanitize_diagnostic_report(&report));
            }
            job.filename = completed.filename;
            job.target_directory = completed
                .final_path
                .parent()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|| job.target_directory.clone());
            job.temp_path.clear();
            job.progress_bytes = completed.downloaded_bytes;
            job.total_bytes = completed.total_bytes.or(job.total_bytes);
            job.progress_percent = Some(100.0);
            job.eta_seconds = Some(0);
            job.supports_resume = completed.supports_resume;
            job.error = None;
            job.error_report = None;
            job.updated_at = unix_seconds_now();
        }
    }

    pub fn mark_failed(&mut self, id: Uuid, error: String, error_report: Option<String>) {
        if let Some(job) = self.find_mut(id) {
            job.status = DownloadStatus::Failed;
            job.error = Some(sanitize_diagnostic_message(&error));
            job.error_report = error_report.map(|report| sanitize_diagnostic_report(&report));
            job.updated_at = unix_seconds_now();
        }
    }

    fn find_mut(&mut self, id: Uuid) -> Option<&mut DownloadJob> {
        self.jobs.iter_mut().find(|job| job.id == id)
    }
}

fn is_history_status(status: DownloadStatus) -> bool {
    matches!(
        status,
        DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Canceled
    )
}

fn known_total_bytes_from_request(request: &CreateDownloadJobRequest) -> Option<u64> {
    if request.adapter_kind().is_direct_http() {
        request.content_length
    } else {
        None
    }
}

fn calculated_progress_percent(progress_bytes: u64, total_bytes: Option<u64>) -> Option<f64> {
    let total_bytes = total_bytes?;
    if total_bytes == 0 {
        return None;
    }

    Some(((progress_bytes as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ldm_types::download_job::DownloadKind;

    use super::*;

    #[test]
    fn enqueue_keeps_safe_request_diagnostics() {
        let mut queue = DownloadQueue::default();
        let job = queue.enqueue(valid_request(), &target_dir()).unwrap();

        assert_eq!(job.adapter_id, "generic-html5-video");
        assert_eq!(job.request_content_type.as_deref(), Some("video/mp4"));
        assert_eq!(job.request_content_length, Some(1000));
        assert_eq!(job.total_bytes, Some(1000));
        assert_eq!(job.source_fallback_count, 1);
        assert_eq!(
            job.forwarded_header_names,
            vec!["accept".to_string(), "user-agent".to_string()]
        );
        assert!(job.prefer_range_request);
        assert!(job.referrer_present);
        assert!(job.page_url.is_some());
        assert!(job
            .quality_report
            .as_deref()
            .is_some_and(|report| report.contains("1080p")));
    }

    #[test]
    fn jobs_with_the_same_filename_get_their_own_partial_file() {
        let mut queue = DownloadQueue::default();
        let first = queue.enqueue(valid_request(), &target_dir()).unwrap();
        let second = queue.enqueue(valid_request(), &target_dir()).unwrap();

        assert_eq!(first.filename, second.filename);
        assert_ne!(first.temp_path, second.temp_path);
        assert_eq!(
            std::path::PathBuf::from(&first.temp_path),
            job_partial_path(&target_dir(), first.id)
        );
    }

    #[test]
    fn enqueue_keeps_stream_manifest_size_out_of_total_bytes() {
        let mut request = valid_request();
        request.adapter_id = "hls".to_string();
        request.source_url = "https://cdn.example.invalid/master.m3u8".to_string();
        request.content_type = Some("application/vnd.apple.mpegurl".to_string());
        request.content_length = Some(10_000);
        let mut queue = DownloadQueue::default();

        let job = queue.enqueue(request, &target_dir()).unwrap();

        assert_eq!(job.request_content_length, Some(10_000));
        assert_eq!(job.total_bytes, None);
    }

    #[test]
    fn enqueue_keeps_provider_request_size_out_of_total_bytes() {
        let mut request = valid_request();
        request.adapter_id = "provider-youtube".to_string();
        request.source_url = "https://www.youtube.com/watch?v=owned".to_string();
        request.content_length = Some(10_000);
        let mut queue = DownloadQueue::default();

        let job = queue.enqueue(request, &target_dir()).unwrap();

        assert_eq!(job.request_content_length, Some(10_000));
        assert_eq!(job.total_bytes, None);
    }

    #[test]
    fn progress_stores_the_measured_speed_and_keeps_it_when_an_update_has_none() {
        let mut queue = DownloadQueue::default();
        let job = queue.enqueue(valid_request(), &target_dir()).unwrap();
        let mut progress = DownloadProgress {
            downloaded_bytes: 16_384,
            total_bytes: Some(1_048_576),
            progress_percent: None,
            speed_bytes_per_second: None,
            eta_seconds: None,
            supports_resume: true,
        };

        // A chunk on its own says nothing about the speed, so none is made up.
        queue.update_progress(job.id, &progress);
        assert_eq!(queue.job(job.id).unwrap().speed_bytes_per_second, None);

        progress.downloaded_bytes = 524_288;
        progress.speed_bytes_per_second = Some(1_048_576);
        progress.eta_seconds = Some(1);
        queue.update_progress(job.id, &progress);
        let measured = queue.job(job.id).unwrap();
        assert_eq!(measured.speed_bytes_per_second, Some(1_048_576));
        assert_eq!(measured.eta_seconds, Some(1));

        progress.downloaded_bytes = 540_672;
        progress.speed_bytes_per_second = None;
        queue.update_progress(job.id, &progress);
        assert_eq!(
            queue.job(job.id).unwrap().speed_bytes_per_second,
            Some(1_048_576)
        );
    }

    #[test]
    fn history_only_contains_terminal_jobs() {
        let mut queue = DownloadQueue::default();
        let queued = queue.enqueue(valid_request(), &target_dir()).unwrap();
        let completed = queue.enqueue(valid_request(), &target_dir()).unwrap();
        let failed = queue.enqueue(valid_request(), &target_dir()).unwrap();

        queue.mark_completed(
            completed.id,
            CompletedDownload {
                final_path: std::path::PathBuf::from("video.mp4"),
                final_url: Some("https://example.invalid/video.mp4".to_string()),
                quality_report: None,
                filename: "video.mp4".to_string(),
                downloaded_bytes: 1000,
                total_bytes: Some(1000),
                supports_resume: false,
            },
        );
        queue.mark_failed(failed.id, "failed".to_string(), None);

        let history = queue.history_jobs();
        assert_eq!(history.len(), 2);
        assert!(history.iter().any(|job| job.id == completed.id));
        assert!(history.iter().any(|job| job.id == failed.id));
        assert!(!history.iter().any(|job| job.id == queued.id));
    }

    #[test]
    fn removing_history_job_keeps_active_jobs() {
        let mut queue = DownloadQueue::default();
        let active = queue.enqueue(valid_request(), &target_dir()).unwrap();
        let canceled = queue.enqueue(valid_request(), &target_dir()).unwrap();
        queue.mark_canceled(canceled.id);

        assert!(queue.remove_history_job(active.id).is_none());
        assert_eq!(
            queue.remove_history_job(canceled.id).unwrap().id,
            canceled.id
        );
        assert!(queue.job(active.id).is_some());
        assert!(queue.job(canceled.id).is_none());
    }

    #[test]
    fn retry_resets_progress_and_counts_attempt() {
        let mut queue = DownloadQueue::default();
        let job = queue.enqueue(valid_request(), &target_dir()).unwrap();
        queue.update_progress(
            job.id,
            &DownloadProgress {
                downloaded_bytes: 500,
                total_bytes: Some(1000),
                progress_percent: None,
                speed_bytes_per_second: Some(250),
                eta_seconds: Some(2),
                supports_resume: true,
            },
        );
        queue.mark_failed(job.id, "failed".to_string(), Some("report".to_string()));

        queue.mark_retry_queued(job.id, Some(2000));

        let retried = queue.job(job.id).unwrap();
        assert_eq!(retried.status, DownloadStatus::Queued);
        assert_eq!(retried.progress_bytes, 0);
        assert_eq!(retried.total_bytes, Some(2000));
        assert_eq!(retried.progress_percent, None);
        assert_eq!(retried.speed_bytes_per_second, None);
        assert_eq!(retried.eta_seconds, None);
        assert_eq!(retried.retry_count, 1);
        assert_eq!(retried.error, None);
        assert_eq!(retried.error_report, None);
    }

    #[test]
    fn restore_marks_interrupted_direct_jobs_as_paused() {
        let mut queue = DownloadQueue::default();
        let running = queue.enqueue(valid_request(), &target_dir()).unwrap();
        queue.mark_running(running.id);

        let restored = DownloadQueue::restore(queue.jobs().to_vec());
        let restored_job = restored.job(running.id).unwrap();
        assert_eq!(restored_job.status, DownloadStatus::Paused);
        assert!(restored_job.error.unwrap().contains("Resume"));
    }

    #[test]
    fn restore_marks_interrupted_provider_jobs_as_failed() {
        let mut request = valid_request();
        request.adapter_id = "provider-youtube".to_string();
        let mut queue = DownloadQueue::default();
        let running = queue.enqueue(request, &target_dir()).unwrap();
        queue.mark_running(running.id);

        let restored = DownloadQueue::restore(queue.jobs().to_vec());
        let restored_job = restored.job(running.id).unwrap();
        assert_eq!(restored_job.status, DownloadStatus::Failed);
        assert!(restored_job.error.unwrap().contains("Retry"));
    }

    #[test]
    fn restore_marks_interrupted_hls_jobs_as_paused() {
        let mut request = valid_request();
        request.adapter_id = "hls".to_string();
        let mut queue = DownloadQueue::default();
        let running = queue.enqueue(request, &target_dir()).unwrap();
        queue.mark_running(running.id);

        let restored = DownloadQueue::restore(queue.jobs().to_vec());
        let restored_job = restored.job(running.id).unwrap();
        assert_eq!(restored_job.status, DownloadStatus::Paused);
        assert!(restored_job.error.unwrap().contains("Resume"));
    }

    #[test]
    fn completed_jobs_store_the_sanitized_quality_report_of_the_download() {
        let mut queue = DownloadQueue::default();
        let job = queue.enqueue(valid_request(), &target_dir()).unwrap();

        queue.mark_completed(
            job.id,
            CompletedDownload {
                final_path: std::path::PathBuf::from("video.mp4"),
                final_url: Some("https://example.invalid/page".to_string()),
                quality_report: Some(
                    r#"{"version":1,"pageUrlFallback":true,"url":"https://cdn.example.invalid/a.m3u8?sig=visible"}"#
                        .to_string(),
                ),
                filename: "video.mp4".to_string(),
                downloaded_bytes: 1000,
                total_bytes: Some(1000),
                supports_resume: false,
            },
        );

        let completed = queue.job(job.id).unwrap();
        assert_eq!(
            completed.final_url.as_deref(),
            Some("https://example.invalid/page")
        );
        let report: serde_json::Value =
            serde_json::from_str(completed.quality_report.as_deref().unwrap()).unwrap();
        assert_eq!(report["pageUrlFallback"], true);
        assert_eq!(report["url"], "https://cdn.example.invalid/a.m3u8?redacted");
    }

    #[test]
    fn failed_jobs_store_only_sanitized_diagnostics() {
        let mut queue = DownloadQueue::default();
        let job = queue.enqueue(valid_request(), &target_dir()).unwrap();

        queue.mark_failed(
            job.id,
            "GET https://cdn.example.invalid/video.mp4?signature=visible failed".to_string(),
            Some(r#"{"error":"output C:\\Users\\Example\\video.mp4 token=visible"}"#.to_string()),
        );

        let failed = queue.job(job.id).unwrap();
        assert!(failed.error.unwrap().contains("?redacted"));
        let report = failed.error_report.unwrap();
        assert!(report.contains("[local path]"));
        assert!(report.contains("token=[redacted]"));
        assert!(!report.contains("visible"));
    }

    /// A queue of jobs with the given status and time of last change, in this order.
    fn queue_of(jobs: &[(DownloadStatus, u64)]) -> (DownloadQueue, Vec<Uuid>) {
        let mut queue = DownloadQueue::default();
        for _ in jobs {
            queue
                .enqueue(valid_request(), Path::new(r"C:\Downloads"))
                .unwrap();
        }
        let mut restored = queue.jobs().to_vec();
        for (job, (status, updated_at)) in restored.iter_mut().zip(jobs) {
            job.status = *status;
            job.updated_at = *updated_at;
        }
        let ids = restored.iter().map(|job| job.id).collect();
        (DownloadQueue { jobs: restored }, ids)
    }

    #[test]
    fn pruning_keeps_the_history_jobs_that_changed_last() {
        use DownloadStatus::{Canceled, Completed, Failed};
        let (mut queue, ids) = queue_of(&[
            (Completed, 300),
            (Failed, 100),
            (Completed, 400),
            (Canceled, 200),
        ]);

        let removed = queue.prune_history(2);

        assert_eq!(
            removed.iter().map(|job| job.id).collect::<Vec<_>>(),
            [ids[1], ids[3]]
        );
        assert_eq!(
            queue.jobs().iter().map(|job| job.id).collect::<Vec<_>>(),
            [ids[0], ids[2]]
        );
    }

    #[test]
    fn pruning_never_removes_a_job_that_is_not_finished() {
        use DownloadStatus::{Completed, Paused, Queued, Running};
        let (mut queue, ids) = queue_of(&[
            (Paused, 1),
            (Completed, 50),
            (Queued, 2),
            (Running, 3),
            (Completed, 60),
        ]);

        let removed = queue.prune_history(1);

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].id, ids[1]);
        assert_eq!(queue.jobs().len(), 4);
        assert!(queue.prune_history(1).is_empty());
        assert_eq!(queue.prune_history(0).len(), 1);
        assert_eq!(queue.jobs().len(), 3);
    }

    #[test]
    fn of_jobs_that_changed_in_the_same_second_the_ones_queued_first_go() {
        use DownloadStatus::Completed;
        let (mut queue, ids) = queue_of(&[(Completed, 7), (Completed, 7), (Completed, 7)]);

        let removed = queue.prune_history(1);

        assert_eq!(
            removed.iter().map(|job| job.id).collect::<Vec<_>>(),
            [ids[0], ids[1]]
        );
    }

    fn valid_request() -> CreateDownloadJobRequest {
        CreateDownloadJobRequest {
            source_url: "https://example.invalid/video.mp4".to_string(),
            source_fallback_urls: vec!["https://example.invalid/fallback.mp4".to_string()],
            page_url: Some("https://example.invalid/page".to_string()),
            page_title: Some("Video".to_string()),
            referrer: Some("https://example.invalid/page".to_string()),
            suggested_filename: Some("video.mp4".to_string()),
            content_type: Some("video/mp4".to_string()),
            content_length: Some(1000),
            kind: DownloadKind::Video,
            adapter_id: "generic-html5-video".to_string(),
            provider_id: None,
            selected_option_id: Some("original".to_string()),
            headers: Some(BTreeMap::from([
                ("user-agent".to_string(), "Browser".to_string()),
                ("accept".to_string(), "*/*".to_string()),
            ])),
            prefer_range_request: true,
            quality_report: Some(r#"{"selectedAfterPrepare":{"quality":"1080p"}}"#.to_string()),
        }
    }

    fn target_dir() -> std::path::PathBuf {
        std::env::temp_dir().join("ldm-queue-test")
    }
}
