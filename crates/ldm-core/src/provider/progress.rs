//! yt-dlp's download progress as numbers instead of text meant for people.
//!
//! `--progress-template` makes yt-dlp print one line per update with the raw values of its
//! progress hook, `NA` where it has none. The template and its parser live together here so
//! they cannot drift apart.

use std::time::Instant;

use crate::download::{DownloadProgress, SpeedMeter};

const PROGRESS_LINE_PREFIX: &str = "LDM-PROGRESS";

/// The value for `--progress-template`. The fields are, in order: status, bytes downloaded,
/// exact file size, estimated file size, fragments completed, fragment count. yt-dlp's own
/// speed and remaining time are left out: with parallel fragments it reports a speed of zero.
pub(super) const YTDLP_PROGRESS_TEMPLATE: &str = concat!(
    "download:LDM-PROGRESS",
    " %(progress.status)s",
    " %(progress.downloaded_bytes)s",
    " %(progress.total_bytes)s",
    " %(progress.total_bytes_estimate)s",
    " %(progress.fragment_index)s",
    " %(progress.fragment_count)s",
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgressStatus {
    Downloading,
    /// One file is complete. A run downloads several when video and audio come separately.
    Finished,
    Other,
}

/// One progress update of the file yt-dlp is downloading right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProgressLine {
    status: ProgressStatus,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    /// What yt-dlp extrapolates from the fragments so far when the size is not known.
    total_bytes_estimate: Option<u64>,
    /// Fragments completed so far.
    fragment_index: Option<u64>,
    fragment_count: Option<u64>,
}

/// Reads a line printed for `YTDLP_PROGRESS_TEMPLATE`; `None` for any other output.
pub(super) fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    let mut fields = line
        .trim()
        .strip_prefix(PROGRESS_LINE_PREFIX)?
        .split_whitespace();
    let status = match fields.next()? {
        "downloading" => ProgressStatus::Downloading,
        "finished" => ProgressStatus::Finished,
        _ => ProgressStatus::Other,
    };
    // Every call takes one field, whether it holds a number or `NA`.
    let mut number = || fields.next().and_then(parse_number);

    Some(ProgressLine {
        status,
        downloaded_bytes: number().unwrap_or(0),
        total_bytes: number(),
        total_bytes_estimate: number(),
        fragment_index: number(),
        fragment_count: number(),
    })
}

/// yt-dlp prints integers and floats (`3000000`, `9251856.0`) depending on the field.
fn parse_number(value: &str) -> Option<u64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number.round() as u64)
}

/// A step back of the percentage smaller than this is not shown.
const SHOWN_STEP_BACK_LIMIT: f64 = 25.0;

/// Progress of one yt-dlp run across the files it downloads one after another.
///
/// yt-dlp counts every file from zero. Taken as is, a video with separate audio would run to
/// 100 %, jump back, and run to 100 % again; here the finished files stay counted.
#[derive(Debug, Default)]
pub(super) struct ProviderProgress {
    /// Bytes of the files this run has completed.
    finished_bytes: u64,
    /// The highest byte count reported for the file being downloaded. Parallel fragments
    /// report from several threads, so the raw count swings back and forth.
    file_bytes: u64,
    /// Started with the first bytes: the time yt-dlp spends looking up the video is not a
    /// slow download.
    meter: Option<SpeedMeter>,
    shown_percent: Option<f64>,
}

impl ProviderProgress {
    /// Takes one update and returns what to show for it, if anything.
    pub(super) fn apply(&mut self, line: ProgressLine, now: Instant) -> Option<DownloadProgress> {
        match line.status {
            ProgressStatus::Downloading => Some(self.downloading(line, now)),
            ProgressStatus::Finished => {
                // Nothing to show: either the next file follows, or the job completes.
                let file_bytes = line
                    .total_bytes
                    .unwrap_or(0)
                    .max(line.downloaded_bytes)
                    .max(self.file_bytes);
                self.finished_bytes = self.finished_bytes.saturating_add(file_bytes);
                self.file_bytes = 0;
                None
            }
            ProgressStatus::Other => None,
        }
    }

    fn downloading(&mut self, line: ProgressLine, now: Instant) -> DownloadProgress {
        self.file_bytes = self.file_bytes.max(line.downloaded_bytes);
        let downloaded_bytes = self.finished_bytes.saturating_add(self.file_bytes);
        let speed = self
            .meter
            .get_or_insert_with(|| SpeedMeter::new(now, downloaded_bytes))
            .record(now, downloaded_bytes);

        // The size of the current file, exact or estimated, and how much of it is there.
        let file_total = line
            .total_bytes
            .or(line.total_bytes_estimate)
            .map(|total| total.max(self.file_bytes));
        let file_fraction = fragment_fraction(line).or_else(|| {
            let total = file_total.filter(|total| *total > 0)?;
            Some(self.file_bytes as f64 / total as f64)
        });
        // The finished files weigh in with their size. A later file is not known yet; it
        // moves the bar back a little when its download starts instead of back to zero.
        let fraction = match (file_fraction, file_total) {
            (Some(fraction), _) if self.finished_bytes == 0 => Some(fraction),
            (Some(fraction), Some(total)) => {
                let finished = self.finished_bytes as f64;
                Some((finished + fraction * total as f64) / (finished + total as f64))
            }
            _ => None,
        }
        .map(|fraction| fraction.clamp(0.0, 1.0));

        let eta_seconds = match (fraction, file_total, speed) {
            (Some(fraction), Some(total), Some(speed)) if speed > 0 => {
                let known_total = (self.finished_bytes as f64) + total as f64;
                Some((known_total * (1.0 - fraction) / speed as f64).ceil() as u64)
            }
            _ => None,
        };

        DownloadProgress {
            downloaded_bytes,
            // An estimate moves with every fragment; it is good for the bar, not for "of 1.2 GB".
            total_bytes: line.total_bytes.map(|total| {
                self.finished_bytes
                    .saturating_add(total.max(self.file_bytes))
            }),
            progress_percent: self.steady_percent(fraction.map(|fraction| fraction * 100.0)),
            speed_bytes_per_second: speed,
            eta_seconds,
            supports_resume: false,
        }
    }
}

impl ProviderProgress {
    /// What to show for `percent`. The weight of a file that has just started rests on an
    /// estimate that swings, so the value dips; a dip is not shown. A large step back is a
    /// correction (the new file is bigger than everything before it) and is.
    fn steady_percent(&mut self, percent: Option<f64>) -> Option<f64> {
        let percent = percent?;
        let shown = match self.shown_percent {
            Some(shown) if percent < shown && shown - percent < SHOWN_STEP_BACK_LIMIT => shown,
            _ => percent,
        };
        self.shown_percent = Some(shown);
        Some(shown)
    }
}

/// The share of completed fragments. Bytes say little about a fragmented download: the total
/// is an extrapolation that swings widely while the first fragments arrive.
fn fragment_fraction(line: ProgressLine) -> Option<f64> {
    let count = line.fragment_count.filter(|count| *count > 0)?;
    Some(line.fragment_index?.min(count) as f64 / count as f64)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Applies `lines` one second apart.
    fn progress_for(lines: &[&str]) -> Vec<Option<DownloadProgress>> {
        let start = Instant::now();
        let mut progress = ProviderProgress::default();
        lines
            .iter()
            .zip(1..)
            .map(|(line, second)| {
                progress.apply(
                    parse_progress_line(line).expect("a progress line"),
                    start + Duration::from_secs(second),
                )
            })
            .collect()
    }

    #[test]
    fn reads_the_line_yt_dlp_prints_for_a_plain_download() {
        let update = progress_for(&["LDM-PROGRESS downloading 523264 3000000 NA NA NA"])
            .remove(0)
            .unwrap();

        assert_eq!(update.downloaded_bytes, 523_264);
        assert_eq!(update.total_bytes, Some(3_000_000));
        let percent = update.progress_percent.unwrap();
        assert!((percent - 17.442).abs() < 0.001, "{percent}");
        assert!(!update.supports_resume);
    }

    #[test]
    fn other_output_is_not_progress() {
        assert_eq!(
            parse_progress_line("[download] Destination: ldm-1.mp4"),
            None
        );
        assert_eq!(
            parse_progress_line("[download]  42.5% of 7.16MiB at 1.00MiB/s ETA 00:04"),
            None
        );
        assert_eq!(parse_progress_line(""), None);
        assert_eq!(parse_progress_line("LDM-PROGRESS"), None);
    }

    #[test]
    fn missing_values_stay_unknown() {
        let update = progress_for(&["LDM-PROGRESS downloading 4096 NA NA NA NA"])
            .remove(0)
            .unwrap();

        assert_eq!(update.downloaded_bytes, 4096);
        assert_eq!(update.total_bytes, None);
        assert_eq!(update.progress_percent, None);
        assert_eq!(update.eta_seconds, None);
    }

    #[test]
    fn a_line_cut_short_is_read_as_far_as_it_goes() {
        let update = progress_for(&["LDM-PROGRESS downloading 4096 8192"])
            .remove(0)
            .unwrap();

        assert_eq!(update.downloaded_bytes, 4096);
        assert_eq!(update.total_bytes, Some(8192));
        assert_eq!(update.progress_percent, Some(50.0));
    }

    #[test]
    fn speed_and_remaining_time_are_measured_here() {
        let updates = progress_for(&[
            "LDM-PROGRESS downloading 1000000 10000000 NA NA NA",
            "LDM-PROGRESS downloading 2000000 10000000 NA NA NA",
            "LDM-PROGRESS downloading 3000000 10000000 NA NA NA",
        ]);

        let last = updates[2].as_ref().unwrap();
        assert_eq!(last.speed_bytes_per_second, Some(1_000_000));
        assert_eq!(last.eta_seconds, Some(7));
    }

    #[test]
    fn a_fragmented_download_counts_completed_fragments() {
        // Lines of a yt-dlp 2026.10.26 run with four parallel fragments: the byte count
        // steps back and the estimate swings between 8 and 37 MB.
        let updates = progress_for(&[
            "LDM-PROGRESS downloading 21504 NA 8362200.0 0 3",
            "LDM-PROGRESS downloading 8192 NA 9264144.0 0 3",
            "LDM-PROGRESS downloading 9395676 NA 37438884.0 1 3",
            "LDM-PROGRESS downloading 9395676 NA 12165668.0 3 3",
        ]);

        let first = updates[0].as_ref().unwrap();
        assert_eq!(first.downloaded_bytes, 21_504);
        assert_eq!(first.total_bytes, None);
        assert_eq!(first.progress_percent, Some(0.0));

        let stepped_back = updates[1].as_ref().unwrap();
        assert_eq!(stepped_back.downloaded_bytes, 21_504);

        let one_of_three = updates[2].as_ref().unwrap().progress_percent.unwrap();
        assert!((one_of_three - 100.0 / 3.0).abs() < 0.001, "{one_of_three}");
        assert_eq!(updates[3].as_ref().unwrap().progress_percent, Some(100.0));
    }

    #[test]
    fn an_estimate_gives_the_percentage_when_there_are_no_fragments() {
        let update = progress_for(&["LDM-PROGRESS downloading 2500000 NA 10000000.0 NA NA"])
            .remove(0)
            .unwrap();

        assert_eq!(update.total_bytes, None);
        assert_eq!(update.progress_percent, Some(25.0));
    }

    #[test]
    fn an_estimate_below_what_is_downloaded_does_not_pass_100_percent() {
        let update = progress_for(&["LDM-PROGRESS downloading 5000 NA 4000.0 NA NA"])
            .remove(0)
            .unwrap();

        assert_eq!(update.progress_percent, Some(100.0));
    }

    #[test]
    fn a_second_file_continues_where_the_first_one_ended() {
        let updates = progress_for(&[
            "LDM-PROGRESS downloading 100000000 200000000 NA NA NA",
            "LDM-PROGRESS downloading 200000000 200000000 NA NA NA",
            "LDM-PROGRESS finished 200000000 200000000 NA NA NA",
            "LDM-PROGRESS downloading 0 5000000 NA NA NA",
            "LDM-PROGRESS downloading 2500000 5000000 NA NA NA",
            "LDM-PROGRESS finished 5000000 5000000 NA NA NA",
        ]);

        assert_eq!(updates[0].as_ref().unwrap().progress_percent, Some(50.0));
        assert_eq!(updates[1].as_ref().unwrap().progress_percent, Some(100.0));
        assert!(updates[2].is_none());

        // The audio is a fortieth of the whole: the bar stays where it was instead of
        // starting over, and the bytes go on counting.
        let audio_start = updates[3].as_ref().unwrap();
        assert_eq!(audio_start.downloaded_bytes, 200_000_000);
        assert_eq!(audio_start.total_bytes, Some(205_000_000));
        assert_eq!(audio_start.progress_percent, Some(100.0));

        let audio_half = updates[4].as_ref().unwrap();
        assert_eq!(audio_half.downloaded_bytes, 202_500_000);
        assert_eq!(audio_half.progress_percent, Some(100.0));
        assert!(updates[5].is_none());
    }

    #[test]
    fn a_much_bigger_second_file_moves_the_bar_back() {
        let updates = progress_for(&[
            "LDM-PROGRESS downloading 5000000 5000000 NA NA NA",
            "LDM-PROGRESS finished 5000000 5000000 NA NA NA",
            "LDM-PROGRESS downloading 0 195000000 NA NA NA",
            "LDM-PROGRESS downloading 95000000 195000000 NA NA NA",
        ]);

        assert_eq!(updates[0].as_ref().unwrap().progress_percent, Some(100.0));
        assert_eq!(updates[2].as_ref().unwrap().progress_percent, Some(2.5));
        assert_eq!(updates[3].as_ref().unwrap().progress_percent, Some(50.0));
    }

    #[test]
    fn the_speed_does_not_count_the_time_before_the_first_bytes() {
        let start = Instant::now();
        let mut progress = ProviderProgress::default();
        let mut apply = |line: &str, seconds: u64| {
            progress
                .apply(
                    parse_progress_line(line).unwrap(),
                    start + Duration::from_secs(seconds),
                )
                .unwrap()
        };

        // yt-dlp needed a minute to get to the first byte.
        apply("LDM-PROGRESS downloading 0 10000000 NA NA NA", 60);
        let update = apply("LDM-PROGRESS downloading 2000000 10000000 NA NA NA", 61);

        assert_eq!(update.speed_bytes_per_second, Some(2_000_000));
        assert_eq!(update.eta_seconds, Some(4));
    }

    #[test]
    fn a_second_fragmented_file_is_weighed_by_its_estimated_size() {
        let updates = progress_for(&[
            "LDM-PROGRESS downloading 9000000 NA 9000000.0 3 3",
            "LDM-PROGRESS finished 9000000 9000000 NA NA NA",
            "LDM-PROGRESS downloading 500000 NA 1000000.0 5 10",
        ]);

        // 95 % by weight; shown is the 100 % the first file ended on.
        let audio_half = updates[2].as_ref().unwrap();
        assert_eq!(audio_half.downloaded_bytes, 9_500_000);
        assert_eq!(audio_half.progress_percent, Some(100.0));
        assert_eq!(audio_half.total_bytes, None);
    }

    #[test]
    fn statuses_other_than_downloading_show_nothing() {
        assert!(progress_for(&[
            "LDM-PROGRESS error 10 NA NA NA NA",
            "LDM-PROGRESS finished 10 10 NA NA NA",
        ])
        .iter()
        .all(Option::is_none));
    }

    #[test]
    fn the_template_asks_for_exactly_the_fields_the_parser_reads() {
        let template = YTDLP_PROGRESS_TEMPLATE
            .strip_prefix("download:")
            .expect("the template is for download progress");
        let mut fields = template.split(' ');

        assert_eq!(fields.next(), Some(PROGRESS_LINE_PREFIX));
        assert_eq!(
            fields.collect::<Vec<_>>(),
            [
                "%(progress.status)s",
                "%(progress.downloaded_bytes)s",
                "%(progress.total_bytes)s",
                "%(progress.total_bytes_estimate)s",
                "%(progress.fragment_index)s",
                "%(progress.fragment_count)s",
            ]
        );
    }
}
