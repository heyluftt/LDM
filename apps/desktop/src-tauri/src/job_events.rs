use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use ldm_types::download_job::DownloadJob;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

pub const JOBS_CHANGED_EVENT: &str = "ldm://jobs-changed";

/// Progress events per job are sent at most this often.
const PROGRESS_EVENT_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum JobsChangedEvent<'a> {
    Job { job: &'a DownloadJob },
    Reload,
}

/// Emits `JOBS_CHANGED_EVENT` once an app handle is attached.
#[derive(Default)]
pub struct JobEventSink {
    app_handle: Mutex<Option<AppHandle>>,
    last_progress_emit: Mutex<HashMap<Uuid, Instant>>,
}

impl JobEventSink {
    pub fn attach(&self, app_handle: AppHandle) {
        if let Ok(mut handle) = self.app_handle.lock() {
            *handle = Some(app_handle);
        }
    }

    pub fn emit_job(&self, job: &DownloadJob) {
        if let Ok(mut last) = self.last_progress_emit.lock() {
            last.remove(&job.id);
        }
        self.emit(&JobsChangedEvent::Job { job });
    }

    /// Whether a progress event for `job_id` is due; `true` claims the slot.
    pub fn progress_due(&self, job_id: Uuid) -> bool {
        let now = Instant::now();
        match self.last_progress_emit.lock() {
            Ok(mut last) => match last.get(&job_id) {
                Some(previous) if now.duration_since(*previous) < PROGRESS_EVENT_INTERVAL => false,
                _ => {
                    last.insert(job_id, now);
                    true
                }
            },
            Err(_) => true,
        }
    }

    pub fn emit_progress(&self, job: &DownloadJob) {
        self.emit(&JobsChangedEvent::Job { job });
    }

    pub fn emit_reload(&self) {
        self.emit(&JobsChangedEvent::Reload);
    }

    fn emit(&self, event: &JobsChangedEvent<'_>) {
        let handle = match self.app_handle.lock() {
            Ok(handle) => handle.clone(),
            Err(_) => return,
        };
        if let Some(handle) = handle {
            if let Err(error) = handle.emit(JOBS_CHANGED_EVENT, event) {
                tracing::warn!("could not emit {JOBS_CHANGED_EVENT}: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_due_once_per_interval_and_per_job() {
        let sink = JobEventSink::default();
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);

        assert!(sink.progress_due(first));
        assert!(!sink.progress_due(first));
        assert!(sink.progress_due(second));

        std::thread::sleep(PROGRESS_EVENT_INTERVAL + Duration::from_millis(20));
        assert!(sink.progress_due(first));
    }
}
