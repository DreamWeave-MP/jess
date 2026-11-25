//! The collector's share of a frame: incremental Luau garbage collection, a time budget's worth
//! at a time, in the time a frame would otherwise spend waiting.
//!
//! Luau collects incrementally on its own, but only in reaction to allocation: once the heap has
//! grown past the collector's threshold, whatever allocates next does a step of its work, and
//! when allocation has outrun the collector the step grows to cover the difference. In a frame
//! loop that lands an arbitrary, sometimes long, stall on an arbitrary statement. A [`Collector`]
//! does the same work at a point the driver chooses, after the frame's own work is handed off,
//! and every step it does there is credit the allocator doesn't have to pay inside the next
//! frame.
//!
//! # Pacing
//!
//! [`Collector::collect`] spends at most its budget, and less when there is nothing to do:
//!
//! - **Mid-cycle**, it steps until the budget is spent or the cycle finishes.
//! - **Between cycles**, it starts the next one only once the heap has grown by half again since
//!   the last cycle it saw finish ([`START_GROWTH`]); Luau's own default lets it double first, so
//!   the cycle starts here rather than in an allocation. Until it has seen a cycle finish it lets
//!   Luau start the first.
//!
//! So an application that stopped allocating pays nothing, and one allocating heavily pays the
//! budget every frame until the collector catches up. What the budget can't cover, Luau's own
//! allocation steps still collect, as they would without a collector; the budget never lets the
//! heap grow unbounded.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::runtime::l3i::Runtime;
use crate::runtime::l3i::memory::GcControl;

/// The budget a frame's collection spends unless the application sets its own: half a
/// millisecond, three percent of a 60 Hz frame.
pub const DEFAULT_BUDGET: Duration = Duration::from_micros(500);

/// The largest budget anything may set: a frame's collection that takes longer than this is a
/// full collection, which `jess.bench.collect` is for.
pub const MAX_BUDGET: Duration = Duration::from_millis(100);

/// How much the heap grows, as a fraction of its size when the last cycle finished, before a
/// collection starts the next cycle.
pub const START_GROWTH: f64 = 0.5;

/// The work one incremental step asks of Luau, in its kilobytes of collector work: small enough
/// that the step after the budget runs out doesn't overrun it by much, large enough that the
/// clock is read rarely.
const STEP_KILOBYTES: i32 = 64;

/// A Luau heap a [`Collector`] can step: a [`Runtime`], or a script's state from inside a call.
pub trait Heap {
    /// The heap's size in kilobytes.
    fn kilobytes(&self) -> u64;
    /// Whether the collector is between cycles.
    fn paused(&self) -> bool;
    /// One incremental step of `kilobytes` of work; true when it finished a cycle.
    fn step(&self, kilobytes: i32) -> bool;
}

impl Heap for Runtime {
    fn kilobytes(&self) -> u64 {
        u64::try_from(self.gc(GcControl::Count)).unwrap_or(0)
    }

    fn paused(&self) -> bool {
        self.gc(GcControl::IsPaused) == 1
    }

    fn step(&self, kilobytes: i32) -> bool {
        self.gc_step(kilobytes)
    }
}

/// What one [`Collector::collect`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Collection {
    /// Incremental steps run: none when there was nothing to collect.
    pub steps: u32,
    /// Whether a cycle finished.
    pub finished: bool,
    /// The time the steps took.
    pub spent: Duration,
    /// The budget it was given. A soft one: the step that runs it out finishes, so `spent` can
    /// pass it by a step, or by all of the step that finishes marking.
    pub budget: Duration,
}

impl Collection {
    /// Whether it ran past its budget by more than the budget again: more than the last step's
    /// usual overshoot, as the step that finishes marking does on a large heap.
    #[must_use]
    pub fn overran(&self) -> bool {
        self.spent > self.budget * 2
    }
}

/// One Luau heap's collection pacing and the budget a frame spends on it. A host's domain
/// carries one for its primary runtime; see the module docs.
#[derive(Debug)]
pub struct Collector {
    /// The budget in nanoseconds.
    budget: AtomicU64,
    /// The heap's size in kilobytes when the last cycle finished; 0 before one has.
    settled: AtomicU64,
    /// Whether the last collection left a cycle running, so the next can tell that Luau finished
    /// it in between.
    running: AtomicBool,
}

impl Default for Collector {
    fn default() -> Collector {
        Collector {
            budget: AtomicU64::new(nanoseconds(DEFAULT_BUDGET)),
            settled: AtomicU64::new(0),
            running: AtomicBool::new(false),
        }
    }
}

fn nanoseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

impl Collector {
    /// The budget a frame's collection spends.
    #[must_use]
    pub fn budget(&self) -> Duration {
        Duration::from_nanos(self.budget.load(Ordering::Relaxed))
    }

    /// Sets the budget a frame's collection spends, at most [`MAX_BUDGET`]; zero turns the
    /// frame's collection off and leaves the heap to Luau's allocation steps.
    pub fn set_budget(&self, budget: Duration) {
        self.budget.store(nanoseconds(budget.min(MAX_BUDGET)), Ordering::Relaxed);
    }

    /// Forgets the heap and the budget, for a new runtime: a restart's generation.
    pub fn reset(&self) {
        self.set_budget(DEFAULT_BUDGET);
        self.settled.store(0, Ordering::Relaxed);
        self.running.store(false, Ordering::Relaxed);
    }

    /// Collects `heap` for at most `budget`, as the module docs pace it. The last step may run
    /// past the budget by as long as one step takes, and a step that reaches the end of marking
    /// takes as long as finishing it does.
    pub fn collect(&self, heap: &impl Heap, budget: Duration) -> Collection {
        let nothing = Collection {
            budget,
            ..Collection::default()
        };
        if budget.is_zero() {
            return nothing;
        }
        if heap.paused() {
            let size = heap.kilobytes();
            if self.running.swap(false, Ordering::Relaxed) {
                // Luau's own steps finished the cycle this one was in.
                self.settled.store(size, Ordering::Relaxed);
            }
            let settled = self.settled.load(Ordering::Relaxed);
            #[expect(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a heap's kilobytes are far inside an f64's exact range, and the growth is positive"
            )]
            let start = settled + (settled as f64 * START_GROWTH) as u64;
            if settled == 0 || size < start {
                return nothing;
            }
        }
        let began = Instant::now();
        let mut collection = Collection {
            budget,
            ..Collection::default()
        };
        loop {
            collection.steps += 1;
            if heap.step(STEP_KILOBYTES) {
                collection.finished = true;
                self.settled.store(heap.kilobytes().max(1), Ordering::Relaxed);
                break;
            }
            if began.elapsed() >= budget {
                break;
            }
        }
        self.running.store(!collection.finished, Ordering::Relaxed);
        collection.spent = began.elapsed();
        collection
    }

    /// [`Collector::collect`] with its own budget.
    pub fn collect_frame(&self, heap: &impl Heap) -> Collection {
        self.collect(heap, self.budget())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    use super::{Collection, Collector, DEFAULT_BUDGET, Heap, MAX_BUDGET};

    /// A heap whose collector finishes a cycle on its `finish_after`th step, and whose every step
    /// takes until the clock has moved, so a budget of a nanosecond is spent by one step on any
    /// machine.
    struct Fake {
        kilobytes: Cell<u64>,
        paused: Cell<bool>,
        steps: Cell<u32>,
        finish_after: u32,
    }

    impl Fake {
        fn new(kilobytes: u64, finish_after: u32) -> Fake {
            Fake {
                kilobytes: Cell::new(kilobytes),
                paused: Cell::new(true),
                steps: Cell::new(0),
                finish_after,
            }
        }
    }

    impl Heap for Fake {
        fn kilobytes(&self) -> u64 {
            self.kilobytes.get()
        }

        fn paused(&self) -> bool {
            self.paused.get()
        }

        fn step(&self, _: i32) -> bool {
            let started = Instant::now();
            while Instant::now() == started {
                std::hint::spin_loop();
            }
            self.steps.set(self.steps.get() + 1);
            self.paused.set(false);
            if self.steps.get().is_multiple_of(self.finish_after) {
                self.paused.set(true);
                return true;
            }
            false
        }
    }

    const PLENTY: Duration = Duration::from_secs(60);

    /// Before it has seen a cycle end, a collector leaves starting one to Luau; mid-cycle, it
    /// steps until the cycle ends; then it starts the next only once the heap has grown by half.
    #[test]
    fn a_collector_starts_a_cycle_only_for_growth() {
        let collector = Collector::default();
        let heap = Fake::new(1000, 4);
        assert_eq!(collector.collect(&heap, PLENTY).steps, 0, "Luau starts the first cycle");

        heap.paused.set(false);
        let first = collector.collect(&heap, PLENTY);
        assert!(first.finished);
        assert_eq!(first.steps, 4, "it stepped until the cycle ended, not until the budget did");

        heap.kilobytes.set(1499);
        assert_eq!(
            collector.collect(&heap, PLENTY).steps,
            0,
            "a third more is not yet half again"
        );
        heap.kilobytes.set(1500);
        let second = collector.collect(&heap, PLENTY);
        assert!(second.finished && second.steps == 4, "half again starts one: {second:?}");
    }

    /// The budget bounds the work: a nanosecond of it is one step, and the cycle is carried on
    /// from where it was the next time; a budget of nothing does nothing at all.
    #[test]
    fn the_budget_bounds_the_work_and_the_cycle_carries_on() {
        let collector = Collector::default();
        let heap = Fake::new(1000, 3);
        heap.paused.set(false);
        for step in 1..=2 {
            let collection = collector.collect(&heap, Duration::from_nanos(1));
            assert_eq!((collection.steps, collection.finished), (1, false), "step {step}");
        }
        let last = collector.collect(&heap, Duration::from_nanos(1));
        assert!(last.finished && last.steps == 1);
        heap.paused.set(false);
        assert_eq!(collector.collect(&heap, Duration::ZERO).steps, 0);
    }

    /// A cycle Luau's own steps finished between two collections still counts: the heap it left
    /// is the one the next cycle waits to grow from.
    #[test]
    fn a_cycle_luau_finished_is_where_growth_counts_from() {
        let collector = Collector::default();
        let heap = Fake::new(1000, 100);
        heap.paused.set(false);
        assert!(!collector.collect(&heap, Duration::from_nanos(1)).finished);
        // Luau finishes it, leaving 2000 KB.
        heap.paused.set(true);
        heap.kilobytes.set(2000);
        assert_eq!(collector.collect(&heap, PLENTY).steps, 0);
        heap.kilobytes.set(2999);
        assert_eq!(collector.collect(&heap, PLENTY).steps, 0, "growth counts from 2000 KB");
        heap.kilobytes.set(3000);
        assert!(collector.collect(&heap, PLENTY).steps > 0);
    }

    #[test]
    fn a_budget_is_capped_and_a_reset_forgets_it() {
        let collector = Collector::default();
        assert_eq!(collector.budget(), DEFAULT_BUDGET);
        collector.set_budget(Duration::from_secs(5));
        assert_eq!(collector.budget(), MAX_BUDGET);
        collector.reset();
        assert_eq!(collector.budget(), DEFAULT_BUDGET);
        // And the heap it knew: a new runtime's first cycle is Luau's to start again.
        let heap = Fake::new(1000, 2);
        heap.paused.set(false);
        assert!(collector.collect(&heap, PLENTY).finished);
        heap.kilobytes.set(5000);
        assert!(collector.collect(&heap, PLENTY).steps > 0, "grown fivefold, it starts one");
        collector.reset();
        heap.paused.set(true);
        assert_eq!(collector.collect(&heap, PLENTY).steps, 0, "after a reset it waits for Luau");
    }

    /// A soft budget: a collection that ran past it says so.
    #[test]
    fn a_collection_past_its_budget_overran() {
        let collector = Collector::default();
        let heap = Fake::new(1000, 1000);
        heap.paused.set(false);
        let collection = collector.collect(&heap, Duration::from_nanos(1));
        assert_eq!(collection.budget, Duration::from_nanos(1));
        assert!(collection.spent > collection.budget, "the step that ran it out finished");
        let ms = Duration::from_millis(1);
        let past = |spent| {
            Collection {
                steps: 1,
                finished: false,
                spent,
                budget: ms,
            }
            .overran()
        };
        assert!(!past(ms * 3 / 2), "a step past its budget is the soft budget at work");
        assert!(past(ms * 3), "three times it is an overrun");
        assert!(!collector.collect(&heap, Duration::ZERO).overran());
    }
}
