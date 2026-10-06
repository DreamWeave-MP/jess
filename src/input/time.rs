//! Monotonic time.
//!
//! Every event and every activator threshold is measured on one monotonic clock: a
//! [`MonotonicClock`] over `std::time::Instant` for real devices, or whatever a synthetic source
//! says the time is in a test. Nothing here reads the wall clock, so a
//! system date change never lengthens a hold or shortens a tap.

use std::ops::{Add, Sub};
use std::time::{Duration, Instant};

/// A point on a monotonic clock, in nanoseconds since the clock started.
///
/// Timestamps from different clocks (two runtimes, or a test and a real backend) don't compare
/// meaningfully; everything one runtime sees comes from one clock.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The clock's start.
    pub const ZERO: Self = Self(0);

    /// `nanos` nanoseconds after the clock's start.
    #[must_use]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// `millis` milliseconds after the clock's start.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis.saturating_mul(1_000_000))
    }

    /// Nanoseconds since the clock's start.
    #[must_use]
    pub const fn nanos(self) -> u64 {
        self.0
    }

    /// The time from `earlier` to this timestamp, or zero when `earlier` is later.
    #[must_use]
    pub fn since(self, earlier: Self) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
}

impl Add<Duration> for Timestamp {
    type Output = Self;

    fn add(self, duration: Duration) -> Self {
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        Self(self.0.saturating_add(nanos))
    }
}

impl Sub for Timestamp {
    type Output = Duration;

    fn sub(self, earlier: Self) -> Duration {
        self.since(earlier)
    }
}

/// The host's monotonic clock: `std::time::Instant`, counted from when the clock was made.
#[derive(Clone, Copy, Debug)]
pub struct MonotonicClock {
    start: Instant,
}

impl MonotonicClock {
    /// A clock whose zero is now.
    #[must_use]
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }

    /// The time since the clock was made.
    #[must_use]
    pub fn now(&self) -> Timestamp {
        Timestamp::ZERO + self.start.elapsed()
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_add_and_subtract_durations() {
        let start = Timestamp::from_millis(100);
        let later = start + Duration::from_millis(350);
        assert_eq!(later, Timestamp::from_millis(450));
        assert_eq!(later - start, Duration::from_millis(350));
        assert_eq!(
            start - later,
            Duration::ZERO,
            "an earlier timestamp is zero time since a later one"
        );
    }

    #[test]
    fn the_monotonic_clock_never_goes_backwards() {
        let clock = MonotonicClock::new();
        let first = clock.now();
        let second = clock.now();
        assert!(second >= first);
    }
}
