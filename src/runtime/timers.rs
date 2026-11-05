//! Bounded deadline queue for domain timers, independent of Luau and wall-clock time.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// Maximum active timers in one domain, including callbacks currently being dispatched.
pub const MAX_TIMERS: usize = 16_384;
/// Maximum callbacks delivered by one advance. Overdue work remains queued in deadline order.
pub const MAX_TIMER_CALLBACKS: usize = 1024;

struct Entry<T> {
    value: T,
    deadline: Duration,
    interval: Option<Duration>,
}

pub(crate) struct Timers<T> {
    now: Duration,
    next: u64,
    entries: BTreeMap<u64, Entry<T>>,
    deadlines: BTreeSet<(Duration, u64)>,
}

impl<T> Default for Timers<T> {
    fn default() -> Self {
        Self {
            now: Duration::ZERO,
            next: 0,
            entries: BTreeMap::new(),
            deadlines: BTreeSet::new(),
        }
    }
}

impl<T> Timers<T> {
    pub(crate) fn insert(&mut self, delay: Duration, repeating: bool, value: T) -> Result<u64, &'static str> {
        if self.entries.len() == MAX_TIMERS {
            return Err("the domain's active timer limit was reached");
        }
        if repeating && delay.is_zero() {
            return Err("a repeating timer needs a positive interval");
        }
        let deadline = self.now.checked_add(delay).ok_or("timer deadline overflow")?;
        let id = self.next.checked_add(1).ok_or("timer identifiers exhausted")?;
        self.next = id;
        self.entries.insert(
            id,
            Entry {
                value,
                deadline,
                interval: repeating.then_some(delay),
            },
        );
        self.deadlines.insert((deadline, id));
        Ok(id)
    }

    pub(crate) fn now(&self) -> Duration {
        self.now
    }

    pub(crate) fn advance(&mut self, delta: Duration) {
        self.now = self.now.saturating_add(delta);
    }

    /// Freeze this advance's batch before invoking anything. New timers cannot enter the batch;
    /// cancelled entries disappear from `get`, even after their deadline was selected.
    pub(crate) fn has_due(&self) -> bool {
        self.deadlines.first().is_some_and(|(deadline, _)| *deadline <= self.now)
    }

    pub(crate) fn due(&mut self) -> Vec<u64> {
        let mut batch = Vec::new();
        while batch.len() < MAX_TIMER_CALLBACKS {
            let Some(&(deadline, id)) = self.deadlines.first() else {
                break;
            };
            if deadline > self.now {
                break;
            }
            self.deadlines.pop_first();
            batch.push(id);
        }
        batch
    }

    pub(crate) fn get(&self, id: u64) -> Option<&T> {
        self.entries.get(&id).map(|entry| &entry.value)
    }

    /// Repeats coalesce missed ticks and schedule from this advance's time. Cancellation inside
    /// the callback wins: it removes the entry before this method sees it.
    pub(crate) fn finish(&mut self, id: u64) {
        let Some(entry) = self.entries.get_mut(&id) else { return };
        if let Some(interval) = entry.interval {
            let Some(deadline) = self.now.checked_add(interval) else {
                self.cancel(id);
                return;
            };
            entry.deadline = deadline;
            self.deadlines.insert((deadline, id));
        } else {
            self.entries.remove(&id);
        }
    }

    pub(crate) fn cancel(&mut self, id: u64) -> bool {
        let Some(entry) = self.entries.remove(&id) else { return false };
        // Not there while it is in a batch; removing it then does nothing.
        self.deadlines.remove(&(entry.deadline, id));
        true
    }

    /// Puts back what a batch didn't reach, under its own deadline, so it keeps its place ahead of
    /// anything scheduled later. One cancelled meanwhile stays cancelled.
    pub(crate) fn restore(&mut self, batch: impl IntoIterator<Item = u64>) {
        for id in batch {
            if let Some(entry) = self.entries.get(&id) {
                self.deadlines.insert((entry.deadline, id));
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.deadlines.clear();
        self.now = Duration::ZERO;
        // IDs stay unique across generations, even if an old native handle survives teardown.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_are_stable_and_dispatch_is_a_snapshot() {
        let mut timers = Timers::default();
        let later = timers.insert(Duration::from_millis(20), false, "later").unwrap();
        let first = timers.insert(Duration::from_millis(10), false, "first").unwrap();
        let second = timers.insert(Duration::from_millis(10), false, "second").unwrap();
        timers.advance(Duration::from_millis(10));
        let batch = timers.due();
        let newborn = timers.insert(Duration::ZERO, false, "new").unwrap();
        assert_eq!(batch, [first, second]);
        assert!(timers.cancel(second));
        assert!(!timers.cancel(second));
        assert!(timers.get(second).is_none());
        timers.finish(first);
        assert_eq!(timers.due(), [newborn]);
        timers.finish(newborn);
        timers.advance(Duration::from_millis(10));
        assert_eq!(timers.due(), [later]);
    }

    #[test]
    fn repeats_skip_missed_ticks_and_self_cancellation_wins() {
        let mut timers = Timers::default();
        let id = timers.insert(Duration::from_millis(10), true, ()).unwrap();
        timers.advance(Duration::from_secs(10));
        assert_eq!(timers.due(), [id]);
        timers.finish(id);
        assert_eq!(timers.due(), Vec::<u64>::new());
        timers.advance(Duration::from_millis(10));
        assert_eq!(timers.due(), [id]);
        timers.cancel(id);
        timers.finish(id);
        assert_eq!(timers.due(), Vec::<u64>::new());
        assert!(timers.insert(Duration::ZERO, true, ()).is_err());
    }

    #[test]
    fn limits_bound_dispatch_and_cancellation_leaves_no_tombstones() {
        let mut timers = Timers::default();
        for _ in 0..MAX_TIMERS {
            timers.insert(Duration::ZERO, false, ()).unwrap();
        }
        assert!(timers.insert(Duration::ZERO, false, ()).is_err());
        let batch = timers.due();
        assert_eq!(batch.len(), MAX_TIMER_CALLBACKS);
        for id in batch {
            timers.finish(id);
        }
        assert_eq!(timers.entries.len(), MAX_TIMERS - MAX_TIMER_CALLBACKS);
        for id in 1..=MAX_TIMERS as u64 {
            timers.cancel(id);
        }
        assert!(timers.entries.is_empty());
        assert!(timers.deadlines.is_empty());
        for _ in 0..MAX_TIMERS * 2 {
            let id = timers.insert(Duration::from_secs(1000), false, ()).unwrap();
            assert!(timers.cancel(id));
        }
        assert!(timers.deadlines.is_empty());
    }

    #[test]
    fn a_restored_batch_keeps_its_order_and_its_cancellations() {
        let mut timers = Timers::default();
        let first = timers.insert(Duration::ZERO, false, ()).unwrap();
        let second = timers.insert(Duration::ZERO, false, ()).unwrap();
        let third = timers.insert(Duration::ZERO, false, ()).unwrap();
        let batch = timers.due();
        assert_eq!(batch, [first, second, third]);
        let newborn = timers.insert(Duration::ZERO, false, ()).unwrap();
        timers.finish(first);
        assert!(timers.cancel(second));
        timers.restore([second, third]);
        assert_eq!(timers.due(), [third, newborn]);
    }

    #[test]
    fn reset_releases_callbacks_and_preserves_unique_ids() {
        let mut timers = Timers::default();
        let first = timers.insert(Duration::ZERO, false, ()).unwrap();
        timers.advance(Duration::from_secs(10));
        timers.clear();
        assert!(timers.get(first).is_none());
        assert_eq!(timers.now(), Duration::ZERO);
        assert!(timers.insert(Duration::ZERO, false, ()).unwrap() > first);
    }
}
