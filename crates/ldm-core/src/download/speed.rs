//! Transfer speed and remaining time of a byte-counted download.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use super::DownloadProgress;

/// How far back the speed looks: long enough to smooth bursty chunks, short enough to follow a
/// connection that slows down.
const WINDOW: Duration = Duration::from_secs(5);
/// Chunks that arrive closer together than this share one sample, so the window holds a few
/// dozen entries however fast the connection is.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(250);
/// Nothing is reported before this much time has been observed; one chunk says nothing about
/// the speed.
const MIN_OBSERVED: Duration = Duration::from_millis(500);

/// Measures speed over a sliding window of the running byte total.
///
/// The download loops feed it every chunk. The clock is passed in so tests can drive it.
#[derive(Debug)]
pub struct SpeedMeter {
    /// `(time, running total)` samples, oldest first; never empty.
    samples: VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    /// Starts at `downloaded_bytes`, the bytes already on disk when a download resumes; they
    /// do not count as transferred.
    pub fn new(now: Instant, downloaded_bytes: u64) -> Self {
        Self {
            samples: VecDeque::from([(now, downloaded_bytes)]),
        }
    }

    /// Records the running total and returns the speed in bytes per second, once known.
    pub fn record(&mut self, now: Instant, downloaded_bytes: u64) -> Option<u64> {
        let newest_is_old = self
            .samples
            .back()
            .is_none_or(|(at, _)| now.duration_since(*at) >= SAMPLE_INTERVAL);
        if newest_is_old {
            self.samples.push_back((now, downloaded_bytes));
        }
        // Keep one sample at or beyond the window's edge as the baseline and drop the rest.
        while self
            .samples
            .get(1)
            .is_some_and(|(at, _)| now.duration_since(*at) >= WINDOW)
        {
            self.samples.pop_front();
        }

        let (oldest_at, oldest_bytes) = *self.samples.front()?;
        let observed = now.duration_since(oldest_at);
        if observed < MIN_OBSERVED {
            return None;
        }
        let transferred = downloaded_bytes.checked_sub(oldest_bytes)?;
        Some((transferred as f64 / observed.as_secs_f64()).round() as u64)
    }

    /// `record`, packaged as the progress update the download loops emit.
    pub fn progress(
        &mut self,
        now: Instant,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        supports_resume: bool,
    ) -> DownloadProgress {
        let speed = self.record(now, downloaded_bytes);
        DownloadProgress {
            downloaded_bytes,
            total_bytes,
            progress_percent: None,
            speed_bytes_per_second: speed,
            eta_seconds: eta_seconds(speed, downloaded_bytes, total_bytes),
            supports_resume,
        }
    }
}

/// Seconds until `total_bytes` is reached at `speed_bytes_per_second`; `None` while either is
/// unknown or the speed is zero.
pub fn eta_seconds(
    speed_bytes_per_second: Option<u64>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> Option<u64> {
    let speed = speed_bytes_per_second.filter(|speed| *speed > 0)?;
    let remaining = total_bytes?.checked_sub(downloaded_bytes)?;
    Some(remaining.div_ceil(speed))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;
    const CHUNK: u64 = 16 * 1024;

    /// Feeds `chunks` chunks of `CHUNK` bytes at `bytes_per_second` and returns the last speed.
    fn feed(
        meter: &mut SpeedMeter,
        now: &mut Instant,
        total: &mut u64,
        chunks: u64,
        bytes_per_second: u64,
    ) -> Option<u64> {
        let gap = Duration::from_secs_f64(CHUNK as f64 / bytes_per_second as f64);
        let mut speed = None;
        for _ in 0..chunks {
            *now += gap;
            *total += CHUNK;
            speed = meter.record(*now, *total);
        }
        speed
    }

    fn assert_close(actual: Option<u64>, expected: u64) {
        let actual = actual.expect("a speed was expected");
        let tolerance = expected / 20;
        assert!(
            actual.abs_diff(expected) <= tolerance,
            "{actual} is not within 5 % of {expected}"
        );
    }

    #[test]
    fn reports_the_real_rate_for_many_small_chunks() {
        // The regression this module exists for: the queue once divided a single chunk by one
        // second, so a 1 MiB/s download showed 16 KiB/s.
        let mut now = Instant::now();
        let mut total = 0;
        let mut meter = SpeedMeter::new(now, total);

        let speed = feed(&mut meter, &mut now, &mut total, 3 * 64, MIB);

        assert_close(speed, MIB);
    }

    #[test]
    fn stays_silent_until_half_a_second_was_observed() {
        let start = Instant::now();
        let mut meter = SpeedMeter::new(start, 0);

        assert_eq!(
            meter.record(start + Duration::from_millis(100), CHUNK),
            None
        );
        assert_eq!(
            meter.record(start + Duration::from_millis(499), 2 * CHUNK),
            None
        );
        assert!(meter
            .record(start + Duration::from_millis(500), 3 * CHUNK)
            .is_some());
    }

    #[test]
    fn bytes_already_on_disk_do_not_count_as_transferred() {
        let mut now = Instant::now();
        let mut total = 500 * MIB;
        let mut meter = SpeedMeter::new(now, total);

        let speed = feed(&mut meter, &mut now, &mut total, 2 * 64, MIB);

        assert_close(speed, MIB);
    }

    #[test]
    fn follows_a_connection_that_slows_down() {
        let mut now = Instant::now();
        let mut total = 0;
        let mut meter = SpeedMeter::new(now, total);
        feed(&mut meter, &mut now, &mut total, 10 * 640, 10 * MIB);

        // Eight seconds at a tenth of the rate: the fast stretch has left the window.
        let speed = feed(&mut meter, &mut now, &mut total, 8 * 64, MIB);

        assert_close(speed, MIB);
        assert!(meter.samples.len() < 40, "{}", meter.samples.len());
    }

    #[test]
    fn progress_carries_speed_and_remaining_time() {
        let mut now = Instant::now();
        let mut total = 0;
        let mut meter = SpeedMeter::new(now, total);
        feed(&mut meter, &mut now, &mut total, 2 * 64, MIB);

        let progress = meter.progress(now, total, Some(total + 10 * MIB), true);

        assert_close(progress.speed_bytes_per_second, MIB);
        assert_eq!(progress.eta_seconds, Some(10));
        assert_eq!(progress.downloaded_bytes, total);
        assert!(progress.supports_resume);
    }

    #[test]
    fn remaining_time_needs_a_speed_and_a_total() {
        assert_eq!(eta_seconds(Some(MIB), 0, Some(10 * MIB)), Some(10));
        assert_eq!(eta_seconds(Some(MIB), 0, Some(10 * MIB + 1)), Some(11));
        assert_eq!(eta_seconds(None, 0, Some(10 * MIB)), None);
        assert_eq!(eta_seconds(Some(0), 0, Some(10 * MIB)), None);
        assert_eq!(eta_seconds(Some(MIB), 0, None), None);
        assert_eq!(eta_seconds(Some(MIB), 2 * MIB, Some(MIB)), None);
    }
}
