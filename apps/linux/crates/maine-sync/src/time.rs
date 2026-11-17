//! Time helpers. Everything uses `std`; there is no chrono.

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Seconds between the Unix epoch and 2001-01-01 UTC, the reference date
/// Swift's default `Date` encoding uses. Profile timestamps are stored in that
/// form so a file written here reads like one written by the macOS client.
pub const APPLE_EPOCH_OFFSET: f64 = 978_307_200.0;

pub fn unix_to_apple(unix_seconds: f64) -> f64 {
    unix_seconds - APPLE_EPOCH_OFFSET
}

pub fn apple_to_unix(apple_seconds: f64) -> f64 {
    apple_seconds + APPLE_EPOCH_OFFSET
}

/// Where the coordinator reads the time. Injected so debounce, polling and
/// backoff are testable without sleeping.
pub trait Clock: Send + Sync {
    /// Monotonic time for deadlines.
    fn now(&self) -> Instant;
    /// Wall-clock Unix seconds, for the "last synced" stamp only.
    fn unix_seconds(&self) -> f64;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }
}

pub fn system_clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock)
}

/// `YYYYMMDD-HHMMSS` in UTC, the suffix of conflict, remote, pre-sync copies.
/// (macOS formats in local time; a caller that wants local time can inject its
/// own formatter with `SyncEngine::with_timestamp`.)
pub fn file_timestamp(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let rest = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

pub fn file_timestamp_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    file_timestamp(seconds)
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_formats_known_instants() {
        assert_eq!(file_timestamp(0), "19700101-000000");
        assert_eq!(file_timestamp(951_827_696), "20000229-123456");
        assert_eq!(
            file_timestamp(1_731_801_600 + 3 * 3600 + 7 * 60 + 9),
            "20241117-030709"
        );
        assert_eq!(file_timestamp(4_102_444_799), "20991231-235959");
    }

    #[test]
    fn apple_epoch_round_trips() {
        assert_eq!(unix_to_apple(APPLE_EPOCH_OFFSET), 0.0);
        assert_eq!(
            apple_to_unix(unix_to_apple(1_700_000_000.5)),
            1_700_000_000.5
        );
    }
}
