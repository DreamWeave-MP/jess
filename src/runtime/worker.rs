//! Workers: isolated Luau states a domain owns, each able to run while the domain's own state
//! and its other workers do.
//!
//! A worker is a runtime built from the same factory as its domain, on a lane of its own. Its
//! module returns one function, the message handler; the lane calls it once per message with the
//! message as a Luau value, and what it returns goes back to the domain. Nothing else crosses:
//! no table, closure, or userdata of one state is ever visible in another.
//!
//! l3i's `Runtime` is not `Send` (it owns a `lua_State` and `Rc` state), so a worker's runtime
//! is created, used, and dropped on its lane, and never moves. Today a lane is one OS thread per
//! worker. That is an implementation choice: the contract is an isolated state that may run
//! concurrently, and nothing in the API names a thread.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use l3i::runtime::{CallContext, CallKind, MemoryCategory};

use crate::runtime::capability::{Authority, AuthorityGuard, CapabilitySet, Observer};
use crate::runtime::context::Budget;
use crate::runtime::diagnostics::{Attribution, Diagnostic, DiagnosticKind, DiagnosticSink, Severity};
use crate::runtime::domain::{RuntimeFactory, SetupHook};
use crate::runtime::error::{RuntimeError, WorkerError, WorkerStage, script_message};
use crate::runtime::ids::{Generation, WorkerId};
use crate::runtime::log::LogSink;
use crate::runtime::message::Message;
use crate::runtime::profile::{Counters, Direction, ProfileEvent, ProfileSink};
use crate::runtime::service::{Services, Shared};

/// What a worker does after its handler raises on a message.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WorkerFailurePolicy {
    /// Report the failure and stop the worker: the default, since a handler that failed once
    /// has state nobody vouches for.
    #[default]
    Stop,
    /// Report the failure and keep handling messages.
    Continue,
}

/// What a domain asks for when it spawns a worker.
#[derive(Clone, Debug)]
pub struct WorkerSpec {
    pub(crate) name: String,
    pub(crate) chunk_name: String,
    pub(crate) source: String,
    pub(crate) capabilities: Option<CapabilitySet>,
    pub(crate) services: Sharing,
    pub(crate) budget: Option<Budget>,
    pub(crate) on_failure: WorkerFailurePolicy,
}

impl WorkerSpec {
    /// A worker named `name` whose module is `source`: a chunk that returns the message
    /// handler, `function(message) ... return reply end`. It inherits the spawner's whole
    /// grant, has no budget, and stops on its first failure, unless the builder says otherwise.
    #[must_use]
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> WorkerSpec {
        let name = name.into();
        WorkerSpec {
            chunk_name: format!("=worker:{name}"),
            name,
            source: source.into(),
            capabilities: None,
            services: Sharing::None,
            budget: None,
            on_failure: WorkerFailurePolicy::Stop,
        }
    }

    /// The chunk name errors and profiles name the module by (`@path` for a file).
    #[must_use]
    pub fn chunk_name(mut self, chunk_name: impl Into<String>) -> WorkerSpec {
        self.chunk_name = chunk_name.into();
        self
    }

    /// The capabilities the worker holds: an explicit subset of its spawner's.
    #[must_use]
    pub fn capabilities(mut self, capabilities: CapabilitySet) -> WorkerSpec {
        self.capabilities = Some(capabilities);
        self
    }

    /// Shares the domain's service of type `T` with the worker: bindings on its lane then find
    /// it with [`crate::runtime::service`]. A worker shares nothing it was not given this way or by
    /// [`WorkerSpec::share_all`], and spawning fails with [`RuntimeError::UnknownService`] if the
    /// domain carries no such service.
    #[must_use]
    pub fn share<T: Send + Sync + 'static>(mut self) -> WorkerSpec {
        let shared = Shared::of::<T>();
        match &mut self.services {
            Sharing::All => {}
            Sharing::Some(services) => {
                if !services.contains(&shared) {
                    services.push(shared);
                }
            }
            Sharing::None => self.services = Sharing::Some(vec![shared]),
        }
        self
    }

    /// Shares every service the domain carries with the worker: what a script spawning a worker
    /// asks for with `shareServices = true`, since it cannot name a Rust type. The worker's
    /// capabilities still say what it may do with them.
    #[must_use]
    pub fn share_all(mut self) -> WorkerSpec {
        self.services = Sharing::All;
        self
    }

    /// The execution budget of each handler call.
    #[must_use]
    pub const fn budget(mut self, budget: Budget) -> WorkerSpec {
        self.budget = Some(budget);
        self
    }

    /// What the worker does after its handler raises.
    #[must_use]
    pub const fn on_failure(mut self, policy: WorkerFailurePolicy) -> WorkerSpec {
        self.on_failure = policy;
        self
    }
}

/// Which of the domain's services a worker sees.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Sharing {
    /// None: the default.
    None,
    /// The listed types, each of which the domain must carry.
    Some(Vec<Shared>),
    /// Every service the domain carries.
    All,
}

/// Where a worker is in its life, as the domain last saw it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorkerState {
    /// Its runtime is being built and its module loaded.
    Starting,
    /// It is handling messages.
    Running,
    /// It was closed and its lane has ended.
    Stopped,
    /// It failed and its lane has ended, or is ending.
    Failed,
}

impl std::fmt::Display for WorkerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WorkerState::Starting => "starting",
            WorkerState::Running => "running",
            WorkerState::Stopped => "stopped",
            WorkerState::Failed => "failed",
        })
    }
}

/// A worker at one moment.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkerInfo {
    /// The worker's id.
    pub id: WorkerId,
    /// Its name.
    pub name: String,
    /// Its state as the domain last saw it.
    pub state: WorkerState,
    /// The generation that spawned it.
    pub generation: Generation,
}

/// How many messages may wait in each direction between a domain and one worker.
///
/// A domain that sends faster than its worker handles is told the queue is full instead of
/// growing it without bound; a worker that replies faster than its domain receives waits for room.
pub const WORKER_QUEUE: usize = 1024;

pub enum Command {
    Message { message: Message, sent: Instant },
    Close,
}

pub enum Output {
    Ready,
    Message(Message),
    Failed(WorkerError),
    Finished,
}

/// What a worker's own `@jess/worker` functions reach: the way back to its domain.
#[derive(Clone)]
pub struct WorkerLink {
    pub(crate) attribution: Attribution,
    pub(crate) outbox: SyncSender<Output>,
    pub(crate) counters: Arc<Counters>,
    pub(crate) profiler: Arc<dyn ProfileSink>,
}

impl WorkerLink {
    /// Sends `message` to the domain without waiting. A full queue is the worker's to handle, as
    /// the domain's own sends report one: waiting would hold the handler inside a native call,
    /// past its watchdog. A domain that stopped listening is not the worker's problem: the
    /// message is dropped with the channel.
    pub(crate) fn send(&self, message: Message) -> Result<(), String> {
        let bytes = message.size_hint();
        match self.outbox.try_send(Output::Message(message)) {
            Ok(()) => {
                Counters::add(&self.counters.messages_from_workers, 1);
                Counters::add(&self.counters.bytes_from_workers, bytes as u64);
                self.profiler.record(ProfileEvent::WorkerMessage {
                    attribution: self.attribution,
                    direction: Direction::FromWorker,
                    bytes,
                });
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(format!(
                "the domain's queue is full: {WORKER_QUEUE} messages wait for it to receive them; send fewer, or reply less often"
            )),
            Err(TrySendError::Disconnected(_)) => Ok(()),
        }
    }
}

/// Everything a worker's lane needs, moved onto it.
pub struct Launch {
    pub(crate) spec: WorkerSpec,
    pub(crate) attribution: Attribution,
    pub(crate) capabilities: Arc<CapabilitySet>,
    pub(crate) services: Arc<Services>,
    pub(crate) factory: Arc<dyn RuntimeFactory>,
    pub(crate) setup: Option<SetupHook>,
    pub(crate) diagnostics: Arc<dyn DiagnosticSink>,
    pub(crate) profiler: Arc<dyn ProfileSink>,
    pub(crate) logs: Option<Arc<dyn LogSink>>,
    pub(crate) counters: Arc<Counters>,
}

/// A worker as its domain holds it.
pub struct WorkerRecord {
    pub(crate) id: WorkerId,
    pub(crate) name: String,
    pub(crate) module: String,
    pub(crate) attribution: Attribution,
    pub(crate) state: WorkerState,
    on_failure: WorkerFailurePolicy,
    inbox: SyncSender<Command>,
    outbox: Receiver<Output>,
    /// Set by `close`: the lane stops once its queue is empty, even when the queue was too full
    /// to take a `Close`.
    closing: Arc<AtomicBool>,
    /// Replies read off the channel while waiting for something else (a join), kept in order.
    pending: VecDeque<Message>,
    /// A failure read off the channel while waiting for something else, reported on the next
    /// receive.
    failure: Option<WorkerError>,
    lane: Option<JoinHandle<()>>,
}

impl WorkerRecord {
    /// Starts a lane for `launch` and returns the domain's side of it.
    pub(crate) fn spawn(id: WorkerId, launch: Launch) -> Result<WorkerRecord, RuntimeError> {
        let (inbox, commands) = sync_channel::<Command>(WORKER_QUEUE);
        let (results, outbox) = sync_channel::<Output>(WORKER_QUEUE);
        let closing = Arc::new(AtomicBool::new(false));
        let lane_closing = Arc::clone(&closing);
        let name = launch.spec.name.clone();
        let module = launch.spec.chunk_name.clone();
        let on_failure = launch.spec.on_failure;
        let attribution = launch.attribution;
        let lane = std::thread::Builder::new()
            .name(format!("jess-{id}"))
            .spawn(move || run(&launch, &commands, &results, &lane_closing))
            .map_err(|error| {
                RuntimeError::Worker(WorkerError {
                    attribution,
                    module: module.clone(),
                    stage: WorkerStage::Spawn,
                    message: format!("the lane could not start: {error}"),
                })
            })?;
        Ok(WorkerRecord {
            id,
            name,
            module,
            attribution,
            state: WorkerState::Starting,
            on_failure,
            inbox,
            outbox,
            closing,
            pending: VecDeque::new(),
            failure: None,
            lane: Some(lane),
        })
    }

    pub(crate) fn info(&self) -> WorkerInfo {
        WorkerInfo {
            id: self.id,
            name: self.name.clone(),
            state: self.state,
            generation: self.attribution.generation,
        }
    }

    fn ended(&self, message: &str) -> RuntimeError {
        RuntimeError::Worker(WorkerError {
            attribution: self.attribution,
            module: self.module.clone(),
            stage: WorkerStage::Handle,
            message: message.to_owned(),
        })
    }

    /// Queues `message` for the worker's handler.
    pub(crate) fn send(&self, message: Message) -> Result<(), RuntimeError> {
        if matches!(self.state, WorkerState::Stopped | WorkerState::Failed) {
            return Err(self.ended(&format!("the worker is {}; it takes no more messages", self.state)));
        }
        let sent = Instant::now();
        self.inbox
            .try_send(Command::Message { message, sent })
            .map_err(|error| match error {
                TrySendError::Full(_) => self.ended(&format!(
                    "the worker's queue is full: {WORKER_QUEUE} messages wait for its handler; receive its replies, or send fewer"
                )),
                TrySendError::Disconnected(_) => self.ended("the worker's lane has ended; it takes no more messages"),
            })
    }

    fn absorb(&mut self, output: Output) -> Option<Result<Message, RuntimeError>> {
        match output {
            Output::Ready => {
                if self.state == WorkerState::Starting {
                    self.state = WorkerState::Running;
                }
                None
            }
            Output::Message(message) => Some(Ok(message)),
            Output::Failed(error) => {
                // A handler failure under `Continue` leaves the worker running.
                if error.stage != WorkerStage::Handle || self.on_failure == WorkerFailurePolicy::Stop {
                    self.state = WorkerState::Failed;
                }
                Some(Err(RuntimeError::Worker(error)))
            }
            Output::Finished => {
                if self.state != WorkerState::Failed {
                    self.state = WorkerState::Stopped;
                }
                None
            }
        }
    }

    /// The next reply, waiting up to `timeout` for one (`None` does not wait). `Ok(None)` is no
    /// reply in that time, or a worker that has ended with nothing left to say; a failure the
    /// worker reported comes back as the error, once.
    pub(crate) fn receive(&mut self, timeout: Option<Duration>) -> Result<Option<Message>, RuntimeError> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(Some(message));
        }
        if let Some(error) = self.failure.take() {
            return Err(RuntimeError::Worker(error));
        }
        // `None` doesn't wait; a wait too long for an `Instant` waits as long as it takes.
        let deadline = timeout.map(|timeout| Instant::now().checked_add(timeout));
        loop {
            let output = match deadline {
                None => match self.outbox.try_recv() {
                    Ok(output) => output,
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(None),
                },
                Some(None) => match self.outbox.recv() {
                    Ok(output) => output,
                    Err(_) => return Ok(None),
                },
                Some(Some(deadline)) => match self.outbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(output) => output,
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return Ok(None),
                },
            };
            let finished = matches!(output, Output::Finished);
            match self.absorb(output) {
                Some(Ok(message)) => return Ok(Some(message)),
                Some(Err(error)) => return Err(error),
                None if finished => return Ok(None),
                None => {}
            }
        }
    }

    /// Asks the worker to stop after the messages already queued.
    pub(crate) fn close(&self) {
        self.closing.store(true, Ordering::Release);
        // Wakes a lane waiting on an empty queue. A full queue can't take it, and needn't: the
        // lane sees the flag once it has handled what is queued. A lane that already ended has
        // dropped its receiver, and there is nobody left to tell.
        drop(self.inbox.try_send(Command::Close));
    }

    /// Closes the worker and waits for its lane to end, up to `timeout` (`None` waits as long
    /// as it takes). Replies it sent before ending stay readable through `receive`.
    pub(crate) fn join(&mut self, timeout: Option<Duration>) -> Result<WorkerState, RuntimeError> {
        self.close();
        // A wait too long for an `Instant` waits as long as it takes, as `None` does.
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        while self.lane.as_ref().is_some_and(|lane| !lane.is_finished()) {
            let output = match deadline {
                None => self.outbox.recv().ok(),
                Some(deadline) => match self.outbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(output) => Some(output),
                    Err(RecvTimeoutError::Timeout) => return Err(RuntimeError::Timeout { worker: self.id }),
                    Err(RecvTimeoutError::Disconnected) => None,
                },
            };
            let Some(output) = output else { break };
            let finished = matches!(output, Output::Finished);
            match self.absorb(output) {
                Some(Ok(message)) => self.pending.push_back(message),
                Some(Err(RuntimeError::Worker(error))) => self.failure = Some(error),
                Some(Err(_)) | None => {}
            }
            if finished {
                break;
            }
        }
        if let Some(lane) = self.lane.take() {
            // The lane has sent `Finished` or dropped its channel, so this returns at once. A
            // lane that panicked is a native bug; say so instead of swallowing it.
            if lane.join().is_err() {
                self.state = WorkerState::Failed;
                return Err(self.ended("the worker's lane panicked"));
            }
        }
        // Whatever the lane sent between `Finished` being read and it ending.
        while let Ok(output) = self.outbox.try_recv() {
            match self.absorb(output) {
                Some(Ok(message)) => self.pending.push_back(message),
                Some(Err(RuntimeError::Worker(error))) => self.failure = Some(error),
                Some(Err(_)) | None => {}
            }
        }
        if self.state != WorkerState::Failed {
            self.state = WorkerState::Stopped;
        }
        Ok(self.state)
    }

    /// True once the lane has been joined.
    pub(crate) fn is_joined(&self) -> bool {
        self.lane.is_none()
    }
}

fn fail(launch: &Launch, results: &SyncSender<Output>, stage: WorkerStage, message: String) {
    let error = WorkerError {
        attribution: launch.attribution,
        module: launch.spec.chunk_name.clone(),
        stage,
        message,
    };
    Counters::add(&launch.counters.worker_failures, 1);
    launch.diagnostics.report(
        Diagnostic::new(
            launch.attribution,
            Severity::Error,
            DiagnosticKind::WorkerFailed,
            error.to_string(),
        )
        .source(Some(launch.spec.chunk_name.clone())),
    );
    // The domain may have dropped the worker already; the diagnostic above still stands.
    drop(results.send(Output::Failed(error)));
}

/// A worker's lane: build the runtime here, load the module, then handle messages until closed.
/// The runtime is dropped here too, before `Finished` is sent, so a joined worker has released
/// its Luau state.
fn run(launch: &Launch, commands: &Receiver<Command>, results: &SyncSender<Output>, closing: &AtomicBool) {
    serve(launch, commands, results, closing);
    drop(results.send(Output::Finished));
}

fn serve(launch: &Launch, commands: &Receiver<Command>, results: &SyncSender<Output>, closing: &AtomicBool) {
    let started = Instant::now();
    let built = launch.factory.create().and_then(|runtime| {
        if let Some(setup) = &launch.setup {
            setup(&runtime)?;
        }
        Ok(runtime)
    });
    let runtime = match built {
        Ok(runtime) => runtime,
        Err(error) => return fail(launch, results, WorkerStage::Spawn, error.to_string()),
    };
    if let Some(budget) = launch.spec.budget {
        runtime.set_limits(budget.limits());
    }
    crate::runtime::luau::attach_worker(
        &runtime,
        WorkerLink {
            attribution: launch.attribution,
            outbox: results.clone(),
            counters: Arc::clone(&launch.counters),
            profiler: Arc::clone(&launch.profiler),
        },
    );
    let observer = Observer {
        diagnostics: Arc::clone(&launch.diagnostics),
        profiler: Arc::clone(&launch.profiler),
        logs: launch.logs.clone(),
        counters: Arc::clone(&launch.counters),
    };
    let _authority = AuthorityGuard::enter(Authority::new(
        launch.attribution,
        Arc::clone(&launch.capabilities),
        Arc::clone(&launch.services),
        Some(Arc::new(observer)),
    ));
    let scope = CallContext {
        id: u64::from(launch.attribution.worker.map_or(0, WorkerId::raw)),
        category: MemoryCategory(0),
    };
    let handler = match load_handler(launch, &runtime, scope) {
        Ok(handler) => handler,
        Err(message) => return fail(launch, results, WorkerStage::Load, message),
    };
    launch.profiler.record(ProfileEvent::WorkerSpawned {
        attribution: launch.attribution,
        duration: started.elapsed(),
    });
    if results.send(Output::Ready).is_err() {
        return;
    }
    handle_messages(launch, &runtime, &handler, scope, commands, results, closing);
}

/// Runs the worker's module and takes the handler it returns; the error is the message for a
/// load-stage failure.
fn load_handler(launch: &Launch, runtime: &l3i::Runtime, scope: CallContext) -> Result<l3i::value::Function, String> {
    let loaded = crate::runtime::chunk::load(runtime, &launch.spec.chunk_name, &launch.spec.source).and_then(|chunk| {
        let _call = runtime.call_scope(scope, CallKind::ScriptCall);
        chunk.invoke::<l3i::value::Value, ()>(&runtime.stack(), ())
    });
    match loaded {
        Ok(value) if value.is_function() => l3i::value::Function::from_value(value).map_err(|error| error.to_string()),
        Ok(value) => Err(format!(
            "a worker module returns its message handler, a function, not {}",
            value.type_of().name()
        )),
        Err(error) => Err(script_message(&error)),
    }
}

/// Handles messages until the inbox closes, a `Close` arrives, or the failure policy stops it.
fn handle_messages(
    launch: &Launch,
    runtime: &l3i::Runtime,
    handler: &l3i::value::Function,
    scope: CallContext,
    commands: &Receiver<Command>,
    results: &SyncSender<Output>,
    closing: &AtomicBool,
) {
    loop {
        // Once closing, what is queued still runs; an empty queue ends the lane.
        let command = if closing.load(Ordering::Acquire) {
            commands.try_recv().ok()
        } else {
            commands.recv().ok()
        };
        let Some(Command::Message { message, sent }) = command else {
            return;
        };
        let queue_delay = sent.elapsed();
        let began = Instant::now();
        let outcome = {
            let _call = runtime.call_scope(scope, CallKind::ScriptCall);
            handler.invoke_with(&runtime.stack(), (&message,), |frame, reply| {
                Message::read(frame, reply, "reply").map_err(|error| l3i::Error::runtime(error.to_string()))
            })
        };
        launch.profiler.record(ProfileEvent::WorkerCall {
            attribution: launch.attribution,
            queue_delay,
            duration: began.elapsed(),
            failed: outcome.is_err(),
        });
        match outcome {
            Ok(Message::Nil) => {}
            Ok(reply) => {
                let bytes = reply.size_hint();
                if results.send(Output::Message(reply)).is_err() {
                    return;
                }
                Counters::add(&launch.counters.messages_from_workers, 1);
                Counters::add(&launch.counters.bytes_from_workers, bytes as u64);
                launch.profiler.record(ProfileEvent::WorkerMessage {
                    attribution: launch.attribution,
                    direction: Direction::FromWorker,
                    bytes,
                });
            }
            Err(error) => {
                fail(launch, results, WorkerStage::Handle, script_message(&error));
                if launch.spec.on_failure == WorkerFailurePolicy::Stop {
                    return;
                }
            }
        }
    }
}
