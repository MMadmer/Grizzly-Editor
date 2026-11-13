//! When a download that failed tries again: at once a few times, then parked - "Waiting for the
//! connection" - after 30 s, 1 min, 2 min and then every 5 min, until it is stopped.

use std::time::Duration;

/// Failed attempts in a row that park the download.
pub const PARK_AFTER: u32 = 5;

/// What the download does after a failed attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Try again after this pause.
    Retry(Duration),
    /// Parked: wait this long, then try again.
    Park(Duration),
}

/// The retry state of one download. `unit` is the pace: a second for the author; a QA run
/// shortens it (`GRIZZLY_UPDATE_PACE_MS`).
#[derive(Clone, Debug)]
pub struct Retry {
    unit: Duration,
    in_row: u32,
    parked: u32,
}

impl Retry {
    #[must_use]
    pub fn new(unit: Duration) -> Self {
        Self { unit, in_row: 0, parked: 0 }
    }

    /// An attempt failed; `progressed`: it got bytes before it did, so the failures before it
    /// were no streak.
    pub fn failed(&mut self, progressed: bool) -> Next {
        self.in_row = if progressed { 1 } else { self.in_row + 1 };
        if progressed {
            self.parked = 0;
        }
        if self.in_row < PARK_AFTER {
            return Next::Retry(self.unit * self.in_row);
        }
        let wait = match self.parked {
            0 => 30,
            1 => 60,
            2 => 120,
            _ => 300,
        };
        self.parked += 1;
        Next::Park(self.unit * wait)
    }

    /// Whether the last answer parked the download.
    #[must_use]
    pub fn parked(&self) -> bool {
        self.in_row >= PARK_AFTER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake clock: the attempts fail at the moment they start, and the schedule says when the
    /// next one starts.
    #[test]
    fn five_failures_park_and_the_waits_grow_to_five_minutes() {
        let mut r = Retry::new(Duration::from_secs(1));
        let mut clock = Duration::ZERO;
        let mut starts = vec![clock];
        let mut parks = Vec::new();
        for _ in 0..9 {
            match r.failed(false) {
                Next::Retry(d) => clock += d,
                Next::Park(d) => {
                    parks.push(d.as_secs());
                    clock += d;
                }
            }
            starts.push(clock);
        }
        let secs: Vec<u64> = starts.iter().map(Duration::as_secs).collect();
        // four quick retries, then parked: 30 s, 1, 2 and then every 5 minutes
        assert_eq!(secs, [0, 1, 3, 6, 10, 40, 100, 220, 520, 820]);
        assert_eq!(parks, [30, 60, 120, 300, 300]);
        assert!(r.parked());
        // an attempt that got bytes ends the streak
        assert_eq!(r.failed(true), Next::Retry(Duration::from_secs(1)));
        assert!(!r.parked());
        for _ in 0..3 {
            assert!(matches!(r.failed(false), Next::Retry(_)));
        }
        assert_eq!(r.failed(false), Next::Park(Duration::from_secs(30)));
    }
}
