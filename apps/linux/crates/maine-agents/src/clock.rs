//! Time as an injected dependency, so waits in the driver are testable
//! without really sleeping.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A monotonic clock that can also sleep.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// A clock that only moves when told to, or when something sleeps on it:
/// `sleep` advances virtual time at once and returns. For tests whose
/// "remote shell" answers synchronously, so a 30 second wait costs nothing.
#[derive(Debug)]
pub struct ManualClock {
    base: Instant,
    offset: Mutex<Duration>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            offset: Mutex::new(Duration::ZERO),
        }
    }

    pub fn advance(&self, duration: Duration) {
        *self.offset.lock().unwrap_or_else(|e| e.into_inner()) += duration;
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.base + *self.offset.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sleep(&self, duration: Duration) {
        self.advance(duration);
        std::thread::yield_now();
    }
}
