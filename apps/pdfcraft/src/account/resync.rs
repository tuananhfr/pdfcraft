//! When a sync asked for by refused edits may run: at most one a minute, doubling to an hour
//! while the refusal stays (the background schedule of erpcons-account `lease.md` §6).

use std::time::{Duration, Instant};

pub(super) const MIN: Duration = Duration::from_secs(60);
pub(super) const MAX: Duration = Duration::from_secs(3600);

pub(super) struct Resync {
    not_before: Instant,
    failures: u32,
    min: Duration,
    max: Duration,
}

impl Resync {
    /// Due at `now`, then waits between `min` and `max` ([`MIN`] and [`MAX`] outside tests).
    pub fn new(now: Instant, min: Duration, max: Duration) -> Self {
        Self { not_before: now, failures: 0, min, max }
    }

    /// Schedules the next sync after one that `failed` (with PI's `retry_after`, if any) or left
    /// editing refused for a reason a sync may lift.
    pub fn after(&mut self, now: Instant, failed: bool, retry_after: Option<Duration>, still_locked: bool) {
        let delay = if failed || still_locked {
            let doubled = self.min.saturating_mul(1 << self.failures.min(6)).min(self.max);
            self.failures = self.failures.saturating_add(1);
            retry_after.map_or(doubled, |after| after.max(doubled))
        } else {
            self.failures = 0;
            self.min
        };
        self.not_before = now.checked_add(delay).unwrap_or(now);
    }

    /// The user's Retry starts the schedule over.
    pub fn reset(&mut self, now: Instant) {
        self.failures = 0;
        self.not_before = now;
    }

    pub fn not_before(&self) -> Instant {
        self.not_before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_a_minute_and_backs_off_to_an_hour_while_locked() {
        let now = Instant::now();
        let mut r = Resync::new(now, MIN, MAX);
        let delays: Vec<u64> = (0..8)
            .map(|_| {
                r.after(now, false, None, true);
                r.not_before().duration_since(now).as_secs()
            })
            .collect();
        assert_eq!(delays, [60, 120, 240, 480, 960, 1920, 3600, 3600]);
        r.after(now, true, Some(Duration::from_secs(7200)), false);
        assert_eq!(r.not_before().duration_since(now), Duration::from_secs(7200), "PI's retry_after wins when longer");
        r.after(now, false, None, false);
        assert_eq!(r.not_before().duration_since(now), Duration::from_secs(60), "a lifted lock resets the schedule");
        r.after(now, false, None, true);
        r.reset(now);
        assert_eq!(r.not_before(), now);
        r.after(now, false, None, true);
        assert_eq!(r.not_before().duration_since(now), Duration::from_secs(60), "Retry starts over");
    }
}
