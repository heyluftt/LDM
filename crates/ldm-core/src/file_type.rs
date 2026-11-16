use ldm_types::download_job::DownloadKind;

#[derive(Debug, Clone, Default)]
pub struct FileTypeInput {
    pub url: Option<String>,
    pub content_type: Option<String>,
    pub suggested_filename: Option<String>,
}

pub fn classify_download(input: &FileTypeInput) -> DownloadKind {
    if let Some(kind) = input
        .content_type
        .as_deref()
        .and_then(classify_by_content_type)
    {
        return kind;
    }

    for candidate in [&input.suggested_filename, &input.url]
        .into_iter()
        .flatten()
    {
        if let Some(kind) = classify_by_extension(candidate) {
            return kind;
        }
    }

    DownloadKind::Other
}

fn classify_by_content_type(content_type: &str) -> Option<DownloadKind> {
    let value = content_type.split(';').next()?.trim().to_ascii_lowercase();
    if value.starts_with("video/") {
        return Some(DownloadKind::Video);
    }
    if value.starts_with("audio/") {
        return Some(DownloadKind::Audio);
    }
    if value.starts_with("image/") {
        return Some(DownloadKind::Image);
    }
    match value.as_str() {
        "application/pdf" | "text/plain" | "application/msword" => Some(DownloadKind::Document),
        "application/zip" | "application/x-7z-compressed" | "application/x-rar-compressed" => {
            Some(DownloadKind::Archive)
        }
        "application/vnd.microsoft.portable-executable" | "application/x-msdownload" => {
            Some(DownloadKind::Program)
        }
        _ => None,
    }
}

fn classify_by_extension(value: &str) -> Option<DownloadKind> {
    let extension = value
        .split('?')
        .next()
        .and_then(|path| path.rsplit('.').next())
        .map(str::to_ascii_lowercase)?;

    match extension.as_str() {
        "mp4" | "webm" | "mkv" | "mov" | "m3u8" | "mpd" => Some(DownloadKind::Video),
        "mp3" | "m4a" | "wav" | "flac" => Some(DownloadKind::Audio),
        "exe" | "msi" | "msix" | "appx" | "bat" | "cmd" | "ps1" | "scr" | "com" => {
            Some(DownloadKind::Program)
        }
        "pdf" | "doc" | "docx" | "txt" | "rtf" => Some(DownloadKind::Document),
        "zip" | "7z" | "rar" | "tar" | "gz" => Some(DownloadKind::Archive),
        "png" | "jpg" | "jpeg" | "webp" | "gif" => Some(DownloadKind::Image),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_wins_over_url_extension() {
        let kind = classify_download(&FileTypeInput {
            url: Some("https://example.invalid/file.bin".to_string()),
            content_type: Some("video/mp4".to_string()),
            suggested_filename: None,
        });

        assert_eq!(kind, DownloadKind::Video);
    }

    #[test]
    fn classifies_program_extensions() {
        let kind = classify_download(&FileTypeInput {
            url: Some("https://example.invalid/setup.msi".to_string()),
            content_type: None,
            suggested_filename: None,
        });

        assert_eq!(kind, DownloadKind::Program);
    }
}
