use std::{
    collections::HashMap,
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, PoisonError,
    },
    time::{Duration, Instant},
};

use ldm_core::download::DownloadCancelHandle;
use ldm_core::queue::DownloadQueue;
use ldm_core::storage::{target_folder, validate_target_folder};
use ldm_types::browser_integration::{
    BrowserIntegrationPreferences, BROWSER_INTEGRATION_PREFERENCES_FILENAME,
};
use ldm_types::download_job::{CreateDownloadJobRequest, DownloadJob, DownloadKind};
use ldm_types::download_settings::{
    clamp_max_concurrent_downloads, DownloadSettings, DOWNLOAD_SETTINGS_FILENAME,
};
use ldm_types::timestamp::unix_seconds_now;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::{oneshot, Notify};
use uuid::Uuid;

use crate::error::{AppError, ErrorCode};
use crate::job_events::JobEventSink;

/// Progress ticks reach download-state.json at most this often; status changes are written at
/// once. See `AppState::start_persist_task`.
const PERSIST_INTERVAL: Duration = Duration::from_secs(2);

pub struct ActiveDownloadTask {
    pub handle: tauri::async_runtime::JoinHandle<()>,
    pub cancel: Option<DownloadCancelHandle>,
}

/// Version of the texts in `docs/legal/`, read from `docs/legal/VERSION` at compile time. Bump
/// the file when a text changes; the app then asks for acceptance again.
pub const LEGAL_TERMS_VERSION: &str = include_str!("../../../../docs/legal/VERSION").trim_ascii();
pub const FIRST_RUN_SETUP_MESSAGE: &str = "Open LDM and complete the first-run setup.";

/// Everything the app knows and changes while it runs.
///
/// It sits behind one lock (`AppState::lock`), so a reader always sees the queue, the stored
/// requests, and the running tasks in the same state, and no code path can take two locks in
/// the wrong order.
#[derive(Default)]
pub struct AppData {
    pub queue: DownloadQueue,
    /// The request each job was created from, kept for resume and retry.
    pub job_requests: HashMap<Uuid, CreateDownloadJobRequest>,
    /// The download tasks that are running, by job.
    pub active_tasks: HashMap<Uuid, ActiveDownloadTask>,
    /// Who waits to hear how the server answers a job's first request; see `FirstAnswer`.
    pub first_answers: HashMap<Uuid, oneshot::Sender<FirstAnswer>>,
    legal_acceptance: Option<LegalAcceptance>,
    browser_integration_preferences: BrowserIntegrationPreferences,
    download_settings: DownloadSettings,
}

/// How the server answered the first request of a download that the browser handed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstAnswer {
    /// It is sending the file.
    Accepted,
    /// It turned the request down with this HTTP status before sending any of it.
    Refused(u16),
}

impl AppData {
    /// How many download tasks the scheduler may run at once.
    pub fn max_concurrent_downloads(&self) -> usize {
        self.download_settings.effective_max_concurrent_downloads()
    }
}

#[derive(Clone, Default)]
pub struct AppState {
    data: Arc<Mutex<AppData>>,
    persistence: Arc<PersistenceState>,
    /// Pushes queue changes to the webview; see `commit_job` and friends.
    pub events: Arc<JobEventSink>,
}

#[derive(Debug, Default)]
struct PersistenceState {
    /// Where the state files are. Set once by `initialize_persistence`; until then (tests)
    /// nothing is written.
    paths: OnceLock<PersistencePaths>,
    /// Held while download-state.json is written.
    write_lock: Mutex<()>,
    last_save: Mutex<Option<Instant>>,
    /// Wakes the persist task (`AppState::start_persist_task`).
    wake: Notify,
    /// The next write must not wait for `PERSIST_INTERVAL`.
    urgent: AtomicBool,
    task_started: AtomicBool,
}

#[derive(Debug)]
struct PersistencePaths {
    state: PathBuf,
    browser_integration: PathBuf,
    download_settings: PathBuf,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAppState {
    version: u32,
    jobs: Vec<DownloadJob>,
    job_requests: HashMap<Uuid, CreateDownloadJobRequest>,
    #[serde(default)]
    legal_acceptance: Option<LegalAcceptance>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegalAcceptance {
    pub legal_terms_version: String,
    pub accepted_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegalStatus {
    pub legal_terms_version: String,
    pub accepted_legal_version: Option<String>,
    pub accepted_at: Option<String>,
    pub accepted: bool,
}

/// A lock that a panicking thread left behind is taken anyway: what it guards here is plain
/// data that stays usable, and one failed task must not lock the whole app out of its state.
fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AppState {
    /// The app's data. Hold the guard for the few lines that read or change it and let it go
    /// before any file access, process start, or `.await`.
    pub fn lock(&self) -> MutexGuard<'_, AppData> {
        lock_ignoring_poison(&self.data)
    }

    /// Loads the three state files next to `state_path` and remembers where they are.
    pub fn initialize_persistence(&self, state_path: PathBuf) -> Result<(), AppError> {
        let state_dir = state_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        fs::create_dir_all(&state_dir).map_err(|error| {
            AppError::storage(format!(
                "could not create download state directory '{}': {error}",
                state_dir.display()
            ))
        })?;
        let paths = PersistencePaths {
            browser_integration: state_dir.join(BROWSER_INTEGRATION_PREFERENCES_FILENAME),
            download_settings: state_dir.join(DOWNLOAD_SETTINGS_FILENAME),
            state: state_path,
        };

        let persisted = load_persisted_state(&paths.state).map_err(AppError::storage)?;
        let browser_integration_preferences =
            load_browser_integration_preferences(&paths.browser_integration)
                .map_err(AppError::storage)?;
        let download_settings =
            load_download_settings(&paths.download_settings).map_err(AppError::storage)?;
        let persisted_job_ids = persisted
            .jobs
            .iter()
            .map(|job| job.id)
            .collect::<std::collections::HashSet<_>>();

        {
            let mut data = self.lock();
            data.queue = DownloadQueue::restore(persisted.jobs);
            data.job_requests = persisted
                .job_requests
                .into_iter()
                .filter(|(id, _)| persisted_job_ids.contains(id))
                .collect();
            data.legal_acceptance = persisted.legal_acceptance;
            data.browser_integration_preferences = browser_integration_preferences;
            data.download_settings = download_settings;
        }
        self.persistence
            .paths
            .set(paths)
            .map_err(|_| AppError::internal("the state files are already loaded"))?;

        self.persist_now()
    }

    /// Writes download-state.json on the calling thread. Used at startup, on exit, for legal
    /// acceptance, and by the persist task; queue changes go through `commit_job`,
    /// `commit_jobs` and `note_job_progress` instead.
    pub fn persist_now(&self) -> Result<(), AppError> {
        let Some(paths) = self.persistence.paths.get() else {
            return Ok(());
        };

        // The snapshot is taken under the write lock so two writers can never store an
        // older snapshot after a newer one.
        let _write_guard = lock_ignoring_poison(&self.persistence.write_lock);
        let snapshot = {
            let data = self.lock();
            PersistedAppState {
                version: 1,
                jobs: data.queue.jobs().to_vec(),
                job_requests: data.job_requests.clone(),
                legal_acceptance: data.legal_acceptance.clone(),
            }
        };

        write_persisted_state(&paths.state, &snapshot).map_err(AppError::storage)?;
        *lock_ignoring_poison(&self.persistence.last_save) = Some(Instant::now());
        Ok(())
    }

    /// A job's status or metadata changed: schedule an immediate write and push the job's
    /// snapshot to the UI. Every queue mutation outside progress ticks goes through this or
    /// `commit_jobs`.
    pub fn commit_job(&self, job_id: Uuid) -> Result<(), AppError> {
        self.schedule_persist(true)?;
        match self.job_snapshot(job_id) {
            Some(job) => self.events.emit_job(&job),
            None => self.events.emit_reload(),
        }
        Ok(())
    }

    /// Jobs were added or removed: schedule an immediate write and tell the UI to reload.
    pub fn commit_jobs(&self) -> Result<(), AppError> {
        self.schedule_persist(true)?;
        self.events.emit_reload();
        Ok(())
    }

    /// Progress ticked: schedule a throttled write and push the snapshot at most every 250 ms
    /// per job. Never blocks the download task on disk I/O.
    pub fn note_job_progress(&self, job_id: Uuid) -> Result<(), AppError> {
        self.schedule_persist(false)?;
        // Ticks arrive per chunk, and a snapshot clones the whole job with its reports, so it
        // is taken only when an event will actually go out.
        if self.events.progress_due(job_id) {
            if let Some(job) = self.job_snapshot(job_id) {
                self.events.emit_progress(&job);
            }
        }
        Ok(())
    }

    /// Starts the background writer that turns `schedule_persist` requests into one
    /// download-state.json write at a time, on a blocking thread, at most every
    /// `PERSIST_INTERVAL` unless a request is urgent. Idempotent.
    pub fn start_persist_task(&self) {
        if self.persistence.task_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                state.persistence.wake.notified().await;
                state.wait_for_persist_slot().await;
                let writer = state.clone();
                let written = tauri::async_runtime::spawn_blocking(move || writer.persist_now())
                    .await
                    .unwrap_or_else(|error| {
                        Err(AppError::internal(format!(
                            "download state writer stopped: {error}"
                        )))
                    });
                if let Err(error) = written {
                    tracing::warn!("could not persist download state: {error}");
                }
            }
        });
    }

    /// Asks the persist task for a write; `urgent` skips the throttle. Before the task runs
    /// (tests, early startup) the write happens synchronously instead.
    fn schedule_persist(&self, urgent: bool) -> Result<(), AppError> {
        if !self.persistence.task_started.load(Ordering::Acquire) {
            return self.persist_now();
        }
        if urgent {
            self.persistence.urgent.store(true, Ordering::Release);
        }
        self.persistence.wake.notify_one();
        Ok(())
    }

    /// Waits until `PERSIST_INTERVAL` has passed since the last write, returning early when an
    /// urgent request is pending or arrives meanwhile.
    async fn wait_for_persist_slot(&self) {
        loop {
            if self.persistence.urgent.swap(false, Ordering::AcqRel) {
                return;
            }
            let last_save = *lock_ignoring_poison(&self.persistence.last_save);
            let remaining =
                last_save.and_then(|last_save| PERSIST_INTERVAL.checked_sub(last_save.elapsed()));
            let Some(remaining) = remaining else {
                return;
            };
            tokio::select! {
                _ = tokio::time::sleep(remaining) => return,
                _ = self.persistence.wake.notified() => {}
            }
        }
    }

    fn job_snapshot(&self, job_id: Uuid) -> Option<DownloadJob> {
        self.lock().queue.job(job_id)
    }

    pub fn legal_status(&self) -> LegalStatus {
        legal_status_from_acceptance(self.lock().legal_acceptance.as_ref())
    }

    pub fn accept_current_legal_terms(&self) -> Result<LegalStatus, AppError> {
        let acceptance = LegalAcceptance {
            legal_terms_version: LEGAL_TERMS_VERSION.to_string(),
            accepted_at: current_timestamp(),
        };
        self.lock().legal_acceptance = Some(acceptance.clone());

        self.persist_now()?;
        Ok(legal_status_from_acceptance(Some(&acceptance)))
    }

    pub fn ensure_legal_accepted(&self) -> Result<(), AppError> {
        if self.legal_status().accepted {
            Ok(())
        } else {
            Err(AppError::new(
                ErrorCode::FirstRunSetupRequired,
                FIRST_RUN_SETUP_MESSAGE,
            ))
        }
    }

    pub fn browser_integration_preferences(&self) -> BrowserIntegrationPreferences {
        self.lock().browser_integration_preferences.clone()
    }

    pub fn set_browser_integration_preferences(
        &self,
        preferences: BrowserIntegrationPreferences,
    ) -> Result<BrowserIntegrationPreferences, AppError> {
        let preferences = preferences.normalized();
        let paths = self.persistence.paths.get().ok_or_else(|| {
            AppError::storage("browser integration preferences path is unavailable")
        })?;

        write_browser_integration_preferences(&paths.browser_integration, &preferences)
            .map_err(AppError::storage)?;
        self.lock().browser_integration_preferences = preferences.clone();
        Ok(preferences)
    }

    pub fn download_settings(&self) -> DownloadSettings {
        self.lock().download_settings.clone()
    }

    /// Validates every folder override, writes the settings file, and keeps them in memory.
    pub fn set_download_settings(
        &self,
        settings: DownloadSettings,
    ) -> Result<DownloadSettings, AppError> {
        let settings = normalize_download_settings(settings)?;

        if let Some(paths) = self.persistence.paths.get() {
            write_json_file(&paths.download_settings, &settings, "download settings")
                .map_err(AppError::storage)?;
        }
        self.lock().download_settings = settings.clone();
        Ok(settings)
    }

    /// The folder a new download of `kind` should go to right now.
    pub fn target_folder_for(&self, kind: DownloadKind) -> PathBuf {
        target_folder(kind, &self.lock().download_settings)
    }
}

fn normalize_download_settings(settings: DownloadSettings) -> Result<DownloadSettings, AppError> {
    let mut normalized = DownloadSettings {
        max_concurrent_downloads: clamp_max_concurrent_downloads(settings.max_concurrent_downloads),
        ..DownloadSettings::default()
    };
    for (kind, value) in settings.target_folders {
        if value.trim().is_empty() {
            continue;
        }
        let path = validate_target_folder(&value)?;
        normalized
            .target_folders
            .insert(kind, path.to_string_lossy().to_string());
    }
    Ok(normalized)
}

fn legal_status_from_acceptance(acceptance: Option<&LegalAcceptance>) -> LegalStatus {
    let accepted = acceptance.is_some_and(|value| value.legal_terms_version == LEGAL_TERMS_VERSION);
    LegalStatus {
        legal_terms_version: LEGAL_TERMS_VERSION.to_string(),
        accepted_legal_version: acceptance.map(|value| value.legal_terms_version.clone()),
        accepted_at: acceptance.map(|value| value.accepted_at.clone()),
        accepted,
    }
}

fn current_timestamp() -> String {
    unix_seconds_now().to_string()
}

fn load_persisted_state(path: &Path) -> Result<PersistedAppState, String> {
    let payload = match fs::read(path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(PersistedAppState::default())
        }
        Err(error) => {
            return Err(format!(
                "could not read download state '{}': {error}",
                path.display()
            ))
        }
    };

    match serde_json::from_slice::<PersistedAppState>(&payload) {
        Ok(state) => Ok(state),
        Err(error) => {
            let corrupt_path = corrupt_state_path(path);
            match fs::rename(path, &corrupt_path) {
                Ok(()) => {
                    tracing::warn!(
                        "download state was unreadable and was moved to '{}': {error}",
                        corrupt_path.display()
                    );
                }
                Err(rename_error) => {
                    tracing::warn!(
                        "download state was unreadable and could not be moved aside: {rename_error}; parse error: {error}"
                    );
                }
            }
            Ok(PersistedAppState::default())
        }
    }
}

fn write_persisted_state(path: &Path, state: &PersistedAppState) -> Result<(), String> {
    let temp_path = path.with_extension("json.tmp");
    let payload = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("could not serialize download state: {error}"))?;

    {
        let mut file = fs::File::create(&temp_path).map_err(|error| {
            format!(
                "could not create temporary download state '{}': {error}",
                temp_path.display()
            )
        })?;
        file.write_all(&payload).map_err(|error| {
            format!(
                "could not write temporary download state '{}': {error}",
                temp_path.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "could not flush temporary download state '{}': {error}",
                temp_path.display()
            )
        })?;
    }

    match fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            fs::remove_file(path).map_err(|remove_error| {
                format!(
                    "could not replace previous download state '{}': {remove_error}",
                    path.display()
                )
            })?;
            fs::rename(&temp_path, path).map_err(|rename_error| {
                format!(
                    "could not save download state '{}': {rename_error}",
                    path.display()
                )
            })
        }
        Err(error) => Err(format!(
            "could not save download state '{}': {error}",
            path.display()
        )),
    }
}

fn load_browser_integration_preferences(
    path: &Path,
) -> Result<BrowserIntegrationPreferences, String> {
    load_json_file_or_default::<BrowserIntegrationPreferences>(
        path,
        "browser integration preferences",
    )
    .map(BrowserIntegrationPreferences::normalized)
}

fn write_browser_integration_preferences(
    path: &Path,
    preferences: &BrowserIntegrationPreferences,
) -> Result<(), String> {
    write_json_file(path, preferences, "browser integration preferences")
}

fn load_download_settings(path: &Path) -> Result<DownloadSettings, String> {
    let settings = load_json_file_or_default::<DownloadSettings>(path, "download settings")?;
    // Overrides that no longer validate (for example a folder that is now a file) are dropped
    // silently so a bad entry never blocks startup; the UI shows the default for that kind.
    let mut cleaned = DownloadSettings {
        max_concurrent_downloads: clamp_max_concurrent_downloads(settings.max_concurrent_downloads),
        ..DownloadSettings::default()
    };
    for (kind, value) in settings.target_folders {
        if validate_target_folder(&value).is_ok() {
            cleaned.target_folders.insert(kind, value);
        } else {
            tracing::warn!("ignoring invalid target folder for {kind:?}");
        }
    }
    Ok(cleaned)
}

/// Reads a JSON settings file, returning `T::default()` when the file does not exist yet.
fn load_json_file_or_default<T: DeserializeOwned + Default>(
    path: &Path,
    label: &str,
) -> Result<T, String> {
    let payload = match fs::read(path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => {
            return Err(format!(
                "could not read {label} '{}': {error}",
                path.display()
            ))
        }
    };

    serde_json::from_slice::<T>(&payload)
        .map_err(|error| format!("could not parse {label} '{}': {error}", path.display()))
}

/// Writes a JSON settings file atomically through a temporary file next to it.
fn write_json_file<T: Serialize>(path: &Path, value: &T, label: &str) -> Result<(), String> {
    let payload = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("could not serialize {label}: {error}"))?;
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, payload)
        .map_err(|error| format!("could not write {label} '{}': {error}", temp_path.display()))?;
    replace_file(&temp_path, path, label)
}

fn replace_file(temp_path: &Path, path: &Path, label: &str) -> Result<(), String> {
    match fs::rename(temp_path, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            fs::remove_file(path).map_err(|remove_error| {
                format!("could not replace previous {label}: {remove_error}")
            })?;
            fs::rename(temp_path, path)
                .map_err(|rename_error| format!("could not save {label}: {rename_error}"))
        }
        Err(error) => Err(format!("could not save {label}: {error}")),
    }
}

fn corrupt_state_path(path: &Path) -> PathBuf {
    let mut candidate = path.with_extension("json.corrupt");
    for index in 1..100 {
        if !candidate.exists() {
            return candidate;
        }
        candidate = path.with_extension(format!("json.corrupt.{index}"));
    }
    candidate
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use ldm_core::download::CompletedDownload;
    use ldm_types::browser_integration::{BrowserDownloadDomainRule, BrowserDownloadMode};
    use ldm_types::download_job::{DownloadKind, DownloadStatus};

    use super::*;

    #[test]
    fn initializes_persisted_jobs_and_requests() {
        let state_path = temp_state_path();
        let parent = state_path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();

        let request = valid_request();
        let mut queue = DownloadQueue::default();
        let running = queue.enqueue(request.clone(), &target_dir()).unwrap();
        queue.mark_running(running.id);
        let completed = queue.enqueue(request.clone(), &target_dir()).unwrap();
        queue.mark_completed(
            completed.id,
            CompletedDownload {
                final_path: PathBuf::from("video.mp4"),
                final_url: Some("https://example.invalid/video.mp4".to_string()),
                quality_report: None,
                filename: "video.mp4".to_string(),
                downloaded_bytes: 1000,
                total_bytes: Some(1000),
                supports_resume: true,
            },
        );

        write_persisted_state(
            &state_path,
            &PersistedAppState {
                version: 1,
                jobs: queue.jobs().to_vec(),
                job_requests: HashMap::from([
                    (running.id, request.clone()),
                    (completed.id, request.clone()),
                ]),
                legal_acceptance: Some(LegalAcceptance {
                    legal_terms_version: LEGAL_TERMS_VERSION.to_string(),
                    accepted_at: "100".to_string(),
                }),
            },
        )
        .unwrap();

        let state = AppState::default();
        state.initialize_persistence(state_path.clone()).unwrap();

        let jobs = state.lock().queue.jobs().to_vec();
        assert_eq!(jobs.len(), 2);
        assert_eq!(
            jobs.iter().find(|job| job.id == running.id).unwrap().status,
            DownloadStatus::Paused
        );
        assert_eq!(
            jobs.iter()
                .find(|job| job.id == completed.id)
                .unwrap()
                .status,
            DownloadStatus::Completed
        );
        assert_eq!(state.lock().job_requests.len(), 2);
        assert!(state.legal_status().accepted);

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn commits_write_synchronously_before_the_persist_task_runs() {
        let state_path = temp_state_path();
        let parent = state_path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        let state = AppState::default();
        state.initialize_persistence(state_path.clone()).unwrap();

        let job = state
            .lock()
            .queue
            .enqueue(valid_request(), &target_dir())
            .unwrap();
        state.commit_jobs().unwrap();

        let persisted = load_persisted_state(&state_path).unwrap();
        assert!(persisted.jobs.iter().any(|entry| entry.id == job.id));

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn persist_task_writes_urgent_commits_promptly_and_progress_later() {
        let state_path = temp_state_path();
        let parent = state_path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        let state = AppState::default();
        state.initialize_persistence(state_path.clone()).unwrap();
        state.start_persist_task();

        let job = state
            .lock()
            .queue
            .enqueue(valid_request(), &target_dir())
            .unwrap();
        state.commit_jobs().unwrap();
        assert!(
            wait_for_persisted(&state_path, Duration::from_millis(1500), |persisted| {
                persisted.jobs.iter().any(|entry| entry.id == job.id)
            }),
            "urgent commit was not written within 1.5 s"
        );

        state.lock().queue.update_progress(
            job.id,
            &ldm_core::download::DownloadProgress {
                downloaded_bytes: 500,
                total_bytes: Some(1000),
                progress_percent: None,
                speed_bytes_per_second: None,
                eta_seconds: None,
                supports_resume: true,
            },
        );
        state.note_job_progress(job.id).unwrap();
        let progress_written = |persisted: &PersistedAppState| {
            persisted
                .jobs
                .iter()
                .any(|entry| entry.id == job.id && entry.progress_bytes == 500)
        };
        assert!(
            !wait_for_persisted(&state_path, Duration::from_millis(800), progress_written),
            "progress was written before the throttle interval passed"
        );
        assert!(
            wait_for_persisted(
                &state_path,
                PERSIST_INTERVAL + Duration::from_secs(2),
                progress_written
            ),
            "progress was not written after the throttle interval"
        );

        let _ = fs::remove_dir_all(parent);
    }

    fn wait_for_persisted(
        path: &Path,
        timeout: Duration,
        predicate: impl Fn(&PersistedAppState) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(persisted) = load_persisted_state(path) {
                if predicate(&persisted) {
                    return true;
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn loads_state_written_with_string_timestamps() {
        let state_path = temp_state_path();
        let parent = state_path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();

        let job_id = Uuid::new_v4();
        let payload = serde_json::json!({
            "version": 1,
            "jobs": [{
                "id": job_id,
                "sourceUrl": "https://example.invalid/video.mp4",
                "finalUrl": null,
                "filename": "video.mp4",
                "targetDirectory": target_dir().to_string_lossy(),
                "tempPath": "",
                "kind": "video",
                "adapterId": "generic-html5-video",
                "providerId": null,
                "selectedOptionId": null,
                "requestContentType": null,
                "requestContentLength": null,
                "sourceFallbackCount": 0,
                "forwardedHeaderNames": [],
                "preferRangeRequest": false,
                "referrerPresent": false,
                "pageUrl": null,
                "status": "completed",
                "progressBytes": 1000,
                "totalBytes": 1000,
                "progressPercent": 100.0,
                "speedBytesPerSecond": null,
                "etaSeconds": 0,
                "supportsResume": false,
                "retryCount": 0,
                "error": null,
                "errorReport": null,
                "qualityReport": null,
                "createdAt": "1760000000",
                "updatedAt": "1760000123"
            }],
            "jobRequests": {},
            "legalAcceptance": null
        });
        fs::write(&state_path, serde_json::to_vec(&payload).unwrap()).unwrap();

        let state = AppState::default();
        state.initialize_persistence(state_path.clone()).unwrap();

        let job = state.lock().queue.job(job_id).unwrap();
        assert_eq!(job.created_at, 1_760_000_000);
        assert_eq!(job.updated_at, 1_760_000_123);
        assert!(!state_path.with_extension("json.corrupt").exists());

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn a_panic_while_the_data_is_locked_does_not_lock_the_app_out() {
        let state = AppState::default();
        let panicking = state.clone();
        let panicked = std::thread::spawn(move || {
            let _data = panicking.lock();
            panic!("a task failed while it held the lock");
        })
        .join();
        assert!(panicked.is_err());

        state
            .lock()
            .queue
            .enqueue(valid_request(), &target_dir())
            .unwrap();
        assert_eq!(state.lock().queue.jobs().len(), 1);
        assert!(!state.legal_status().accepted);
    }

    #[test]
    fn the_state_files_are_loaded_once() {
        let state_path = temp_state_path();
        let parent = state_path.parent().unwrap().to_path_buf();
        let state = AppState::default();
        state.initialize_persistence(state_path.clone()).unwrap();

        assert!(state.initialize_persistence(state_path).is_err());

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn moves_corrupt_state_aside_and_starts_empty() {
        let state_path = temp_state_path();
        let parent = state_path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::write(&state_path, b"{not json").unwrap();

        let state = AppState::default();
        state.initialize_persistence(state_path.clone()).unwrap();

        assert!(state.lock().queue.jobs().is_empty());
        assert!(state_path.exists());
        assert!(state_path.with_extension("json.corrupt").exists());

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn legal_status_requires_current_version() {
        let state = AppState::default();
        assert!(!state.legal_status().accepted);
        assert!(state.ensure_legal_accepted().is_err());

        state.accept_current_legal_terms().unwrap();
        assert!(state.legal_status().accepted);
        assert!(state.ensure_legal_accepted().is_ok());
    }

    #[test]
    fn old_legal_version_requires_acceptance_again() {
        let state = AppState::default();
        state.lock().legal_acceptance = Some(LegalAcceptance {
            legal_terms_version: "2026-01-01".to_string(),
            accepted_at: "100".to_string(),
        });

        let status = state.legal_status();
        assert!(!status.accepted);
        assert_eq!(status.accepted_legal_version.as_deref(), Some("2026-01-01"));
    }

    #[test]
    fn persists_normalized_browser_integration_preferences() {
        let state_path = temp_state_path();
        let parent = state_path.parent().unwrap().to_path_buf();
        let preferences_path = parent.join(BROWSER_INTEGRATION_PREFERENCES_FILENAME);
        fs::create_dir_all(&parent).unwrap();

        let state = AppState::default();
        state.initialize_persistence(state_path).unwrap();
        let preferences = BrowserIntegrationPreferences {
            mode: BrowserDownloadMode::Ask,
            domain_rules: vec![BrowserDownloadDomainRule {
                host: " Example.COM. ".to_string(),
                action: BrowserDownloadMode::Auto,
            }],
            ..Default::default()
        };

        let saved = state
            .set_browser_integration_preferences(preferences)
            .unwrap();
        assert_eq!(saved.mode, BrowserDownloadMode::Ask);
        assert_eq!(saved.domain_rules[0].host, "example.com");

        let loaded = load_browser_integration_preferences(&preferences_path).unwrap();
        assert_eq!(loaded, saved);

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn persists_target_folder_overrides_and_resolves_them() {
        let state_path = temp_state_path();
        let parent = state_path.parent().unwrap().to_path_buf();
        let settings_path = parent.join(DOWNLOAD_SETTINGS_FILENAME);
        fs::create_dir_all(&parent).unwrap();
        let custom_dir = parent.join("custom-videos");

        let state = AppState::default();
        state.initialize_persistence(state_path).unwrap();
        assert_eq!(
            state.target_folder_for(DownloadKind::Video),
            ldm_core::storage::default_target_folder(DownloadKind::Video)
        );

        let saved = state
            .set_download_settings(DownloadSettings {
                target_folders: std::collections::BTreeMap::from([
                    (
                        DownloadKind::Video,
                        custom_dir.to_string_lossy().to_string(),
                    ),
                    (DownloadKind::Audio, "   ".to_string()),
                ]),
                max_concurrent_downloads: 0,
            })
            .unwrap();
        assert_eq!(saved.target_folders.len(), 1);
        assert_eq!(saved.max_concurrent_downloads, 1);
        assert_eq!(state.target_folder_for(DownloadKind::Video), custom_dir);
        assert!(state
            .set_download_settings(DownloadSettings {
                target_folders: std::collections::BTreeMap::from([(
                    DownloadKind::Audio,
                    "relative/folder".to_string()
                )]),
                ..Default::default()
            })
            .is_err());

        let loaded = load_download_settings(&settings_path).unwrap();
        assert_eq!(loaded, saved);

        let _ = fs::remove_dir_all(parent);
    }

    fn target_dir() -> PathBuf {
        std::env::temp_dir().join("ldm-app-state-test")
    }

    fn temp_state_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!("ldm-state-test-{}", Uuid::new_v4()))
            .join("download-state.json")
    }

    fn valid_request() -> CreateDownloadJobRequest {
        CreateDownloadJobRequest {
            source_url: "https://example.invalid/video.mp4".to_string(),
            source_fallback_urls: Vec::new(),
            page_url: Some("https://example.invalid/page".to_string()),
            page_title: Some("Video".to_string()),
            referrer: Some("https://example.invalid/page".to_string()),
            suggested_filename: Some("video.mp4".to_string()),
            content_type: Some("video/mp4".to_string()),
            content_length: Some(1000),
            kind: DownloadKind::Video,
            adapter_id: "generic-html5-video".to_string(),
            provider_id: None,
            selected_option_id: Some("direct".to_string()),
            headers: Some(BTreeMap::from([(
                "user-agent".to_string(),
                "Browser".to_string(),
            )])),
            prefer_range_request: true,
            quality_report: None,
        }
    }
}

#[cfg(test)]
mod legal_version_tests {
    use super::LEGAL_TERMS_VERSION;

    #[test]
    fn legal_terms_version_is_the_date_in_docs_legal_version() {
        assert_eq!(
            LEGAL_TERMS_VERSION.len(),
            "2026-05-26".len(),
            "{LEGAL_TERMS_VERSION:?}"
        );
        assert!(
            LEGAL_TERMS_VERSION
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'-'),
            "{LEGAL_TERMS_VERSION:?}"
        );
    }
}
