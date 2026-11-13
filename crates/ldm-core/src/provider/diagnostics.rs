//! Turns a failed yt-dlp run into a message the user can act on, and writes the tail of the
//! tool's output to the log so that message can be traced back to what yt-dlp printed.

use std::process::Output;

use ldm_types::download_job::CreateDownloadJobRequest;
use uuid::Uuid;

use crate::diagnostics::sanitize_diagnostic_message;

use super::common::{first_non_empty, is_stream_request, is_youtube_request};
use super::ytdlp_command::{
    ProviderQualityPreference, PROVIDER_QUALITY_ENV, YTDLP_COOKIES_FILE_ENV,
    YTDLP_EXTRACTOR_ARGS_ENV, YTDLP_PROXY_ENV,
};

/// How many trailing output lines of a failed yt-dlp run go to the log.
const PROVIDER_OUTPUT_LOG_LINES: usize = 20;

/// Logs the tail of a failed yt-dlp run (stderr, or stdout when stderr is empty). Query strings,
/// header values, and local paths are redacted first; the classified message that reaches the
/// queue is derived from the same output by `provider_backend_failure`.
pub(super) fn log_provider_backend_output(job_id: Uuid, output: &Output) {
    let message = provider_backend_output_log_message(job_id, output);
    tracing::warn!("{message}");
}

fn provider_backend_output_log_message(job_id: Uuid, output: &Output) -> String {
    let tail = provider_output_tail(output, PROVIDER_OUTPUT_LOG_LINES);
    let detail = if tail.is_empty() {
        "no output".to_string()
    } else {
        format!("last output lines:\n{}", sanitize_diagnostic_message(&tail))
    };
    format!(
        "job {job_id} provider backend exited with {}; {detail}",
        output.status
    )
}

/// The last `max_lines` non-empty lines of the output `process_output_text` selects.
fn provider_output_tail(output: &Output, max_lines: usize) -> String {
    let output_text = process_output_text(output);
    let lines = output_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

pub(super) fn provider_backend_failure(
    output: &std::process::Output,
    quality: ProviderQualityPreference,
    request: &CreateDownloadJobRequest,
) -> String {
    let output_text = process_output_text(output);
    let message = provider_output_summary(&output_text);

    if quality.requires_ffmpeg_merge() && looks_like_missing_ffmpeg(&output_text) {
        return format!(
            "{message}\nFFmpeg is required to merge the best separate video and audio streams. Install FFmpeg or set {PROVIDER_QUALITY_ENV}=bestCompatible to use a single-file fallback."
        );
    }

    if is_youtube_request(request)
        && (looks_like_youtube_bot_challenge(&output_text)
            || (looks_like_youtube_login_wall(&output_text)
                && !has_actionable_provider_error(&output_text)))
    {
        return youtube_bot_challenge_message();
    }

    if is_youtube_request(request) && looks_like_youtube_rate_limit(&output_text) {
        return "YouTube rate-limited this VPN/IP session. Wait a few minutes or switch VPN server before retrying.".to_string();
    }

    if is_stream_request(request) {
        if let Some(status) = looks_like_stream_link_rejected(&output_text) {
            return stream_link_rejected_message(status);
        }
    }

    if is_youtube_request(request) && !has_actionable_provider_error(&output_text) {
        return format!(
            "YouTube provider backend ended without a usable download. Runtime detail: {message}"
        );
    }

    message
}

fn youtube_bot_challenge_message() -> String {
    format!(
        "YouTube rejected this request as bot-like. This commonly happens on shared VPN or datacenter IPs. LDM will not read browser cookies automatically. Try another VPN server or wait before retrying. Optional advanced setup: set {YTDLP_COOKIES_FILE_ENV} to a manually exported YouTube cookie file, {YTDLP_PROXY_ENV} to a trusted proxy, or {YTDLP_EXTRACTOR_ARGS_ENV} for yt-dlp extractor options."
    )
}

pub(super) fn process_output_text(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    first_non_empty(stderr.trim(), stdout.trim(), "").to_string()
}

/// The last actionable line of the output (or its last line), the raw form of what
/// `provider_backend_failure` explains.
pub(super) fn provider_output_summary(output_text: &str) -> String {
    let lines = output_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    lines
        .iter()
        .rev()
        .find(|line| is_actionable_provider_output_line(line))
        .or_else(|| lines.last())
        .copied()
        .unwrap_or("provider backend exited with an error")
        .to_string()
}

fn has_actionable_provider_error(output_text: &str) -> bool {
    output_text.lines().any(is_actionable_provider_output_line)
}

fn is_actionable_provider_output_line(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    line.contains("error:")
        || line.contains("failed")
        || line.contains("unable to")
        || line.contains("not available")
        || line.contains("sign in to confirm")
}

fn looks_like_missing_ffmpeg(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("ffmpeg")
        && (message.contains("not installed")
            || message.contains("not found")
            || message.contains("unable to find")
            || message.contains("is not recognized"))
}

pub(super) fn looks_like_youtube_bot_challenge(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("sign in to confirm") && message.contains("not a bot")
        || message.contains("confirm you") && message.contains("not a bot")
}

/// The symptoms yt-dlp prints while YouTube answers every client with its sign-in wall. They
/// accompany the bot challenge, and on some runs they are all that is left in the output once
/// the final `ERROR:` line is missing.
pub(super) fn looks_like_youtube_login_wall(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("no title found in player responses") || message.contains("login_required")
}

fn looks_like_youtube_rate_limit(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("try again later")
        || message.contains("request rate limit")
        || message.contains("too many requests")
}

pub(super) fn looks_like_youtube_client_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("not available on this app")
        || message.contains("this content isn't available")
        || message.contains("this video is not available")
}

/// The statuses a CDN answers a stream link with when it rejects it: 404 and 403 for a signed
/// URL bound to another IP address or an elapsed time window, 410 for an expired link.
const REJECTED_STREAM_LINK_STATUSES: &[(u16, &str)] =
    &[(404, "not found"), (403, "forbidden"), (410, "gone")];

/// The status of a rejected stream link in the output, taken from the last line that names
/// one. yt-dlp prints `HTTP Error 404: Not Found`; FFmpeg and plain reasons read `404 Not Found`.
pub(super) fn looks_like_stream_link_rejected(message: &str) -> Option<u16> {
    message.lines().rev().find_map(rejected_stream_link_status)
}

fn rejected_stream_link_status(line: &str) -> Option<u16> {
    let line = line.to_ascii_lowercase();
    REJECTED_STREAM_LINK_STATUSES
        .iter()
        .find(|(status, reason)| {
            line.contains(&format!("http error {status}"))
                || line.contains(&format!("{status} {reason}"))
        })
        .map(|(status, _)| *status)
}

pub(super) fn stream_link_rejected_message(status: u16) -> String {
    format!(
        "The stream link was rejected (HTTP {status}). Signed stream URLs are often bound to the IP address and time window of the browser session, so a VPN or proxy that only the browser uses, or a link captured a while ago, makes them fail outside the browser. Refresh the page or restart playback, then click the LDM button again."
    )
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{download_request, failed_output};
    use super::*;

    #[test]
    fn reports_missing_ffmpeg_for_merge_quality() {
        let message = provider_backend_failure(
            &failed_output("ERROR: ffmpeg is not installed"),
            ProviderQualityPreference::Best,
            &download_request("provider-youtube"),
        );

        assert!(message.contains("FFmpeg is required"));
        assert!(message.contains(PROVIDER_QUALITY_ENV));
    }

    #[test]
    fn explains_youtube_bot_challenge_without_suggesting_automatic_cookie_access() {
        let message = provider_backend_failure(
            &failed_output("ERROR: [youtube] owned: Sign in to confirm you're not a bot."),
            ProviderQualityPreference::BestCompatible,
            &download_request("provider-youtube"),
        );

        assert!(message.contains("shared VPN"));
        assert!(message.contains("will not read browser cookies automatically"));
        assert!(message.contains(YTDLP_COOKIES_FILE_ENV));
    }

    #[test]
    fn explains_rejected_stream_links_with_their_status() {
        for (output, status) in [
            (
                "ERROR: master.m3u8: Unable to download webpage: HTTP Error 404: Not Found",
                404,
            ),
            (
                "ERROR: [generic] Unable to download webpage: HTTP Error 403: Forbidden",
                403,
            ),
            (
                "ERROR: [generic] Unable to download webpage: HTTP Error 410: Gone",
                410,
            ),
            ("[hls] Server returned 404 Not Found", 404),
        ] {
            for adapter_id in ["hls", "dash"] {
                let message = provider_backend_failure(
                    &failed_output(output),
                    ProviderQualityPreference::BestCompatible,
                    &download_request(adapter_id),
                );

                assert!(
                    message.starts_with(&format!("The stream link was rejected (HTTP {status}).")),
                    "{adapter_id}: {message}"
                );
                assert!(message.contains("VPN or proxy that only the browser uses"));
                assert!(message.contains("click the LDM button again"));
            }
        }
    }

    #[test]
    fn rejected_stream_link_takes_the_status_of_the_last_line() {
        let output = "WARNING: [generic] Falling back on generic information extractor
ERROR: 1080p.m3u8: Unable to download webpage: HTTP Error 403: Forbidden
[download] retrying
ERROR: master.m3u8: Unable to download webpage: HTTP Error 404: Not Found";

        assert_eq!(looks_like_stream_link_rejected(output), Some(404));
        assert_eq!(
            looks_like_stream_link_rejected("ERROR: Unable to download webpage: HTTP Error 503"),
            None
        );
        assert_eq!(looks_like_stream_link_rejected(""), None);
    }

    #[test]
    fn rejected_stream_link_message_is_only_for_stream_requests() {
        let output = "ERROR: [generic] Unable to download webpage: HTTP Error 404: Not Found";

        let message = provider_backend_failure(
            &failed_output(output),
            ProviderQualityPreference::BestCompatible,
            &download_request("provider-x"),
        );

        assert_eq!(message, output);
    }

    #[test]
    fn provider_failure_prefers_last_actionable_error_over_earlier_warning() {
        let message = provider_backend_failure(
            &failed_output(
                "WARNING: metadata may be missing\nERROR: [youtube] owned: Unable to download API page",
            ),
            ProviderQualityPreference::BestCompatible,
            &download_request("provider-youtube"),
        );

        assert_eq!(
            message,
            "ERROR: [youtube] owned: Unable to download API page"
        );
    }

    #[test]
    fn provider_failure_explains_warning_only_youtube_exit() {
        let message = provider_backend_failure(
            &failed_output("WARNING: [youtube] owned: Some formats may be missing"),
            ProviderQualityPreference::BestCompatible,
            &download_request("provider-youtube"),
        );

        assert!(message.contains("ended without a usable download"));
        assert!(message.contains("Some formats may be missing"));
    }

    #[test]
    fn recognizes_youtube_bot_challenge_with_typographic_apostrophe() {
        let output = "ERROR: [youtube] owned: Sign in to confirm you\u{2019}re not a bot. Use --cookies-from-browser or --cookies for the authentication.";

        assert!(looks_like_youtube_bot_challenge(output));

        let message = provider_backend_failure(
            &failed_output(output),
            ProviderQualityPreference::BestCompatible,
            &download_request("provider-youtube"),
        );

        assert!(message.contains("shared VPN"), "{message}");
    }

    #[test]
    fn explains_missing_title_without_error_line_as_youtube_bot_challenge() {
        let message = provider_backend_failure(
            &failed_output(
                "WARNING: [youtube] No title found in player responses; falling back to title from initial data",
            ),
            ProviderQualityPreference::BestCompatible,
            &download_request("provider-youtube"),
        );

        assert!(message.contains("shared VPN"), "{message}");
        assert!(message.contains(YTDLP_COOKIES_FILE_ENV));
        assert!(message.contains(YTDLP_PROXY_ENV));
        assert!(!message.contains("ended without a usable download"));
    }

    #[test]
    fn explains_login_required_playability_without_error_line_as_youtube_bot_challenge() {
        let message = provider_backend_failure(
            &failed_output(
                "[youtube] owned: Downloading web client config\n[youtube] owned: Player response for client 'web' has playability status: LOGIN_REQUIRED\n[youtube] owned: Player response for client 'android' has playability status: LOGIN_REQUIRED",
            ),
            ProviderQualityPreference::BestCompatible,
            &download_request("provider-youtube"),
        );

        assert!(message.contains("shared VPN"), "{message}");
        assert!(!message.contains("ended without a usable download"));
    }

    #[test]
    fn login_wall_symptoms_do_not_hide_a_specific_error_line() {
        let message = provider_backend_failure(
            &failed_output(
                "WARNING: [youtube] No title found in player responses; falling back to title from initial data\nERROR: [youtube] owned: Unable to download API page",
            ),
            ProviderQualityPreference::BestCompatible,
            &download_request("provider-youtube"),
        );

        assert_eq!(
            message,
            "ERROR: [youtube] owned: Unable to download API page"
        );
    }

    #[test]
    fn login_wall_symptoms_only_apply_to_youtube_requests() {
        let message = provider_backend_failure(
            &failed_output("WARNING: No title found in player responses"),
            ProviderQualityPreference::BestCompatible,
            &download_request("provider-x"),
        );

        assert_eq!(message, "WARNING: No title found in player responses");
    }

    #[test]
    fn provider_output_log_keeps_only_the_sanitized_tail() {
        let mut lines = (1..=25)
            .map(|index| format!("[debug] line {index}"))
            .collect::<Vec<_>>();
        lines.push(String::new());
        lines.push(
            r"[download] Destination: C:\Users\Example\Videos\.ldm-partials\ldm-1.mp4".to_string(),
        );
        lines.push(
            "ERROR: [youtube] owned: Unable to download https://example.invalid/api?key=visible"
                .to_string(),
        );

        let message =
            provider_backend_output_log_message(Uuid::nil(), &failed_output(&lines.join("\n")));

        assert!(
            message.starts_with(
                "job 00000000-0000-0000-0000-000000000000 provider backend exited with"
            ),
            "{message}"
        );
        assert_eq!(message.lines().count(), 1 + PROVIDER_OUTPUT_LOG_LINES);
        assert!(!message.contains("[debug] line 7\n"));
        assert!(message.contains("[debug] line 8\n"));
        assert!(message.contains("Destination: [local path]"), "{message}");
        assert!(
            message.contains("https://example.invalid/api?redacted"),
            "{message}"
        );
        assert!(!message.contains("visible"));
    }

    #[test]
    fn provider_output_log_falls_back_to_stdout_and_reports_missing_output() {
        let mut output = failed_output("");
        output.stdout = b"[download]  10.0% of 7.16MiB\n\n".to_vec();

        let message = provider_backend_output_log_message(Uuid::nil(), &output);

        assert!(
            message.ends_with("last output lines:\n[download]  10.0% of 7.16MiB"),
            "{message}"
        );

        let message = provider_backend_output_log_message(Uuid::nil(), &failed_output(""));

        assert!(message.ends_with("; no output"), "{message}");
    }
}
