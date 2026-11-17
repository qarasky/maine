//! Dates as Swift's `Codable` writes them: seconds since 2001-01-01 UTC as a
//! JSON number. Kept as the raw `f64` so values survive a round trip exactly.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Seconds between 1970-01-01 and 2001-01-01.
const APPLE_EPOCH_UNIX_SECONDS: f64 = 978_307_200.0;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AppleDate(pub f64);

impl AppleDate {
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    pub fn from_system_time(time: SystemTime) -> Self {
        let unix = match time.duration_since(UNIX_EPOCH) {
            Ok(after) => after.as_secs_f64(),
            Err(before) => -before.duration().as_secs_f64(),
        };
        Self(unix - APPLE_EPOCH_UNIX_SECONDS)
    }

    pub fn to_system_time(self) -> SystemTime {
        let unix = self.0 + APPLE_EPOCH_UNIX_SECONDS;
        if unix >= 0.0 {
            UNIX_EPOCH + Duration::from_secs_f64(unix)
        } else {
            UNIX_EPOCH - Duration::from_secs_f64(-unix)
        }
    }

    pub fn unix_seconds(self) -> f64 {
        self.0 + APPLE_EPOCH_UNIX_SECONDS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_date_is_the_year_2001() {
        assert_eq!(AppleDate(0.0).unix_seconds(), 978_307_200.0);
        assert_eq!(
            AppleDate(0.0).to_system_time(),
            UNIX_EPOCH + Duration::from_secs(978_307_200)
        );
        assert_eq!(
            AppleDate::from_system_time(UNIX_EPOCH + Duration::from_secs(978_307_201)),
            AppleDate(1.0)
        );
    }
}
