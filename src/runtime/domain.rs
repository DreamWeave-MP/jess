//! The [`ScriptDomain`]: one isolated Luau authority, its lifetime, and its scheduler.
//!
//! A domain owns one primary l3i `Runtime` and everything attached to it: the contexts whose
//! handlers run on it, the workers it spawned, the native resources services created on its
//! behalf, and the queue of events waiting for it. It has an explicit lifecycle
//! ([`DomainState`]), a stable [`DomainId`], and a [`Generation`] that moves on every restart,
//! so work queued for old code never reaches new code.
//!
//! Every entry into script code goes through the scheduler here: a broadcast phase
//! ([`ScriptDomain::run_phase`], [`ScriptDomain::advance`]) or a queued event
//! ([`ScriptDomain::drain_events`]). Each handler call runs under the context's
//! [`Authority`](crate::runtime::Authority), its budget, an l3i call scope carrying its [`ContextId`],
//! and a timer, and a Luau error in it becomes a [`Diagnostic`] and a [`RuntimeError::Script`],
//! never a panic.
//!
//! A domain lives on the thread that created it: l3i's `Runtime` is not `Send`. Other threads
//! reach it through a [`DomainMailbox`], which only queues.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use l3i::Runtime;
use l3i::extension::RuntimePlan;
use l3i::runtime::{CallContext, CallKind, MemoryCategory};
use l3i::value::Function;

use crate::runtime::capability::{Authority, AuthorityGuard, CapabilitySet, Grant, Observer, WORKER_SPAWN};
use crate::runtime::context::{Budget, ContextInfo, ContextRecord, ContextSpec, FixedStep, Phase};
use crate::runtime::diagnostics::{Attribution, Diagnostic, DiagnosticKind, DiagnosticSink, DiscardDiagnostics, Severity};
use crate::runtime::error::{RuntimeError, script_message};
use crate::runtime::ids::{CapabilityId, ContextId, DomainId, Generation, ResourceId, WorkerId};
use crate::runtime::log::LogSink;
use crate::runtime::message::Message;
use crate::runtime::profile::{Counters, Direction, DiscardProfile, DomainStats, ProfileEvent, ProfileSink};
use crate::runtime::resources::{OwnedResource, ResourceRegistry};
use crate::runtime::service::Services;
use crate::runtime::worker::{Launch, WorkerInfo, WorkerRecord, WorkerSpec, WorkerState};

/// Makes the runtimes of a domain: its primary one, on the domain's thread, and one per worker,
/// each on that worker's lane.
///
/// It is a factory and not a plan because an l3i `RuntimePlan` is `Rc`-based and cannot leave
/// its thread, so every lane needs the plan made (or cached) there. [`FromPlan`] wraps a function
/// that returns the calling thread's plan; a closure that returns a finished `Runtime` works
/// too, for hosts that set up globals after `Runtime::from_plan`.
pub trait RuntimeFactory: Send + Sync + 'static {
    /// A new runtime, on the calling thread.
    ///
    /// # Errors
    ///
    /// Whatever building the runtime fails with: a plan that does not finalize, a VM that
    /// cannot be set up.
    fn create(&self) -> l3i::Result<Runtime>;
}

impl<F> RuntimeFactory for F
where
    F: Fn() -> l3i::Result<Runtime> + Send + Sync + 'static,
{
    fn create(&self) -> l3i::Result<Runtime> {
        self()
    }
}

/// A [`RuntimeFactory`] over a function returning the calling thread's `RuntimePlan`, such as
/// `jess::lua::plan`: each runtime is `Runtime::from_plan` of it, and nothing more.
#[derive(Clone, Copy, Debug)]
pub struct FromPlan<F>(pub F);

impl<F> RuntimeFactory for FromPlan<F>
where
    F: Fn() -> l3i::Result<Rc<RuntimePlan>> + Send + Sync + 'static,
{
    fn create(&self) -> l3i::Result<Runtime> {
        Runtime::from_plan(&(self.0)()?)
    }
}

/// Host setup run on every runtime a domain creates, primary and worker alike, right after the
/// factory returns it: the place to bind a native service into each state.
pub type SetupHook = Arc<dyn Fn(&Runtime) -> l3i::Result<()> + Send + Sync>;

/// Where a domain is in its life.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DomainState {
    /// Its runtime exists; no code has loaded.
    Created,
    /// Its code is loading.
    Loading,
    /// Its code has loaded; it has not started.
    Ready,
    /// It is running: phases and events are dispatched.
    Running,
    /// It is being torn down.
    Stopping,
    /// It is torn down: no runtime, no workers, no contexts, no resources.
    Stopped,
    /// Its code failed to load. It can only be stopped.
    Failed,
}

impl fmt::Display for DomainState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DomainState::Created => "created",
            DomainState::Loading => "loading",
            DomainState::Ready => "ready",
            DomainState::Running => "running",
            DomainState::Stopping => "stopping",
            DomainState::Stopped => "stopped",
            DomainState::Failed => "failed",
        })
    }
}

/// What a host states to create a domain.
#[derive(Clone)]
pub struct DomainSpec {
    name: String,
    grant: Grant,
    parent: Option<DomainId>,
    diagnostics: Arc<dyn DiagnosticSink>,
    profiler: Arc<dyn ProfileSink>,
    logs: Option<Arc<dyn LogSink>>,
    fixed_step: Option<FixedStep>,
    budget: Option<Budget>,
    setup: Option<SetupHook>,
    services: Services,
}

impl fmt::Debug for DomainSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DomainSpec")
            .field("name", &self.name)
            .field("grant", &self.grant)
            .field("parent", &self.parent)
            .field("fixed_step", &self.fixed_step)
            .field("budget", &self.budget)
            .field("services", &self.services)
            .field("logs", &self.logs.is_some())
            .finish_non_exhaustive()
    }
}

impl DomainSpec {
    /// A domain named `name` (for people; the [`DomainId`] is its identity) holding `grant`.
    /// Diagnostics and profile events are discarded, there is no log stream, no fixed step,
    /// and no budget, until the builder says otherwise.
    #[must_use]
    pub fn new(name: impl Into<String>, grant: Grant) -> DomainSpec {
        DomainSpec {
            name: name.into(),
            grant,
            parent: None,
            diagnostics: Arc::new(DiscardDiagnostics),
            profiler: Arc::new(DiscardProfile),
            logs: None,
            fixed_step: None,
            budget: None,
            setup: None,
            services: Services::new(),
        }
    }

    /// The domain that launched this one. Identity only: nothing is shared with it.
    #[must_use]
    pub const fn parent(mut self, parent: DomainId) -> DomainSpec {
        self.parent = Some(parent);
        self
    }

    /// Where the domain's diagnostics go.
    #[must_use]
    pub fn diagnostics(mut self, sink: Arc<dyn DiagnosticSink>) -> DomainSpec {
        self.diagnostics = sink;
        self
    }

    /// Where the domain's profile events go.
    #[must_use]
    pub fn profiler(mut self, sink: Arc<dyn ProfileSink>) -> DomainSpec {
        self.profiler = sink;
        self
    }

    /// The domain's log stream: every record its contexts and workers log goes to `sink`,
    /// attributed. Without one, a logging module keeps its runtime's own destination.
    #[must_use]
    pub fn logs(mut self, sink: Arc<dyn LogSink>) -> DomainSpec {
        self.logs = Some(sink);
        self
    }

    /// Fixed-step scheduling for [`ScriptDomain::advance`].
    #[must_use]
    pub const fn fixed_step(mut self, fixed_step: FixedStep) -> DomainSpec {
        self.fixed_step = Some(fixed_step);
        self
    }

    /// The execution budget of every handler call whose context has none of its own.
    #[must_use]
    pub const fn budget(mut self, budget: Budget) -> DomainSpec {
        self.budget = Some(budget);
        self
    }

    /// Host setup for every runtime the domain creates.
    #[must_use]
    pub fn setup(mut self, setup: impl Fn(&Runtime) -> l3i::Result<()> + Send + Sync + 'static) -> DomainSpec {
        self.setup = Some(Arc::new(setup));
        self
    }

    /// Attaches a service the domain carries for its code: native state a binding finds by
    /// type with [`crate::runtime::service`] while it runs under the domain's authority. One per type;
    /// attaching a second replaces the first. Services are fixed at creation and survive every
    /// restart; a worker sees one only if its [`WorkerSpec::share`] names the type.
    #[must_use]
    pub fn service<T: Send + Sync + 'static>(mut self, service: Arc<T>) -> DomainSpec {
        self.services.insert(service);
        self
    }
}

/// Who a queued event is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventTarget {
    /// One context's event handler.
    Context(ContextId),
    /// Every context that schedules the event phase, in scheduler order.
    All,
}

/// What a queued event is. A raw event goes to contexts' `event` handlers; a named one goes to
/// the handlers bound to its name. The two never meet: a name is not a target, and a raw event
/// has no name, so neither can be dressed up as the other.
#[derive(Clone, Debug)]
enum Delivery {
    Raw(EventTarget),
    Named(String),
}

struct Envelope {
    delivery: Delivery,
    generation: Generation,
    payload: Message,
    posted: Instant,
}

/// The domain a [`DomainMailbox`] posted to has stopped; the event was not queued.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MailboxClosed(pub DomainId);

impl fmt::Display for MailboxClosed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} has stopped; its mailbox is closed", self.0)
    }
}

impl std::error::Error for MailboxClosed {}

/// The way native code on any thread queues an event into a domain.
///
/// Posting never runs script code: the event waits for the domain's next
/// [`ScriptDomain::drain_events`], and it is stamped with the domain's generation at the moment
/// of posting, so an event posted before a restart is discarded after it.
#[derive(Clone, Debug)]
pub struct DomainMailbox {
    domain: DomainId,
    generation: Arc<AtomicU64>,
    sender: Sender<Envelope>,
}

impl DomainMailbox {
    /// The domain this mailbox belongs to.
    #[must_use]
    pub const fn domain(&self) -> DomainId {
        self.domain
    }

    /// Queues `payload` for `target`, stamped with the domain's current generation.
    ///
    /// # Errors
    ///
    /// [`MailboxClosed`] once the domain has stopped.
    pub fn post(&self, target: EventTarget, payload: Message) -> Result<(), MailboxClosed> {
        self.post_for(target, Generation::from_raw(self.generation.load(Ordering::Acquire)), payload)
    }

    /// Queues `payload` for `target` as work of `generation`: for a native service that
    /// captured the generation when the work was requested and completes it later.
    ///
    /// # Errors
    ///
    /// [`MailboxClosed`] once the domain has stopped.
    pub fn post_for(&self, target: EventTarget, generation: Generation, payload: Message) -> Result<(), MailboxClosed> {
        self.send(Delivery::Raw(target), generation, payload)
    }

    /// Queues the named event `event` with `payload`, stamped with the domain's current
    /// generation. At the next drain it goes to every handler bound to that name (a managed
    /// module's `on.event`), in scheduler order, and to nothing else: never to a raw `event`
    /// handler, and never to a phase handler that happens to share the name.
    ///
    /// This is how one domain's events reach another: a host routes them here explicitly
    /// ([`ScriptDomain::route`]), and only [`Message`] payloads cross.
    ///
    /// # Errors
    ///
    /// [`MailboxClosed`] once the domain has stopped.
    pub fn post_named(&self, event: &str, payload: Message) -> Result<(), MailboxClosed> {
        self.post_named_for(event, Generation::from_raw(self.generation.load(Ordering::Acquire)), payload)
    }

    /// [`DomainMailbox::post_named`] as work of `generation`.
    ///
    /// # Errors
    ///
    /// [`MailboxClosed`] once the domain has stopped.
    pub fn post_named_for(&self, event: &str, generation: Generation, payload: Message) -> Result<(), MailboxClosed> {
        self.send(Delivery::Named(event.to_owned()), generation, payload)
    }

    fn send(&self, delivery: Delivery, generation: Generation, payload: Message) -> Result<(), MailboxClosed> {
        self.sender
            .send(Envelope {
                delivery,
                generation,
                payload,
                posted: Instant::now(),
            })
            .map_err(|_| MailboxClosed(self.domain))
    }
}

/// What one broadcast phase did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PhaseReport {
    /// The phase.
    pub phase: Phase,
    /// How many handlers ran, failed ones included.
    pub ran: usize,
    /// The handlers that raised, in the order they ran.
    pub failures: Vec<RuntimeError>,
}

/// What one drain of the event queue did.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct EventReport {
    /// Handler calls made.
    pub delivered: usize,
    /// Events discarded because their generation had passed.
    pub stale: usize,
    /// Events whose target has no event handler bound.
    pub unhandled: usize,
    /// The handlers that raised.
    pub failures: Vec<RuntimeError>,
}

/// What one [`ScriptDomain::advance`] did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TickReport {
    /// Fixed-update steps run.
    pub fixed_steps: u32,
    /// Whole fixed steps of backlog dropped because the advance was past its catch-up limit.
    pub dropped_steps: u64,
    /// The phases run, in order.
    pub phases: Vec<PhaseReport>,
    /// The event drain at the end of the advance.
    pub events: EventReport,
}

impl TickReport {
    /// Every handler failure of the advance, in the order they happened.
    #[must_use]
    pub fn failures(&self) -> Vec<&RuntimeError> {
        self.phases
            .iter()
            .flat_map(|phase| &phase.failures)
            .chain(&self.events.failures)
            .collect()
    }
}

/// What a teardown released.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TeardownReport {
    /// How long it took.
    pub duration: Duration,
    /// Shutdown handlers that raised.
    pub shutdown_failures: Vec<RuntimeError>,
    /// Queued events dropped undelivered.
    pub events_discarded: usize,
    /// Workers closed and joined.
    pub workers_stopped: usize,
    /// Owned resources released.
    pub resources_released: usize,
    /// Contexts removed.
    pub contexts_removed: usize,
}

/// One handler call the scheduler is about to make.
struct Dispatch {
    context: ContextId,
    handler: Function,
    /// Where the handler was bound, for diagnostics.
    source: Option<String>,
    /// The named event it handles, for diagnostics.
    event: Option<String>,
    capabilities: Arc<CapabilitySet>,
    budget: Option<Budget>,
}

/// The part of a domain its own runtime's bindings reach: everything but the runtime.
pub struct DomainCore {
    id: DomainId,
    name: String,
    parent: Option<DomainId>,
    grant: Grant,
    state: Cell<DomainState>,
    generation: Cell<Generation>,
    /// The generation again, where mailboxes on other threads can read it.
    shared_generation: Arc<AtomicU64>,
    factory: Arc<dyn RuntimeFactory>,
    setup: Option<SetupHook>,
    services: Arc<Services>,
    diagnostics: Arc<dyn DiagnosticSink>,
    profiler: Arc<dyn ProfileSink>,
    counters: Arc<Counters>,
    observer: Arc<Observer>,
    fixed_step: Option<FixedStep>,
    budget: Option<Budget>,
    accumulator: Cell<Duration>,
    contexts: RefCell<Vec<ContextRecord>>,
    next_order: Cell<u64>,
    workers: RefCell<Vec<WorkerRecord>>,
    resources: RefCell<ResourceRegistry>,
    queue: RefCell<Option<(Sender<Envelope>, Receiver<Envelope>)>>,
    /// Where the host routes named events its code emits, besides this domain's own handlers.
    routes: RefCell<Vec<(String, DomainMailbox)>>,
    /// The phase being dispatched, if the scheduler is inside script code.
    dispatching: Cell<Option<Phase>>,
}

/// Marks the scheduler as inside script code until dropped.
struct DispatchGuard<'a>(&'a DomainCore);

impl Drop for DispatchGuard<'_> {
    fn drop(&mut self) {
        self.0.dispatching.set(None);
    }
}

impl DomainCore {
    pub(crate) fn attribution(&self) -> Attribution {
        Attribution::domain(self.id, self.generation.get())
    }

    pub(crate) fn report(&self, diagnostic: Diagnostic) {
        self.diagnostics.report(diagnostic);
    }

    fn authority(&self, attribution: Attribution, capabilities: Arc<CapabilitySet>) -> Authority {
        Authority::new(
            attribution,
            capabilities,
            Arc::clone(&self.services),
            Some(Arc::clone(&self.observer)),
        )
    }

    fn lifecycle(&self, action: &'static str) -> RuntimeError {
        let error = RuntimeError::Lifecycle {
            domain: self.id,
            state: self.state.get(),
            action,
        };
        self.report(Diagnostic::new(
            self.attribution(),
            Severity::Error,
            DiagnosticKind::InvalidLifecycleTransition,
            error.to_string(),
        ));
        error
    }

    fn require_state(&self, allowed: &[DomainState], action: &'static str) -> Result<(), RuntimeError> {
        if allowed.contains(&self.state.get()) {
            Ok(())
        } else {
            Err(self.lifecycle(action))
        }
    }

    fn enter_dispatch(&self, phase: Phase, action: &'static str) -> Result<DispatchGuard<'_>, RuntimeError> {
        if let Some(running) = self.dispatching.get() {
            return Err(RuntimeError::Reentrant {
                domain: self.id,
                running,
                action,
            });
        }
        self.dispatching.set(Some(phase));
        Ok(DispatchGuard(self))
    }

    fn refuse_reentry(&self, action: &'static str) -> Result<(), RuntimeError> {
        self.dispatching.get().map_or(Ok(()), |running| {
            Err(RuntimeError::Reentrant {
                domain: self.id,
                running,
                action,
            })
        })
    }

    fn register_context(&self, spec: ContextSpec) -> Result<ContextId, RuntimeError> {
        self.require_state(
            &[
                DomainState::Created,
                DomainState::Loading,
                DomainState::Ready,
                DomainState::Running,
            ],
            "register a context in",
        )?;
        let capabilities = self.grant.subset(self.id, spec.capabilities)?;
        let mut contexts = self.contexts.borrow_mut();
        if contexts.iter().any(|context| context.name == spec.name) {
            return Err(RuntimeError::Native(l3i::Error::logic(format!(
                "{} already has a context named '{}'",
                self.id, spec.name
            ))));
        }
        let id = ContextId::allocate();
        let order = self.next_order.get();
        self.next_order.set(order + 1);
        contexts.push(ContextRecord {
            id,
            name: spec.name,
            schedule: spec.schedule,
            priority: spec.priority,
            order,
            capabilities,
            budget: spec.budget,
            handlers: std::collections::BTreeMap::new(),
            sources: std::collections::BTreeMap::new(),
            events: std::collections::BTreeMap::new(),
            event_sources: std::collections::BTreeMap::new(),
            calls: 0,
            failures: 0,
            time: Duration::ZERO,
        });
        Ok(id)
    }

    /// Binds `handlers` to the context named `name`; all of them or, on the first one the
    /// context does not schedule, none. `source` is the script line that bound them.
    pub(crate) fn bind_handlers(
        &self,
        name: &str,
        handlers: Vec<(Phase, Function)>,
        source: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.require_state(
            &[
                DomainState::Created,
                DomainState::Loading,
                DomainState::Ready,
                DomainState::Running,
            ],
            "bind handlers in",
        )?;
        let mut contexts = self.contexts.borrow_mut();
        let Some(context) = contexts.iter_mut().find(|context| context.name == name) else {
            return Err(RuntimeError::UnknownContext {
                domain: self.id,
                name: name.to_owned(),
            });
        };
        if let Some((phase, _)) = handlers.iter().find(|(phase, _)| !context.schedule.contains(phase)) {
            return Err(RuntimeError::UnscheduledPhase {
                context: context.id,
                phase: *phase,
            });
        }
        for (phase, handler) in handlers {
            match source.filter(|source| !source.is_empty()) {
                Some(source) => context.sources.insert(phase, source.to_owned()),
                None => context.sources.remove(&phase),
            };
            context.handlers.insert(phase, handler);
        }
        Ok(())
    }

    /// Declares a context for a host that declares contexts as its code loads, once it has seen
    /// what the code asks for: registers it, or, for a name the host declared in an earlier
    /// generation, gives that context this declaration, keeping its id, order, and counters.
    pub(crate) fn declare_context(&self, spec: ContextSpec) -> Result<ContextId, RuntimeError> {
        let existing = self
            .contexts
            .borrow()
            .iter()
            .find(|context| context.name == spec.name)
            .map(|context| context.id);
        let Some(id) = existing else {
            return self.register_context(spec);
        };
        let capabilities = self.grant.subset(self.id, spec.capabilities)?;
        let mut contexts = self.contexts.borrow_mut();
        if let Some(context) = contexts.iter_mut().find(|context| context.id == id) {
            context.schedule = spec.schedule;
            context.priority = spec.priority;
            context.capabilities = capabilities;
            context.budget = spec.budget;
        }
        Ok(id)
    }

    /// Calls `function` as a handler of `context` would be called in `phase`: under the context's
    /// authority and budget, timed, counted, and with a failure attributed to it. For a host's own
    /// call into code a context owns, outside any phase broadcast: `invoke` makes the call with
    /// whatever arguments the host has. A failure is returned, not reported: the host makes it
    /// part of a failure of its own, with what it knows about the call, and reports that.
    pub(crate) fn call_in_context(
        &self,
        runtime: &Runtime,
        context: ContextId,
        phase: Phase,
        source: Option<String>,
        function: Function,
        invoke: impl FnOnce(&Function) -> l3i::Result<()>,
    ) -> Result<(), RuntimeError> {
        let (capabilities, budget) = {
            let contexts = self.contexts.borrow();
            let record =
                contexts
                    .iter()
                    .find(|record| record.id == context)
                    .ok_or_else(|| RuntimeError::UnknownContext {
                        domain: self.id,
                        name: context.to_string(),
                    })?;
            (Arc::clone(&record.capabilities), record.budget)
        };
        let target = Dispatch {
            context,
            handler: function,
            source,
            event: None,
            capabilities,
            budget,
        };
        self.call(runtime, &target, phase, Duration::ZERO, false, invoke)
    }

    /// Binds `handlers` to named events of the context named `name`: each an event name, its
    /// handler, and where the handler came from. The context must schedule [`Phase::Event`], the
    /// phase named events are delivered in. All of them or none.
    pub(crate) fn bind_event_handlers(
        &self,
        name: &str,
        handlers: Vec<(String, Function, Option<String>)>,
    ) -> Result<(), RuntimeError> {
        self.require_state(
            &[
                DomainState::Created,
                DomainState::Loading,
                DomainState::Ready,
                DomainState::Running,
            ],
            "bind handlers in",
        )?;
        let mut contexts = self.contexts.borrow_mut();
        let Some(context) = contexts.iter_mut().find(|context| context.name == name) else {
            return Err(RuntimeError::UnknownContext {
                domain: self.id,
                name: name.to_owned(),
            });
        };
        if !handlers.is_empty() && !context.schedule.contains(&Phase::Event) {
            return Err(RuntimeError::UnscheduledPhase {
                context: context.id,
                phase: Phase::Event,
            });
        }
        for (event, handler, source) in handlers {
            match source.filter(|source| !source.is_empty()) {
                Some(source) => context.event_sources.insert(event.clone(), source),
                None => context.event_sources.remove(&event),
            };
            context.events.insert(event, handler);
        }
        Ok(())
    }

    fn dispatch_of(context: &ContextRecord, phase: Phase) -> Option<Dispatch> {
        Some(Dispatch {
            context: context.id,
            handler: context.handlers.get(&phase)?.clone(),
            source: context.sources.get(&phase).cloned(),
            event: None,
            capabilities: Arc::clone(&context.capabilities),
            budget: context.budget,
        })
    }

    /// The handlers to call for the named event `event`: every context with a handler bound to
    /// that name, by priority, then registration order.
    fn named_targets(&self, event: &str) -> Vec<Dispatch> {
        let contexts = self.contexts.borrow();
        let mut order: Vec<&ContextRecord> = contexts.iter().filter(|context| context.events.contains_key(event)).collect();
        order.sort_by_key(|context| (context.priority, context.order));
        order
            .into_iter()
            .filter_map(|context| {
                Some(Dispatch {
                    context: context.id,
                    handler: context.events.get(event)?.clone(),
                    source: context.event_sources.get(event).cloned(),
                    event: Some(event.to_owned()),
                    capabilities: Arc::clone(&context.capabilities),
                    budget: context.budget,
                })
            })
            .collect()
    }

    /// Queues the named event `event` from the domain's own code: stamped with the current
    /// generation, delivered at the next drain, never synchronously. Then the host's routes for
    /// the name forward a copy to each mailbox it named; a stopped recipient is reported, not
    /// raised, since it isn't the emitter's doing.
    pub(crate) fn emit(&self, event: &str, payload: Message) -> Result<(), RuntimeError> {
        {
            let queue = self.queue.borrow();
            let Some((sender, _)) = queue.as_ref() else {
                return Err(self.lifecycle("emit an event in"));
            };
            for (name, mailbox) in self.routes.borrow().iter() {
                if name == event
                    && let Err(closed) = mailbox.post_named(event, payload.clone())
                {
                    self.report(Diagnostic::new(
                        self.attribution(),
                        Severity::Warning,
                        DiagnosticKind::TeardownDiscarded,
                        format!("the event '{event}' routed to {} was dropped: {closed}", closed.0),
                    ));
                }
            }
            // The receiver lives as long as the sender, in the same slot.
            drop(sender.send(Envelope {
                delivery: Delivery::Named(event.to_owned()),
                generation: self.generation.get(),
                payload,
                posted: Instant::now(),
            }));
        }
        Ok(())
    }

    /// The handlers to call for a broadcast of `phase`: every context that schedules it and
    /// has a handler bound, by priority, then registration order.
    fn targets(&self, phase: Phase) -> Vec<Dispatch> {
        let contexts = self.contexts.borrow();
        let mut order: Vec<&ContextRecord> = contexts
            .iter()
            .filter(|context| context.handlers.contains_key(&phase))
            .collect();
        order.sort_by_key(|context| (context.priority, context.order));
        order
            .into_iter()
            .filter_map(|context| Self::dispatch_of(context, phase))
            .collect()
    }

    fn target(&self, id: ContextId, phase: Phase) -> Option<Dispatch> {
        let contexts = self.contexts.borrow();
        Self::dispatch_of(contexts.iter().find(|context| context.id == id)?, phase)
    }

    fn deny(&self, capability: CapabilityId, attribution: Attribution) -> RuntimeError {
        let denied = crate::runtime::capability::CapabilityDenied { capability, attribution };
        self.observer.denied(&denied);
        RuntimeError::CapabilityDenied(denied)
    }

    /// Spawns a worker. `spawner` is the authority of the script code asking, which must hold
    /// [`WORKER_SPAWN`] and bounds the worker's capabilities; `None` is the host asking, bounded
    /// by the domain's grant.
    pub(crate) fn spawn_worker(&self, spec: WorkerSpec, spawner: Option<&Authority>) -> Result<WorkerId, RuntimeError> {
        let allowed = [
            DomainState::Created,
            DomainState::Loading,
            DomainState::Ready,
            DomainState::Running,
        ];
        if !allowed.contains(&self.state.get()) {
            let error = RuntimeError::Lifecycle {
                domain: self.id,
                state: self.state.get(),
                action: "spawn a worker in",
            };
            self.report(Diagnostic::new(
                self.attribution(),
                Severity::Error,
                DiagnosticKind::SpawnRefused,
                error.to_string(),
            ));
            return Err(error);
        }
        let holder = match spawner {
            Some(authority) => {
                let capability = CapabilityId::intern(WORKER_SPAWN)?;
                if !authority.has(capability) {
                    return Err(self.deny(capability, authority.attribution()));
                }
                Arc::clone(authority.capabilities())
            }
            None => Arc::clone(self.grant.capabilities()),
        };
        let capabilities = match &spec.capabilities {
            None => holder,
            Some(requested) => {
                let missing = requested.missing_from(&holder);
                if !missing.is_empty() {
                    let error = RuntimeError::OverDelegation {
                        grantor: crate::runtime::capability::Grantor::Domain(self.id),
                        missing,
                    };
                    self.report(Diagnostic::new(
                        self.attribution(),
                        Severity::Error,
                        DiagnosticKind::SpawnRefused,
                        error.to_string(),
                    ));
                    return Err(error);
                }
                Arc::new(requested.clone())
            }
        };
        let services = match self.services.shared(&spec.services) {
            Ok(services) => Arc::new(services),
            Err(service) => {
                let error = RuntimeError::UnknownService {
                    domain: self.id,
                    service,
                };
                self.report(Diagnostic::new(
                    self.attribution(),
                    Severity::Error,
                    DiagnosticKind::SpawnRefused,
                    error.to_string(),
                ));
                return Err(error);
            }
        };
        let id = WorkerId::allocate();
        let attribution = self.attribution().in_worker(id);
        let record = WorkerRecord::spawn(
            id,
            Launch {
                spec,
                attribution,
                capabilities,
                services,
                factory: Arc::clone(&self.factory),
                setup: self.setup.clone(),
                diagnostics: Arc::clone(&self.diagnostics),
                profiler: Arc::clone(&self.profiler),
                logs: self.observer.logs.clone(),
                counters: Arc::clone(&self.counters),
            },
        )?;
        Counters::add(&self.counters.workers_spawned, 1);
        self.workers.borrow_mut().push(record);
        Ok(id)
    }

    fn with_worker<R>(
        &self,
        id: WorkerId,
        body: impl FnOnce(&mut WorkerRecord) -> Result<R, RuntimeError>,
    ) -> Result<R, RuntimeError> {
        let mut workers = self.workers.borrow_mut();
        let worker = workers
            .iter_mut()
            .find(|worker| worker.id == id)
            .ok_or(RuntimeError::UnknownWorker {
                domain: self.id,
                worker: id,
            })?;
        body(worker)
    }

    pub(crate) fn send_to_worker(&self, id: WorkerId, message: Message) -> Result<(), RuntimeError> {
        let bytes = message.size_hint();
        let attribution = self.with_worker(id, |worker| worker.send(message).map(|()| worker.attribution))?;
        Counters::add(&self.counters.messages_to_workers, 1);
        Counters::add(&self.counters.bytes_to_workers, bytes as u64);
        self.profiler.record(ProfileEvent::WorkerMessage {
            attribution,
            direction: Direction::ToWorker,
            bytes,
        });
        Ok(())
    }

    pub(crate) fn receive_from_worker(
        &self,
        id: WorkerId,
        timeout: Option<Duration>,
    ) -> Result<Option<Message>, RuntimeError> {
        self.with_worker(id, |worker| worker.receive(timeout))
    }

    pub(crate) fn close_worker(&self, id: WorkerId) -> Result<(), RuntimeError> {
        self.with_worker(id, |worker| {
            worker.close();
            Ok(())
        })
    }

    pub(crate) fn join_worker(&self, id: WorkerId, timeout: Option<Duration>) -> Result<WorkerState, RuntimeError> {
        self.with_worker(id, |worker| worker.join(timeout))
    }

    /// Closes and joins every worker, then forgets them. Returns how many there were.
    fn stop_workers(&self) -> usize {
        let mut workers = std::mem::take(&mut *self.workers.borrow_mut());
        for worker in &workers {
            worker.close();
        }
        for worker in &mut workers {
            if let Err(error) = worker.join(None) {
                self.report(Diagnostic::new(
                    worker.attribution,
                    Severity::Error,
                    DiagnosticKind::WorkerFailed,
                    error.to_string(),
                ));
            }
            debug_assert!(worker.is_joined(), "a joined worker has no lane");
        }
        workers.len()
    }

    fn release_resources(&self) -> usize {
        let releases = self.resources.borrow_mut().take_all();
        let count = releases.len();
        for release in releases {
            release();
        }
        count
    }

    fn set_generation(&self, generation: Generation) {
        self.generation.set(generation);
        self.shared_generation.store(generation.raw(), Ordering::Release);
    }
}

/// One isolated Luau authority inside the process. See the module docs.
pub struct ScriptDomain {
    core: Rc<DomainCore>,
    /// The primary runtime: `Some` from creation until teardown drops it. [`DomainState`] says
    /// where the domain is; this only holds what that state owns.
    runtime: RefCell<Option<Runtime>>,
}

impl fmt::Debug for ScriptDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptDomain")
            .field("id", &self.core.id)
            .field("name", &self.core.name)
            .field("state", &self.core.state.get())
            .field("generation", &self.core.generation.get())
            .finish_non_exhaustive()
    }
}

fn build_runtime(core: &Rc<DomainCore>) -> Result<Runtime, RuntimeError> {
    let runtime = core.factory.create()?;
    if let Some(setup) = &core.setup {
        setup(&runtime)?;
    }
    if let Some(budget) = core.budget {
        runtime.set_limits(budget.limits());
    }
    crate::runtime::luau::attach_domain(&runtime, Rc::downgrade(core));
    Ok(runtime)
}

impl ScriptDomain {
    /// Creates a domain: allocates its id, builds its primary runtime from `factory` on this
    /// thread, and leaves it [`DomainState::Created`]. The same factory can create any number
    /// of domains; each gets a runtime of its own, and they share nothing.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Native`] if the factory or the setup hook fails.
    pub fn create(factory: Arc<dyn RuntimeFactory>, spec: DomainSpec) -> Result<ScriptDomain, RuntimeError> {
        let started = Instant::now();
        let counters = Arc::new(Counters::default());
        let observer = Arc::new(Observer {
            diagnostics: Arc::clone(&spec.diagnostics),
            profiler: Arc::clone(&spec.profiler),
            logs: spec.logs,
            counters: Arc::clone(&counters),
        });
        let core = Rc::new(DomainCore {
            id: DomainId::allocate(),
            name: spec.name,
            parent: spec.parent,
            grant: spec.grant,
            state: Cell::new(DomainState::Created),
            generation: Cell::new(Generation::FIRST),
            shared_generation: Arc::new(AtomicU64::new(Generation::FIRST.raw())),
            factory,
            setup: spec.setup,
            services: Arc::new(spec.services),
            diagnostics: spec.diagnostics,
            profiler: spec.profiler,
            counters,
            observer,
            fixed_step: spec.fixed_step,
            budget: spec.budget,
            accumulator: Cell::new(Duration::ZERO),
            contexts: RefCell::new(Vec::new()),
            next_order: Cell::new(0),
            workers: RefCell::new(Vec::new()),
            resources: RefCell::new(ResourceRegistry::default()),
            queue: RefCell::new(Some(channel())),
            routes: RefCell::new(Vec::new()),
            dispatching: Cell::new(None),
        });
        let runtime = build_runtime(&core)?;
        core.profiler.record(ProfileEvent::DomainCreated {
            attribution: core.attribution(),
            duration: started.elapsed(),
        });
        Ok(ScriptDomain {
            core,
            runtime: RefCell::new(Some(runtime)),
        })
    }

    /// The domain's identity, stable for its whole life.
    #[must_use]
    pub fn id(&self) -> DomainId {
        self.core.id
    }

    /// The domain's name, for people.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.core.name
    }

    /// The domain that launched this one, if it has a parent.
    #[must_use]
    pub fn parent(&self) -> Option<DomainId> {
        self.core.parent
    }

    /// Where the domain is in its life.
    #[must_use]
    pub fn state(&self) -> DomainState {
        self.core.state.get()
    }

    /// The generation of the code the domain is running.
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.core.generation.get()
    }

    /// The domain's attribution: its id and current generation.
    #[must_use]
    pub fn attribution(&self) -> Attribution {
        self.core.attribution()
    }

    /// What the domain was granted.
    #[must_use]
    pub fn grant(&self) -> &Grant {
        &self.core.grant
    }

    /// The domain's counters so far.
    #[must_use]
    pub fn stats(&self) -> DomainStats {
        self.core.counters.snapshot()
    }

    /// The services the domain carries, as [`DomainSpec::service`] attached them.
    #[must_use]
    pub fn services(&self) -> &Services {
        &self.core.services
    }

    /// The domain's service of type `T`, for the host that attached it.
    #[must_use]
    pub fn service<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.core.services.get::<T>()
    }

    /// A grant of `requested` for a child of this domain: a subset of this domain's own grant.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::OverDelegation`] naming what this domain lacks.
    pub fn delegate(&self, requested: CapabilitySet) -> Result<Grant, RuntimeError> {
        self.core.grant.delegate(self.core.id, requested)
    }

    /// A mailbox for queueing events into the domain from any thread.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Lifecycle`] once the domain has stopped.
    pub fn mailbox(&self) -> Result<DomainMailbox, RuntimeError> {
        let queue = self.core.queue.borrow();
        let Some((sender, _)) = queue.as_ref() else {
            return Err(self.core.lifecycle("open a mailbox of"));
        };
        Ok(DomainMailbox {
            domain: self.core.id,
            generation: Arc::clone(&self.core.shared_generation),
            sender: sender.clone(),
        })
    }

    /// Runs `body` with the primary runtime at host level: no authority ticket, no budget, no
    /// attribution. For host setup and inspection; script entry points belong to the scheduler.
    /// `None` once the domain has stopped.
    pub fn with_runtime<R>(&self, body: impl FnOnce(&Runtime) -> R) -> Option<R> {
        self.runtime.borrow().as_ref().map(body)
    }

    /// Declares a context. Its handlers are bound later, by script code (`context.bind`) or by
    /// [`ScriptDomain::bind_handler`].
    ///
    /// # Errors
    ///
    /// [`RuntimeError::OverDelegation`] if the context asks for capabilities the domain does not
    /// hold; [`RuntimeError::Lifecycle`] once the domain is stopping or stopped; a logic error if
    /// the name is taken.
    pub fn register_context(&self, spec: ContextSpec) -> Result<ContextId, RuntimeError> {
        self.core.register_context(spec)
    }

    /// Binds `handler` to `phase` of the context named `context`, replacing an earlier handler
    /// for that phase.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::UnknownContext`] or [`RuntimeError::UnscheduledPhase`].
    pub fn bind_handler(&self, context: &str, phase: Phase, handler: Function) -> Result<(), RuntimeError> {
        self.core.bind_handlers(context, vec![(phase, handler)], None)
    }

    /// Binds `handler` to the named event `event` of the context named `context`, replacing an
    /// earlier handler for that name. Named events are delivered in [`Phase::Event`], so the
    /// context must schedule it.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::UnknownContext`] or [`RuntimeError::UnscheduledPhase`].
    pub fn bind_event_handler(&self, context: &str, event: &str, handler: Function) -> Result<(), RuntimeError> {
        self.core
            .bind_event_handlers(context, vec![(event.to_owned(), handler, None)])
    }

    /// Routes the named event `event` to another domain: from now on, every time this domain's
    /// code emits it, a copy is posted to `to` as well as delivered here. This is the only way a
    /// domain's events reach another; nothing is broadcast, and only the event's [`Message`]
    /// payload crosses. Routes are the host's and survive restarts.
    ///
    /// # Errors
    ///
    /// A logic error for a route back to this domain, which would deliver every event twice.
    pub fn route(&self, event: &str, to: DomainMailbox) -> Result<(), RuntimeError> {
        if to.domain == self.core.id {
            return Err(RuntimeError::Native(l3i::Error::logic(format!(
                "{} can't route '{event}' to itself; its own handlers already get it",
                self.core.id
            ))));
        }
        self.core.routes.borrow_mut().push((event.to_owned(), to));
        Ok(())
    }

    /// Every registered context, in registration order.
    #[must_use]
    pub fn contexts(&self) -> Vec<ContextInfo> {
        self.core.contexts.borrow().iter().map(ContextRecord::info).collect()
    }

    /// The context named `name`.
    #[must_use]
    pub fn context(&self, name: &str) -> Option<ContextInfo> {
        self.core
            .contexts
            .borrow()
            .iter()
            .find(|context| context.name == name)
            .map(ContextRecord::info)
    }

    /// Loads the domain's code: `body` runs with the primary runtime under the domain's own
    /// authority, then every context's `load` handler runs. `Created` becomes `Ready`; a failure
    /// leaves the domain `Failed`, to be stopped.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Lifecycle`] unless the domain is `Created`; [`RuntimeError::Script`] if
    /// `body` or a load handler raises.
    pub fn load(&self, body: impl FnOnce(&Runtime) -> l3i::Result<()>) -> Result<(), RuntimeError> {
        self.core.refuse_reentry("load")?;
        self.core.require_state(&[DomainState::Created], "load")?;
        self.core.state.set(DomainState::Loading);
        match self.load_inner(body) {
            Ok(()) => {
                self.core.state.set(DomainState::Ready);
                Ok(())
            }
            Err(error) => {
                self.core.state.set(DomainState::Failed);
                Err(error)
            }
        }
    }

    /// [`ScriptDomain::load`] of one chunk: compiles `source` as `chunk_name` and runs it.
    ///
    /// # Errors
    ///
    /// As [`ScriptDomain::load`]; a compile error is a [`RuntimeError::Script`] too.
    pub fn load_source(&self, chunk_name: &str, source: &str) -> Result<(), RuntimeError> {
        self.load(|runtime| crate::runtime::chunk::load(runtime, chunk_name, source)?.invoke::<(), ()>(&runtime.stack(), ()))
    }

    fn load_inner(&self, body: impl FnOnce(&Runtime) -> l3i::Result<()>) -> Result<(), RuntimeError> {
        let attribution = self.core.attribution();
        {
            let _dispatch = self.core.enter_dispatch(Phase::Load, "load")?;
            let runtime = self.runtime.borrow();
            let Some(runtime) = runtime.as_ref() else {
                return Err(self.core.lifecycle("load"));
            };
            let _authority =
                AuthorityGuard::enter(self.core.authority(attribution, Arc::clone(self.core.grant.capabilities())));
            let _scope = runtime.call_scope(
                CallContext {
                    id: 0,
                    category: MemoryCategory(0),
                },
                CallKind::ScriptCall,
            );
            if let Err(error) = body(runtime) {
                let message = script_message(&error);
                self.core.report(
                    Diagnostic::new(attribution, Severity::Error, DiagnosticKind::HandlerFailed, message.clone())
                        .phase(Phase::Load),
                );
                return Err(RuntimeError::Script {
                    attribution,
                    phase: Some(Phase::Load),
                    message,
                });
            }
        }
        let mut report = self.broadcast(Phase::Load, &Message::Nil)?;
        if report.failures.is_empty() {
            Ok(())
        } else {
            Err(report.failures.remove(0))
        }
    }

    /// Starts the domain: `Ready` becomes `Running`, and every context's `startup` handler runs.
    /// A handler that raises is reported in the result and as a diagnostic; the domain runs on.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Lifecycle`] unless the domain is `Ready`.
    pub fn start(&self) -> Result<PhaseReport, RuntimeError> {
        self.core.refuse_reentry("start")?;
        self.core.require_state(&[DomainState::Ready], "start")?;
        let started = Instant::now();
        self.core.state.set(DomainState::Running);
        let report = self.broadcast(Phase::Startup, &Message::Nil)?;
        self.core.profiler.record(ProfileEvent::DomainStarted {
            attribution: self.core.attribution(),
            duration: started.elapsed(),
        });
        Ok(report)
    }

    /// Runs one broadcast phase: the handler of every context that schedules `phase`, lowest
    /// priority first and in registration order among equals, each called with `payload`.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Lifecycle`] unless the domain is `Running`; [`RuntimeError::Reentrant`]
    /// from inside a handler of this domain. Handler errors are in the report, not here.
    pub fn run_phase(&self, phase: Phase, payload: &Message) -> Result<PhaseReport, RuntimeError> {
        self.core.refuse_reentry("run a phase of")?;
        self.core.require_state(&[DomainState::Running], "run a phase of")?;
        self.broadcast(phase, payload)
    }

    fn broadcast(&self, phase: Phase, payload: &Message) -> Result<PhaseReport, RuntimeError> {
        let _dispatch = self.core.enter_dispatch(phase, "run a phase of")?;
        let runtime = self.runtime.borrow();
        let Some(runtime) = runtime.as_ref() else {
            return Err(self.core.lifecycle("run a phase of"));
        };
        let mut report = PhaseReport {
            phase,
            ran: 0,
            failures: Vec::new(),
        };
        for target in self.core.targets(phase) {
            report.ran += 1;
            if let Err(error) = self.call(runtime, &target, phase, payload, Duration::ZERO) {
                report.failures.push(error);
            }
        }
        Ok(report)
    }

    fn call(
        &self,
        runtime: &Runtime,
        target: &Dispatch,
        phase: Phase,
        payload: &Message,
        queue_delay: Duration,
    ) -> Result<(), RuntimeError> {
        self.core.call(runtime, target, phase, queue_delay, true, |handler| {
            handler.invoke::<(), _>(&runtime.stack(), (payload,))
        })
    }

    /// Advances the domain by `delta`: fixed-update steps owed by the accumulator (at most the
    /// configured catch-up, the rest dropped), then update, late update, and a drain of the
    /// event queue. Update handlers receive `delta` in seconds; fixed-update handlers the step.
    ///
    /// # Errors
    ///
    /// As [`ScriptDomain::run_phase`].
    pub fn advance(&self, delta: Duration) -> Result<TickReport, RuntimeError> {
        self.core.refuse_reentry("advance")?;
        self.core.require_state(&[DomainState::Running], "advance")?;
        let mut report = TickReport {
            fixed_steps: 0,
            dropped_steps: 0,
            phases: Vec::new(),
            events: EventReport::default(),
        };
        if let Some(fixed) = self.core.fixed_step.filter(|fixed| !fixed.step.is_zero()) {
            let mut accumulator = self.core.accumulator.get() + delta;
            let step = Message::Number(fixed.step.as_secs_f64());
            while accumulator >= fixed.step && report.fixed_steps < fixed.max_catch_up {
                accumulator -= fixed.step;
                report.fixed_steps += 1;
                Counters::add(&self.core.counters.fixed_updates, 1);
                report.phases.push(self.broadcast(Phase::FixedUpdate, &step)?);
            }
            while accumulator >= fixed.step {
                accumulator -= fixed.step;
                report.dropped_steps += 1;
            }
            self.core.accumulator.set(accumulator);
        }
        let seconds = Message::Number(delta.as_secs_f64());
        Counters::add(&self.core.counters.updates, 1);
        report.phases.push(self.broadcast(Phase::Update, &seconds)?);
        report.phases.push(self.broadcast(Phase::LateUpdate, &seconds)?);
        report.events = self.drain()?;
        Ok(report)
    }
}

impl DomainCore {
    /// One handler call, with everything the scheduler owes it: the context's authority, its
    /// budget, an l3i call scope carrying its id, timing, and attribution of any failure.
    /// `invoke` makes the call itself, with whatever arguments the caller has. A failure is
    /// reported as a diagnostic when `report` says so; a caller that folds it into a failure of
    /// its own, which it reports, says not, so nothing is reported twice.
    fn call(
        &self,
        runtime: &Runtime,
        target: &Dispatch,
        phase: Phase,
        queue_delay: Duration,
        report: bool,
        invoke: impl FnOnce(&Function) -> l3i::Result<()>,
    ) -> Result<(), RuntimeError> {
        let core = self;
        let attribution = core.attribution().in_context(target.context);
        let _authority = AuthorityGuard::enter(core.authority(attribution, Arc::clone(&target.capabilities)));
        let inherited = runtime.limits();
        if let Some(budget) = target.budget {
            runtime.set_limits(budget.limits());
        }
        let started = Instant::now();
        let outcome = {
            let scope = CallContext {
                id: u64::from(target.context.raw()),
                category: MemoryCategory(0),
            };
            let _scope = runtime.call_scope(scope, CallKind::ScriptCall);
            invoke(&target.handler)
        };
        let duration = started.elapsed();
        if target.budget.is_some() {
            runtime.set_limits(inherited);
        }
        if let Some(context) = core
            .contexts
            .borrow_mut()
            .iter_mut()
            .find(|context| context.id == target.context)
        {
            context.calls += 1;
            context.time += duration;
            context.failures += u64::from(outcome.is_err());
        }
        Counters::add(&core.counters.context_calls, 1);
        core.profiler.record(ProfileEvent::ContextCall {
            attribution,
            phase,
            queue_delay,
            duration,
            failed: outcome.is_err(),
        });
        outcome.map_err(|error| {
            Counters::add(&core.counters.context_failures, 1);
            let message = script_message(&error);
            if report {
                core.report(
                    Diagnostic::new(attribution, Severity::Error, DiagnosticKind::HandlerFailed, message.clone())
                        .phase(phase)
                        .event(target.event.clone())
                        .source(target.source.clone()),
                );
            }
            RuntimeError::Script {
                attribution,
                phase: Some(phase),
                message,
            }
        })
    }
}

impl ScriptDomain {
    /// Delivers the events queued so far, in the order they were posted or emitted: a raw event
    /// to its target's `event` handler, a named one to every handler bound to its name, in
    /// scheduler order. An event stamped with an earlier generation is discarded, counted, and
    /// reported, never delivered; one nothing handles is counted as unhandled and dropped.
    /// Events posted or emitted while draining wait for the next drain.
    ///
    /// # Errors
    ///
    /// As [`ScriptDomain::run_phase`].
    pub fn drain_events(&self) -> Result<EventReport, RuntimeError> {
        self.core.refuse_reentry("drain the events of")?;
        self.core.require_state(&[DomainState::Running], "drain the events of")?;
        self.drain()
    }

    fn queued(&self) -> Vec<Envelope> {
        let queue = self.core.queue.borrow();
        queue
            .as_ref()
            .map(|(_, receiver)| receiver.try_iter().collect())
            .unwrap_or_default()
    }

    fn drain(&self) -> Result<EventReport, RuntimeError> {
        let core = &self.core;
        let envelopes = self.queued();
        let mut report = EventReport::default();
        if envelopes.is_empty() {
            return Ok(report);
        }
        let _dispatch = core.enter_dispatch(Phase::Event, "drain the events of")?;
        let runtime = self.runtime.borrow();
        let Some(runtime) = runtime.as_ref() else {
            return Err(core.lifecycle("drain the events of"));
        };
        for envelope in envelopes {
            if envelope.generation != core.generation.get() {
                report.stale += 1;
                Counters::add(&core.counters.stale_discarded, 1);
                let context = match envelope.delivery {
                    Delivery::Raw(EventTarget::Context(context)) => Some(context),
                    Delivery::Raw(EventTarget::All) | Delivery::Named(_) => None,
                };
                let what = match &envelope.delivery {
                    Delivery::Raw(_) => "an event".to_owned(),
                    Delivery::Named(event) => format!("the event '{event}'"),
                };
                core.profiler.record(ProfileEvent::StaleEventDiscarded {
                    attribution: core.attribution(),
                    context,
                });
                core.report(Diagnostic::new(
                    Attribution {
                        context,
                        ..core.attribution()
                    },
                    Severity::Info,
                    DiagnosticKind::StaleGenerationDiscarded,
                    format!(
                        "{what} queued for {} was discarded; the domain is at {}",
                        envelope.generation,
                        core.generation.get()
                    ),
                ));
                continue;
            }
            let targets = match &envelope.delivery {
                Delivery::Raw(EventTarget::Context(context)) => core.target(*context, Phase::Event).into_iter().collect(),
                Delivery::Raw(EventTarget::All) => core.targets(Phase::Event),
                Delivery::Named(event) => core.named_targets(event),
            };
            if targets.is_empty() {
                report.unhandled += 1;
                continue;
            }
            for target in targets {
                report.delivered += 1;
                Counters::add(&core.counters.events_delivered, 1);
                if let Err(error) = self.call(runtime, &target, Phase::Event, &envelope.payload, envelope.posted.elapsed()) {
                    report.failures.push(error);
                }
            }
        }
        Ok(report)
    }

    /// Spawns a worker on the host's authority: its capabilities are a subset of the domain's
    /// grant. Script code spawns through `@jess/worker`, which also needs the `worker.spawn`
    /// capability.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Lifecycle`] once the domain is stopping or stopped;
    /// [`RuntimeError::OverDelegation`] if the worker asks for more than the domain holds;
    /// [`RuntimeError::Worker`] if its lane cannot start. A module that fails to load is
    /// reported by the worker itself, on the first receive or join.
    pub fn spawn_worker(&self, spec: WorkerSpec) -> Result<WorkerId, RuntimeError> {
        self.core.spawn_worker(spec, None)
    }

    /// Queues `message` for a worker's handler.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::UnknownWorker`], or [`RuntimeError::Worker`] once the worker has ended.
    pub fn send_to_worker(&self, worker: WorkerId, message: Message) -> Result<(), RuntimeError> {
        self.core.send_to_worker(worker, message)
    }

    /// The next reply from a worker, waiting up to `timeout` (`None` does not wait). `Ok(None)`
    /// is no reply in that time or a worker that has ended.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::UnknownWorker`], or a failure the worker reported as
    /// [`RuntimeError::Worker`], once.
    pub fn receive_from_worker(&self, worker: WorkerId, timeout: Option<Duration>) -> Result<Option<Message>, RuntimeError> {
        self.core.receive_from_worker(worker, timeout)
    }

    /// Asks a worker to stop after the messages already queued for it.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::UnknownWorker`].
    pub fn close_worker(&self, worker: WorkerId) -> Result<(), RuntimeError> {
        self.core.close_worker(worker)
    }

    /// Closes a worker and waits for it to end, up to `timeout` (`None` waits as long as it
    /// takes). Replies it sent before ending stay readable.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Timeout`] if it has not ended in time; it stays owned and can be joined
    /// again.
    pub fn join_worker(&self, worker: WorkerId, timeout: Option<Duration>) -> Result<WorkerState, RuntimeError> {
        self.core.join_worker(worker, timeout)
    }

    /// Every worker the domain owns, in spawn order.
    #[must_use]
    pub fn workers(&self) -> Vec<WorkerInfo> {
        self.core.workers.borrow().iter().map(WorkerRecord::info).collect()
    }

    /// Registers a native resource the domain owns, with how to release it. It is released by
    /// [`ScriptDomain::release`], at the next restart, or at teardown, whichever comes first.
    pub fn own(&self, kind: &'static str, release: impl FnOnce() + 'static) -> ResourceId {
        self.core
            .resources
            .borrow_mut()
            .own(kind, self.core.generation.get(), release)
    }

    /// Releases one owned resource now. False if it was not owned (already released).
    pub fn release(&self, resource: ResourceId) -> bool {
        let release = self.core.resources.borrow_mut().take(resource);
        release.is_some_and(|release| {
            release();
            true
        })
    }

    /// Every resource the domain owns, in registration order.
    #[must_use]
    pub fn owned_resources(&self) -> Vec<OwnedResource> {
        self.core.resources.borrow().list()
    }

    /// Ends the current generation: shutdown handlers (when running), then workers, owned
    /// resources, and bound handlers. Context declarations stay; they are the host's.
    fn end_generation(&self, was_running: bool) -> (Vec<RuntimeError>, usize, usize) {
        let shutdown_failures = if was_running {
            self.broadcast(Phase::Shutdown, &Message::Nil)
                .map(|report| report.failures)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let workers_stopped = self.core.stop_workers();
        let resources_released = self.core.release_resources();
        for context in self.core.contexts.borrow_mut().iter_mut() {
            context.handlers.clear();
            context.sources.clear();
            context.events.clear();
            context.event_sources.clear();
        }
        (shutdown_failures, workers_stopped, resources_released)
    }

    /// Restarts the domain as a new generation: shutdown handlers run, workers stop, owned
    /// resources are released, the runtime is dropped and a fresh one built, `load` loads the
    /// new code, and a domain that was running starts again. Context declarations survive;
    /// their handlers do not, and neither do events queued for the old generation.
    ///
    /// This is the coarse, safe reload boundary: no state migrates. It is destructive, not
    /// transactional: the old generation is gone before the new code loads, so if the new code
    /// fails to load, nothing is left running, and the domain is `Failed` and says so. It is not
    /// a last-known-good reload; one that keeps the old generation running until its replacement
    /// validates builds the replacement beside it and swaps, and is a different operation.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Lifecycle`] unless the domain is `Ready` or `Running`;
    /// [`RuntimeError::Reentrant`] from inside a handler; whatever [`ScriptDomain::load`] and
    /// the factory return.
    pub fn restart(&self, load: impl FnOnce(&Runtime) -> l3i::Result<()>) -> Result<Generation, RuntimeError> {
        self.core.refuse_reentry("restart")?;
        self.core
            .require_state(&[DomainState::Ready, DomainState::Running], "restart")?;
        let was_running = self.core.state.get() == DomainState::Running;
        self.end_generation(was_running);
        self.runtime.borrow_mut().take();
        self.core.set_generation(self.core.generation.get().next());
        self.core.accumulator.set(Duration::ZERO);
        match build_runtime(&self.core) {
            Ok(runtime) => *self.runtime.borrow_mut() = Some(runtime),
            Err(error) => {
                self.core.state.set(DomainState::Failed);
                return Err(error);
            }
        }
        self.core.state.set(DomainState::Created);
        self.load(load)?;
        if was_running {
            self.start()?;
        }
        Ok(self.core.generation.get())
    }

    /// Tears the domain down, in order: shutdown handlers (when it was running), queued events
    /// dropped, workers closed and joined, owned resources released newest first, contexts
    /// removed, the primary runtime dropped. The domain is then `Stopped` and owns nothing.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Lifecycle`] if it is already stopped; [`RuntimeError::Reentrant`] from
    /// inside one of its handlers.
    pub fn stop(&self) -> Result<TeardownReport, RuntimeError> {
        self.core.refuse_reentry("stop")?;
        self.core.require_state(
            &[
                DomainState::Created,
                DomainState::Ready,
                DomainState::Running,
                DomainState::Failed,
            ],
            "stop",
        )?;
        let started = Instant::now();
        let attribution = self.core.attribution();
        let was_running = self.core.state.get() == DomainState::Running;
        self.core.state.set(DomainState::Stopping);
        let (shutdown_failures, workers_stopped, resources_released) = self.end_generation(was_running);
        let events_discarded = self.queued().len();
        self.core.queue.borrow_mut().take();
        if events_discarded > 0 {
            self.core.report(Diagnostic::new(
                attribution,
                Severity::Info,
                DiagnosticKind::TeardownDiscarded,
                format!("{events_discarded} queued events were dropped at teardown"),
            ));
        }
        let contexts_removed = std::mem::take(&mut *self.core.contexts.borrow_mut()).len();
        self.runtime.borrow_mut().take();
        self.core.state.set(DomainState::Stopped);
        let duration = started.elapsed();
        self.core.profiler.record(ProfileEvent::Teardown {
            attribution,
            duration,
            workers_stopped,
        });
        Ok(TeardownReport {
            duration,
            shutdown_failures,
            events_discarded,
            workers_stopped,
            resources_released,
            contexts_removed,
        })
    }
}

impl Drop for ScriptDomain {
    /// A domain dropped without [`ScriptDomain::stop`] is torn down the same way: nothing it
    /// owns waits for process exit.
    fn drop(&mut self) {
        if !matches!(self.core.state.get(), DomainState::Stopped | DomainState::Stopping) {
            self.core.dispatching.set(None);
            // `stop` only fails on a state this branch has ruled out.
            drop(self.stop());
        }
    }
}

/// The domain a binding's runtime belongs to, for [`crate::runtime::luau`].
pub type DomainLink = Weak<DomainCore>;
