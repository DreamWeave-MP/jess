//! The Luau surface: `@jess/context`, `@jess/worker`, `@jess/events`, and `@jess/interfaces`, as an
//! l3i extension.
//!
//! [`RuntimeExtension`] goes into the plan every runtime of a domain is made from. It declares
//! the modules and the handle types, and stores one slot per runtime that the domain or the
//! worker lane fills right after the runtime is built; the module functions reach their owner
//! through that slot. A runtime nobody attached (a plain `Runtime::from_plan`) has the modules
//! and every function raises, saying no `ScriptDomain` owns it.
//!
//! The surface is what a script inside an existing domain needs and nothing more. A script
//! binds handlers to contexts the host declared, emits named events for the domain's own
//! handlers, looks up the interfaces the host composed in this domain, and spawns workers when
//! it holds `worker.spawn`, sharing the domain's services with one only by saying so
//! (`shareServices = true`). It cannot create a domain, declare a context, enter a phase,
//! publish an interface, or name a capability it was not granted.
//!
//! Interfaces are plain tables in this runtime, so they are this domain's alone: a worker or
//! another domain never sees them, and a restart, which builds a new runtime, starts without.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use l3i::Runtime;
use l3i::bind::{Call, StackResults};
use l3i::convert::{Integer, Push};
use l3i::extension::{Extension, ExtensionDescriptor, InstallContext, TagPolicy};
use l3i::stack::{Scope, Type, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::{Function, Table};

use crate::runtime::capability::{CapabilitySet, current_authority};
use crate::runtime::context::Phase;
use crate::runtime::diagnostics::{Diagnostic, DiagnosticKind, Severity};
use crate::runtime::domain::{DomainCore, DomainLink};
use crate::runtime::error::RuntimeError;
use crate::runtime::ids::{CapabilityId, WorkerId};
use crate::runtime::message::{Message, ResourceHandle};
use crate::runtime::worker::{WorkerLink, WorkerSpec};

/// The extension's id.
pub const EXTENSION_ID: &str = "jess.runtime";

/// The `require` path of the context module.
pub const CONTEXT_MODULE: &str = "@jess/context";

/// The `require` path of the worker module.
pub const WORKER_MODULE: &str = "@jess/worker";

/// The `require` path of the named-event module.
pub const EVENTS_MODULE: &str = "@jess/events";

/// The `require` path of the interface module.
pub const INTERFACES_MODULE: &str = "@jess/interfaces";

/// The `@jess/context`, `@jess/worker`, `@jess/events`, and `@jess/interfaces` modules and their
/// handle types.
#[derive(Clone, Copy, Debug, Default)]
pub struct RuntimeExtension;

/// The module tables of one runtime, stored as the extension's state so an extension that
/// requires this one can expose them under its own names (`jess.context`, `jess.worker`, ...).
#[derive(Debug)]
pub struct Modules {
    /// The `@jess/context` table.
    pub context: Table,
    /// The `@jess/worker` table.
    pub worker: Table,
    /// The `@jess/events` table.
    pub events: Table,
    /// The `@jess/interfaces` table.
    pub interfaces: Table,
}

/// Who owns the runtime a binding runs in.
enum Owner {
    /// Nobody attached it.
    Detached,
    /// A domain's primary runtime.
    Domain(DomainLink),
    /// A worker's runtime.
    Worker(WorkerLink),
}

/// Where a domain's interface registry is in its generation.
enum Interfaces {
    /// Nothing composes interfaces in this runtime: no application host loaded it.
    Absent,
    /// The host is loading the modules that provide them; none is published yet.
    Composing,
    /// Composed, by id: the effective table of each, as the host published it.
    Published(BTreeMap<String, Table>),
}

/// The per-runtime slot the module functions read their owner from.
struct Bindings {
    owner: RefCell<Owner>,
    /// The domain's interfaces. They live with the runtime, so a restart, which builds a new
    /// runtime, starts with none, and nothing can reach an old generation's.
    interfaces: RefCell<Interfaces>,
}

impl Bindings {
    fn domain(&self, function: &str) -> l3i::Result<Rc<DomainCore>> {
        match &*self.owner.borrow() {
            Owner::Domain(domain) => domain
                .upgrade()
                .ok_or_else(|| l3i::Error::runtime(format!("{function}: the ScriptDomain that owned this runtime is gone"))),
            Owner::Worker(_) => Err(l3i::Error::runtime(format!(
                "{function} is for a domain's own code; a worker handles messages and answers them"
            ))),
            Owner::Detached => Err(detached(function)),
        }
    }

    fn worker(&self, function: &str) -> l3i::Result<WorkerLink> {
        match &*self.owner.borrow() {
            Owner::Worker(link) => Ok(link.clone()),
            Owner::Domain(_) => Err(l3i::Error::runtime(format!(
                "{function} is for a worker's own code; a domain sends through the handle worker.spawn returned"
            ))),
            Owner::Detached => Err(detached(function)),
        }
    }
}

fn detached(function: &str) -> l3i::Error {
    l3i::Error::permission(format!(
        "{function}: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and command targets run scripts outside one"
    ))
}

fn attach(runtime: &Runtime, owner: Owner) {
    // A plan without the extension has no slot, and no `@jess/context` to call either.
    if let Some(bindings) = runtime.state_of::<Bindings>(EXTENSION_ID) {
        *bindings.owner.borrow_mut() = owner;
    }
}

/// Makes `runtime` the primary runtime of `domain` for the module functions.
pub(crate) fn attach_domain(runtime: &Runtime, domain: DomainLink) {
    attach(runtime, Owner::Domain(domain));
}

/// Makes `runtime` a worker's runtime for the module functions.
pub(crate) fn attach_worker(runtime: &Runtime, link: WorkerLink) {
    attach(runtime, Owner::Worker(link));
}

/// The domain whose primary runtime `runtime` is, for a host loading code into it.
pub(crate) fn domain_of(runtime: &Runtime) -> Option<Rc<DomainCore>> {
    let bindings = runtime.state_of::<Bindings>(EXTENSION_ID)?;
    match &*bindings.owner.borrow() {
        Owner::Domain(domain) => domain.upgrade(),
        Owner::Worker(_) | Owner::Detached => None,
    }
}

/// Starts composing `runtime`'s interfaces: from now until [`publish_interfaces`], a lookup says
/// they are being composed, rather than that there are none.
pub(crate) fn compose_interfaces(runtime: &Runtime) {
    if let Some(bindings) = runtime.state_of::<Bindings>(EXTENSION_ID) {
        *bindings.interfaces.borrow_mut() = Interfaces::Composing;
    }
}

/// Publishes `runtime`'s interfaces, all at once: no lookup ever sees part of them.
pub(crate) fn publish_interfaces(runtime: &Runtime, interfaces: BTreeMap<String, Table>) {
    if let Some(bindings) = runtime.state_of::<Bindings>(EXTENSION_ID) {
        *bindings.interfaces.borrow_mut() = Interfaces::Published(interfaces);
    }
}

/// The interface `id` of the domain whose code is calling, `None` when it has no such interface;
/// an error when the caller is not the domain's own code or the domain's interfaces aren't
/// composed yet.
fn interface(bindings: &Bindings, function: &str, id: &str) -> l3i::Result<Option<Table>> {
    match &*bindings.owner.borrow() {
        Owner::Domain(_) => {}
        Owner::Worker(_) => {
            return Err(l3i::Error::runtime(format!(
                "{function}: interfaces are the domain's own code's; a worker has none, so send it what it needs in a message"
            )));
        }
        Owner::Detached => return Err(detached(function)),
    }
    match &*bindings.interfaces.borrow() {
        Interfaces::Absent => Err(l3i::Error::runtime(format!(
            "{function}: this domain has no interfaces; an application target composes them from its packages' [managed] modules"
        ))),
        Interfaces::Composing => Err(l3i::Error::runtime(format!(
            "{function}: interfaces are composed once every managed module has loaded; look '{id}' up in a handler or in the application's entry"
        ))),
        Interfaces::Published(interfaces) => Ok(interfaces.get(id).cloned()),
    }
}

/// `interfaces.get(id)`.
fn get_interface(bindings: &Bindings, call: &Call<'_>, id: &str) -> l3i::Result<Table> {
    if let Some(table) = interface(bindings, "interfaces.get", id)? {
        return Ok(table);
    }
    let has = match &*bindings.interfaces.borrow() {
        Interfaces::Published(interfaces) if !interfaces.is_empty() => {
            format!("it has {}", interfaces.keys().cloned().collect::<Vec<_>>().join(", "))
        }
        _ => "it has none".to_owned(),
    };
    let location = l3i::runtime::caller_location(call);
    let asked = if location.is_empty() {
        String::new()
    } else {
        format!(" (asked at {location})")
    };
    Err(l3i::Error::runtime(format!(
        "interfaces.get: no package in this domain implements '{id}'{asked}; {has}"
    )))
}

/// A worker as the domain's scripts hold it.
pub struct WorkerHandle {
    domain: DomainLink,
    id: WorkerId,
}

impl std::fmt::Debug for WorkerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerHandle").field("id", &self.id).finish_non_exhaustive()
    }
}

// SAFETY: a weak reference and an id; the destructor never touches the Lua API.
unsafe impl Userdata for WorkerHandle {
    const NAME: &'static str = "jess.runtime.Worker";
}

impl WorkerHandle {
    fn domain(&self) -> l3i::Result<Rc<DomainCore>> {
        self.domain
            .upgrade()
            .ok_or_else(|| l3i::Error::runtime(format!("{}'s ScriptDomain is gone", self.id)))
    }
}

/// How long a script asked to wait: absent is `None`, and a wait too long to represent waits as
/// long as it takes.
fn timeout(seconds: Option<f64>) -> l3i::Result<Option<Duration>> {
    match seconds {
        None => Ok(None),
        Some(seconds) if seconds.is_finite() && seconds >= 0.0 => {
            Ok(Some(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX)))
        }
        Some(seconds) => Err(l3i::Error::runtime(format!(
            "a timeout is a number of seconds from 0, not {seconds}"
        ))),
    }
}

/// `wait` clamped to what the runtime's watchdog has left, so a script blocked on a worker is still
/// stopped by the execution time limit (`None` waits as long as it takes), and whether the clamp
/// cut it short.
fn clamped(call: &Call<'_>, wait: Option<Duration>) -> (Option<Duration>, bool) {
    let Some(remaining) = l3i::runtime::watchdog_remaining(call) else {
        return (wait, false);
    };
    match wait {
        Some(wait) if wait <= remaining => (Some(wait), false),
        _ => (Some(remaining), true),
    }
}

/// Reads a script's value as a message, reporting one that cannot cross to the domain's
/// diagnostics before raising.
fn read_message(call: &Call<'_>, value: ValueView<'_>, domain: Option<&DomainCore>, root: &str) -> l3i::Result<Message> {
    call.with_frame(|frame| Ok(Message::read(frame, value, root)))?
        .map_err(|error| {
            if let Some(domain) = domain {
                let attribution =
                    current_authority().map_or_else(|| domain.attribution(), |authority| authority.attribution());
                domain.report(
                    Diagnostic::new(
                        attribution,
                        Severity::Error,
                        DiagnosticKind::TransferUnsupported,
                        error.to_string(),
                    )
                    .source(Some(l3i::runtime::caller_location(call))),
                );
            }
            l3i::Error::from(RuntimeError::Transfer(error))
        })
}

/// `context.bind(name, handlers)`.
fn bind(bindings: &Bindings, call: &Call<'_>, name: &str, handlers: ValueView<'_>) -> l3i::Result<()> {
    let domain = bindings.domain("context.bind")?;
    if !handlers.is_table() {
        return Err(handlers.type_error(Type::Table));
    }
    let mut bound: Vec<(Phase, Function)> = Vec::new();
    call.with_frame(|frame| {
        handlers.as_table()?.for_each(frame, |_, key, value| {
            let Ok(key) = key.read::<&str>() else {
                return Err(l3i::Error::runtime("context.bind: handlers are keyed by phase name"));
            };
            let phase = Phase::from_name(key).ok_or_else(|| l3i::Error::from(RuntimeError::UnknownPhase(key.to_owned())))?;
            if !value.is_function() {
                return Err(l3i::Error::runtime(format!(
                    "context.bind: the {phase} handler is a function, not {}",
                    value.type_of().name()
                )));
            }
            bound.push((phase, value.read::<Function>()?));
            Ok(())
        })
    })?;
    let source = l3i::runtime::caller_location(call);
    domain.bind_handlers(name, bound, Some(&source)).map_err(l3i::Error::from)
}

/// `events.emit(name, payload)`: queues a named event for the domain's next drain. Nothing runs
/// now, so an emit from inside a handler never reenters one.
fn emit(bindings: &Bindings, call: &Call<'_>, name: &str, payload: Option<ValueView<'_>>) -> l3i::Result<()> {
    if name.is_empty() {
        return Err(l3i::Error::runtime("events.emit: an event needs a name"));
    }
    let domain = bindings.domain("events.emit")?;
    let payload = match payload {
        Some(payload) => read_message(call, payload, Some(&domain), "payload")?,
        None => Message::Nil,
    };
    domain.emit(name, payload).map_err(|problem| match problem {
        RuntimeError::QueueFull(full) => l3i::Error::runtime(format!("events.emit: '{name}' wasn't queued: {full}")),
        other => l3i::Error::from(other),
    })
}

const SPAWN_OPTIONS: [&str; 4] = ["capabilities", "name", "shareServices", "source"];

/// `worker.spawn(options)`. The options are checked before the owner is, so a misspelled key
/// reads the same in every runtime, owned or not.
fn spawn(bindings: &Bindings, call: &Call<'_>, options: ValueView<'_>) -> l3i::Result<Owned<WorkerHandle>> {
    if !options.is_table() {
        return Err(options.type_error(Type::Table));
    }
    let (source, name, capabilities, share_services) = call.with_frame(|frame| {
        let table = options.as_table()?;
        table.for_each(frame, |_, key, _| match key.read::<&str>() {
            Ok(key) if SPAWN_OPTIONS.contains(&key) => Ok(()),
            Ok(key) => Err(l3i::Error::runtime(format!(
                "unknown worker.spawn option '{key}': expected capabilities, name, shareServices, or source"
            ))),
            Err(_) => Err(l3i::Error::runtime("worker.spawn options are named by strings")),
        })?;
        let source: Option<String> = table.get_optional(frame, "source", "worker.spawn options")?;
        let name: Option<String> = table.get_optional(frame, "name", "worker.spawn options")?;
        let share_services: Option<bool> = table.get_optional(frame, "shareServices", "worker.spawn options")?;
        let listed = table.get(frame, "capabilities")?;
        let capabilities = if listed.is_nil() {
            None
        } else if listed.is_table() {
            let mut names: Vec<String> = Vec::new();
            listed.as_table()?.for_each_array(frame, |_, _, item| {
                names.push(item.read::<String>()?);
                Ok(())
            })?;
            Some(names)
        } else {
            return Err(l3i::Error::runtime(format!(
                "capabilities must be a list of names, not {}",
                listed.type_of().name()
            )));
        };
        Ok((source, name, capabilities, share_services))
    })?;
    let Some(source) = source else {
        return Err(l3i::Error::runtime(
            "worker.spawn needs source, the worker module's Luau source",
        ));
    };
    let domain = bindings.domain("worker.spawn")?;
    let location = l3i::runtime::caller_location(call);
    let name = name.unwrap_or_else(|| {
        if location.is_empty() {
            "worker".to_owned()
        } else {
            location.clone()
        }
    });
    let mut spec = WorkerSpec::new(name, source);
    if !location.is_empty() {
        spec = spec.chunk_name(format!("=worker@{location}"));
    }
    if let Some(names) = capabilities {
        let mut set = CapabilitySet::empty();
        for name in &names {
            // Looked up, not interned: interned names live as long as the process, and a name
            // nothing interned is one nobody holds, so it can't be delegated anyway.
            let id = CapabilityId::lookup(name).ok_or_else(|| {
                l3i::Error::permission(format!("worker.spawn: capability '{name}' isn't one this domain holds"))
            })?;
            set.insert(id);
        }
        spec = spec.capabilities(set);
    }
    if share_services == Some(true) {
        spec = spec.share_all();
    }
    // Script code spawns on its own authority; the host's is `ScriptDomain::spawn_worker`.
    let Some(authority) = current_authority() else {
        return Err(l3i::Error::permission(
            "worker.spawn: no authority is running; spawn from a context handler or at load",
        ));
    };
    let id = domain.spawn_worker(spec, Some(&authority)).map_err(l3i::Error::from)?;
    Ok(Owned(WorkerHandle {
        domain: Rc::downgrade(&domain),
        id,
    }))
}

impl Extension for RuntimeExtension {
    fn id(&self) -> &'static str {
        EXTENSION_ID
    }

    fn describe(&self, d: &mut ExtensionDescriptor) -> l3i::Result<()> {
        d.module(CONTEXT_MODULE)
            .doc("Script contexts: bind handlers to the phases a context the host declared may run in.")
            .installed("bind")
            .signature("(name: string, handlers: { [string]: (payload: any) -> () }) -> ()")
            .doc("Binds a handler per phase (startup, update, fixedUpdate, event, shutdown, ...) to the context `name`.");
        d.module(WORKER_MODULE)
            .doc("Workers: isolated Luau states that run beside the domain and talk to it in messages.")
            .installed("spawn")
            .signature(
                "(options: { source: string, name: string?, capabilities: { string }?, shareServices: boolean? }) -> jess_runtime_Worker",
            )
            .doc("Spawns a worker from its module source; needs the worker.spawn capability. shareServices = true gives it every service the domain carries.")
            .installed("send")
            .signature("(message: any) -> ()")
            .doc("From inside a worker: sends a message to the domain, besides what the handler returns.");
        d.module(EVENTS_MODULE)
            .doc("Named events: queued for the handlers bound to a name, delivered at the domain's next event drain.")
            .installed("emit")
            .signature("(name: string, payload: any?) -> ()")
            .doc("Queues the named event `name` with a transferable payload; no handler runs before the next drain.");
        d.module(INTERFACES_MODULE)
            .doc("The domain's interfaces: plain tables its application's packages provide, by id, in this domain only.")
            .installed("get")
            .signature("(id: string) -> any")
            .doc("The effective implementation of the interface `id`; an error naming what the domain has if none.")
            .installed("tryGet")
            .signature("(id: string) -> any?")
            .doc("The effective implementation of the interface `id`, or nil when no package provides it.");

        let mut worker = d.userdata::<WorkerHandle>(WorkerHandle::NAME);
        worker.tag(TagPolicy::Never).doc("A worker the domain spawned.");
        worker
            .method(
                "send",
                |this: &WorkerHandle, call: &Call<'_>, message: ValueView<'_>| -> l3i::Result<()> {
                    let domain = this.domain()?;
                    let message = read_message(call, message, Some(&domain), "message")?;
                    domain.send_to_worker(this.id, message).map_err(l3i::Error::from)
                },
            )
            .signature("(self, message: any): ()")
            .doc("Queues a message for the worker's handler.");
        worker
            .method(
                "receive",
                |this: &WorkerHandle, call: &Call<'_>, seconds: Option<f64>| -> l3i::Result<StackResults> {
                    // Absent or zero doesn't wait at all.
                    let asked = timeout(seconds)?;
                    let (wait, cut) = if asked.is_some() {
                        clamped(call, asked)
                    } else {
                        (None, false)
                    };
                    let reply = this.domain()?.receive_from_worker(this.id, wait).map_err(l3i::Error::from)?;
                    if cut && reply.is_none() {
                        l3i::runtime::expire_if_due(call);
                    }
                    reply.unwrap_or(Message::Nil).push_into(call)?;
                    Ok(StackResults)
                },
            )
            .signature("(self, timeoutSeconds: number?): any")
            .doc("The worker's next reply, waiting up to timeoutSeconds; nil when there is none.");
        worker
            .method("close", |this: &WorkerHandle| -> l3i::Result<()> {
                this.domain()?.close_worker(this.id).map_err(l3i::Error::from)
            })
            .signature("(self): ()")
            .doc("Asks the worker to stop after the messages already queued.");
        worker
            .method(
                "join",
                |this: &WorkerHandle, call: &Call<'_>, seconds: Option<f64>| -> l3i::Result<String> {
                    // Absent waits as long as it takes, or as long as the watchdog allows.
                    let (wait, cut) = clamped(call, timeout(seconds)?);
                    let joined = this.domain()?.join_worker(this.id, wait);
                    if cut && joined.is_err() {
                        l3i::runtime::expire_if_due(call);
                    }
                    let state = joined.map_err(l3i::Error::from)?;
                    Ok(state.to_string())
                },
            )
            .signature("(self, timeoutSeconds: number?): string")
            .doc("Closes the worker and waits for it to end; returns its final state.");
        worker.metamethod("__tostring", |this: &WorkerHandle| format!("Worker({})", this.id));

        let mut handle = d.userdata::<ResourceHandle>(ResourceHandle::NAME);
        handle
            .tag(TagPolicy::Never)
            .doc("A handle to a native resource that may cross domain and worker boundaries.");
        handle
            .getter("kind", |this: &ResourceHandle| this.kind.clone())
            .signature("string");
        #[allow(clippy::cast_possible_wrap)] // The id's bit pattern, as the minting service wrote it.
        handle
            .getter("id", |this: &ResourceHandle| Integer(this.id as i64))
            .signature("integer");
        Ok(())
    }

    fn install(&self, cx: &mut InstallContext<'_>) -> l3i::Result<()> {
        let bindings = cx.insert_state(Bindings {
            owner: RefCell::new(Owner::Detached),
            interfaces: RefCell::new(Interfaces::Absent),
        });

        let context = cx.module(CONTEXT_MODULE)?;
        let state = Rc::clone(&bindings);
        context.function(
            "bind",
            move |call: &Call<'_>, name: String, handlers: ValueView<'_>| -> l3i::Result<()> {
                bind(&state, call, &name, handlers)
            },
        )?;
        let context = context.table().clone();

        let worker = cx.module(WORKER_MODULE)?;
        let state = Rc::clone(&bindings);
        worker.function(
            "spawn",
            move |call: &Call<'_>, options: ValueView<'_>| -> l3i::Result<Owned<WorkerHandle>> {
                spawn(&state, call, options)
            },
        )?;
        let state = Rc::clone(&bindings);
        worker.function("send", move |call: &Call<'_>, message: ValueView<'_>| -> l3i::Result<()> {
            let link = state.worker("worker.send")?;
            link.send(read_message(call, message, None, "message")?)
                .map_err(|problem| l3i::Error::runtime(format!("jess.worker.send: {problem}")))
        })?;
        let worker = worker.table().clone();

        let events = cx.module(EVENTS_MODULE)?;
        let state = Rc::clone(&bindings);
        events.function(
            "emit",
            move |call: &Call<'_>, name: String, payload: Option<ValueView<'_>>| -> l3i::Result<()> {
                emit(&state, call, &name, payload)
            },
        )?;
        let events = events.table().clone();

        let interfaces = cx.module(INTERFACES_MODULE)?;
        let state = Rc::clone(&bindings);
        interfaces.function("get", move |call: &Call<'_>, id: String| -> l3i::Result<Table> {
            get_interface(&state, call, &id)
        })?;
        let state = Rc::clone(&bindings);
        interfaces.function("tryGet", move |id: String| -> l3i::Result<Option<Table>> {
            interface(&state, "interfaces.tryGet", &id)
        })?;
        let interfaces = interfaces.table().clone();

        cx.insert_state(Modules {
            context,
            worker,
            events,
            interfaces,
        });
        Ok(())
    }
}
