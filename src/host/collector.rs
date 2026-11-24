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
        self.budget
            .store(nanoseconds(budget.min(MAX_BUDGET)), Ordering::Relaxed);
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
        if budget.is_zero() {
            return Collection::default();
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
                return Collection::default();
            }
        }
        let began = Instant::now();
        let mut collection = Collection::default();
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
