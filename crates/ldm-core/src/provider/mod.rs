//! Downloads that go through an external tool: provider pages (YouTube, X, TikTok) and
//! HLS/DASH manifests the native HLS path cannot take. yt-dlp is the only implemented engine.
//!
//! - `common`: naming and request helpers shared with the native HLS path
//! - `ytdlp_command`: backend discovery and the yt-dlp command line
//! - `process`: running the process and collecting its output
//! - `progress`: the progress lines yt-dlp is asked for, and what they add up to
//! - `diagnostics`: user-facing failure messages and the failure log
//! - `page_fallback`: the page URL as the last attempt for a rejected stream link
//! - `artifacts`: output files, thumbnails, cleanup

pub(crate) mod common;

mod artifacts;
mod diagnostics;
mod page_fallback;
mod process;
mod progress;
#[cfg(test)]
mod test_support;
mod ytdlp_command;

use std::{
    path::{Path, PathBuf},
    process::Output,
};

use ldm_types::download_job::CreateDownloadJobRequest;
use uuid::Uuid;

pub use artifacts::cleanup_provider_job_artifacts;
pub use ytdlp_command::ProviderQualityPreference;

use crate::{
    diagnostics::sanitize_diagnostic_message,
    download::{build_download_plan, CompletedDownload, DownloadCancellation, DownloadProgress},
    error::{CoreError, ProviderFailure},
    filename::{ensure_partials_dir_async, move_to_unique_path},
    hls::{hls_request_quality_score, native, resolve_hls_request},
};

use artifacts::{
    cleanup_provider_temp_artifacts, cleanup_provider_temp_artifacts_async, finalize_thumbnails,
    find_provider_artifacts,
};
use common::{
    filename_with_extension, is_stream_request, is_youtube_request, provider_temp_stem,
    requires_stream_merge_tool,
};
use diagnostics::{
    log_provider_backend_output, looks_like_youtube_bot_challenge, looks_like_youtube_client_error,
    looks_like_youtube_login_wall, process_output_text, provider_backend_failure,
    provider_output_summary,
};
use page_fallback::{page_url_fallback_request, with_page_url_fallback_report};
use process::run_provider_process;
use ytdlp_command::{
    build_ytdlp_compatible_command, external_provider_backend, ExternalProviderBackend,
    YtDlpAttempt, DEFAULT_YTDLP_ATTEMPT, PROVIDER_ENGINE_ENV, YOUTUBE_COMPAT_ATTEMPT,
    YOUTUBE_EMBEDDED_ATTEMPT,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ProviderBackendKind {
    YtDlp,
    Native,
}

impl ProviderBackendKind {
    fn from_env() -> Self {
        std::env::var(PROVIDER_ENGINE_ENV)
            .ok()
            .as_deref()
            .and_then(Self::parse)
            .unwrap_or(Self::YtDlp)
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "yt-dlp" | "ytdlp" | "external" => Some(Self::YtDlp),
            "native" | "ldm-native" | "ldm" => Some(Self::Native),
            _ => None,
        }
    }
}

pub async fn run_external_provider_download(
    request: &CreateDownloadJobRequest,
    target_dir: &Path,
    job_id: Uuid,
    mut on_progress: impl FnMut(DownloadProgress),
    mut cancellation: DownloadCancellation,
) -> Result<CompletedDownload, CoreError> {
    on_progress(DownloadProgress::byte_progress(
        0,
        None,
        request.adapter_kind().is_hls(),
    ));

    let mut resolved_attempts = Vec::new();
    for attempt in provider_download_attempts(request) {
        let resolved = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(CoreError::Canceled),
            resolved = resolve_hls_request(&attempt) => resolved,
        };
        match resolved {
            Ok(resolved) => resolved_attempts.push(resolved),
            Err(_) if is_stream_request(&attempt) => resolved_attempts.push(attempt),
            Err(error) => return Err(error),
        }
    }
    resolved_attempts.sort_by_key(|request| std::cmp::Reverse(hls_request_quality_score(request)));
    resolved_attempts.dedup_by(|left, right| left.source_url == right.source_url);

    // Why the native HLS path gave up, kept for the error report of the yt-dlp run that follows.
    let mut native_hls_error = None;
    if native::can_run_native_hls(&resolved_attempts) {
        match native::run_native_hls_download(
            &resolved_attempts,
            target_dir,
            job_id,
            &mut on_progress,
            cancellation.clone(),
        )
        .await
        {
            Ok(completed) => return Ok(completed),
            // A live stream stays a failure: yt-dlp would record it until the broadcast ends.
            Err(error @ (CoreError::Canceled | CoreError::LiveStream)) => return Err(error),
            Err(error) => {
                let message = sanitize_diagnostic_message(&error.to_string());
                tracing::warn!("job {job_id} native HLS download failed, trying yt-dlp: {message}");
                native_hls_error = Some(message);
            }
        }
    }

    run_ytdlp_download(
        &resolved_attempts,
        target_dir,
        job_id,
        &mut on_progress,
        cancellation,
    )
    .await
    .map_err(|error| error.with_native_hls_error(native_hls_error))
}

/// Downloads the first of `requests` that yt-dlp gets through, then moves its output into the
/// target folder.
async fn run_ytdlp_download(
    requests: &[CreateDownloadJobRequest],
    target_dir: &Path,
    job_id: Uuid,
    on_progress: &mut impl FnMut(DownloadProgress),
    cancellation: DownloadCancellation,
) -> Result<CompletedDownload, CoreError> {
    let request = requests.first().ok_or_else(|| {
        CoreError::ProviderBackend("provider backend has no download request".to_string())
    })?;
    let plan = build_download_plan(request, target_dir)?;
    tokio::fs::create_dir_all(&plan.target_dir).await?;

    if selected_provider_backend(request) == ProviderBackendKind::Native {
        return run_native_provider_download(request);
    }

    // Looking for the tools walks through folders and PATH.
    let backend = blocking(external_provider_backend).await?;
    if requires_stream_merge_tool(request) && backend.ffmpeg.is_none() {
        return Err(CoreError::ProviderBackend(
            "FFmpeg is required to download and merge this stream into a playable video file. Add bundled FFmpeg, set LDM_FFMPEG_EXE, or make FFmpeg available on PATH.".to_string(),
        ));
    }

    let temp_stem = provider_temp_stem(job_id);
    let work_dir = ensure_partials_dir_async(&plan.target_dir).await?;
    let output_template = work_dir.join(format!("{temp_stem}.%(ext)s"));

    let run = ProviderRun {
        backend: &backend,
        output_template: &output_template,
        work_dir: &work_dir,
        temp_stem: &temp_stem,
    };

    // The request whose yt-dlp run produced the output files: the stream URL, one of its
    // fallbacks, or the page URL.
    let mut completed_request = None;
    for (index, request) in requests.iter().enumerate() {
        if index > 0 {
            run.cleanup().await?;
        }

        let output = run
            .run_with_youtube_retries(request, on_progress, &cancellation)
            .await?;
        if output.status.success() {
            completed_request = Some(request.clone());
            break;
        }

        if should_try_next_stream_fallback(index, requests.len(), request) {
            continue;
        }

        log_provider_backend_output(job_id, &output);
        let output_text = process_output_text(&output);
        let mut failure = ProviderFailure {
            message: provider_backend_failure(&output, backend.quality, request),
            backend_error_line: Some(provider_output_summary(&output_text)),
            ..ProviderFailure::default()
        };

        // Last attempt for a stream whose captured link was rejected: let yt-dlp fetch the page
        // and sign its own stream URL. The message stays the one for the rejected link; the
        // report gets both errors when this fails too.
        let Some(page_request) = page_url_fallback_request(request, &output_text) else {
            return Err(CoreError::ProviderFailure(failure));
        };
        tracing::info!("job {job_id} stream link failed, trying yt-dlp with the page URL");
        run.cleanup().await?;
        let page_output = run
            .run(
                &page_request,
                DEFAULT_YTDLP_ATTEMPT,
                on_progress,
                &cancellation,
            )
            .await?;
        if page_output.status.success() {
            completed_request = Some(CreateDownloadJobRequest {
                quality_report: Some(with_page_url_fallback_report(
                    page_request.quality_report.as_deref(),
                )),
                ..page_request
            });
            break;
        }

        log_provider_backend_output(job_id, &page_output);
        failure.page_url_fallback_error =
            Some(provider_output_summary(&process_output_text(&page_output)));
        return Err(CoreError::ProviderFailure(failure));
    }
    let completed_request = completed_request.unwrap_or_else(|| request.clone());

    let output = {
        let backend = backend.clone();
        let target_dir = plan.target_dir.clone();
        let filename = plan.filename.clone();
        blocking(move || {
            move_provider_output(&backend, &work_dir, &temp_stem, &target_dir, &filename)
        })
        .await?
    };

    Ok(CompletedDownload {
        final_path: output.final_path,
        final_url: Some(completed_request.source_url),
        quality_report: completed_request.quality_report,
        filename: output.filename,
        downloaded_bytes: output.bytes,
        total_bytes: Some(output.bytes),
        supports_resume: false,
    })
}

/// The media file of a finished yt-dlp run at its final place.
struct ProviderOutput {
    final_path: PathBuf,
    filename: String,
    bytes: u64,
}

/// Finds what yt-dlp wrote for `temp_stem`, moves the media file into `target_dir` under a free
/// name based on `filename`, and embeds the thumbnail. Blocking: it moves files and runs FFmpeg.
fn move_provider_output(
    backend: &ExternalProviderBackend,
    work_dir: &Path,
    temp_stem: &str,
    target_dir: &Path,
    filename: &str,
) -> Result<ProviderOutput, CoreError> {
    let artifacts = match find_provider_artifacts(work_dir, temp_stem) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            let _ = cleanup_provider_temp_artifacts(work_dir, temp_stem);
            return Err(error);
        }
    };
    let extension = artifacts
        .media_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mp4");
    let final_filename = filename_with_extension(filename, extension);
    let final_path = move_to_unique_path(&artifacts.media_path, target_dir, &final_filename)?;
    finalize_thumbnails(backend, &artifacts.thumbnail_paths, &final_path)?;
    let bytes = final_path.metadata()?.len();

    let filename = final_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&final_filename)
        .to_string();

    Ok(ProviderOutput {
        final_path,
        filename,
        bytes,
    })
}

/// Runs work that blocks (file system walks, FFmpeg) on a thread meant for it.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, CoreError> + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| CoreError::ProviderBackend(error.to_string()))?
}

/// Everything one yt-dlp run needs besides the request: the backend, the output template, and
/// the partials folder the run writes to.
struct ProviderRun<'a> {
    backend: &'a ExternalProviderBackend,
    output_template: &'a Path,
    work_dir: &'a Path,
    temp_stem: &'a str,
}

impl ProviderRun<'_> {
    async fn run(
        &self,
        request: &CreateDownloadJobRequest,
        attempt: YtDlpAttempt,
        on_progress: &mut impl FnMut(DownloadProgress),
        cancellation: &DownloadCancellation,
    ) -> Result<Output, CoreError> {
        run_provider_process(
            build_ytdlp_compatible_command(self.backend, self.output_template, request, attempt),
            self.backend,
            on_progress,
            self.work_dir,
            self.temp_stem,
            cancellation.clone(),
            is_stream_request(request),
        )
        .await
    }

    /// yt-dlp's default attempt, then the YouTube retries it calls for.
    async fn run_with_youtube_retries(
        &self,
        request: &CreateDownloadJobRequest,
        on_progress: &mut impl FnMut(DownloadProgress),
        cancellation: &DownloadCancellation,
    ) -> Result<Output, CoreError> {
        let mut output = self
            .run(request, DEFAULT_YTDLP_ATTEMPT, on_progress, cancellation)
            .await?;

        if !output.status.success()
            && should_retry_with_youtube_compat_clients(request, &output, self.backend)
        {
            output = self
                .run(request, YOUTUBE_COMPAT_ATTEMPT, on_progress, cancellation)
                .await?;
        }

        if !output.status.success()
            && should_retry_with_youtube_embedded_client(request, &output, self.backend)
        {
            output = self
                .run(request, YOUTUBE_EMBEDDED_ATTEMPT, on_progress, cancellation)
                .await?;
        }

        Ok(output)
    }

    /// Removes the output files of the previous run before the next one writes its own.
    async fn cleanup(&self) -> Result<(), CoreError> {
        cleanup_provider_temp_artifacts_async(self.work_dir, self.temp_stem).await
    }
}

fn provider_download_attempts(request: &CreateDownloadJobRequest) -> Vec<CreateDownloadJobRequest> {
    let mut attempts = vec![request.clone()];
    for fallback_url in &request.source_fallback_urls {
        if attempts
            .iter()
            .any(|attempt| attempt.source_url == *fallback_url)
        {
            continue;
        }

        let mut fallback = request.clone();
        fallback.source_url = fallback_url.clone();
        fallback.source_fallback_urls.clear();
        attempts.push(fallback);
    }

    attempts
}

fn selected_provider_backend(request: &CreateDownloadJobRequest) -> ProviderBackendKind {
    if request.adapter_kind().is_provider() {
        ProviderBackendKind::from_env()
    } else {
        ProviderBackendKind::YtDlp
    }
}

fn run_native_provider_download(
    request: &CreateDownloadJobRequest,
) -> Result<CompletedDownload, CoreError> {
    match request.provider_id.as_deref() {
        Some("youtube") => Err(CoreError::ProviderBackend(
            "Native YouTube provider backend is selected but not implemented yet. Set LDM_PROVIDER_ENGINE=yt-dlp to use the external backend while the native extractor pipeline is being built.".to_string(),
        )),
        Some(provider_id) => Err(CoreError::ProviderBackend(format!(
            "Native provider backend is selected but {provider_id} is not implemented yet. Set {PROVIDER_ENGINE_ENV}=yt-dlp to use the external backend."
        ))),
        None => Err(CoreError::ProviderBackend(format!(
            "Native provider backend is selected but this job has no provider id. Set {PROVIDER_ENGINE_ENV}=yt-dlp to use the external backend."
        ))),
    }
}

/// Second YouTube attempt: browser-like clients after a client error. A bot challenge skips it,
/// because those clients get the same sign-in wall; the embedded client below handles that case.
fn should_retry_with_youtube_compat_clients(
    request: &CreateDownloadJobRequest,
    output: &Output,
    backend: &ExternalProviderBackend,
) -> bool {
    is_youtube_request(request)
        && backend.extractor_args.is_none()
        && !looks_like_youtube_bot_challenge(&process_output_text(output))
        && looks_like_youtube_client_error(&process_output_text(output))
}

/// Third YouTube attempt: the embedded-player client after a bot challenge or a client error from
/// whichever attempt ran last. Any other failure (network, FFmpeg, unknown) is reported as is.
/// The `tv_embedded` client gets past YouTube's sign-in wall without cookies (for embeddable
/// videos). It is tried after a bot challenge, after the login-wall symptoms yt-dlp prints when
/// the final `ERROR:` line is missing, and after a client error; never when the user pinned
/// extractor args.
fn should_retry_with_youtube_embedded_client(
    request: &CreateDownloadJobRequest,
    output: &Output,
    backend: &ExternalProviderBackend,
) -> bool {
    if !is_youtube_request(request) || backend.extractor_args.is_some() {
        return false;
    }

    let output_text = process_output_text(output);
    looks_like_youtube_bot_challenge(&output_text)
        || looks_like_youtube_login_wall(&output_text)
        || looks_like_youtube_client_error(&output_text)
}

fn should_try_next_stream_fallback(
    current_index: usize,
    request_count: usize,
    request: &CreateDownloadJobRequest,
) -> bool {
    current_index + 1 < request_count && is_stream_request(request)
}

#[cfg(test)]
mod tests {
    use super::test_support::{download_request, failed_output};
    use super::*;

    fn backend_with_extractor_args(extractor_args: Option<&str>) -> ExternalProviderBackend {
        ExternalProviderBackend {
            executable: "yt-dlp".to_string(),
            quality: ProviderQualityPreference::BestCompatible,
            quality_overridden: false,
            ffmpeg: None,
            deno: None,
            cookies_file: None,
            proxy: None,
            extractor_args: extractor_args.map(str::to_string),
        }
    }

    const BOT_CHALLENGE_OUTPUT: &str = "[youtube] owned: Player response for client 'web' has playability status: LOGIN_REQUIRED\nERROR: [youtube] owned: Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies for the authentication.";
    const CLIENT_ERROR_OUTPUT: &str =
        "ERROR: [youtube] owned: This video is not available on this app. Watch on the latest version of YouTube.";
    const NETWORK_ERROR_OUTPUT: &str =
        "ERROR: [youtube] owned: Unable to download API page: HTTP Error 503";

    #[test]
    fn compat_clients_retry_only_after_a_client_error() {
        let request = download_request("provider-youtube");
        let backend = backend_with_extractor_args(None);

        assert!(should_retry_with_youtube_compat_clients(
            &request,
            &failed_output(CLIENT_ERROR_OUTPUT),
            &backend
        ));
        assert!(!should_retry_with_youtube_compat_clients(
            &request,
            &failed_output(BOT_CHALLENGE_OUTPUT),
            &backend
        ));
        assert!(!should_retry_with_youtube_compat_clients(
            &request,
            &failed_output(NETWORK_ERROR_OUTPUT),
            &backend
        ));
    }

    #[test]
    fn embedded_client_retries_after_a_bot_challenge_or_a_client_error() {
        let request = download_request("provider-youtube");
        let backend = backend_with_extractor_args(None);

        assert!(should_retry_with_youtube_embedded_client(
            &request,
            &failed_output(BOT_CHALLENGE_OUTPUT),
            &backend
        ));
        assert!(should_retry_with_youtube_embedded_client(
            &request,
            &failed_output(CLIENT_ERROR_OUTPUT),
            &backend
        ));
        assert!(!should_retry_with_youtube_embedded_client(
            &request,
            &failed_output(NETWORK_ERROR_OUTPUT),
            &backend
        ));
    }

    #[test]
    fn embedded_client_retries_when_only_the_login_wall_symptoms_remain() {
        let request = download_request("provider-youtube");
        let backend = backend_with_extractor_args(None);
        let symptoms_only = failed_output(
            "[youtube] owned: Player response for client 'web' has playability status: LOGIN_REQUIRED\nWARNING: [youtube] No title found in player responses; falling back to title from initial data. Other metadata may also be missing",
        );

        assert!(should_retry_with_youtube_embedded_client(
            &request,
            &symptoms_only,
            &backend
        ));
        assert!(!should_retry_with_youtube_compat_clients(
            &request,
            &symptoms_only,
            &backend
        ));
    }

    #[test]
    fn youtube_retries_stay_off_for_other_providers_and_user_extractor_args() {
        let output = failed_output(BOT_CHALLENGE_OUTPUT);

        assert!(!should_retry_with_youtube_embedded_client(
            &download_request("provider-x"),
            &output,
            &backend_with_extractor_args(None)
        ));
        assert!(!should_retry_with_youtube_embedded_client(
            &download_request("provider-youtube"),
            &output,
            &backend_with_extractor_args(Some("youtube:player_client=android"))
        ));
        assert!(!should_retry_with_youtube_compat_clients(
            &download_request("provider-youtube"),
            &failed_output(CLIENT_ERROR_OUTPUT),
            &backend_with_extractor_args(Some("youtube:player_client=android"))
        ));
    }

    #[test]
    fn builds_deduplicated_provider_download_attempts() {
        let mut request = download_request("hls");
        request.source_url = "https://cdn.example.test/master.m3u8".to_string();
        request.source_fallback_urls = vec![
            "https://cdn.example.test/master.m3u8".to_string(),
            "https://cdn.example.test/720/index.m3u8".to_string(),
        ];

        let attempts = provider_download_attempts(&request);

        assert_eq!(attempts.len(), 2);
        assert_eq!(
            attempts[0].source_url,
            "https://cdn.example.test/master.m3u8"
        );
        assert_eq!(
            attempts[1].source_url,
            "https://cdn.example.test/720/index.m3u8"
        );
        assert!(attempts[1].source_fallback_urls.is_empty());
    }

    #[test]
    fn tries_next_stream_fallback_for_any_backend_failure() {
        assert!(should_try_next_stream_fallback(
            0,
            2,
            &download_request("hls")
        ));
        assert!(!should_try_next_stream_fallback(
            1,
            2,
            &download_request("hls")
        ));
        assert!(!should_try_next_stream_fallback(
            0,
            2,
            &download_request("provider-youtube")
        ));
    }

    #[test]
    fn provider_backend_kind_parses_native_engine_without_affecting_direct_streams() {
        assert_eq!(
            ProviderBackendKind::parse("native"),
            Some(ProviderBackendKind::Native)
        );
        assert_eq!(
            ProviderBackendKind::parse("yt-dlp"),
            Some(ProviderBackendKind::YtDlp)
        );
        assert_eq!(ProviderBackendKind::parse("unknown"), None);
        assert_eq!(
            selected_provider_backend(&download_request("generic-html5-video")),
            ProviderBackendKind::YtDlp
        );
    }
}
