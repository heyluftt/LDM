//! The record of how far a native HLS download has come.
//!
//! A small text file: one header line that identifies the playlist, then one line per segment
//! that is synced into the transport file, in playlist order, holding the segment's size in
//! bytes. Recording a segment appends one line, so the cost does not grow with the playlist,
//! and a line torn by a crash is simply not counted.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::error::CoreError;

use super::parser::HlsMediaSegment;

const HLS_SEGMENT_JOURNAL_VERSION: u8 = 2;

/// What a journal was written for. A journal is only continued when all of it matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalHeader {
    version: u8,
    manifest_url: String,
    transport_filename: String,
    segment_count: usize,
    /// Fingerprint of the segment addresses without their query strings: signed playlists
    /// hand out new tokens on every fetch while the segments stay the same.
    playlist_digest: String,
}

/// The segments of one playlist that are safely in the transport file, in order.
#[derive(Debug)]
pub struct HlsSegmentJournal {
    path: PathBuf,
    header: JournalHeader,
    /// Size in bytes of every recorded segment, first segment first.
    completed: Vec<u64>,
}

impl HlsSegmentJournal {
    /// Opens the journal at `path` for a playlist. A journal written for the same playlist is
    /// continued; anything else is replaced by an empty one. Returns the journal and whether it
    /// was continued.
    pub async fn open(
        path: &Path,
        manifest_url: &str,
        transport_filename: &str,
        segments: &[HlsMediaSegment],
    ) -> Result<(Self, bool), CoreError> {
        let header = JournalHeader {
            version: HLS_SEGMENT_JOURNAL_VERSION,
            manifest_url: manifest_url.to_string(),
            transport_filename: transport_filename.to_string(),
            segment_count: segments.len(),
            playlist_digest: playlist_digest(segments),
        };
        let recorded = read_recorded_segments(path, &header).await?;
        let continued = recorded.is_some();
        let mut journal = Self {
            path: path.to_path_buf(),
            header,
            completed: recorded.unwrap_or_default(),
        };
        // Written out again either way, so a torn last line never ends up in the middle.
        journal.rewrite().await?;
        Ok((journal, continued))
    }

    /// How many segments from the start of the playlist are recorded.
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    /// The size of the transport file up to the last recorded segment.
    pub fn completed_bytes(&self) -> u64 {
        self.completed.iter().sum()
    }

    /// Records the next segments in playlist order. Call it only after their bytes are synced
    /// into the transport file; the journal must never be ahead of the data.
    pub async fn append(&mut self, segment_bytes: &[u64]) -> Result<(), CoreError> {
        if segment_bytes.is_empty() {
            return Ok(());
        }
        let mut lines = String::new();
        for bytes in segment_bytes {
            lines.push_str(&bytes.to_string());
            lines.push('\n');
        }
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(lines.as_bytes()).await?;
        file.flush().await?;
        file.sync_data().await?;
        self.completed.extend_from_slice(segment_bytes);
        Ok(())
    }

    /// Forgets every recorded segment; the download starts over.
    pub async fn reset(&mut self) -> Result<(), CoreError> {
        self.completed.clear();
        self.rewrite().await
    }

    async fn rewrite(&mut self) -> Result<(), CoreError> {
        let mut text = serde_json::to_string(&self.header)
            .map_err(|error| CoreError::ProviderBackend(error.to_string()))?;
        text.push('\n');
        for bytes in &self.completed {
            text.push_str(&bytes.to_string());
            text.push('\n');
        }

        let mut staging = self.path.clone().into_os_string();
        staging.push(".tmp");
        tokio::fs::write(&staging, text).await?;
        tokio::fs::rename(&staging, &self.path).await?;
        Ok(())
    }
}

/// The segments a journal at `path` records for `header`, or `None` when there is no journal
/// or it belongs to something else.
async fn read_recorded_segments(
    path: &Path,
    header: &JournalHeader,
) -> Result<Option<Vec<u64>>, CoreError> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => text,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData
            ) =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error.into()),
    };

    // Only lines that end with a line break were written completely.
    let mut lines = text
        .split_inclusive('\n')
        .filter_map(|line| line.strip_suffix('\n'))
        .map(|line| line.trim_end_matches('\r'));
    let Some(recorded_header) = lines
        .next()
        .and_then(|line| serde_json::from_str::<JournalHeader>(line).ok())
    else {
        return Ok(None);
    };
    if recorded_header != *header {
        return Ok(None);
    }

    Ok(Some(
        lines
            .map_while(|line| line.parse::<u64>().ok())
            .take(header.segment_count)
            .collect(),
    ))
}

/// FNV-1a over the segment addresses up to their query string. Stable across versions, which
/// the standard library's hasher does not promise.
fn playlist_digest(segments: &[HlsMediaSegment]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for segment in segments {
        let address = segment.uri.split(['?', '#']).next().unwrap_or(&segment.uri);
        for byte in address.bytes().chain(std::iter::once(b'\n')) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "https://cdn.test/index.m3u8";
    const TRANSPORT: &str = "ldm-test.native.ts";

    fn segments(uris: &[&str]) -> Vec<HlsMediaSegment> {
        uris.iter()
            .map(|uri| HlsMediaSegment {
                uri: (*uri).to_string(),
            })
            .collect()
    }

    fn playlist() -> Vec<HlsMediaSegment> {
        segments(&[
            "https://cdn.test/0.ts?token=a",
            "https://cdn.test/1.ts?token=a",
            "https://cdn.test/2.ts?token=a",
        ])
    }

    #[tokio::test]
    async fn a_new_journal_is_empty_and_recorded_segments_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ldm-test.native.hls.jsonl");

        let (mut journal, continued) =
            HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
                .await
                .unwrap();
        assert!(!continued);
        assert_eq!(journal.completed_count(), 0);

        journal.append(&[100]).await.unwrap();
        journal.append(&[250, 300]).await.unwrap();
        journal.append(&[]).await.unwrap();
        assert_eq!(journal.completed_count(), 3);
        assert_eq!(journal.completed_bytes(), 650);

        let (reopened, continued) =
            HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
                .await
                .unwrap();
        assert!(continued);
        assert_eq!(reopened.completed_count(), 3);
        assert_eq!(reopened.completed_bytes(), 650);
    }

    #[tokio::test]
    async fn new_tokens_keep_the_journal_and_other_segments_replace_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ldm-test.native.hls.jsonl");
        let (mut journal, _) = HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
            .await
            .unwrap();
        journal.append(&[100, 200]).await.unwrap();

        let resigned = segments(&[
            "https://cdn.test/0.ts?token=b",
            "https://cdn.test/1.ts?token=b",
            "https://cdn.test/2.ts?token=b",
        ]);
        let (same_playlist, continued) =
            HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &resigned)
                .await
                .unwrap();
        assert!(continued);
        assert_eq!(same_playlist.completed_count(), 2);

        let other = segments(&[
            "https://cdn.test/0.ts",
            "https://cdn.test/1.ts",
            "https://cdn.test/other.ts",
        ]);
        let (replaced, continued) = HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &other)
            .await
            .unwrap();
        assert!(!continued);
        assert_eq!(replaced.completed_count(), 0);

        let (after_replacement, _) =
            HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
                .await
                .unwrap();
        assert_eq!(after_replacement.completed_count(), 0);
    }

    #[tokio::test]
    async fn a_torn_last_line_is_not_counted_and_does_not_corrupt_later_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ldm-test.native.hls.jsonl");
        let (mut journal, _) = HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
            .await
            .unwrap();
        journal.append(&[100]).await.unwrap();
        // A crash in the middle of writing "250\n".
        let mut torn = std::fs::read(&path).unwrap();
        torn.extend_from_slice(b"25");
        std::fs::write(&path, torn).unwrap();

        let (mut reopened, continued) =
            HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
                .await
                .unwrap();
        assert!(continued);
        assert_eq!(reopened.completed_count(), 1);

        reopened.append(&[250]).await.unwrap();
        let (again, _) = HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
            .await
            .unwrap();
        assert_eq!(again.completed_count(), 2);
        assert_eq!(again.completed_bytes(), 350);
    }

    #[tokio::test]
    async fn reset_forgets_every_segment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ldm-test.native.hls.jsonl");
        let (mut journal, _) = HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
            .await
            .unwrap();
        journal.append(&[100, 200]).await.unwrap();

        journal.reset().await.unwrap();

        assert_eq!(journal.completed_count(), 0);
        let (reopened, continued) =
            HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
                .await
                .unwrap();
        assert!(continued);
        assert_eq!(reopened.completed_count(), 0);
    }

    #[tokio::test]
    async fn a_journal_in_the_old_format_starts_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ldm-test.native.hls.jsonl");
        std::fs::write(
            &path,
            "{\n  \"version\": 1,\n  \"manifest_url\": \"https://cdn.test/index.m3u8\"\n}\n",
        )
        .unwrap();

        let (journal, continued) = HlsSegmentJournal::open(&path, MANIFEST, TRANSPORT, &playlist())
            .await
            .unwrap();

        assert!(!continued);
        assert_eq!(journal.completed_count(), 0);
    }
}
