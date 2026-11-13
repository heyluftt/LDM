use ldm_core::{
    diagnostics::sanitize_diagnostic_message,
    download::{
        download_cancellation_pair, remove_partial_download, run_http_download, CompletedDownload,
        DownloadProgress,
    },
    error::{CoreError, ProviderFailure},
    filename::job_partial_path,
    provider::{cleanup_provider_job_artifacts, run_external_provider_download},
};
use ldm_types::download_job::{CreateDownloadJobRequest, DownloadJob, DownloadStatus};
use ldm_types::timestamp::unix_seconds_now;
use serde_json::json;
use tauri::State;
use uuid::Uuid;

use std::time::Duration;

use tokio::sync::oneshot;

use crate::app_state::{AppState, FirstAnswer};
use crate::error::{AppError, ErrorCode};

/// How many completed, failed, and canceled jobs the history keeps. Every job is written to
/// download-state.json with its reports, several kilobytes each, and the file is rewritten
/// every few seconds while a download runs, so a history without a limit makes each of those
/// writes slower for as long as the app is in use.
const HISTORY_LIMIT: usize = 200;

/// How long the reply to the extension waits for the server's first answer to a download that
/// the browser handed over. A server that refuses does so in well under a second; one that
/// takes longer than this is treated as serving the file.
const FIRST_ANSWER_WAIT: Duration = Duration::from_secs(4);

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> Vec<DownloadJob> {
    state.lock().queue.jobs().to_vec()
}

#[tauri::command]
pub fn list_history_jobs(state: State<'_, AppState>) -> Vec<DownloadJob> {
    state.lock().queue.history_jobs()
}

#[tauri::command]
pub async fn create_job(
    state: State<'_, AppState>,
    request: CreateDownloadJobRequest,
) -> Result<DownloadJob, AppError> {
    start_download_job(state.inner(), request)
}

pub fn start_download_job(
    state: &AppState,
    request: CreateDownloadJobRequest,
) -> Result<DownloadJob, AppError> {
    enqueue_download_job(state, request, false).map(|(job, _)| job)
}

/// Starts a job for the extension. A download that the browser handed over goes back to the
/// browser when the server refuses LDM.
///
/// The browser asks for a file with the user's login; LDM asks without it. A file behind a
/// login (a draft release, a customer portal, a cloud drive) is therefore refused to LDM with
/// 401, 403, or 404 although the browser would get it. By then the extension has stopped the
/// browser's own download, so accepting the job at once left the user with a failed entry and
/// no file. For such a job the reply now waits for the server's first answer: if it is a
/// refusal, the job is removed and rejected with `SourceRefused`, and the extension lets the
/// browser continue.
pub async fn start_job_from_extension(
    state: &AppState,
    request: CreateDownloadJobRequest,
) -> Result<DownloadJob, AppError> {
    if !browser_can_take_back(&request) {
        return start_download_job(state, request);
    }
    let (job, first_answer) = enqueue_download_job(state, request, true)?;
    let Some(mut first_answer) = first_answer else {
        return Ok(job);
    };

    // A job that waits for a free slot has not asked the server yet. The browser's download
    // is not kept on hold for that.
    let waits_for_a_slot = {
        let mut data = state.lock();
        let waits = !data.active_tasks.contains_key(&job.id)
            && data
                .queue
                .job(job.id)
                .is_some_and(|current| current.status == DownloadStatus::Queued);
        if waits {
            data.first_answers.remove(&job.id);
        }
        waits
    };
    if waits_for_a_slot {
        return Ok(job);
    }

    let answer = match tokio::time::timeout(FIRST_ANSWER_WAIT, &mut first_answer).await {
        Ok(answer) => answer.ok(),
        Err(_) => {
            // The task may be answering at this very moment. After `close` it no longer can,
            // and what it sent before is still there to be read.
            first_answer.close();
            first_answer.try_recv().ok()
        }
    };
    match answer {
        Some(FirstAnswer::Refused(status)) => Err(AppError::new(
            ErrorCode::SourceRefused,
            format!(
                "The server refused LDM's request for this file (HTTP {status}); it may need the browser's login. The browser downloads it instead."
            ),
        )),
        // The server is sending the file, the job failed for another reason, or there is no
        // answer yet: the job stays with LDM.
        Some(FirstAnswer::Accepted) | None => Ok(job),
    }
}

/// Jobs the extension took away from the browser, which the browser can still download
/// itself: a download it had started, or a link that was clicked.
fn browser_can_take_back(request: &CreateDownloadJobRequest) -> bool {
    matches!(
        request.adapter_id.as_str(),
        "browser-download" | "generic-download-links"
    )
}

/// The HTTP status of a request the server turned down for good, as opposed to an answer
/// that is worth another attempt.
fn refusal_status(error: &CoreError) -> Option<u16> {
    match error {
        CoreError::HttpStatus(status)
            if status.is_client_error() && !ldm_core::http::is_transient_status(*status) =>
        {
            Some(status.as_u16())
        }
        _ => None,
    }
}

/// Adds the job and starts it when a slot is free. With `watch_first_answer`, the returned
/// receiver hears how the server answers the job's first request.
fn enqueue_download_job(
    state: &AppState,
    request: CreateDownloadJobRequest,
    watch_first_answer: bool,
) -> Result<(DownloadJob, Option<oneshot::Receiver<FirstAnswer>>), AppError> {
    state.ensure_legal_accepted()?;
    let target_dir = state.target_folder_for(request.kind);

    let (job, first_answer) = {
        let mut data = state.lock();
        let job = data.queue.enqueue(request.clone(), &target_dir)?;
        data.job_requests.insert(job.id, request);
        // Registered before the job can start, so its first answer cannot be missed.
        let first_answer = watch_first_answer.then(|| {
            let (sender, receiver) = oneshot::channel();
            data.first_answers.insert(job.id, sender);
            receiver
        });
        (job, first_answer)
    };
    state.commit_jobs()?;

    tracing::info!(
        "job {} created: {} via {} into '{}'",
        job.id,
        job.filename,
        job.adapter_id,
        job.target_directory
    );
    schedule_pending_jobs(state);

    Ok((job, first_answer))
}

/// Starts waiting jobs, oldest first, until `max_concurrent_downloads` tasks run. Called after
/// every change that queues a job or frees a slot: create, resume, retry, pause, cancel, task
/// end, and a raised limit.
pub fn schedule_pending_jobs(state: &AppState) {
    while let Some(job_id) = next_job_to_start(state) {
        let started = stored_job_request(state, job_id)
            .and_then(|request| job_paths(state, job_id).map(|paths| (request, paths)))
            .and_then(|(request, paths)| spawn_download_task(state, job_id, request, paths));
        if let Err(error) = started {
            if is_active_task(state, job_id) {
                // Another scheduler run started it between our lookup and the spawn.
                continue;
            }
            tracing::warn!("job {job_id} could not start: {error}");
            state.lock().queue.mark_failed(job_id, error.message, None);
            commit_job_or_warn(state, job_id, "failing a download that could not start");
            prune_history(state);
        }
    }
}

/// Drops the history entries beyond `HISTORY_LIMIT`, oldest first, with their stored requests
/// and the partial files a failed download left for a resume. Downloaded files stay where
/// they are; only the entry in the list goes.
pub fn prune_history(state: &AppState) {
    let removed = {
        let mut data = state.lock();
        let removed = data.queue.prune_history(HISTORY_LIMIT);
        for job in &removed {
            data.job_requests.remove(&job.id);
        }
        removed
    };
    if removed.is_empty() {
        return;
    }

    tracing::info!(
        "history is over {HISTORY_LIMIT} entries: removed the {} oldest",
        removed.len()
    );
    remove_leftover_files(&removed);
    if let Err(error) = state.commit_jobs() {
        tracing::warn!("could not persist download state after pruning the history: {error}");
    }
}

/// Removes the partial files of jobs that left the history. Only a failed job still has any:
/// they are what a resume would have continued.
fn remove_leftover_files(removed: &[DownloadJob]) {
    for job in removed {
        if let Err(error) = cleanup_temp_files_of(job) {
            tracing::warn!(
                "could not remove the partial files of job {}: {error}",
                job.id
            );
        }
    }
}

/// The oldest queued job without a task, if a slot is free.
fn next_job_to_start(state: &AppState) -> Option<Uuid> {
    let data = state.lock();
    if data.active_tasks.len() >= data.max_concurrent_downloads() {
        return None;
    }
    data.queue
        .jobs()
        .iter()
        .find(|job| {
            job.status == DownloadStatus::Queued && !data.active_tasks.contains_key(&job.id)
        })
        .map(|job| job.id)
}

#[tauri::command]
pub async fn pause_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<DownloadJob, AppError> {
    let job_id = parse_job_id(&job_id)?;
    let request = stored_job_request(state.inner(), job_id)?;
    ensure_pause_resume_control(&request)?;
    let current = current_job(state.inner(), job_id)?;
    if current.status == DownloadStatus::Paused {
        return Ok(current);
    }
    ensure_status(
        current.status,
        &[DownloadStatus::Queued, DownloadStatus::Running],
        "only running resumable downloads can be paused",
    )?;

    abort_active_task(state.inner(), job_id).await;

    let job = {
        let mut data = state.lock();
        let current = data.queue.job(job_id).ok_or_else(AppError::job_not_found)?;
        if !is_terminal_status(current.status) {
            data.queue.mark_paused(job_id);
        }
        data.queue.job(job_id).ok_or_else(AppError::job_not_found)?
    };
    state.inner().commit_job(job_id)?;
    schedule_pending_jobs(state.inner());
    Ok(job)
}

#[tauri::command]
pub fn resume_job(state: State<'_, AppState>, job_id: String) -> Result<DownloadJob, AppError> {
    let job_id = parse_job_id(&job_id)?;
    ensure_pause_resume_control(&stored_job_request(state.inner(), job_id)?)?;
    if is_active_task(state.inner(), job_id) {
        return current_job(state.inner(), job_id);
    }
    {
        let mut data = state.lock();
        let job = data.queue.job(job_id).ok_or_else(AppError::job_not_found)?;
        ensure_status(
            job.status,
            &[DownloadStatus::Paused, DownloadStatus::Failed],
            "only paused or failed resumable downloads can be resumed",
        )?;
        data.queue.mark_queued(job_id);
    }
    state.inner().commit_job(job_id)?;
    schedule_pending_jobs(state.inner());
    current_job(state.inner(), job_id)
}

#[tauri::command]
pub async fn retry_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<DownloadJob, AppError> {
    let job_id = parse_job_id(&job_id)?;
    let current = current_job(state.inner(), job_id)?;
    ensure_status(
        current.status,
        &[DownloadStatus::Failed, DownloadStatus::Canceled],
        "only failed or canceled downloads can be retried",
    )?;
    abort_active_task(state.inner(), job_id).await;
    let request = stored_job_request(state.inner(), job_id)?;
    let total_bytes = if request.adapter_kind().uses_external_backend() {
        None
    } else {
        request.content_length
    };
    state.lock().queue.mark_retry_queued(job_id, total_bytes);
    state.inner().commit_job(job_id)?;
    schedule_pending_jobs(state.inner());
    current_job(state.inner(), job_id)
}

#[tauri::command]
pub async fn cancel_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<DownloadJob, AppError> {
    let job_id = parse_job_id(&job_id)?;
    let _ = stored_job_request(state.inner(), job_id)?;
    let current = current_job(state.inner(), job_id)?;
    if is_terminal_status(current.status) {
        return Ok(current);
    }
    ensure_status(
        current.status,
        &[
            DownloadStatus::Queued,
            DownloadStatus::Running,
            DownloadStatus::Paused,
        ],
        "only queued, running, or paused downloads can be canceled",
    )?;
    abort_active_task(state.inner(), job_id).await;

    // The task may have finished the download while it was being stopped.
    if !is_terminal_status(current_job(state.inner(), job_id)?.status) {
        cleanup_job_temp_files(state.inner(), job_id)?;
        state.lock().queue.mark_canceled(job_id);
    }

    let job = current_job(state.inner(), job_id)?;
    state.inner().commit_job(job_id)?;
    prune_history(state.inner());
    schedule_pending_jobs(state.inner());
    Ok(job)
}

#[tauri::command]
pub fn remove_history_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<DownloadJob, AppError> {
    let job_id = parse_job_id(&job_id)?;
    let removed = {
        let mut data = state.lock();
        let removed = data
            .queue
            .remove_history_job(job_id)
            .ok_or_else(|| AppError::new(ErrorCode::JobNotFound, "history entry was not found"))?;
        data.job_requests.remove(&job_id);
        removed
    };

    remove_leftover_files(std::slice::from_ref(&removed));
    state.inner().commit_jobs()?;
    Ok(removed)
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> Result<usize, AppError> {
    let removed = {
        let mut data = state.lock();
        let removed = data.queue.clear_history();
        for job in &removed {
            data.job_requests.remove(&job.id);
        }
        removed
    };

    remove_leftover_files(&removed);
    state.inner().commit_jobs()?;
    Ok(removed.len())
}

#[tauri::command]
pub fn open_job_file(state: State<'_, AppState>, job_id: String) -> Result<(), AppError> {
    let job = completed_job(state.inner(), parse_job_id(&job_id)?)?;
    open_path(std::path::PathBuf::from(job.target_directory).join(job.filename))
}

#[tauri::command]
pub fn show_job_in_folder(state: State<'_, AppState>, job_id: String) -> Result<(), AppError> {
    let job = completed_job(state.inner(), parse_job_id(&job_id)?)?;
    let file_path = std::path::PathBuf::from(&job.target_directory).join(&job.filename);
    show_path_in_folder(file_path, std::path::PathBuf::from(job.target_directory))
}

/// Where a job downloads to and where it keeps its partial file.
struct JobPaths {
    target_dir: std::path::PathBuf,
    temp_path: std::path::PathBuf,
}

/// Both paths were fixed when the job was created. Retries and resumes keep them even if the
/// settings changed since, so the partial file is found again and the job's history stays
/// consistent.
fn job_paths(state: &AppState, job_id: Uuid) -> Result<JobPaths, AppError> {
    let job = current_job(state, job_id)?;
    let target_dir = if job.target_directory.trim().is_empty() {
        state.target_folder_for(job.kind)
    } else {
        std::path::PathBuf::from(&job.target_directory)
    };
    let temp_path = if job.temp_path.trim().is_empty() {
        job_partial_path(&target_dir, job_id)
    } else {
        std::path::PathBuf::from(&job.temp_path)
    };
    Ok(JobPaths {
        target_dir,
        temp_path,
    })
}

fn spawn_download_task(
    state: &AppState,
    job_id: Uuid,
    request: CreateDownloadJobRequest,
    paths: JobPaths,
) -> Result<(), AppError> {
    let JobPaths {
        target_dir,
        temp_path,
    } = paths;
    // Held until the task is registered below. The task's first step needs the same lock, so
    // it cannot finish, and try to unregister itself, before it is in the registry.
    let mut data = state.lock();
    if data.active_tasks.contains_key(&job_id) {
        return Err(AppError::invalid_job_state(
            "download job is already running",
        ));
    }

    let task_state = state.clone();
    let (cancel_handle, cancellation) = download_cancellation_pair();

    let handle = tauri::async_runtime::spawn(async move {
        task_state.lock().queue.mark_running(job_id);
        commit_job_or_warn(&task_state, job_id, "marking a download as running");

        let progress_state = task_state.clone();
        let on_progress = move |progress: DownloadProgress| {
            {
                let mut data = progress_state.lock();
                data.queue.update_progress(job_id, &progress);
                // Progress means the server is sending the file.
                if let Some(waiting) = data.first_answers.remove(&job_id) {
                    let _ = waiting.send(FirstAnswer::Accepted);
                }
            }
            if let Err(error) = progress_state.note_job_progress(job_id) {
                tracing::warn!("could not record download progress: {error}");
            }
        };

        // The download runs as a task of its own, so a panic inside it arrives here as an
        // error instead of ending this task before it has updated the job and freed the slot.
        let download_request = request.clone();
        let download = tauri::async_runtime::spawn(async move {
            if download_request.adapter_kind().uses_external_backend() {
                run_external_provider_download(
                    &download_request,
                    &target_dir,
                    job_id,
                    on_progress,
                    cancellation,
                )
                .await
            } else {
                run_http_download(
                    &download_request,
                    &target_dir,
                    &temp_path,
                    on_progress,
                    cancellation,
                )
                .await
            }
        });
        let result = download_result(download.await);

        // The job, when it went back to the browser instead of ending up in the history.
        let handed_back = {
            let mut data = task_state.lock();
            // Still there only if no progress was ever reported, that is, no byte arrived.
            let waiting = data.first_answers.remove(&job_id);
            match result {
                Ok(completed) => {
                    tracing::info!(
                        "job {job_id} completed: {} ({} bytes)",
                        completed.filename,
                        completed.downloaded_bytes
                    );
                    data.queue.mark_completed(job_id, completed);
                    None
                }
                Err(CoreError::Canceled) => {
                    tracing::info!("job {job_id} canceled");
                    None
                }
                Err(error) => {
                    let message = sanitize_diagnostic_message(&error.to_string());
                    // Told while the extension still waits for its reply, a refusal gives
                    // the download back to the browser. `send` fails when nobody waits any
                    // more; then this is a failed job like any other.
                    let goes_back = match (waiting, refusal_status(&error)) {
                        (Some(waiting), Some(status)) => {
                            waiting.send(FirstAnswer::Refused(status)).is_ok()
                        }
                        _ => false,
                    };
                    if goes_back {
                        tracing::info!("job {job_id} goes back to the browser: {message}");
                        data.queue.mark_failed(job_id, message, None);
                        data.job_requests.remove(&job_id);
                        data.queue.remove_history_job(job_id)
                    } else {
                        tracing::warn!("job {job_id} failed: {message}");
                        let job = data.queue.job(job_id);
                        let report = build_error_report(
                            &request,
                            &message,
                            error.provider_failure(),
                            job.as_ref(),
                        );
                        data.queue.mark_failed(job_id, message, Some(report));
                        None
                    }
                }
            }
        };
        match handed_back {
            Some(job) => {
                remove_leftover_files(std::slice::from_ref(&job));
                if let Err(error) = task_state.commit_jobs() {
                    tracing::warn!(
                        "could not persist download state after handing a download back: {error}"
                    );
                }
            }
            None => {
                commit_job_or_warn(&task_state, job_id, "finishing a download");
                prune_history(&task_state);
            }
        }

        task_state.lock().active_tasks.remove(&job_id);
        schedule_pending_jobs(&task_state);
    });

    data.active_tasks.insert(
        job_id,
        crate::app_state::ActiveDownloadTask {
            handle,
            cancel: Some(cancel_handle),
        },
    );

    Ok(())
}

/// What a download task produced. A task that panicked counts as a failed download: the job
/// leaves "running" and its slot is free again, instead of both being stuck until LDM restarts.
fn download_result(
    joined: tauri::Result<Result<CompletedDownload, CoreError>>,
) -> Result<CompletedDownload, CoreError> {
    joined.unwrap_or_else(|error| {
        Err(CoreError::Io(std::io::Error::other(format!(
            "the download task stopped unexpectedly: {error}"
        ))))
    })
}

fn commit_job_or_warn(state: &AppState, job_id: Uuid, context: &str) {
    if let Err(error) = state.commit_job(job_id) {
        tracing::warn!("could not persist download state after {context}: {error}");
    }
}

/// Stops the job's download task, if one runs, and waits until it is gone.
async fn abort_active_task(state: &AppState, job_id: Uuid) {
    let task = state.lock().active_tasks.remove(&job_id);
    if let Some(task) = task {
        if let Some(cancel) = task.cancel {
            cancel.cancel();
        } else {
            task.handle.abort();
        }
        let _ = task.handle.await;
    }
}

fn stored_job_request(
    state: &AppState,
    job_id: Uuid,
) -> Result<CreateDownloadJobRequest, AppError> {
    state
        .lock()
        .job_requests
        .get(&job_id)
        .cloned()
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::JobNotFound,
                "download request is unavailable for this job",
            )
        })
}

fn is_active_task(state: &AppState, job_id: Uuid) -> bool {
    state.lock().active_tasks.contains_key(&job_id)
}

fn ensure_pause_resume_control(request: &CreateDownloadJobRequest) -> Result<(), AppError> {
    if request.adapter_kind().supports_pause_resume() {
        return Ok(());
    }

    Err(AppError::invalid_job_state(
        "only direct downloads and native HLS streams can be paused or resumed safely yet",
    ))
}

fn ensure_status(
    status: DownloadStatus,
    allowed: &[DownloadStatus],
    message: &str,
) -> Result<(), AppError> {
    if allowed.contains(&status) {
        Ok(())
    } else {
        Err(AppError::invalid_job_state(message))
    }
}

fn is_terminal_status(status: DownloadStatus) -> bool {
    matches!(
        status,
        DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Canceled
    )
}

fn current_job(state: &AppState, job_id: Uuid) -> Result<DownloadJob, AppError> {
    state
        .lock()
        .queue
        .job(job_id)
        .ok_or_else(AppError::job_not_found)
}

fn completed_job(state: &AppState, job_id: Uuid) -> Result<DownloadJob, AppError> {
    let job = current_job(state, job_id)?;
    if job.status != DownloadStatus::Completed {
        return Err(AppError::invalid_job_state(
            "only completed downloads can be opened",
        ));
    }

    Ok(job)
}

fn open_path(path: std::path::PathBuf) -> Result<(), AppError> {
    if !path.exists() {
        return Err(AppError::file_not_found("downloaded file was not found"));
    }

    open_with_system(&path)
}

fn show_path_in_folder(
    file_path: std::path::PathBuf,
    folder_path: std::path::PathBuf,
) -> Result<(), AppError> {
    if !folder_path.exists() {
        return Err(AppError::file_not_found("download folder was not found"));
    }

    #[cfg(target_os = "windows")]
    {
        if file_path.exists() {
            return std::process::Command::new("explorer.exe")
                .args(explorer_select_args(&file_path))
                .spawn()
                .map(|_| ())
                .map_err(|error| AppError::internal(error.to_string()));
        }

        std::process::Command::new("explorer.exe")
            .arg(folder_path)
            .spawn()
            .map(|_| ())
            .map_err(|error| AppError::internal(error.to_string()))
    }

    #[cfg(not(target_os = "windows"))]
    {
        open_with_system(&folder_path)
    }
}

#[cfg(target_os = "windows")]
fn explorer_select_args(file_path: &std::path::Path) -> [std::ffi::OsString; 2] {
    [
        std::ffi::OsString::from("/select,"),
        file_path.as_os_str().to_os_string(),
    ]
}

fn open_with_system(path: &std::path::Path) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer.exe");
        command.arg(path);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| AppError::internal(error.to_string()))
}

fn cleanup_job_temp_files(state: &AppState, job_id: Uuid) -> Result<(), AppError> {
    cleanup_temp_files_of(&current_job(state, job_id)?)
}

/// Removes what a download of `job` left in the partials folder.
fn cleanup_temp_files_of(job: &DownloadJob) -> Result<(), AppError> {
    let job_id = job.id;
    if job.adapter_kind().uses_external_backend() {
        let target_dir = std::path::PathBuf::from(&job.target_directory);
        cleanup_provider_job_artifacts(&target_dir, job_id)?;
    }

    if job.temp_path.trim().is_empty() {
        return Ok(());
    }

    let temp_path = std::path::PathBuf::from(&job.temp_path);
    remove_partial_download(&temp_path).map_err(|error| AppError::storage(error.to_string()))?;

    // Versions before the partials folder kept the file next to the download.
    if let Some(filename) = temp_path.file_name() {
        let legacy_temp_path = std::path::PathBuf::from(&job.target_directory).join(filename);
        if legacy_temp_path != temp_path {
            remove_partial_download(&legacy_temp_path)
                .map_err(|error| AppError::storage(error.to_string()))?;
        }
    }

    Ok(())
}

fn parse_job_id(value: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value).map_err(|_| AppError::invalid_request("download job id is invalid"))
}

/// The JSON behind the "Report" button of a failed job. `error` is the message the queue shows;
/// `failure` adds the details of a failed yt-dlp run, when that is what failed; `job` adds how
/// old the job was when it failed, which together with `source.hasQuery` tells an expired
/// signed link (old job) from one bound to another IP address (fresh job, query string).
fn build_error_report(
    request: &CreateDownloadJobRequest,
    error: &str,
    failure: Option<&ProviderFailure>,
    job: Option<&DownloadJob>,
) -> String {
    let mut header_names = request
        .headers
        .as_ref()
        .map(|headers| headers.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    header_names.sort();

    json!({
        "version": 2,
        "error": error,
        "backendErrorLine": failure.and_then(|failure| failure.backend_error_line.as_deref()),
        "nativeHlsError": failure.and_then(|failure| failure.native_hls_error.as_deref()),
        "pageUrlFallbackError": failure.and_then(|failure| failure.page_url_fallback_error.as_deref()),
        "adapterId": request.adapter_id,
        "kind": request.kind,
        "source": summarize_url(&request.source_url),
        "page": request.page_url.as_ref().map(|url| summarize_url(url)),
        "referrerPresent": request.referrer.as_ref().is_some_and(|value| !value.trim().is_empty()),
        "contentType": request.content_type,
        "contentLength": request.content_length,
        "preferRangeRequest": request.prefer_range_request,
        "forwardedHeaderNames": header_names,
        "fallbackUrlCount": request.source_fallback_urls.len(),
        "timing": job.map(|job| {
            let failed_at = unix_seconds_now();
            json!({
                "createdAt": job.created_at,
                "failedAt": failed_at,
                "jobAgeSeconds": failed_at.saturating_sub(job.created_at),
                "retryCount": job.retry_count,
            })
        }),
        "qualityReport": parse_json_report(request.quality_report.as_deref()),
        "note": "Report omits query strings, header values, cookies, authorization, and local file paths."
    })
    .to_string()
}

fn parse_json_report(value: Option<&str>) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::Value::Null;
    };

    serde_json::from_str(value).unwrap_or_else(|_| json!({ "parseError": true }))
}

fn summarize_url(value: &str) -> serde_json::Value {
    let without_fragment = value.split('#').next().unwrap_or(value);
    let (without_query, has_query) = match without_fragment.split_once('?') {
        Some((url, _)) => (url, true),
        None => (without_fragment, false),
    };
    let after_scheme = without_query
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(without_query);
    let (host, path) = match after_scheme.split_once('/') {
        Some((host, path)) => (host, path),
        None => (after_scheme, ""),
    };
    let host = host.rsplit_once('@').map(|(_, host)| host).unwrap_or(host);
    let filename = path
        .split('/')
        .rfind(|segment| !segment.is_empty())
        .unwrap_or("");

    json!({
        "host": host,
        "filename": filename,
        "hasQuery": has_query,
    })
}

#[cfg(test)]
mod tests {
    use ldm_core::error::ProviderFailure;
    use ldm_types::download_job::{CreateDownloadJobRequest, DownloadKind, DownloadStatus};
    use serde_json::Value;

    use crate::app_state::{AppState, FIRST_RUN_SETUP_MESSAGE};
    use crate::error::ErrorCode;

    #[cfg(target_os = "windows")]
    use super::explorer_select_args;
    use super::{
        browser_can_take_back, build_error_report, download_result, next_job_to_start,
        prune_history, refusal_status, start_download_job, start_job_from_extension,
        FIRST_ANSWER_WAIT, HISTORY_LIMIT,
    };

    #[test]
    fn new_downloads_require_legal_acceptance() {
        let state = AppState::default();
        let error = start_download_job(&state, valid_request()).unwrap_err();

        assert_eq!(error.code, ErrorCode::FirstRunSetupRequired);
        assert_eq!(error.message, FIRST_RUN_SETUP_MESSAGE);
    }

    #[test]
    fn scheduler_starts_oldest_queued_job_until_the_limit_is_reached() {
        let state = AppState::default();
        state
            .set_download_settings(ldm_types::download_settings::DownloadSettings {
                max_concurrent_downloads: 2,
                ..Default::default()
            })
            .unwrap();
        let target_dir = std::env::temp_dir().join("ldm-scheduler-test");
        let ids = {
            let mut data = state.lock();
            (0..3)
                .map(|_| data.queue.enqueue(valid_request(), &target_dir).unwrap().id)
                .collect::<Vec<_>>()
        };

        assert_eq!(next_job_to_start(&state), Some(ids[0]));

        for id in &ids[..2] {
            state.lock().active_tasks.insert(
                *id,
                crate::app_state::ActiveDownloadTask {
                    handle: tauri::async_runtime::spawn(async {}),
                    cancel: None,
                },
            );
        }
        assert_eq!(next_job_to_start(&state), None);

        state.lock().active_tasks.remove(&ids[1]);
        assert_eq!(next_job_to_start(&state), Some(ids[1]));

        state.lock().queue.mark_paused(ids[1]);
        assert_eq!(next_job_to_start(&state), Some(ids[2]));
    }

    #[test]
    fn a_download_task_that_panics_is_reported_as_a_failed_download() {
        let joined = tauri::async_runtime::block_on(tauri::async_runtime::spawn(async {
            if std::hint::black_box(true) {
                panic!("simulated bug inside a download");
            }
            Err(ldm_core::error::CoreError::Canceled)
        }));

        let error = download_result(joined).unwrap_err();

        assert!(
            error.to_string().contains("stopped unexpectedly"),
            "{error}"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn explorer_select_keeps_file_path_as_separate_argument() {
        let path = std::path::Path::new(r"C:\Downloads\Video\Owned Video.mp4");

        let args = explorer_select_args(path);

        assert_eq!(args[0], "/select,");
        assert_eq!(args[1], path.as_os_str());
    }

    #[test]
    fn error_report_carries_the_backend_line_and_the_native_hls_reason() {
        let request = CreateDownloadJobRequest {
            adapter_id: "hls".to_string(),
            source_url: "https://hv-h.cdn.example.invalid/videos/master.m3u8?sig=visible"
                .to_string(),
            ..valid_request()
        };
        let failure = ProviderFailure {
            message: "The stream link was rejected (HTTP 404).".to_string(),
            backend_error_line: Some(
                "ERROR: master.m3u8: Unable to download webpage: HTTP Error 404: Not Found"
                    .to_string(),
            ),
            native_hls_error: Some(
                "server returned unsuccessful status: 404 Not Found".to_string(),
            ),
            page_url_fallback_error: Some(
                "ERROR: [generic] Unsupported URL: https://videos.example.invalid/view_video.php?viewkey=visible".to_string(),
            ),
        };

        let mut queue = ldm_core::queue::DownloadQueue::default();
        let mut job = queue
            .enqueue(
                request.clone(),
                &std::env::temp_dir().join("ldm-report-test"),
            )
            .unwrap();
        job.created_at -= 90;
        job.retry_count = 1;

        let report: Value = serde_json::from_str(&build_error_report(
            &request,
            &failure.message,
            Some(&failure),
            Some(&job),
        ))
        .unwrap();

        assert_eq!(report["error"], "The stream link was rejected (HTTP 404).");
        assert_eq!(
            report["backendErrorLine"],
            "ERROR: master.m3u8: Unable to download webpage: HTTP Error 404: Not Found"
        );
        assert_eq!(
            report["nativeHlsError"],
            "server returned unsuccessful status: 404 Not Found"
        );
        assert_eq!(
            report["pageUrlFallbackError"],
            "ERROR: [generic] Unsupported URL: https://videos.example.invalid/view_video.php?viewkey=visible"
        );
        assert_eq!(report["adapterId"], "hls");
        assert_eq!(report["source"]["hasQuery"], true);
        assert!(!report["source"].to_string().contains("visible"));
        let age = report["timing"]["jobAgeSeconds"].as_u64().unwrap();
        assert!((90..100).contains(&age), "{age}");
        assert_eq!(report["timing"]["createdAt"], job.created_at);
        assert_eq!(report["timing"]["retryCount"], 1);
        assert!(report["timing"]["failedAt"].as_u64().unwrap() >= job.created_at + 90);

        let report: Value = serde_json::from_str(&build_error_report(
            &request,
            "file operation failed",
            None,
            None,
        ))
        .unwrap();

        assert!(report["backendErrorLine"].is_null());
        assert!(report["nativeHlsError"].is_null());
        assert!(report["pageUrlFallbackError"].is_null());
        assert!(report["timing"].is_null());
    }

    #[test]
    fn the_history_is_cut_to_its_limit_and_takes_requests_and_partial_files_along() {
        let target = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let ids = {
            let mut data = state.lock();
            (0..HISTORY_LIMIT + 2)
                .map(|_| {
                    let job = data.queue.enqueue(valid_request(), target.path()).unwrap();
                    data.job_requests.insert(job.id, valid_request());
                    data.queue
                        .mark_failed(job.id, "connection lost".to_string(), None);
                    job
                })
                .collect::<Vec<_>>()
        };
        // What a failed download keeps for a resume.
        for job in &ids[..3] {
            std::fs::create_dir_all(std::path::Path::new(&job.temp_path).parent().unwrap())
                .unwrap();
            std::fs::write(&job.temp_path, b"partial").unwrap();
        }
        let waiting = state
            .lock()
            .queue
            .enqueue(valid_request(), target.path())
            .unwrap();

        prune_history(&state);

        let data = state.lock();
        assert_eq!(data.queue.history_jobs().len(), HISTORY_LIMIT);
        assert_eq!(data.queue.jobs().len(), HISTORY_LIMIT + 1);
        assert!(data.queue.job(waiting.id).is_some());
        for job in &ids[..2] {
            assert!(data.queue.job(job.id).is_none());
            assert!(!data.job_requests.contains_key(&job.id));
            assert!(!std::path::Path::new(&job.temp_path).exists());
        }
        // The third one is within the limit: nothing of it is touched.
        assert!(data.queue.job(ids[2].id).is_some());
        assert!(data.job_requests.contains_key(&ids[2].id));
        assert!(std::path::Path::new(&ids[2].temp_path).exists());
    }

    #[test]
    fn a_history_within_its_limit_is_left_alone() {
        let state = AppState::default();
        let job = {
            let mut data = state.lock();
            let job = data
                .queue
                .enqueue(
                    valid_request(),
                    &std::env::temp_dir().join("ldm-prune-test"),
                )
                .unwrap();
            data.queue.mark_canceled(job.id);
            job
        };

        prune_history(&state);

        assert!(state.lock().queue.job(job.id).is_some());
    }

    /// A state that may download and puts programs into `dir`.
    fn state_downloading_into(dir: &std::path::Path) -> AppState {
        let state = AppState::default();
        state.accept_current_legal_terms().unwrap();
        state
            .set_download_settings(ldm_types::download_settings::DownloadSettings {
                target_folders: std::collections::BTreeMap::from([(
                    DownloadKind::Program,
                    dir.to_string_lossy().to_string(),
                )]),
                ..Default::default()
            })
            .unwrap();
        state
    }

    /// What the extension sends for a download it took over from the browser.
    fn browser_download(url: String) -> CreateDownloadJobRequest {
        CreateDownloadJobRequest {
            source_url: url,
            suggested_filename: Some("setup.exe".to_string()),
            kind: DownloadKind::Program,
            adapter_id: "browser-download".to_string(),
            ..valid_request()
        }
    }

    async fn serve(route: &str, response: wiremock::ResponseTemplate) -> wiremock::MockServer {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(route))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn a_download_the_server_refuses_goes_back_to_the_browser() {
        // What a file behind a login looks like to LDM: the server acts as if it were not there.
        let server = serve("/draft/setup.exe", wiremock::ResponseTemplate::new(404)).await;
        let target = tempfile::tempdir().unwrap();
        let state = state_downloading_into(target.path());

        let error = start_job_from_extension(
            &state,
            browser_download(format!("{}/draft/setup.exe", server.uri())),
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::SourceRefused);
        assert!(error.message.contains("HTTP 404"), "{}", error.message);
        // Nothing of the job stays behind: no failed entry, no stored request, no file.
        let data = state.lock();
        assert!(data.queue.jobs().is_empty());
        assert!(data.job_requests.is_empty());
        assert!(data.first_answers.is_empty());
        drop(data);
        assert!(!target.path().join("setup.exe").exists());
    }

    #[tokio::test]
    async fn a_download_the_server_serves_is_accepted_at_its_first_answer() {
        let server = serve(
            "/public/setup.exe",
            wiremock::ResponseTemplate::new(200).set_body_bytes(b"installer bytes".to_vec()),
        )
        .await;
        let target = tempfile::tempdir().unwrap();
        let state = state_downloading_into(target.path());
        let started = std::time::Instant::now();

        let job = start_job_from_extension(
            &state,
            browser_download(format!("{}/public/setup.exe", server.uri())),
        )
        .await
        .unwrap();

        // Accepted with the answer, not after the full wait.
        assert!(
            started.elapsed() < FIRST_ANSWER_WAIT,
            "{:?}",
            started.elapsed()
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while state.lock().queue.job(job.id).map(|job| job.status)
            != Some(DownloadStatus::Completed)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the download did not finish"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            std::fs::read(target.path().join("setup.exe")).unwrap(),
            b"installer bytes"
        );
        assert!(state.lock().first_answers.is_empty());
    }

    #[tokio::test]
    async fn a_job_that_waits_for_a_free_slot_is_accepted_without_waiting() {
        let target = tempfile::tempdir().unwrap();
        let state = state_downloading_into(target.path());
        let mut settings = state.download_settings();
        settings.max_concurrent_downloads = 1;
        state.set_download_settings(settings).unwrap();
        // The only slot is taken.
        state.lock().active_tasks.insert(
            uuid::Uuid::new_v4(),
            crate::app_state::ActiveDownloadTask {
                handle: tauri::async_runtime::spawn(async {}),
                cancel: None,
            },
        );
        let started = std::time::Instant::now();

        let job = start_job_from_extension(
            &state,
            browser_download("https://example.invalid/setup.exe".to_string()),
        )
        .await
        .unwrap();

        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        let data = state.lock();
        assert_eq!(
            data.queue.job(job.id).unwrap().status,
            DownloadStatus::Queued
        );
        assert!(data.first_answers.is_empty());
    }

    #[test]
    fn only_what_the_browser_can_download_itself_is_given_back() {
        for (adapter_id, expected) in [
            ("browser-download", true),
            ("generic-download-links", true),
            ("generic-html5-video", false),
            ("hls", false),
            ("provider-youtube", false),
        ] {
            let request = CreateDownloadJobRequest {
                adapter_id: adapter_id.to_string(),
                ..valid_request()
            };
            assert_eq!(browser_can_take_back(&request), expected, "{adapter_id}");
        }
    }

    #[test]
    fn a_refusal_is_a_client_error_that_another_attempt_would_not_change() {
        use ldm_core::error::CoreError;
        use reqwest::StatusCode;

        for (status, expected) in [
            (StatusCode::UNAUTHORIZED, Some(401)),
            (StatusCode::FORBIDDEN, Some(403)),
            (StatusCode::NOT_FOUND, Some(404)),
            (StatusCode::GONE, Some(410)),
            (StatusCode::REQUEST_TIMEOUT, None),
            (StatusCode::TOO_MANY_REQUESTS, None),
            (StatusCode::INTERNAL_SERVER_ERROR, None),
            (StatusCode::SERVICE_UNAVAILABLE, None),
        ] {
            assert_eq!(
                refusal_status(&CoreError::HttpStatus(status)),
                expected,
                "{status}"
            );
        }
        assert_eq!(refusal_status(&CoreError::Canceled), None);
        assert_eq!(refusal_status(&CoreError::LiveStream), None);
    }

    fn valid_request() -> CreateDownloadJobRequest {
        CreateDownloadJobRequest {
            source_url: "https://example.invalid/video.mp4".to_string(),
            source_fallback_urls: Vec::new(),
            page_url: Some("https://example.invalid/page".to_string()),
            page_title: Some("Video".to_string()),
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
