//! When AutoSave sends a host document (rule 3): 3 s after the last change, and at least every
//! 30 s while changes keep coming. Pure: it takes `now`, so it is tested without a clock.

/// Send once the document has been left alone this long.
pub const IDLE_MS: f64 = 3000.0;
/// Send at least this often while changes keep coming.
pub const CHECKPOINT_MS: f64 = 30_000.0;
/// Longest time between ticks while a save is due later.
pub const MAX_WAKE_MS: f64 = 1000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Tick {
    /// Nothing to send.
    Idle,
    Send,
    /// Due in this many milliseconds.
    Wait(f64),
}

#[derive(Debug, Default)]
pub(super) struct Timer {
    seen: Option<u64>,
    last_change: f64,
    /// When the first change not yet sent was seen.
    first_unsent: Option<f64>,
}

impl Timer {
    /// `generation` is the document's now; `unsent` says it has changes AutoSave may send;
    /// `typing` that typing not committed yet is open (its commit is a change of its own).
    pub fn tick(&mut self, generation: u64, unsent: bool, typing: bool, now: f64) -> Tick {
        if self.seen != Some(generation) {
            self.seen = Some(generation);
            self.last_change = now;
        }
        if !unsent {
            self.first_unsent = None;
            return Tick::Idle;
        }
        let first = *self.first_unsent.get_or_insert(now);
        if typing {
            return Tick::Idle;
        }
        let due = (self.last_change + IDLE_MS).min(first + CHECKPOINT_MS);
        if now >= due { Tick::Send } else { Tick::Wait(due - now) }
    }

    /// The current state went to the shell: the checkpoint counts from the next change.
    pub fn sent(&mut self) {
        self.first_unsent = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_sent_three_seconds_after_the_last_edit() {
        let mut t = Timer::default();
        assert_eq!(t.tick(0, false, false, 0.0), Tick::Idle);
        assert_eq!(t.tick(1, true, false, 1000.0), Tick::Wait(3000.0));
        assert_eq!(t.tick(2, true, false, 2500.0), Tick::Wait(3000.0), "a newer edit restarts the wait");
        assert_eq!(t.tick(2, true, false, 5400.0), Tick::Wait(100.0));
        assert_eq!(t.tick(2, true, false, 5500.0), Tick::Send);
        t.sent();
        assert_eq!(t.tick(2, false, false, 6000.0), Tick::Idle, "sent: nothing more until the next change");
    }

    #[test]
    fn continuous_edits_are_sent_at_the_thirty_second_checkpoint() {
        let mut t = Timer::default();
        let mut sent_at = None;
        for step in 1..=40u64 {
            let now = step as f64 * 1000.0;
            if t.tick(step, true, false, now) == Tick::Send {
                sent_at = Some(now);
                break;
            }
        }
        assert_eq!(sent_at, Some(31_000.0), "30 s after the first unsent change");
    }

    #[test]
    fn open_typing_waits_for_its_commit() {
        let mut t = Timer::default();
        t.tick(1, true, false, 0.0);
        assert_eq!(t.tick(1, true, true, 10_000.0), Tick::Idle);
        assert_eq!(t.tick(2, true, false, 10_500.0), Tick::Wait(3000.0));
    }
}
