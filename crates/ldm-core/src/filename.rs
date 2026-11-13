use std::path::{Path, PathBuf};

use sanitize_filename::sanitize;
use url::Url;
use uuid::Uuid;

use crate::error::CoreError;

pub const PARTIALS_DIR_NAME: &str = ".ldm-partials";
/// How many numbered names `claim_unique_path` tries; far beyond any real folder.
const MAX_NAME_ATTEMPTS: u32 = 10_000;
const MEDIA_FILENAME_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "webm", "mkv", "avi", "ts", "mp3", "m4a", "aac", "opus", "flac",
];

pub fn safe_filename(
    suggested_filename: Option<&str>,
    source_url: &str,
) -> Result<String, CoreError> {
    let raw = suggested_filename
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| filename_from_url(source_url))
        .ok_or(CoreError::MissingFilename)?;

    let sanitized = sanitize(raw).trim().trim_matches('.').to_string();
    if sanitized.is_empty() {
        return Ok("download".to_string());
    }

    Ok(clean_media_release_filename(&sanitized, source_url))
}

pub fn partials_dir(target_dir: &Path) -> PathBuf {
    target_dir.join(PARTIALS_DIR_NAME)
}

pub fn ensure_partials_dir(target_dir: &Path) -> Result<PathBuf, CoreError> {
    let dir = partials_dir(target_dir);
    std::fs::create_dir_all(&dir)?;
    hide_partials_dir(&dir)?;
    Ok(dir)
}

pub async fn ensure_partials_dir_async(target_dir: &Path) -> Result<PathBuf, CoreError> {
    let dir = partials_dir(target_dir);
    tokio::fs::create_dir_all(&dir).await?;
    hide_partials_dir(&dir)?;
    Ok(dir)
}

/// The partial file of a job: `<target>/.ldm-partials/ldm-<job id>.part`.
///
/// It is named after the job, not after the download, so two jobs that fetch files of the same
/// name never share partial data, and the path is known before the first byte arrives.
pub fn job_partial_path(target_dir: &Path, job_id: Uuid) -> PathBuf {
    partials_dir(target_dir).join(format!("ldm-{}.part", job_id.simple()))
}

/// A name in `target_dir` that is free right now: `filename`, or `stem (1).ext`,
/// `stem (2).ext`, ... Only for scratch files; a finished download goes through
/// `move_to_unique_path`, which cannot be raced.
pub fn unique_path(target_dir: &Path, filename: &str) -> PathBuf {
    (0..=u32::MAX)
        .map(|index| target_dir.join(numbered_filename(filename, index)))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| target_dir.join(filename))
}

/// Reserves a free name in `target_dir` by creating an empty file there: `filename` itself, or
/// the first free `stem (n).ext`. Creating the file is what makes the choice safe: a second
/// download that finishes at the same moment finds the name taken and moves on to the next.
pub fn claim_unique_path(target_dir: &Path, filename: &str) -> std::io::Result<PathBuf> {
    for index in 0..MAX_NAME_ATTEMPTS {
        let candidate = target_dir.join(numbered_filename(filename, index));
        if candidate.exists() {
            continue;
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    Err(std::io::Error::other(format!(
        "no free name for '{filename}' in '{}'",
        target_dir.display()
    )))
}

/// Moves a finished download to a free name in `target_dir` and returns where it landed.
///
/// The name is claimed first (`claim_unique_path`) and the file is then renamed over the
/// placeholder, so two downloads that finish together can neither pick the same name nor
/// overwrite each other, and an existing file is never replaced.
pub fn move_to_unique_path(
    source: &Path,
    target_dir: &Path,
    filename: &str,
) -> std::io::Result<PathBuf> {
    let final_path = claim_unique_path(target_dir, filename)?;
    match std::fs::rename(source, &final_path) {
        Ok(()) => Ok(final_path),
        Err(error) => {
            let _ = std::fs::remove_file(&final_path);
            Err(error)
        }
    }
}

/// `move_to_unique_path` off the async runtime's worker threads.
pub async fn move_to_unique_path_async(
    source: &Path,
    target_dir: &Path,
    filename: &str,
) -> Result<PathBuf, CoreError> {
    let source = source.to_path_buf();
    let target_dir = target_dir.to_path_buf();
    let filename = filename.to_string();
    tokio::task::spawn_blocking(move || move_to_unique_path(&source, &target_dir, &filename))
        .await
        .map_err(|error| CoreError::Io(std::io::Error::other(error.to_string())))?
        .map_err(CoreError::from)
}

/// `filename` for index 0, then `stem (1).ext`, `stem (2).ext`, ...
fn numbered_filename(filename: &str, index: u32) -> String {
    if index == 0 {
        return filename.to_string();
    }

    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    match path.extension().and_then(|value| value.to_str()) {
        Some(extension) => format!("{stem} ({index}).{extension}"),
        None => format!("{stem} ({index})"),
    }
}

fn filename_from_url(source_url: &str) -> Option<String> {
    let url = Url::parse(source_url).ok()?;
    let segment = url.path_segments()?.rfind(|segment| !segment.is_empty())?;
    Some(segment.to_string())
}

fn clean_media_release_filename(filename: &str, source_url: &str) -> String {
    let (stem, extension) = match split_filename_extension(filename) {
        Some((stem, extension)) if is_media_filename_extension(extension) => {
            (stem, Some(extension))
        }
        Some(_) => return filename.to_string(),
        None if is_media_source_url(source_url) => (filename, None),
        None => return filename.to_string(),
    };

    let tokens = filename_tokens(stem);
    if tokens.is_empty() {
        return filename.to_string();
    }

    let mut keep_count = tokens.len();
    while keep_count > 0 && is_release_tag(tokens[keep_count - 1].text) {
        keep_count -= 1;
    }

    if keep_count == tokens.len() || keep_count == 0 {
        return filename.to_string();
    }

    let cleaned_stem = normalize_media_title_stem(&stem[..tokens[keep_count - 1].end]);
    if cleaned_stem.len() < 3 {
        return filename.to_string();
    }

    match extension {
        Some(extension) => format!("{cleaned_stem}.{extension}"),
        None => cleaned_stem,
    }
}

fn split_filename_extension(filename: &str) -> Option<(&str, &str)> {
    let (stem, extension) = filename.rsplit_once('.')?;
    if stem.trim().is_empty() || extension.trim().is_empty() || extension.len() > 5 {
        return None;
    }
    Some((stem, extension))
}

fn is_media_filename_extension(extension: &str) -> bool {
    MEDIA_FILENAME_EXTENSIONS
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

fn is_media_source_url(source_url: &str) -> bool {
    let Some(filename) = filename_from_url(source_url) else {
        return false;
    };
    let Some((_, extension)) = split_filename_extension(&filename) else {
        return false;
    };

    is_media_filename_extension(extension)
        || matches!(extension.to_ascii_lowercase().as_str(), "m3u8" | "mpd")
}

#[derive(Debug)]
struct FilenameToken<'a> {
    text: &'a str,
    end: usize,
}

fn filename_tokens(value: &str) -> Vec<FilenameToken<'_>> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, character) in value.char_indices() {
        if is_filename_token_separator(character) {
            if let Some(token_start) = start.take() {
                tokens.push(FilenameToken {
                    text: &value[token_start..index],
                    end: index,
                });
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }

    if let Some(token_start) = start {
        tokens.push(FilenameToken {
            text: &value[token_start..],
            end: value.len(),
        });
    }

    tokens
}

fn is_filename_token_separator(character: char) -> bool {
    matches!(
        character,
        '.' | '_' | '-' | ' ' | '[' | ']' | '(' | ')' | '{' | '}'
    )
}

fn is_release_tag(token: &str) -> bool {
    let normalized = token
        .trim()
        .trim_matches(|character: char| !character.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    if is_resolution_tag(&normalized) || is_bitrate_tag(&normalized) {
        return true;
    }

    matches!(
        normalized.as_str(),
        "aac"
            | "ac3"
            | "eac3"
            | "dts"
            | "opus"
            | "flac"
            | "mp3"
            | "dd"
            | "ddp"
            | "truehd"
            | "atmos"
            | "x264"
            | "x265"
            | "h264"
            | "h265"
            | "avc"
            | "hevc"
            | "av1"
            | "vp9"
            | "xvid"
            | "divx"
            | "web"
            | "dl"
            | "webdl"
            | "webhd"
            | "webrip"
            | "bluray"
            | "brrip"
            | "hdrip"
            | "hdtv"
            | "dvdrip"
            | "remux"
            | "cr"
            | "dk"
            | "de"
            | "ger"
            | "german"
            | "eng"
            | "english"
            | "multi"
            | "dub"
            | "dubbed"
            | "sub"
            | "subbed"
            | "proper"
            | "repack"
            | "internal"
            | "euhd"
            | "uhd"
            | "hd"
    )
}

fn is_resolution_tag(value: &str) -> bool {
    matches!(value, "4k" | "8k")
        || value
            .strip_suffix('p')
            .and_then(|height| height.parse::<u16>().ok())
            .is_some_and(|height| {
                matches!(
                    height,
                    240 | 360 | 480 | 540 | 576 | 720 | 900 | 950 | 1080 | 1440 | 2160 | 4320
                )
            })
}

fn is_bitrate_tag(value: &str) -> bool {
    value
        .strip_suffix('k')
        .and_then(|bitrate| bitrate.parse::<u16>().ok())
        .is_some_and(|bitrate| (64..=20_000).contains(&bitrate))
}

fn normalize_media_title_stem(stem: &str) -> String {
    stem.replace(['.', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|character: char| is_filename_token_separator(character))
        .trim()
        .to_string()
}

fn hide_partials_dir(path: &Path) -> Result<(), CoreError> {
    hide_partials_dir_platform(path)
}

#[cfg(windows)]
fn hide_partials_dir_platform(path: &Path) -> Result<(), CoreError> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
    const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;

    extern "system" {
        fn GetFileAttributesW(lpFileName: *const u16) -> u32;
        fn SetFileAttributesW(lpFileName: *const u16, dwFileAttributes: u32) -> i32;
    }

    let wide_path = OsStr::new(path.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let attributes = unsafe { GetFileAttributesW(wide_path.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return Err(std::io::Error::last_os_error().into());
    }
    if attributes & FILE_ATTRIBUTE_HIDDEN != 0 {
        return Ok(());
    }
    let ok = unsafe { SetFileAttributesW(wide_path.as_ptr(), attributes | FILE_ATTRIBUTE_HIDDEN) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(())
}

#[cfg(not(windows))]
fn hide_partials_dir_platform(_path: &Path) -> Result<(), CoreError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn falls_back_to_url_filename() {
        let filename = safe_filename(None, "https://example.invalid/files/video.mp4").unwrap();
        assert_eq!(filename, "video.mp4");
    }

    #[test]
    fn creates_unique_path() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"existing").unwrap();

        let path = unique_path(dir.path(), "file.txt");
        assert_eq!(
            path.file_name().and_then(|value| value.to_str()),
            Some("file (1).txt")
        );
    }

    #[test]
    fn job_partial_paths_live_in_the_partials_folder_and_differ_per_job() {
        let dir = tempdir().unwrap();
        let first = job_partial_path(dir.path(), Uuid::from_u128(1));
        let second = job_partial_path(dir.path(), Uuid::from_u128(2));

        assert_eq!(first.parent(), Some(partials_dir(dir.path()).as_path()));
        assert_eq!(
            first.file_name().and_then(|value| value.to_str()),
            Some("ldm-00000000000000000000000000000001.part")
        );
        assert_ne!(first, second);
    }

    #[test]
    fn claims_the_plain_name_first_and_numbered_names_after_it() {
        let dir = tempdir().unwrap();

        let first = claim_unique_path(dir.path(), "file.txt").unwrap();
        let second = claim_unique_path(dir.path(), "file.txt").unwrap();
        let third = claim_unique_path(dir.path(), "file.txt").unwrap();

        assert_eq!(first, dir.path().join("file.txt"));
        assert_eq!(second, dir.path().join("file (1).txt"));
        assert_eq!(third, dir.path().join("file (2).txt"));
        assert!(first.is_file() && second.is_file() && third.is_file());
        assert_eq!(
            claim_unique_path(dir.path(), "README").unwrap(),
            dir.path().join("README")
        );
        assert_eq!(
            claim_unique_path(dir.path(), "README").unwrap(),
            dir.path().join("README (1)")
        );
    }

    #[test]
    fn concurrent_claims_never_share_a_name() {
        let dir = tempdir().unwrap();
        let claims = std::thread::scope(|scope| {
            let handles = (0..16)
                .map(|_| scope.spawn(|| claim_unique_path(dir.path(), "clip.mp4").unwrap()))
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<std::collections::BTreeSet<_>>()
        });

        assert_eq!(claims.len(), 16);
    }

    #[test]
    fn moving_a_finished_file_never_replaces_an_existing_one() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("clip.mp4"), b"first").unwrap();
        let finished = dir.path().join("ldm-job.part");
        std::fs::write(&finished, b"second").unwrap();

        let landed = move_to_unique_path(&finished, dir.path(), "clip.mp4").unwrap();

        assert_eq!(landed, dir.path().join("clip (1).mp4"));
        assert_eq!(std::fs::read(&landed).unwrap(), b"second");
        assert_eq!(
            std::fs::read(dir.path().join("clip.mp4")).unwrap(),
            b"first"
        );
        assert!(!finished.exists());
    }

    #[test]
    fn a_failed_move_releases_the_claimed_name() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("ldm-missing.part");

        assert!(move_to_unique_path(&missing, dir.path(), "clip.mp4").is_err());
        assert!(!dir.path().join("clip.mp4").exists());
    }

    #[test]
    fn cleans_common_video_release_tags_from_filename_suffix() {
        let filename = safe_filename(
            Some("Show.Name.S01E02.AAC.720p.CR.WebDL.x264-DK.mkv"),
            "https://example.invalid/video.mkv",
        )
        .unwrap();

        assert_eq!(filename, "Show Name S01E02.mkv");
    }

    #[test]
    fn cleans_language_webhd_codec_and_release_group_suffix() {
        let filename = safe_filename(
            Some("Show.Name.GERMAN.DUBBED.720p.WebHD.h264-euHD.mkv"),
            "https://example.invalid/video.mkv",
        )
        .unwrap();

        assert_eq!(filename, "Show Name.mkv");
    }

    #[test]
    fn cleans_hyphenated_web_dl_suffix() {
        let filename = safe_filename(
            Some("Show.Name.GERMAN.WEB-DL.x264-euHD.mkv"),
            "https://example.invalid/video.mkv",
        )
        .unwrap();

        assert_eq!(filename, "Show Name.mkv");
    }

    #[test]
    fn keeps_plain_media_and_program_filenames() {
        assert_eq!(
            safe_filename(
                Some("sample-5s.mp4"),
                "https://example.invalid/sample-5s.mp4"
            )
            .unwrap(),
            "sample-5s.mp4"
        );
        assert_eq!(
            safe_filename(
                Some("app_1.0.0_x64-setup.exe"),
                "https://example.invalid/app_1.0.0_x64-setup.exe",
            )
            .unwrap(),
            "app_1.0.0_x64-setup.exe"
        );
    }

    #[test]
    fn cleans_release_tags_without_filename_extension_when_source_is_media() {
        let filename = safe_filename(
            Some("Show Name AAC 720p CR WebDL x264 DK"),
            "https://example.invalid/master.m3u8",
        )
        .unwrap();

        assert_eq!(filename, "Show Name");
    }

    #[test]
    fn ensure_partials_dir_creates_the_folder() {
        let dir = tempdir().unwrap();
        let partials = ensure_partials_dir(dir.path()).unwrap();

        assert!(partials.is_dir());
        assert_eq!(
            partials.file_name().and_then(|value| value.to_str()),
            Some(PARTIALS_DIR_NAME)
        );
    }
}
