//! Profiler hooks: what the runtime measures, attributed to the domain, generation, context, and
//! worker that paid for it. The runtime records; the host aggregates and presents.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::runtime::context::Phase;
use crate::runtime::diagnostics::Attribution;
use crate::runtime::ids::{CapabilityId, ContextId, WorkerId};

/// Which way a worker message went, seen from the domain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// From the domain to the worker.
    ToWorker,
    /// From the worker to the domain.
    FromWorker,
}

/// One measured thing. Every variant carries the attribution of whoever paid for it.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ProfileEvent {
    /// A domain's primary runtime was created from its plan.
    DomainCreated {
        /// Where.
        attribution: Attribution,
        /// How long the runtime took to build and set up.
        duration: Duration,
    },
    /// A domain ran its startup phase.
    DomainStarted {
        /// Where.
        attribution: Attribution,
        /// How long the startup phase took, handlers included.
        duration: Duration,
    },
    /// One context handler ran.
    ContextCall {
        /// Where, with the context set.
        attribution: Attribution,
        /// The phase it ran in.
        phase: Phase,
        /// How long the event waited in the queue before dispatch; zero for a broadcast phase.
        queue_delay: Duration,
        /// How long the handler ran.
        duration: Duration,
        /// Whether it raised an error.
        failed: bool,
    },
    /// An event for an earlier generation was discarded.
    StaleEventDiscarded {
        /// Where.
        attribution: Attribution,
        /// The context it was for, if it named one.
        context: Option<ContextId>,
    },
    /// A worker's runtime was created and its module loaded.
    WorkerSpawned {
        /// Where, with the worker set.
        attribution: Attribution,
        /// How long from spawn to ready.
        duration: Duration,
    },
    /// One message crossed a worker boundary.
    WorkerMessage {
        /// Where, with the worker set.
        attribution: Attribution,
        /// Which way.
        direction: Direction,
        /// The message's size, as [`Message::size_hint`](crate::runtime::Message::size_hint) counts it.
        bytes: usize,
    },
    /// A worker's handler ran for one message.
    WorkerCall {
        /// Where, with the worker set.
        attribution: Attribution,
        /// How long the message waited in the worker's inbox.
        queue_delay: Duration,
        /// How long the handler ran.
        duration: Duration,
        /// Whether it raised an error.
        failed: bool,
    },
    /// A capability check was refused.
    CapabilityDenied {
        /// Where.
        attribution: Attribution,
        /// Which capability.
        capability: CapabilityId,
    },
    /// A domain was torn down.
    Teardown {
        /// Where.
        attribution: Attribution,
        /// How long the teardown took, shutdown handlers and worker joins included.
        duration: Duration,
        /// How many workers were stopped.
        workers_stopped: usize,
    },
}

impl ProfileEvent {
    /// Whose event it is.
    #[must_use]
    pub fn attribution(&self) -> Attribution {
        match self {
            ProfileEvent::DomainCreated { attribution, .. }
            | ProfileEvent::DomainStarted { attribution, .. }
            | ProfileEvent::ContextCall { attribution, .. }
            | ProfileEvent::StaleEventDiscarded { attribution, .. }
            | ProfileEvent::WorkerSpawned { attribution, .. }
            | ProfileEvent::WorkerMessage { attribution, .. }
            | ProfileEvent::WorkerCall { attribution, .. }
            | ProfileEvent::CapabilityDenied { attribution, .. }
            | ProfileEvent::Teardown { attribution, .. } => *attribution,
        }
    }

    /// The worker the event belongs to, if any.
    #[must_use]
    pub fn worker(&self) -> Option<WorkerId> {
        self.attribution().worker
    }
}

/// Where a domain's profile events go. Workers record from their own threads.
pub trait ProfileSink: Send + Sync + 'static {
    /// Receives one event.
    fn record(&self, event: ProfileEvent);
}

/// A sink that keeps every event.
#[derive(Debug, Default)]
pub struct CollectedProfile {
    events: Mutex<Vec<ProfileEvent>>,
}

impl CollectedProfile {
    /// An empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything recorded so far, in order.
    #[must_use]
    pub fn events(&self) -> Vec<ProfileEvent> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl ProfileSink for CollectedProfile {
    fn record(&self, event: ProfileEvent) {
        self.events.lock().unwrap_or_else(PoisonError::into_inner).push(event);
    }
}

/// A sink that drops everything: the default when a host does not profile.
#[derive(Debug, Default)]
pub struct DiscardProfile;

impl ProfileSink for DiscardProfile {
    fn record(&self, _: ProfileEvent) {}
}

/// Counters a domain keeps for its whole life, readable at any time
/// ([`ScriptDomain::stats`](crate::runtime::ScriptDomain::stats)). Shared with worker threads, so they
/// count what they send.
#[derive(Debug, Default)]
pub struct Counters {
    pub(crate) context_calls: AtomicU64,
    pub(crate) context_failures: AtomicU64,
    pub(crate) fixed_updates: AtomicU64,
    pub(crate) updates: AtomicU64,
    pub(crate) events_delivered: AtomicU64,
    pub(crate) stale_discarded: AtomicU64,
    pub(crate) capability_denials: AtomicU64,
    pub(crate) workers_spawned: AtomicU64,
    pub(crate) worker_failures: AtomicU64,
    pub(crate) messages_to_workers: AtomicU64,
    pub(crate) messages_from_workers: AtomicU64,
    pub(crate) bytes_to_workers: AtomicU64,
    pub(crate) bytes_from_workers: AtomicU64,
}

impl Counters {
    pub(crate) fn add(counter: &AtomicU64, amount: u64) {
        counter.fetch_add(amount, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> DomainStats {
        let read = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        DomainStats {
            context_calls: read(&self.context_calls),
            context_failures: read(&self.context_failures),
            fixed_updates: read(&self.fixed_updates),
            updates: read(&self.updates),
            events_delivered: read(&self.events_delivered),
            stale_discarded: read(&self.stale_discarded),
            capability_denials: read(&self.capability_denials),
            workers_spawned: read(&self.workers_spawned),
            worker_failures: read(&self.worker_failures),
            messages_to_workers: read(&self.messages_to_workers),
            messages_from_workers: read(&self.messages_from_workers),
            bytes_to_workers: read(&self.bytes_to_workers),
            bytes_from_workers: read(&self.bytes_from_workers),
        }
    }
}

/// A domain's counters at one moment.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DomainStats {
    /// Context handler calls, failed ones included.
    pub context_calls: u64,
    /// Context handler calls that raised.
    pub context_failures: u64,
    /// Fixed-update steps run.
    pub fixed_updates: u64,
    /// Update phases run.
    pub updates: u64,
    /// Events delivered to a context handler.
    pub events_delivered: u64,
    /// Events discarded because their generation had passed.
    pub stale_discarded: u64,
    /// Capability checks refused.
    pub capability_denials: u64,
    /// Workers spawned over the domain's life.
    pub workers_spawned: u64,
    /// Worker failures reported.
    pub worker_failures: u64,
    /// Messages sent to workers.
    pub messages_to_workers: u64,
    /// Messages received from workers.
    pub messages_from_workers: u64,
    /// Bytes sent to workers, by size hint.
    pub bytes_to_workers: u64,
    /// Bytes received from workers, by size hint.
    pub bytes_from_workers: u64,
}
