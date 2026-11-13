//! The external yt-dlp backend: discovery and environment knobs, quality selection, and the
//! command line for one download attempt.

use std::{path::Path, process::Command};

use ldm_types::download_job::CreateDownloadJobRequest;

use crate::{
    error::CoreError,
    tool_discovery::{discover_tool, ToolKind, ToolStatus},
};

use super::common::{
    forwarded_headers, is_youtube_request, requires_stream_merge_tool, source_referrer,
};
use super::progress::YTDLP_PROGRESS_TEMPLATE;

pub(super) const PROVIDER_BACKEND_ENV: &str = "LDM_PROVIDER_BACKEND";
pub(super) const PROVIDER_ENGINE_ENV: &str = "LDM_PROVIDER_ENGINE";
pub(super) const PROVIDER_QUALITY_ENV: &str = "LDM_PROVIDER_QUALITY";
pub(super) const YTDLP_COOKIES_FILE_ENV: &str = "LDM_YTDLP_COOKIES_FILE";
pub(super) const YTDLP_PROXY_ENV: &str = "LDM_YTDLP_PROXY";
pub(super) const YTDLP_EXTRACTOR_ARGS_ENV: &str = "LDM_YTDLP_EXTRACTOR_ARGS";
const STREAM_CONCURRENT_FRAGMENTS: &str = "4";

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum ProviderQualityPreference {
    #[default]
    Best,
    BestMp4,
    BestCompatible,
    AudioOnly,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct YtDlpFormatSpec {
    format_selector: &'static str,
    merge_output_format: Option<&'static str>,
}

impl ProviderQualityPreference {
    fn from_env() -> (Self, bool) {
        match std::env::var(PROVIDER_QUALITY_ENV)
            .ok()
            .as_deref()
            .and_then(Self::parse)
        {
            Some(quality) => (quality, true),
            None => (Self::default(), false),
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "best" => Some(Self::Best),
            "bestmp4" | "best-mp4" | "best_mp4" => Some(Self::BestMp4),
            "bestcompatible" | "best-compatible" | "best_compatible" => Some(Self::BestCompatible),
            "audioonly" | "audio-only" | "audio_only" => Some(Self::AudioOnly),
            _ => None,
        }
    }

    fn ytdlp_spec(self, allow_merge: bool) -> YtDlpFormatSpec {
        match self {
            Self::Best if allow_merge => {
                YtDlpFormatSpec {
                    format_selector: "bv*+ba/b",
                    merge_output_format: Some("mp4"),
                }
            }
            Self::Best => {
                YtDlpFormatSpec {
                    format_selector: "b",
                    merge_output_format: None,
                }
            }
            Self::BestMp4 if allow_merge => {
                YtDlpFormatSpec {
                    format_selector: "bv*[ext=mp4]+ba[ext=m4a]/bv*+ba/b[ext=mp4]/b",
                    merge_output_format: Some("mp4"),
                }
            }
            Self::BestMp4 => {
                YtDlpFormatSpec {
                    format_selector: "b[ext=mp4]/b",
                    merge_output_format: None,
                }
            }
            Self::BestCompatible if allow_merge => {
                YtDlpFormatSpec {
                    format_selector: "bv*[vcodec^=avc1][ext=mp4]+ba[acodec^=mp4a][ext=m4a]/bv*[vcodec^=avc1]+ba[acodec^=mp4a]/b[vcodec^=avc1][acodec^=mp4a][ext=mp4]/b[ext=mp4]/b",
                    merge_output_format: Some("mp4"),
                }
            }
            Self::BestCompatible => {
                YtDlpFormatSpec {
                    format_selector: "b[vcodec^=avc1][acodec^=mp4a][ext=mp4]/b[ext=mp4]/b",
                    merge_output_format: None,
                }
            }
            Self::AudioOnly => YtDlpFormatSpec {
                format_selector: "ba/bestaudio/b",
                merge_output_format: None,
            },
        }
    }

    pub(super) fn requires_ffmpeg_merge(self) -> bool {
        self.ytdlp_spec(true).merge_output_format.is_some()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct ExternalProviderBackend {
    pub(super) executable: String,
    pub(super) quality: ProviderQualityPreference,
    pub(super) quality_overridden: bool,
    pub(super) ffmpeg: Option<ToolStatus>,
    pub(super) deno: Option<ToolStatus>,
    pub(super) cookies_file: Option<String>,
    pub(super) proxy: Option<String>,
    pub(super) extractor_args: Option<String>,
}

/// One yt-dlp run. YouTube jobs go through up to three of these in `provider::mod`: the default
/// attempt, then `YOUTUBE_COMPAT_ATTEMPT`, then `YOUTUBE_EMBEDDED_ATTEMPT`. A user-set
/// `LDM_YTDLP_EXTRACTOR_ARGS` replaces the attempt's extractor args on every run.
#[derive(Debug, Clone, Copy)]
pub(super) struct YtDlpAttempt {
    youtube_extractor_args: Option<&'static str>,
}

/// yt-dlp's own client selection.
pub(super) const DEFAULT_YTDLP_ATTEMPT: YtDlpAttempt = YtDlpAttempt {
    youtube_extractor_args: None,
};

/// Browser-like clients for the "not available on this app" family of errors.
pub(super) const YOUTUBE_COMPAT_ATTEMPT: YtDlpAttempt = YtDlpAttempt {
    youtube_extractor_args: Some("youtube:player_client=web,web_safari,ios,mweb"),
};

/// The embedded-player client, which YouTube serves without the sign-in wall that the browser
/// and mobile clients get on shared or datacenter IPs ("Sign in to confirm you're not a bot").
/// It only works for videos whose owner allows embedding, so it is the last attempt, not the
/// first. Keep it alone: combining it with other clients (for example `android_vr`) makes yt-dlp
/// fall back to their 360p formats.
pub(super) const YOUTUBE_EMBEDDED_ATTEMPT: YtDlpAttempt = YtDlpAttempt {
    youtube_extractor_args: Some("youtube:player_client=tv_embedded"),
};

pub(super) fn external_provider_backend() -> Result<ExternalProviderBackend, CoreError> {
    let executable = match env_var_trimmed(PROVIDER_BACKEND_ENV) {
        Some(executable) => executable,
        None => {
            let yt_dlp = discover_tool(ToolKind::YtDlp);
            let Some(path) = yt_dlp.path else {
                return Err(CoreError::ProviderBackend(yt_dlp.message));
            };
            path
        }
    };

    let ffmpeg = discover_tool(ToolKind::Ffmpeg);
    let deno = discover_tool(ToolKind::Deno);
    let (quality, quality_overridden) = ProviderQualityPreference::from_env();

    Ok(ExternalProviderBackend {
        executable,
        quality,
        quality_overridden,
        ffmpeg: ffmpeg.found.then_some(ffmpeg),
        deno: deno.found.then_some(deno),
        cookies_file: env_var_trimmed(YTDLP_COOKIES_FILE_ENV),
        proxy: env_var_trimmed(YTDLP_PROXY_ENV),
        extractor_args: env_var_trimmed(YTDLP_EXTRACTOR_ARGS_ENV),
    })
}

pub(super) fn build_ytdlp_compatible_command(
    backend: &ExternalProviderBackend,
    output_template: &Path,
    request: &CreateDownloadJobRequest,
    attempt: YtDlpAttempt,
) -> Command {
    let quality = effective_provider_quality(backend, request);
    let spec = quality.ytdlp_spec(backend.ffmpeg.is_some());
    let mut command = Command::new(&backend.executable);

    command
        .arg("--no-playlist")
        .arg("--newline")
        .arg("--progress-template")
        .arg(YTDLP_PROGRESS_TEMPLATE)
        .arg("--restrict-filenames")
        .arg("--write-thumbnail")
        .arg("--socket-timeout")
        .arg("15")
        .arg("--retries")
        .arg("2")
        .arg("--fragment-retries")
        .arg("2")
        .arg("--format")
        .arg(spec.format_selector);

    if let Some(merge_output_format) = spec.merge_output_format {
        command
            .arg("--merge-output-format")
            .arg(merge_output_format);
    }

    if requires_stream_merge_tool(request) {
        command
            .arg("--concurrent-fragments")
            .arg(STREAM_CONCURRENT_FRAGMENTS)
            .arg("--remux-video")
            .arg("mp4");
    }

    if let Some(ffmpeg_path) = backend
        .ffmpeg
        .as_ref()
        .and_then(|tool| tool.path.as_deref())
    {
        command.arg("--ffmpeg-location").arg(ffmpeg_path);
    }

    if let Some(deno_path) = backend.deno.as_ref().and_then(|tool| tool.path.as_deref()) {
        command
            .arg("--js-runtimes")
            .arg(format!("deno:{deno_path}"));
    }

    if let Some(cookies_file) = backend.cookies_file.as_deref() {
        command.arg("--cookies").arg(cookies_file);
    }

    if let Some(proxy) = backend.proxy.as_deref() {
        command.arg("--proxy").arg(proxy);
    }

    if let Some(extractor_args) = backend.extractor_args.as_deref() {
        command.arg("--extractor-args").arg(extractor_args);
    } else if is_youtube_request(request) {
        if let Some(extractor_args) = attempt.youtube_extractor_args {
            command.arg("--extractor-args").arg(extractor_args);
        }
    }

    if let Some(referrer) = source_referrer(request) {
        command.arg("--referer").arg(referrer);
    }

    for (name, value) in forwarded_headers(request) {
        command.arg("--add-header").arg(format!("{name}:{value}"));
    }

    command
        .arg("--output")
        .arg(output_template)
        .arg(&request.source_url);

    command
}

fn effective_provider_quality(
    backend: &ExternalProviderBackend,
    request: &CreateDownloadJobRequest,
) -> ProviderQualityPreference {
    if backend.quality_overridden {
        return backend.quality;
    }

    if is_youtube_request(request) {
        return ProviderQualityPreference::BestCompatible;
    }

    backend.quality
}

fn env_var_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::test_support::{download_request, found_tool};
    use super::*;

    #[test]
    fn default_quality_prefers_separate_best_video_and_audio() {
        assert_eq!(
            ProviderQualityPreference::Best.ytdlp_spec(true),
            YtDlpFormatSpec {
                format_selector: "bv*+ba/b",
                merge_output_format: Some("mp4"),
            }
        );
    }

    #[test]
    fn compatible_quality_prefers_h264_aac_when_merge_is_available() {
        assert_eq!(
            ProviderQualityPreference::BestCompatible.ytdlp_spec(true),
            YtDlpFormatSpec {
                format_selector: "bv*[vcodec^=avc1][ext=mp4]+ba[acodec^=mp4a][ext=m4a]/bv*[vcodec^=avc1]+ba[acodec^=mp4a]/b[vcodec^=avc1][acodec^=mp4a][ext=mp4]/b[ext=mp4]/b",
                merge_output_format: Some("mp4"),
            }
        );
    }

    #[test]
    fn compatible_quality_uses_single_file_fallback_without_ffmpeg() {
        assert_eq!(
            ProviderQualityPreference::BestCompatible.ytdlp_spec(false),
            YtDlpFormatSpec {
                format_selector: "b[vcodec^=avc1][acodec^=mp4a][ext=mp4]/b[ext=mp4]/b",
                merge_output_format: None,
            }
        );
    }

    #[test]
    fn command_includes_ffmpeg_location_and_deno_runtime_when_discovered() {
        let backend = ExternalProviderBackend {
            executable: "yt-dlp".to_string(),
            quality: ProviderQualityPreference::Best,
            quality_overridden: false,
            ffmpeg: Some(found_tool(ToolKind::Ffmpeg, "C:\\tools\\ffmpeg.exe")),
            deno: Some(found_tool(ToolKind::Deno, "C:\\tools\\deno.exe")),
            cookies_file: Some("C:\\tools\\youtube-cookies.txt".to_string()),
            proxy: Some("socks5://127.0.0.1:1080".to_string()),
            extractor_args: None,
        };

        let command = build_ytdlp_compatible_command(
            &backend,
            Path::new("C:\\Downloads\\ldm-test.%(ext)s"),
            &CreateDownloadJobRequest {
                referrer: Some("https://example.invalid/page".to_string()),
                ..download_request("provider-youtube")
            },
            YOUTUBE_COMPAT_ATTEMPT,
        );
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args
            .windows(2)
            .any(|items| items[0] == "--format" && items[1].contains("vcodec^=avc1")));
        assert!(args
            .windows(2)
            .any(|items| items[0] == "--merge-output-format" && items[1] == "mp4"));
        assert!(args
            .windows(2)
            .any(|items| items[0] == "--ffmpeg-location" && items[1] == "C:\\tools\\ffmpeg.exe"));
        assert!(args
            .windows(2)
            .any(|items| items[0] == "--js-runtimes" && items[1] == "deno:C:\\tools\\deno.exe"));
        assert!(args.iter().any(|item| item == "--write-thumbnail"));
        // One progress line per update, in the format `provider::progress` reads.
        assert!(args.iter().any(|item| item == "--newline"));
        assert!(args
            .windows(2)
            .any(|items| items[0] == "--progress-template" && items[1] == YTDLP_PROGRESS_TEMPLATE));
        assert!(args
            .windows(2)
            .any(|items| items[0] == "--referer" && items[1] == "https://example.invalid/page"));
        assert!(args
            .windows(2)
            .any(|items| items[0] == "--cookies" && items[1] == "C:\\tools\\youtube-cookies.txt"));
        assert!(args
            .windows(2)
            .any(|items| items[0] == "--proxy" && items[1] == "socks5://127.0.0.1:1080"));
        assert!(args.windows(2).any(|items| items[0] == "--extractor-args"
            && items[1] == "youtube:player_client=web,web_safari,ios,mweb"));
    }

    fn youtube_backend(extractor_args: Option<&str>) -> ExternalProviderBackend {
        ExternalProviderBackend {
            executable: "yt-dlp".to_string(),
            quality: ProviderQualityPreference::Best,
            quality_overridden: false,
            ffmpeg: Some(found_tool(ToolKind::Ffmpeg, "C:\\tools\\ffmpeg.exe")),
            deno: None,
            cookies_file: None,
            proxy: None,
            extractor_args: extractor_args.map(str::to_string),
        }
    }

    fn extractor_args_of(command: &Command) -> Vec<String> {
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        args.windows(2)
            .filter(|items| items[0] == "--extractor-args")
            .map(|items| items[1].clone())
            .collect()
    }

    #[test]
    fn embedded_attempt_uses_only_the_tv_embedded_player_client() {
        let command = build_ytdlp_compatible_command(
            &youtube_backend(None),
            Path::new("C:\\Downloads\\ldm-test.%(ext)s"),
            &download_request("provider-youtube"),
            YOUTUBE_EMBEDDED_ATTEMPT,
        );

        assert_eq!(
            extractor_args_of(&command),
            vec!["youtube:player_client=tv_embedded".to_string()]
        );
    }

    #[test]
    fn embedded_attempt_adds_no_extractor_args_for_other_providers() {
        let command = build_ytdlp_compatible_command(
            &youtube_backend(None),
            Path::new("C:\\Downloads\\ldm-test.%(ext)s"),
            &download_request("provider-x"),
            YOUTUBE_EMBEDDED_ATTEMPT,
        );

        assert!(extractor_args_of(&command).is_empty());
    }

    #[test]
    fn user_extractor_args_replace_the_attempt_extractor_args() {
        let backend = youtube_backend(Some("youtube:player_client=android;po_token=abc"));
        let request = download_request("provider-youtube");

        for attempt in [
            DEFAULT_YTDLP_ATTEMPT,
            YOUTUBE_COMPAT_ATTEMPT,
            YOUTUBE_EMBEDDED_ATTEMPT,
        ] {
            let command = build_ytdlp_compatible_command(
                &backend,
                Path::new("C:\\Downloads\\ldm-test.%(ext)s"),
                &request,
                attempt,
            );

            assert_eq!(
                extractor_args_of(&command),
                vec!["youtube:player_client=android;po_token=abc".to_string()]
            );
        }
    }

    #[test]
    fn hls_command_uses_parallel_fragments_and_mp4_remux() {
        let backend = ExternalProviderBackend {
            executable: "yt-dlp".to_string(),
            quality: ProviderQualityPreference::Best,
            quality_overridden: false,
            ffmpeg: Some(found_tool(ToolKind::Ffmpeg, "C:\\tools\\ffmpeg.exe")),
            deno: None,
            cookies_file: None,
            proxy: None,
            extractor_args: None,
        };

        let command = build_ytdlp_compatible_command(
            &backend,
            Path::new("C:\\Downloads\\ldm-test.%(ext)s"),
            &download_request("hls"),
            DEFAULT_YTDLP_ATTEMPT,
        );
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args
            .windows(2)
            .any(|items| items[0] == "--concurrent-fragments" && items[1] == "4"));
        assert!(!args.iter().any(|item| item == "--downloader"));
        assert!(args
            .windows(2)
            .any(|items| items[0] == "--remux-video" && items[1] == "mp4"));
    }

    #[test]
    fn provider_command_forwards_only_safe_browser_headers() {
        let backend = ExternalProviderBackend {
            executable: "yt-dlp".to_string(),
            quality: ProviderQualityPreference::BestCompatible,
            quality_overridden: false,
            ffmpeg: None,
            deno: None,
            cookies_file: None,
            proxy: None,
            extractor_args: None,
        };
        let mut request = download_request("hls");
        request.headers = Some(BTreeMap::from([
            ("Accept-Language".to_string(), "de-DE".to_string()),
            ("Cookie".to_string(), "blocked=true".to_string()),
        ]));

        let command = build_ytdlp_compatible_command(
            &backend,
            Path::new("C:\\Downloads\\ldm-test.%(ext)s"),
            &request,
            DEFAULT_YTDLP_ATTEMPT,
        );
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args
            .windows(2)
            .any(|items| items[0] == "--add-header" && items[1] == "accept-language:de-DE"));
        assert!(!args.iter().any(|item| item.contains("blocked=true")));
    }
}
