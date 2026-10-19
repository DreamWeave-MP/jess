//! Script contexts: when a body of code may run.
//!
//! A context is declared by the host with a [`ContextSpec`]: a
//! name, the scheduler [`Phase`]s it may run in, a priority, an optional subset of the domain's
//! capabilities, and an optional execution budget. Script code then binds handlers to the
//! phases the context schedules. When the code may run (the context) and what it may do (its
//! capabilities) are separate fields on purpose.
//!
//! The phases are generic scheduler entries. The kernel has no `Player` or `Menu` phase, and
//! never will: a layer that wants `world.pre-physics` runs [`Phase::OnDemand`] or posts an
//! event with its own meaning.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use l3i::value::Function;

use crate::runtime::capability::CapabilitySet;
use crate::runtime::ids::ContextId;

/// A generic scheduler phase: a moment at which the host may enter script code.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[non_exhaustive]
pub enum Phase {
    /// Once, after the domain's code has loaded.
    Load,
    /// Once, when the domain starts running.
    Startup,
    /// Zero or more fixed-size steps per advance; the payload is the step in seconds.
    FixedUpdate,
    /// Once per advance; the payload is the elapsed time in seconds.
    Update,
    /// After update; the payload is the elapsed time in seconds.
    LateUpdate,
    /// When the host delivers input.
    Input,
    /// Before the host renders a frame that asked for script preparation.
    RenderPrepare,
    /// When a message for the context arrives.
    Message,
    /// When a timer the context owns fires.
    Timer,
    /// When a queued event for the context is delivered; the payload is the event's.
    Event,
    /// Whenever the host or a layer above asks, with whatever meaning it gives the payload.
    OnDemand,
    /// Once, when the domain stops or restarts.
    Shutdown,
}

impl Phase {
    /// Every phase, in declaration order.
    pub const ALL: [Phase; 12] = [
        Phase::Load,
        Phase::Startup,
        Phase::FixedUpdate,
        Phase::Update,
        Phase::LateUpdate,
        Phase::Input,
        Phase::RenderPrepare,
        Phase::Message,
        Phase::Timer,
        Phase::Event,
        Phase::OnDemand,
        Phase::Shutdown,
    ];

    /// The phase's name as scripts spell it: the key of its handler in `context.bind`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Phase::Load => "load",
            Phase::Startup => "startup",
            Phase::FixedUpdate => "fixedUpdate",
            Phase::Update => "update",
            Phase::LateUpdate => "lateUpdate",
            Phase::Input => "input",
            Phase::RenderPrepare => "renderPrepare",
            Phase::Message => "message",
            Phase::Timer => "timer",
            Phase::Event => "event",
            Phase::OnDemand => "onDemand",
            Phase::Shutdown => "shutdown",
        }
    }

    /// The phase named `name`: the camelCase spelling scripts use, or the hyphenated one
    /// manifests use (`fixed-update`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Phase> {
        Phase::ALL.into_iter().find(|phase| phase.name() == name).or(match name {
            "fixed-update" => Some(Phase::FixedUpdate),
            "late-update" => Some(Phase::LateUpdate),
            "render-prepare" => Some(Phase::RenderPrepare),
            "on-demand" => Some(Phase::OnDemand),
            _ => None,
        })
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// An execution budget for one context or worker, enforced by l3i's watchdog at Luau
/// safepoints. Zero disables a limit.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Budget {
    /// Wall-clock time one handler call may take.
    pub execution_time: Duration,
    /// Heap ceiling of the Luau state in bytes, polled, not a hard quota (see
    /// `l3i::runtime::RuntimeBuilder::memory_limit`).
    pub memory_bytes: usize,
}

impl Budget {
    /// A budget of `execution_time` per handler call and no heap ceiling.
    #[must_use]
    pub const fn time(execution_time: Duration) -> Budget {
        Budget {
            execution_time,
            memory_bytes: 0,
        }
    }

    pub(crate) const fn limits(self) -> l3i::runtime::Limits {
        l3i::runtime::Limits {
            execution_time: self.execution_time,
            memory_bytes: self.memory_bytes,
        }
    }
}

/// Fixed-step scheduling: how long a step is, and how many an overloaded advance may run to
/// catch up before the rest of the backlog is dropped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FixedStep {
    /// The length of one fixed step.
    pub step: Duration,
    /// The most steps one advance runs. Time beyond that is discarded, so a stalled host never
    /// owes an unbounded burst of ticks.
    pub max_catch_up: u32,
}

/// What the host declares about a context before any code binds to it.
#[derive(Clone, Debug)]
pub struct ContextSpec {
    pub(crate) name: String,
    pub(crate) schedule: Vec<Phase>,
    pub(crate) capabilities: Option<CapabilitySet>,
    pub(crate) priority: i32,
    pub(crate) budget: Option<Budget>,
}

impl ContextSpec {
    /// A context named `name` that schedules nothing yet, inherits the domain's whole grant,
    /// and has priority 0 and no budget of its own.
    #[must_use]
    pub fn new(name: impl Into<String>) -> ContextSpec {
        ContextSpec {
            name: name.into(),
            schedule: Vec::new(),
            capabilities: None,
            priority: 0,
            budget: None,
        }
    }

    /// The phases the context may run in. A handler bound to any other phase is refused.
    #[must_use]
    pub fn schedule(mut self, phases: impl IntoIterator<Item = Phase>) -> ContextSpec {
        self.schedule = phases.into_iter().collect();
        self.schedule.sort_unstable();
        self.schedule.dedup();
        self
    }

    /// The capabilities the context holds: an explicit subset of the domain's grant. Asking for
    /// more than the domain holds fails registration.
    #[must_use]
    pub fn capabilities(mut self, capabilities: CapabilitySet) -> ContextSpec {
        self.capabilities = Some(capabilities);
        self
    }

    /// Where the context runs within a phase: lower first, registration order among equals.
    #[must_use]
    pub const fn priority(mut self, priority: i32) -> ContextSpec {
        self.priority = priority;
        self
    }

    /// The execution budget of each handler call.
    #[must_use]
    pub const fn budget(mut self, budget: Budget) -> ContextSpec {
        self.budget = Some(budget);
        self
    }
}

/// A context's handler calls so far.
#[derive(Debug, Default)]
pub struct ContextStats {
    pub(crate) calls: Cell<u64>,
    pub(crate) failures: Cell<u64>,
    pub(crate) time: Cell<Duration>,
}

/// One registered context, as the scheduler keeps it.
pub struct ContextRecord {
    pub(crate) id: ContextId,
    pub(crate) name: String,
    pub(crate) schedule: Vec<Phase>,
    pub(crate) priority: i32,
    /// Registration order, the tie-break among equal priorities.
    pub(crate) order: u64,
    pub(crate) capabilities: Arc<CapabilitySet>,
    pub(crate) budget: Option<Budget>,
    /// The handlers bound in the current generation, pinned on the domain's runtime.
    pub(crate) handlers: BTreeMap<Phase, Function>,
    /// `source:line` of the script code that bound each handler, when it was bound from Luau.
    pub(crate) sources: BTreeMap<Phase, Rc<str>>,
    /// The named-event handlers bound in the current generation, by event name. They run in
    /// [`Phase::Event`], apart from the phase's own handler, which takes the raw events.
    pub(crate) events: BTreeMap<String, Function>,
    /// Where each named-event handler was defined or bound, when known.
    pub(crate) event_sources: BTreeMap<String, Rc<str>>,
    /// What its handler calls have cost, shared with each dispatch so a call counts itself
    /// without looking the context up.
    pub(crate) stats: Rc<ContextStats>,
}

impl ContextRecord {
    pub(crate) fn info(&self) -> ContextInfo {
        ContextInfo {
            id: self.id,
            name: self.name.clone(),
            schedule: self.schedule.clone(),
            priority: self.priority,
            capabilities: Arc::clone(&self.capabilities),
            budget: self.budget,
            bound: self.handlers.keys().copied().collect(),
            events: self.events.keys().cloned().collect(),
            calls: self.stats.calls.get(),
            failures: self.stats.failures.get(),
            time: self.stats.time.get(),
        }
    }
}

/// A registered context at one moment: its declaration, what is bound, and what it has cost.
#[derive(Clone, Debug)]
pub struct ContextInfo {
    /// The context's id.
    pub id: ContextId,
    /// Its name, unique in its domain.
    pub name: String,
    /// The phases it may run in.
    pub schedule: Vec<Phase>,
    /// Its priority within a phase.
    pub priority: i32,
    /// The capabilities it holds.
    pub capabilities: Arc<CapabilitySet>,
    /// Its per-call execution budget, if it has one of its own.
    pub budget: Option<Budget>,
    /// The phases with a handler bound in the current generation.
    pub bound: Vec<Phase>,
    /// The named events with a handler bound in the current generation, sorted.
    pub events: Vec<String>,
    /// Handler calls so far.
    pub calls: u64,
    /// Handler calls that raised.
    pub failures: u64,
    /// Total handler time so far.
    pub time: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_have_one_script_spelling_and_a_manifest_alias() {
        for phase in Phase::ALL {
            assert_eq!(Phase::from_name(phase.name()), Some(phase));
            assert_eq!(phase.to_string(), phase.name());
        }
        assert_eq!(Phase::from_name("fixed-update"), Some(Phase::FixedUpdate));
        assert_eq!(Phase::from_name("render-prepare"), Some(Phase::RenderPrepare));
        assert_eq!(Phase::from_name("menu"), None);
    }

    #[test]
    fn a_schedule_is_a_sorted_set() {
        let spec = ContextSpec::new("main").schedule([Phase::Update, Phase::Startup, Phase::Update]);
        assert_eq!(spec.schedule, vec![Phase::Startup, Phase::Update]);
    }
}
