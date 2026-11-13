//! Runs one yt-dlp process, collects what it prints, and reports its progress.

use std::{
    path::Path,
    process::{Output, Stdio},
    time::{Duration, Instant},
};

use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
    sync::mpsc,
    time::MissedTickBehavior,
};

use crate::{
    download::{DownloadCancellation, DownloadProgress},
    error::CoreError,
    process_tree::ProcessTree,
    tool_discovery::ToolKind,
};

use super::artifacts::{cleanup_provider_temp_artifacts_async, is_thumbnail_path};
use super::progress::{parse_progress_line, ProviderProgress};
use super::ytdlp_command::{ExternalProviderBackend, PROVIDER_BACKEND_ENV};

/// How often the output files are measured while yt-dlp itself reports nothing.
const PROVIDER_ARTIFACT_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Lines waiting to be handled; a full queue makes the readers, and so yt-dlp, wait.
const PROCESS_LINE_QUEUE: usize = 256;

#[derive(Debug)]
struct ProcessLine {
    stream: ProcessStream,
    line: String,
}

#[derive(Debug, Clone, Copy)]
enum ProcessStream {
    Stdout,
    Stderr,
}

/// Runs `command` until it exits and returns its exit status and output, without the progress
/// lines. A cancel ends the process and everything it started, removes the files of
/// `temp_stem` in `target_dir`, and returns `CoreError::Canceled`.
///
/// `watch_files` is for stream downloads: yt-dlp hands some of them to FFmpeg, which reports no
/// progress, so the growing output files are measured until a first progress line arrives.
pub(super) async fn run_provider_process(
    command: std::process::Command,
    backend: &ExternalProviderBackend,
    on_progress: &mut impl FnMut(DownloadProgress),
    target_dir: &Path,
    temp_stem: &str,
    mut cancellation: DownloadCancellation,
    watch_files: bool,
) -> Result<Output, CoreError> {
    if cancellation.is_cancelled() {
        return Err(CoreError::Canceled);
    }

    // yt-dlp starts FFmpeg and, depending on how it was installed, a Python interpreter. The
    // tree ties all of them to this run: a cancel ends them together, and so does LDM exiting.
    let mut command = Command::from(command);
    let (mut child, tree) = ProcessTree::spawn_async(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .map_err(|error| {
        CoreError::ProviderBackend(format!(
            "could not start provider backend '{}'. Add bundled tools, set {PROVIDER_BACKEND_ENV}, or configure {}. {error}",
            backend.executable,
            ToolKind::YtDlp.env_var()
        ))
    })?;

    let (sender, mut lines) = mpsc::channel(PROCESS_LINE_QUEUE);
    if let Some(stdout) = child.stdout.take() {
        forward_lines(stdout, ProcessStream::Stdout, sender.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        forward_lines(stderr, ProcessStream::Stderr, sender.clone());
    }
    // The readers hold the only senders, so the queue closes when both pipes do.
    drop(sender);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut progress = ProviderProgress::default();
    let mut reports_progress = false;
    let mut observed_bytes = 0;
    let mut artifact_poll = tokio::time::interval(PROVIDER_ARTIFACT_POLL_INTERVAL);
    artifact_poll.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return Err(stop_canceled_process(tree, child, target_dir, temp_stem).await);
            }
            next = lines.recv() => {
                let Some(ProcessLine { stream, line }) = next else {
                    break;
                };
                if let Some(progress_line) = parse_progress_line(&line) {
                    reports_progress = true;
                    if let Some(update) = progress.apply(progress_line, Instant::now()) {
                        on_progress(update);
                    }
                    // Thousands of these per download, and none says why a run failed.
                    continue;
                }

                let collected = match stream {
                    ProcessStream::Stdout => &mut stdout,
                    ProcessStream::Stderr => &mut stderr,
                };
                collected.extend_from_slice(line.as_bytes());
                collected.push(b'\n');
            }
            _ = artifact_poll.tick(), if watch_files && !reports_progress => {
                let measured = measure_provider_temp_artifacts(target_dir, temp_stem).await;
                if measured > observed_bytes {
                    observed_bytes = measured;
                    on_progress(DownloadProgress::byte_progress(measured, None, false));
                }
            }
        }
    }

    // Both pipes are closed; the process itself follows at once unless something it started
    // still holds on to them.
    let status = tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            return Err(stop_canceled_process(tree, child, target_dir, temp_stem).await);
        }
        status = child.wait() => status?,
    };

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Ends the process tree, waits until it is gone so its files are no longer in use, and removes
/// them.
async fn stop_canceled_process(
    tree: ProcessTree,
    mut child: Child,
    target_dir: &Path,
    temp_stem: &str,
) -> CoreError {
    tree.terminate();
    let _ = child.wait().await;
    let _ = cleanup_provider_temp_artifacts_async(target_dir, temp_stem).await;
    CoreError::Canceled
}

/// Sends every line of `stream` to `sender` until the stream ends.
fn forward_lines(
    stream: impl AsyncRead + Unpin + Send + 'static,
    process_stream: ProcessStream,
    sender: mpsc::Sender<ProcessLine>,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            // Titles and error texts arrive in the console code page now and then. A line that
            // is not UTF-8 is kept with replacement characters instead of ending the read,
            // which would leave yt-dlp writing into a pipe nobody empties.
            let line = String::from_utf8_lossy(&buffer)
                .trim_end_matches(['\r', '\n'])
                .to_string();
            let line = ProcessLine {
                stream: process_stream,
                line,
            };
            if sender.send(line).await.is_err() {
                break;
            }
        }
    });
}

async fn measure_provider_temp_artifacts(target_dir: &Path, temp_stem: &str) -> u64 {
    let target_dir = target_dir.to_path_buf();
    let temp_stem = temp_stem.to_string();
    tokio::task::spawn_blocking(move || provider_temp_artifact_bytes(&target_dir, &temp_stem))
        .await
        .unwrap_or(0)
}

fn provider_temp_artifact_bytes(target_dir: &Path, temp_stem: &str) -> u64 {
    std::fs::read_dir(target_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|filename| {
                    (filename == temp_stem || filename.starts_with(&format!("{temp_stem}.")))
                        && !is_thumbnail_path(path)
                })
        })
        .filter_map(|path| path.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use crate::download::download_cancellation_pair;

    use super::super::ytdlp_command::ProviderQualityPreference;
    use super::*;

    const TEMP_STEM: &str = "ldm-test";

    fn backend() -> ExternalProviderBackend {
        ExternalProviderBackend {
            executable: "cmd".to_string(),
            quality: ProviderQualityPreference::Best,
            quality_overridden: false,
            ffmpeg: None,
            deno: None,
            cookies_file: None,
            proxy: None,
            extractor_args: None,
        }
    }

    /// A batch file standing in for yt-dlp.
    fn script(dir: &Path, lines: &[&str]) -> std::process::Command {
        let path = dir.join("backend.cmd");
        std::fs::write(&path, format!("@echo off\r\n{}\r\n", lines.join("\r\n"))).unwrap();
        let mut command = std::process::Command::new("cmd");
        command.arg("/C").arg(path);
        command
    }

    async fn run(
        command: std::process::Command,
        dir: &Path,
        cancellation: DownloadCancellation,
        watch_files: bool,
    ) -> (Result<Output, CoreError>, Vec<DownloadProgress>) {
        let mut updates = Vec::new();
        let result = run_provider_process(
            command,
            &backend(),
            &mut |update| updates.push(update),
            dir,
            TEMP_STEM,
            cancellation,
            watch_files,
        )
        .await;
        (result, updates)
    }

    #[tokio::test]
    async fn progress_lines_become_progress_and_the_rest_is_collected() {
        let dir = tempfile::tempdir().unwrap();
        let command = script(
            dir.path(),
            &[
                "echo [info] starting",
                "echo LDM-PROGRESS downloading 512 1024 NA NA NA",
                "echo LDM-PROGRESS finished 1024 1024 NA NA NA",
                "echo ERROR: no luck 1>&2",
                "exit /b 3",
            ],
        );
        let (_cancel, cancellation) = download_cancellation_pair();

        let (result, updates) = run(command, dir.path(), cancellation, false).await;

        let output = result.unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "[info] starting"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            "ERROR: no luck"
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].downloaded_bytes, 512);
        assert_eq!(updates[0].total_bytes, Some(1024));
        assert_eq!(updates[0].progress_percent, Some(50.0));
    }

    #[tokio::test]
    async fn a_cancel_ends_the_process_at_once_and_removes_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join(format!("{TEMP_STEM}.mp4.part"));
        std::fs::write(&partial, b"partial").unwrap();
        let unrelated = dir.path().join("ldm-other.mp4.part");
        std::fs::write(&unrelated, b"other").unwrap();
        // Half a minute unless it is stopped.
        let command = script(dir.path(), &["ping -n 30 127.0.0.1 > nul"]);
        let (cancel, cancellation) = download_cancellation_pair();
        let started = Instant::now();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            cancel.cancel();
        });

        let (result, _) = run(command, dir.path(), cancellation, false).await;

        assert!(matches!(result, Err(CoreError::Canceled)), "{result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        assert!(!partial.exists());
        assert!(unrelated.exists());
    }

    #[tokio::test]
    async fn a_run_that_is_already_canceled_does_not_start() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started.txt");
        let command = script(dir.path(), &[&format!("echo x> \"{}\"", marker.display())]);
        let (cancel, cancellation) = download_cancellation_pair();
        cancel.cancel();

        let (result, _) = run(command, dir.path(), cancellation, false).await;

        assert!(matches!(result, Err(CoreError::Canceled)), "{result:?}");
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn output_that_is_not_utf8_does_not_end_the_read() {
        let dir = tempfile::tempdir().unwrap();
        let latin1 = dir.path().join("latin1.txt");
        std::fs::write(&latin1, b"caf\xe9 title\r\n").unwrap();
        let command = script(
            dir.path(),
            &[
                &format!("type \"{}\"", latin1.display()),
                "echo after the odd line",
            ],
        );
        let (_cancel, cancellation) = download_cancellation_pair();

        let (result, _) = run(command, dir.path(), cancellation, false).await;

        let output = result.unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(output.status.success());
        assert!(stdout.contains("caf\u{fffd} title"), "{stdout}");
        assert!(stdout.contains("after the odd line"), "{stdout}");
    }

    #[tokio::test]
    async fn the_output_files_are_measured_while_the_backend_reports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(format!("{TEMP_STEM}.mp4.part")), b"video").unwrap();
        // About two seconds without a progress line.
        let command = script(dir.path(), &["ping -n 3 127.0.0.1 > nul"]);
        let (_cancel, cancellation) = download_cancellation_pair();

        let (result, updates) = run(command, dir.path(), cancellation, true).await;

        assert!(result.unwrap().status.success());
        assert_eq!(updates.len(), 1, "{updates:?}");
        assert_eq!(updates[0].downloaded_bytes, 5);
        assert_eq!(updates[0].total_bytes, None);
    }

    #[tokio::test]
    async fn a_backend_that_cannot_be_started_names_the_ways_to_configure_one() {
        let dir = tempfile::tempdir().unwrap();
        let command = std::process::Command::new(dir.path().join("missing-backend.exe"));
        let (_cancel, cancellation) = download_cancellation_pair();

        let (result, _) = run(command, dir.path(), cancellation, false).await;

        let message = result.unwrap_err().to_string();
        assert!(message.contains(PROVIDER_BACKEND_ENV), "{message}");
    }

    #[test]
    fn stream_progress_counts_only_matching_non_thumbnail_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ldm-test.mp4.part"), b"video").unwrap();
        std::fs::write(dir.path().join("ldm-test.webp"), b"thumbnail").unwrap();
        std::fs::write(dir.path().join("ldm-other.mp4.part"), b"other").unwrap();

        assert_eq!(provider_temp_artifact_bytes(dir.path(), "ldm-test"), 5);
    }
}
