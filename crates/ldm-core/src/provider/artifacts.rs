//! Output files of a provider run: finding the media file, thumbnails, and cleanup.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use uuid::Uuid;

use crate::{
    error::CoreError,
    filename::{partials_dir, unique_path},
    process_tree::ProcessTree,
};

use super::common::{first_non_empty, provider_temp_stem};
use super::ytdlp_command::ExternalProviderBackend;

const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

const MEDIA_ARTIFACT_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "webm", "mkv", "ts", "m4a", "mp3", "aac", "opus",
];

#[derive(Debug)]
pub(super) struct ProviderArtifacts {
    pub(super) media_path: PathBuf,
    pub(super) thumbnail_paths: Vec<PathBuf>,
}

pub(super) fn find_provider_artifacts(
    target_dir: &Path,
    temp_stem: &str,
) -> Result<ProviderArtifacts, CoreError> {
    let mut media_matches = Vec::new();
    let mut thumbnail_matches = Vec::new();

    for path in std::fs::read_dir(target_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_stem()
                .and_then(|value| value.to_str())
                .is_some_and(|stem| stem == temp_stem)
        })
    {
        if is_thumbnail_path(&path) {
            thumbnail_matches.push(path);
        } else {
            media_matches.push(path);
        }
    }

    media_matches.sort_by_key(|path| {
        (
            std::cmp::Reverse(media_artifact_preference(path).unwrap_or(usize::MAX)),
            path.clone(),
        )
    });
    thumbnail_matches.sort();
    let media_path = media_matches.pop().ok_or_else(|| {
        CoreError::ProviderBackend(
            "provider backend did not create a media output file".to_string(),
        )
    })?;
    if media_artifact_preference(&media_path).is_none() {
        return Err(CoreError::ProviderBackend(format!(
            "provider backend created an unknown media artifact '{}'. The stream needs FFmpeg remuxing into a playable container.",
            media_path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("without-extension")
        )));
    }

    Ok(ProviderArtifacts {
        media_path,
        thumbnail_paths: thumbnail_matches,
    })
}

pub(super) fn cleanup_provider_temp_artifacts(
    target_dir: &Path,
    temp_stem: &str,
) -> Result<(), CoreError> {
    for path in std::fs::read_dir(target_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
    {
        let Some(filename) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };

        if filename == temp_stem || filename.starts_with(&format!("{temp_stem}.")) {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    Ok(())
}

/// `cleanup_provider_temp_artifacts` off the async runtime's threads.
pub(super) async fn cleanup_provider_temp_artifacts_async(
    target_dir: &Path,
    temp_stem: &str,
) -> Result<(), CoreError> {
    let target_dir = target_dir.to_path_buf();
    let temp_stem = temp_stem.to_string();
    tokio::task::spawn_blocking(move || cleanup_provider_temp_artifacts(&target_dir, &temp_stem))
        .await
        .map_err(|error| CoreError::ProviderBackend(error.to_string()))?
}

pub fn cleanup_provider_job_artifacts(target_dir: &Path, job_id: Uuid) -> Result<(), CoreError> {
    let temp_stem = provider_temp_stem(job_id);
    cleanup_provider_temp_artifacts(target_dir, &temp_stem)?;
    let work_dir = partials_dir(target_dir);
    if work_dir.exists() {
        cleanup_provider_temp_artifacts(&work_dir, &temp_stem)?;
    }
    Ok(())
}

pub(super) fn is_thumbnail_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            COVER_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

fn media_artifact_preference(path: &Path) -> Option<usize> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();

    MEDIA_ARTIFACT_EXTENSIONS
        .iter()
        .position(|candidate| extension == *candidate)
}

pub(super) fn finalize_thumbnails(
    backend: &ExternalProviderBackend,
    thumbnail_paths: &[PathBuf],
    final_media_path: &Path,
) -> Result<(), CoreError> {
    let created_thumbnail = if thumbnail_paths.is_empty() {
        generate_thumbnail_sidecar(backend, final_media_path)?
    } else {
        None
    };

    let mut cleanup_paths = thumbnail_paths.to_vec();
    if let Some(path) = created_thumbnail {
        cleanup_paths.push(path);
    }

    if cleanup_paths.is_empty() {
        return Ok(());
    }

    let embed_result = preferred_thumbnail(&cleanup_paths)
        .map(|thumbnail_path| embed_thumbnail(backend, thumbnail_path, final_media_path))
        .unwrap_or(Ok(()));
    let cleanup_result = cleanup_thumbnail_sidecars(&cleanup_paths);

    embed_result?;
    cleanup_result
}

fn preferred_thumbnail(thumbnail_paths: &[PathBuf]) -> Option<&Path> {
    thumbnail_paths
        .iter()
        .min_by_key(|path| thumbnail_preference(path))
        .map(PathBuf::as_path)
}

fn thumbnail_preference(path: &Path) -> usize {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => 0,
        Some("png") => 1,
        Some("webp") => 2,
        _ => 3,
    }
}

fn generate_thumbnail_sidecar(
    backend: &ExternalProviderBackend,
    final_media_path: &Path,
) -> Result<Option<PathBuf>, CoreError> {
    if !supports_embedded_thumbnail(final_media_path) {
        return Ok(None);
    }

    let Some(ffmpeg_path) = backend
        .ffmpeg
        .as_ref()
        .and_then(|tool| tool.path.as_deref())
    else {
        return Ok(None);
    };

    let thumbnail_path = created_thumbnail_path(final_media_path);
    let output = ProcessTree::output(
        Command::new(ffmpeg_path)
            .arg("-y")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-ss")
            .arg("00:00:01")
            .arg("-i")
            .arg(final_media_path)
            .arg("-frames:v")
            .arg("1")
            .arg("-q:v")
            .arg("2")
            .arg(&thumbnail_path),
    )
    .map_err(|error| {
        CoreError::ProviderBackend(format!(
            "could not start FFmpeg for thumbnail generation: {error}"
        ))
    })?;

    if output.status.success() && thumbnail_path.exists() {
        return Ok(Some(thumbnail_path));
    }

    let _ = std::fs::remove_file(&thumbnail_path);
    Ok(None)
}

fn embed_thumbnail(
    backend: &ExternalProviderBackend,
    thumbnail_path: &Path,
    final_media_path: &Path,
) -> Result<(), CoreError> {
    if !supports_embedded_thumbnail(final_media_path) {
        return Ok(());
    }

    let Some(ffmpeg_path) = backend
        .ffmpeg
        .as_ref()
        .and_then(|tool| tool.path.as_deref())
    else {
        return Ok(());
    };

    let temp_path = embedded_temp_path(final_media_path);
    let output = ProcessTree::output(
        Command::new(ffmpeg_path)
            .arg("-y")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-i")
            .arg(final_media_path)
            .arg("-i")
            .arg(thumbnail_path)
            .arg("-map")
            .arg("0")
            .arg("-map")
            .arg("1")
            .arg("-c")
            .arg("copy")
            .arg("-c:v:1")
            .arg("mjpeg")
            .arg("-disposition:v:1")
            .arg("attached_pic")
            .arg("-movflags")
            .arg("+faststart")
            .arg(&temp_path),
    )
    .map_err(|error| {
        CoreError::ProviderBackend(format!(
            "could not start FFmpeg for thumbnail embedding: {error}"
        ))
    })?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(CoreError::ProviderBackend(format!(
            "thumbnail embedding failed: {}",
            first_non_empty(
                String::from_utf8_lossy(&output.stderr).trim(),
                String::from_utf8_lossy(&output.stdout).trim(),
                "FFmpeg exited with an error"
            )
        )));
    }

    replace_media_with_embedded_thumbnail(final_media_path, &temp_path)
}

fn supports_embedded_thumbnail(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mp4" | "m4v" | "mov"
            )
        })
}

fn embedded_temp_path(final_media_path: &Path) -> PathBuf {
    let target_dir = final_media_path.parent().unwrap_or_else(|| Path::new("."));
    let filename = final_media_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("video.mp4");
    unique_path(target_dir, &format!("{filename}.ldm-cover.mp4"))
}

fn created_thumbnail_path(final_media_path: &Path) -> PathBuf {
    let target_dir = final_media_path.parent().unwrap_or_else(|| Path::new("."));
    let filename = final_media_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("video.mp4");
    unique_path(target_dir, &format!("{filename}.ldm-cover.jpg"))
}

fn replace_media_with_embedded_thumbnail(
    final_media_path: &Path,
    embedded_temp_path: &Path,
) -> Result<(), CoreError> {
    let target_dir = final_media_path.parent().unwrap_or_else(|| Path::new("."));
    let filename = final_media_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("video.mp4");
    let backup_path = unique_path(target_dir, &format!("{filename}.ldm-original"));

    std::fs::rename(final_media_path, &backup_path)?;
    match std::fs::rename(embedded_temp_path, final_media_path) {
        Ok(()) => {
            let _ = std::fs::remove_file(backup_path);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::rename(&backup_path, final_media_path);
            Err(error.into())
        }
    }
}

fn cleanup_thumbnail_sidecars(thumbnail_paths: &[PathBuf]) -> Result<(), CoreError> {
    for thumbnail_path in thumbnail_paths {
        match std::fs::remove_file(thumbnail_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::ytdlp_command::ProviderQualityPreference;
    use super::*;

    #[test]
    fn provider_artifacts_ignore_thumbnail_when_selecting_media() {
        let dir = tempfile::tempdir().unwrap();
        let media_path = dir.path().join("ldm-test.mp4");
        let thumbnail_path = dir.path().join("ldm-test.webp");
        std::fs::write(&media_path, b"video").unwrap();
        std::fs::write(&thumbnail_path, b"thumbnail").unwrap();

        let artifacts = find_provider_artifacts(dir.path(), "ldm-test").unwrap();

        assert_eq!(artifacts.media_path, media_path);
        assert_eq!(artifacts.thumbnail_paths, vec![thumbnail_path]);
    }

    #[test]
    fn provider_job_cleanup_removes_partial_media_and_thumbnail_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let job_id = Uuid::parse_str("66d4cf17-e86b-42ae-9b08-ad6862ed268d").unwrap();
        let temp_stem = provider_temp_stem(job_id);
        let partial_path = dir.path().join(format!("{temp_stem}.f399.mp4.part"));
        let thumbnail_path = dir.path().join(format!("{temp_stem}.webp"));
        let unrelated_path = dir.path().join("ldm-other.f399.mp4.part");
        std::fs::write(&partial_path, b"partial").unwrap();
        std::fs::write(&thumbnail_path, b"thumbnail").unwrap();
        std::fs::write(&unrelated_path, b"other").unwrap();

        cleanup_provider_job_artifacts(dir.path(), job_id).unwrap();

        assert!(!partial_path.exists());
        assert!(!thumbnail_path.exists());
        assert!(unrelated_path.exists());
    }

    #[test]
    fn provider_job_cleanup_removes_native_hls_resume_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let job_id = Uuid::parse_str("78ce2f1f-0064-488d-a346-cdf63fd26933").unwrap();
        let temp_stem = provider_temp_stem(job_id);
        let work_dir = partials_dir(dir.path());
        std::fs::create_dir_all(&work_dir).unwrap();
        let transport_path = work_dir.join(format!("{temp_stem}.native.ts"));
        let remux_path = work_dir.join(format!("{temp_stem}.native.mp4"));
        let journal_path = work_dir.join(format!("{temp_stem}.native.hls.jsonl"));
        let legacy_transport_path = dir.path().join(format!("{temp_stem}.native.ts"));
        let unrelated_path = dir.path().join("ldm-other.native.hls.jsonl");
        std::fs::write(&transport_path, b"transport").unwrap();
        std::fs::write(&remux_path, b"remux").unwrap();
        std::fs::write(&journal_path, b"journal").unwrap();
        std::fs::write(&legacy_transport_path, b"legacy").unwrap();
        std::fs::write(&unrelated_path, b"other").unwrap();

        cleanup_provider_job_artifacts(dir.path(), job_id).unwrap();

        assert!(!transport_path.exists());
        assert!(!remux_path.exists());
        assert!(!journal_path.exists());
        assert!(!legacy_transport_path.exists());
        assert!(unrelated_path.exists());
    }

    #[test]
    fn provider_artifacts_prefer_playable_media_over_unknown_output() {
        let dir = tempfile::tempdir().unwrap();
        let media_path = dir.path().join("ldm-test.mp4");
        let unknown_path = dir.path().join("ldm-test.unknown_video");
        std::fs::write(&unknown_path, b"unknown").unwrap();
        std::fs::write(&media_path, b"video").unwrap();

        let artifacts = find_provider_artifacts(dir.path(), "ldm-test").unwrap();

        assert_eq!(artifacts.media_path, media_path);
    }

    #[test]
    fn provider_artifacts_reject_unknown_output_without_playable_media() {
        let dir = tempfile::tempdir().unwrap();
        let unknown_path = dir.path().join("ldm-test.unknown_video");
        std::fs::write(&unknown_path, b"unknown").unwrap();

        let error = find_provider_artifacts(dir.path(), "ldm-test")
            .unwrap_err()
            .to_string();

        assert!(error.contains("unknown media artifact"));
        assert!(error.contains("FFmpeg remuxing"));
    }

    #[test]
    fn thumbnail_cleanup_removes_sidecar_files() {
        let dir = tempfile::tempdir().unwrap();
        let media_path = dir.path().join("video.mp4");
        let thumbnail_path = dir.path().join("video.webp");
        std::fs::write(&media_path, b"video").unwrap();
        std::fs::write(&thumbnail_path, b"thumbnail").unwrap();
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

        finalize_thumbnails(&backend, std::slice::from_ref(&thumbnail_path), &media_path).unwrap();

        assert!(media_path.exists());
        assert!(!thumbnail_path.exists());
    }

    #[test]
    fn thumbnail_finalize_without_sidecars_is_ok_without_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let media_path = dir.path().join("video.mp4");
        std::fs::write(&media_path, b"video").unwrap();
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

        finalize_thumbnails(&backend, &[], &media_path).unwrap();

        assert!(media_path.exists());
    }

    #[test]
    fn embedded_thumbnail_support_is_limited_to_mp4_family() {
        assert!(supports_embedded_thumbnail(Path::new("video.mp4")));
        assert!(supports_embedded_thumbnail(Path::new("video.m4v")));
        assert!(supports_embedded_thumbnail(Path::new("video.mov")));
        assert!(!supports_embedded_thumbnail(Path::new("video.webm")));
    }
}
