//! Application targets: the supervisor that runs a package's `kind = "application"` target in a
//! [`Host`] of its own, and `@jess/application`, what the target's code asks of that host.
//!
//! An application is one [`ScriptDomain`]. Its package resolves with its dependencies exactly as a
//! command target's does; then, inside [`ScriptDomain::load`], in the domain's one primary
//! runtime:
//!
//! 1. the plan is mounted and its layers' entrypoints run, as for any activation;
//! 2. every selected package's `[managed]` modules run, packages in dependency order (smallest
//!    id first among packages free to go) and each package's modules by managed id. A module
//!    returns `{ on = { host = ..., event = ... }, interface = ... }`; the supervisor checks the
//!    shape, declares a context named `<package>/<managed>` that schedules exactly the host
//!    phases it declared (and the event phase for named events), and binds its handlers there;
//! 3. interfaces are composed in that same order: a later provider of an interface id replaces
//!    an earlier one, after its `on.host.interfaceOverride(previous)` ran with the table it
//!    replaces. Each table is frozen once its turn is done, and the effective ones are published
//!    together, so no lookup sees a half-composed registry;
//! 4. the target's private `entry` runs, its return values ignored;
//!
//! and then `load` handlers, and at [`Application::start`] `startup` handlers. Frames come from
//! whoever drives the application ([`Application::run`] for the command line), until the code
//! asks to stop (`application.exit`) or the driver stops it; [`Application::stop`] runs the
//! `shutdown` handlers and tears everything down.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use l3i::Runtime;
use l3i::debug::DebugScope;
use l3i::stack::{Frame, ValueView};
use l3i::value::{Function, Table, Value};

use crate::host::{ExitRequest, FrameReport, Host, HostSpec};
use crate::lua::package::{self, Inspection, Managed, Search, Target, TargetRequest};
use crate::lua::{error, kind, state};
use crate::runtime::{
    CapabilitySet, CollectedDiagnostics, ContextId, ContextSpec, Diagnostic, DomainSpec, FixedStep, Generation, Phase,
    PhaseReport, RootAuthority, RuntimeError, RuntimeFactory, ScriptDomain, TeardownReport,
};

/// `application.exit(status)`: asks the host to stop with `status` (0 without one) after the
/// current frame. Only the domain's own code asks; a worker tells the domain in a message.
fn exit(status: Option<f64>) -> l3i::Result<()> {
    let status = status.unwrap_or(0.0);
    if status.fract() != 0.0 || !(0.0..=f64::from(u8::MAX)).contains(&status) {
        return Err(error(format!(
            "application.exit: the status is an integer from 0 to 255, not {status}"
        )));
    }
    let no_host = || {
        error(
            "application.exit: needs an application host; jess run, jess test, and command targets end when their script returns",
        )
    };
    let authority = crate::runtime::current_authority().ok_or_else(no_host)?;
    if authority.attribution().worker.is_some() {
        return Err(error(
            "application.exit is for the domain's own code; a worker asks the domain in a message",
        ));
    }
    let request = authority.service::<ExitRequest>().ok_or_else(no_host)?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the guard above requires an integral value in 0..=255"
    )]
    request.request(status as u8);
    Ok(())
}

/// The `jess.application` table.
pub(crate) fn module(runtime: &Runtime) -> l3i::Result<Table> {
    let module = crate::lua::plan::Functions::module(runtime, "application")?;
    module.function("exit", exit)?;
    module.finish()
}

/// The host callbacks a managed module may declare under `on.host`, and the phase each binds to:
/// `interfaceOverride` binds to none, since only the loader calls it.
const HOST_CALLBACKS: [(&str, Option<Phase>); 10] = [
    ("load", Some(Phase::Load)),
    ("startup", Some(Phase::Startup)),
    ("fixedUpdate", Some(Phase::FixedUpdate)),
    ("update", Some(Phase::Update)),
    ("lateUpdate", Some(Phase::LateUpdate)),
    ("input", Some(Phase::Input)),
    ("renderPrepare", Some(Phase::RenderPrepare)),
    ("onDemand", Some(Phase::OnDemand)),
    ("shutdown", Some(Phase::Shutdown)),
    ("interfaceOverride", None),
];

/// How long one frame of [`Application::run`] is: the fixed step, and the pace frames are driven
/// at.
pub const FRAME: Duration = Duration::from_micros(16_667);

/// The most fixed steps one frame catches up on.
const MAX_CATCH_UP: u32 = 5;

/// A managed module that has run and passed its checks: what it declared, ready to bind.
struct Declared {
    /// `managed '<id>' of <package> (<chunk>)`, for messages.
    label: String,
    /// The context it runs in, `<package>/<managed>`.
    context: String,
    phases: Vec<(Phase, Function, String)>,
    events: Vec<(String, Function, String)>,
    interface: Option<(String, Table)>,
    replaces: Option<(Function, String)>,
}

/// One interface provider, in composition order.
struct Provider {
    label: String,
    context: ContextId,
    id: String,
    table: Table,
    replaces: Option<(Function, String)>,
}

/// Everything an application needs to load its code, again at every restart.
struct Setup {
    package: String,
    target: String,
    entry: String,
    locations: Vec<String>,
    layers: Vec<String>,
    capabilities: Vec<String>,
    search: Search,
    args: Vec<String>,
}

/// Where a function was defined, `source:line`, for diagnostics.
fn defined(frame: &Frame<'_>, function: ValueView<'_>) -> String {
    frame
        .function_info(function)
        .map(|info| format!("{}:{}", info.source, info.line_defined))
        .unwrap_or_default()
}

/// A stored function from `value`, or an error naming where it was and what it was instead.
fn handler(frame: &Frame<'_>, label: &str, path: &str, value: ValueView<'_>) -> l3i::Result<(Function, String)> {
    if !value.is_function() {
        return Err(error(format!("{label}: {path} is a {}, not a function", kind(value))));
    }
    Ok((Function::from_value(Value::store(value)?)?, defined(frame, value)))
}

/// The keys of the table `value`, which must all be strings: `what` names it in errors.
fn string_keys<'f>(frame: &'f Frame<'_>, label: &str, what: &str, table: ValueView<'f>) -> l3i::Result<Vec<String>> {
    let mut keys = Vec::new();
    table.as_table()?.for_each(frame, |_, key, _| {
        if !key.is_string() {
            return Err(error(format!(
                "{label}: {what} is keyed by names, and has a {} key",
                kind(key)
            )));
        }
        keys.push(key.read::<String>()?);
        Ok(())
    })?;
    keys.sort();
    Ok(keys)
}

/// Checks a managed module's `on` table and reads its host callbacks and named-event handlers
/// into `declared`.
fn read_on(frame: &Frame<'_>, label: &str, on: ValueView<'_>, declared: &mut Declared) -> l3i::Result<()> {
    if !on.is_table() {
        return Err(error(format!("{label}: on is a {}, not a table of host and event", kind(on))));
    }
    for key in string_keys(frame, label, "on", on)? {
        if key != "host" && key != "event" {
            return Err(error(format!(
                "{label}: on.{key} is not a handler group; expected host or event"
            )));
        }
    }
    let on = on.as_table()?;
    let host = on.raw_get(frame, "host")?;
    if !host.is_nil() {
        if !host.is_table() {
            return Err(error(format!("{label}: on.host is a {}, not a table", kind(host))));
        }
        for key in string_keys(frame, label, "on.host", host)? {
            let Some((_, phase)) = HOST_CALLBACKS.iter().find(|(name, _)| *name == key) else {
                return Err(unknown_callback(label, &key));
            };
            let value = host.as_table()?.raw_get(frame, key.as_str())?;
            let (function, source) = handler(frame, label, &format!("on.host.{key}"), value)?;
            match phase {
                Some(phase) => declared.phases.push((*phase, function, source)),
                None => declared.replaces = Some((function, source)),
            }
        }
    }
    let event = on.raw_get(frame, "event")?;
    if event.is_nil() {
        return Ok(());
    }
    if !event.is_table() {
        return Err(error(format!("{label}: on.event is a {}, not a table", kind(event))));
    }
    for key in string_keys(frame, label, "on.event", event)? {
        if key.is_empty() {
            return Err(error(format!("{label}: on.event has a handler for an empty event name")));
        }
        let value = event.as_table()?.raw_get(frame, key.as_str())?;
        let (function, source) = handler(frame, label, &format!("on.event[\"{key}\"]"), value)?;
        declared.events.push((key, function, source));
    }
    Ok(())
}

/// Checks what a managed module returned and reads it into a [`Declared`].
fn declared(
    frame: &Frame<'_>,
    package: &str,
    managed: &Managed,
    chunk: &str,
    value: ValueView<'_>,
) -> l3i::Result<Declared> {
    let label = format!("managed '{}' of {package} ({})", managed.id, chunk.trim_start_matches('@'));
    if !value.is_table() {
        return Err(error(format!(
            "{label}: returned a {}, not a table of on and interface",
            kind(value)
        )));
    }
    let table = value.as_table()?;
    for key in string_keys(frame, &label, "the returned table", value)? {
        if key != "on" && key != "interface" {
            return Err(error(format!(
                "{label}: returned an unknown key '{key}'; a managed module returns on and interface"
            )));
        }
    }
    let mut declared = Declared {
        context: format!("{package}/{}", managed.id),
        label: label.clone(),
        phases: Vec::new(),
        events: Vec::new(),
        interface: None,
        replaces: None,
    };
    let on = table.raw_get(frame, "on")?;
    if !on.is_nil() {
        read_on(frame, &label, on, &mut declared)?;
    }
    let interface = table.raw_get(frame, "interface")?;
    match (&managed.interface, interface.is_nil()) {
        (Some(id), true) => {
            return Err(error(format!(
                "{label}: [managed.{}] provides the interface '{id}', and the module returned no interface table",
                managed.id
            )));
        }
        (None, false) => {
            return Err(error(format!(
                "{label}: returned an interface, and [managed.{}] declares none; add interface = \"<id>\" to it",
                managed.id
            )));
        }
        (Some(id), false) => {
            if !interface.is_table() {
                return Err(error(format!("{label}: interface is a {}, not a table", kind(interface))));
            }
            declared.interface = Some((id.clone(), Table::from_value(Value::store(interface)?)?));
        }
        (None, true) => {}
    }
    if declared.replaces.is_some() && declared.interface.is_none() {
        return Err(error(format!(
            "{label}: on.host.interfaceOverride needs an interface to override, and [managed.{}] declares none",
            managed.id
        )));
    }
    Ok(declared)
}

fn unknown_callback(label: &str, key: &str) -> l3i::Error {
    let hint = match key {
        "event" => "raw events are @jess/context's; named events go in on.event".to_owned(),
        "message" | "timer" => format!("nothing in the kernel dispatches the {key} phase yet"),
        _ => {
            let names = HOST_CALLBACKS.iter().map(|(name, _)| *name).collect::<Vec<_>>();
            format!("expected {}", names.join(", "))
        }
    };
    error(format!("{label}: on.host.{key} is not a host callback; {hint}"))
}

/// The message of a runtime error, without the attribution a diagnostic carries anyway.
fn message_of(problem: &RuntimeError) -> String {
    match problem {
        RuntimeError::Script { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

impl Setup {
    /// Loads the application into `runtime`, the domain's primary one. See the module docs.
    fn load(&self, runtime: &Runtime) -> l3i::Result<()> {
        let domain = crate::runtime::luau::domain_of(runtime)
            .ok_or_else(|| error("an application loads into a ScriptDomain's primary runtime"))?;
        package::mount_with(runtime, &self.locations, &self.layers, Some(&self.capabilities), &self.search)?;
        package::run_entrypoints(runtime)?;
        crate::runtime::luau::compose_interfaces(runtime);
        let jess = state(runtime)?;
        let mut providers = Vec::new();
        let order = {
            let mounted = jess.packages.borrow();
            let mounted = mounted.as_ref().ok_or_else(|| error("the application mounted no plan"))?;
            mounted
                .dependency_order()
                .into_iter()
                .flat_map(|index| {
                    let id = mounted.id(index).to_owned();
                    mounted
                        .managed(index)
                        .iter()
                        .map(move |managed| (index, id.clone(), managed.clone()))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        for (index, package, managed) in order {
            let declared = Self::run_managed(runtime, index, &package, &managed)?;
            let context = domain
                .declare_context(
                    ContextSpec::new(&declared.context).schedule(
                        declared
                            .phases
                            .iter()
                            .map(|(phase, _, _)| *phase)
                            .chain((!declared.events.is_empty()).then_some(Phase::Event)),
                    ),
                )
                .map_err(|problem| error(format!("{}: {problem}", declared.label)))?;
            for (phase, function, source) in declared.phases {
                domain
                    .bind_handlers(&declared.context, vec![(phase, function)], Some(&source))
                    .map_err(|problem| error(format!("{}: {problem}", declared.label)))?;
            }
            domain
                .bind_event_handlers(
                    &declared.context,
                    declared
                        .events
                        .into_iter()
                        .map(|(event, function, source)| (event, function, Some(source)))
                        .collect(),
                )
                .map_err(|problem| error(format!("{}: {problem}", declared.label)))?;
            if let Some((id, table)) = declared.interface {
                providers.push(Provider {
                    label: declared.label,
                    context,
                    id,
                    table,
                    replaces: declared.replaces,
                });
            }
        }
        let mut effective: BTreeMap<String, Table> = BTreeMap::new();
        for provider in providers {
            if let (Some(previous), Some((callback, source))) = (effective.get(&provider.id), provider.replaces) {
                domain
                    .call_in_context(runtime, provider.context, Phase::Load, Some(source), callback, |callback| {
                        callback.invoke::<(), _>(&runtime.stack(), (previous,))
                    })
                    .map_err(|problem| {
                        error(format!(
                            "{}: on.host.interfaceOverride of '{}' failed: {}",
                            provider.label,
                            provider.id,
                            message_of(&problem)
                        ))
                    })?;
            }
            // Its turn is done: frozen, like a package export, so neither a consumer nor the next
            // provider can patch it for the whole domain.
            runtime.stack().with_frame(|frame| {
                let table = provider.table.push_to(frame)?;
                if !table.is_read_only() {
                    table.set_read_only(true)?;
                }
                Ok(())
            })?;
            effective.insert(provider.id, provider.table);
        }
        crate::runtime::luau::publish_interfaces(runtime, effective);
        self.run_entry(runtime)
    }

    /// Runs one managed module and checks what it returned. It loads as `require` and a layer's
    /// entrypoints load a package module: once per runtime, cached by its package identity, so
    /// a relative `require` of the same file, before or after, is the same instance.
    fn run_managed(runtime: &Runtime, index: usize, package: &str, managed: &Managed) -> l3i::Result<Declared> {
        let jess = state(runtime)?;
        let stack = runtime.stack();
        let module = {
            let mounted = jess.packages.borrow();
            let mounted = mounted.as_ref().ok_or_else(|| error("the application mounted no plan"))?;
            mounted.load(&stack, index, &managed.module)?.ok_or_else(|| {
                error(format!(
                    "managed '{}' of {package}: {package} doesn't hold '{}'",
                    managed.id, managed.module
                ))
            })?
        };
        let value = stack.with_frame(|frame| {
            let loaded = jess.loaded.push_to(frame)?;
            let cached = loaded.raw_get(frame, &module.key)?;
            if !cached.is_nil() {
                return Value::store(cached);
            }
            let chunk = jess.load_as(frame, &module.key, &module.chunk, &module.source)?;
            let value = chunk
                .invoke_with(frame, (managed.id.as_str(),), |_, value| Value::store(value))
                .map_err(|problem| {
                    error(format!(
                        "managed '{}' of {package} ({}) failed to load: {}",
                        managed.id,
                        module.chunk.trim_start_matches('@'),
                        crate::lua::error_message(&problem)
                    ))
                })?;
            crate::lua::require::freeze_export(&module.key, value.push_to(frame)?)?;
            value.push_to(frame)?;
            loaded.raw_set(frame, &module.key)?;
            Ok(value)
        })?;
        stack.with_frame(|frame| declared(frame, package, managed, &module.chunk, value.push_to(frame)?))
    }

    /// Runs the target's entry as a script, with the target's arguments; what it returns is
    /// ignored.
    fn run_entry(&self, runtime: &Runtime) -> l3i::Result<()> {
        let jess = state(runtime)?;
        let (chunk, function) = {
            let mounted = jess.packages.borrow();
            let mounted = mounted.as_ref().ok_or_else(|| error("the application mounted no plan"))?;
            let index = mounted
                .index_of(&self.package)
                .ok_or_else(|| error(format!("{} activated without itself", self.package)))?;
            let stack = runtime.stack();
            let module = mounted.load(&stack, index, &self.entry)?.ok_or_else(|| {
                error(format!(
                    "target {}: {} doesn't hold '{}'",
                    self.target, self.package, self.entry
                ))
            })?;
            let function = jess.load_as(&stack, &module.key, &module.chunk, &module.source)?;
            (module.chunk, function)
        };
        crate::lua::scripts::run_loaded(runtime, chunk.trim_start_matches('@'), &function, &self.args)?;
        Ok(())
    }
}

/// An application target, loaded into a host of its own and ready to start. See the module docs.
pub struct Application {
    host: Host,
    setup: Rc<Setup>,
    diagnostics: Arc<CollectedDiagnostics>,
    /// Context names by id, kept past teardown for the diagnostics it reports.
    names: BTreeMap<ContextId, String>,
}

impl std::fmt::Debug for Application {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Application")
            .field("package", &self.setup.package)
            .field("target", &self.setup.target)
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

/// Loads the application target `target` of the package `inspection` describes, as `request`
/// asks, into a host of its own: the package's plan is checked against the target's capabilities
/// before anything is built, and nothing a package holds runs until the domain loads.
///
/// # Errors
///
/// Returns an error if the plan doesn't resolve or needs a capability the target doesn't declare,
/// the host can't be built, or the application fails to load: a malformed managed module, an
/// override that raised, an entry that raised, or a `load` handler that did.
pub(crate) fn launch(
    options: &crate::lua::Options,
    request: &TargetRequest,
    inspection: &Inspection,
    target: &Target,
) -> l3i::Result<Application> {
    let layers = inspection.layers.iter().map(|layer| layer.id.clone()).collect::<Vec<_>>();
    let mut locations = vec![request.package.clone()];
    locations.extend_from_slice(&request.packages);
    let search = Search {
        paths: request.paths.clone(),
        candidates: request.candidates.clone(),
        pins: request.pins.clone(),
        roots: vec![inspection.id.clone()],
        dev: Vec::new(),
    };
    // The whole plan is resolved and admitted against the target's capabilities in a runtime of
    // its own first, so a plan that can't run fails before any domain exists.
    package::mount_with(
        &crate::lua::runtime(options.clone())?,
        &locations,
        &layers,
        Some(&target.capabilities),
        &search,
    )?;
    let setup = Rc::new(Setup {
        package: inspection.id.clone(),
        target: target.id.clone(),
        entry: target.entry.clone(),
        locations,
        layers,
        capabilities: target.capabilities.clone(),
        search,
        args: request.args.clone(),
    });
    let capabilities = target.capabilities.clone();
    let runtime_options = options.clone();
    let factory: Arc<dyn RuntimeFactory> =
        Arc::new(move || crate::lua::runtime_with_capabilities(runtime_options.clone(), &capabilities));
    let grant = RootAuthority::new().grant(CapabilitySet::from_names(&target.capabilities).map_err(l3i::Error::from)?);
    let diagnostics = Arc::new(CollectedDiagnostics::new());
    let spec = DomainSpec::new(format!("{}:{}", inspection.id, target.id), grant)
        .diagnostics(diagnostics.clone())
        .fixed_step(FixedStep {
            step: FRAME,
            max_catch_up: MAX_CATCH_UP,
        });
    let host = Host::create(factory, HostSpec::new(spec)).map_err(l3i::Error::from)?;
    let loading = Rc::clone(&setup);
    if let Err(problem) = host.domain().load(|runtime| loading.load(runtime)) {
        drop(host.stop());
        return Err(error(format!(
            "{} target {}: {}",
            inspection.id,
            target.id,
            message_of(&problem)
        )));
    }
    let mut application = Application {
        host,
        setup,
        diagnostics,
        names: BTreeMap::new(),
    };
    application.remember_contexts();
    Ok(application)
}

impl Application {
    /// The host the application runs in.
    #[must_use]
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The application's domain.
    #[must_use]
    pub fn domain(&self) -> &ScriptDomain {
        self.host.domain()
    }

    fn remember_contexts(&mut self) {
        for context in self.host.domain().contexts() {
            self.names.insert(context.id, context.name);
        }
    }

    /// Starts the application: its `startup` handlers run.
    ///
    /// # Errors
    ///
    /// As [`ScriptDomain::start`]. A handler that raised is in the report and the diagnostics.
    pub fn start(&self) -> Result<PhaseReport, RuntimeError> {
        self.host.domain().start()
    }

    /// One frame of `delta`: input, fixed steps, update, late update, and the event drain.
    ///
    /// # Errors
    ///
    /// As [`Host::frame`].
    pub fn frame(&mut self, delta: Duration) -> Result<FrameReport, crate::host::Error> {
        self.host.frame(delta)
    }

    /// The status the application's code asked to stop with, if it asked.
    #[must_use]
    pub fn exit_requested(&self) -> Option<u8> {
        self.host.exit_requested()
    }

    /// Restarts the application as a new generation: shutdown handlers run, and its code loads
    /// again from the packages, all of it, in a new runtime: module tables, handlers, and
    /// interfaces are new, and nothing of the old generation is reachable or delivered.
    ///
    /// # Errors
    ///
    /// As [`Host::restart`]; a failed load leaves nothing running.
    pub fn restart(&mut self) -> Result<Generation, RuntimeError> {
        let loading = Rc::clone(&self.setup);
        let generation = self.host.restart(|runtime| loading.load(runtime));
        self.remember_contexts();
        generation
    }

    /// The diagnostics reported since the last call, each as a line for people: what failed,
    /// where (the managed module's context, by name), in which phase or named event, and the
    /// source line its handler was defined at.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<String> {
        self.diagnostics
            .drain()
            .into_iter()
            .map(|diagnostic| self.describe(&diagnostic))
            .collect()
    }

    /// The diagnostics reported since the last call, as the runtime reported them.
    #[must_use]
    pub fn drain_diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics.drain()
    }

    /// `diagnostic` as a line for people.
    #[must_use]
    pub fn describe(&self, diagnostic: &Diagnostic) -> String {
        describe(&self.setup, &self.names, diagnostic)
    }

    /// Tears the application down: `shutdown` handlers, then the domain and host.
    ///
    /// # Errors
    ///
    /// As [`ScriptDomain::stop`].
    pub fn stop(self) -> Result<TeardownReport, RuntimeError> {
        self.host.stop()
    }

    /// Runs the application to the end: starts it, drives frames of [`FRAME`] at that pace until
    /// its code asks to stop, `frames` frames have run, or `interrupt` is set, and stops it,
    /// passing each diagnostic line to `report` as it comes. The status is the one the code asked
    /// for, 0 when it ran out of frames, and 130 when interrupted.
    ///
    /// # Errors
    ///
    /// Returns an error if the domain refuses a frame (it isn't running), or input fails to poll.
    pub fn run(mut self, frames: Option<u64>, interrupt: &AtomicBool, mut report: impl FnMut(&str)) -> l3i::Result<u8> {
        let flush = |application: &Application, report: &mut dyn FnMut(&str)| {
            for line in application.diagnostics() {
                report(&line);
            }
        };
        let started = self.start();
        flush(&self, &mut report);
        started.map_err(l3i::Error::from)?;
        let mut ran = 0u64;
        let mut next = Instant::now();
        let mut last = Instant::now();
        let status = loop {
            if let Some(status) = self.exit_requested() {
                break status;
            }
            if frames.is_some_and(|frames| ran >= frames) {
                break 0;
            }
            if interrupt.load(Ordering::Acquire) {
                break 130;
            }
            next += FRAME;
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            } else {
                // Behind: frames resume from now, and the fixed step catches up what it may.
                next = Instant::now();
            }
            let now = Instant::now();
            let outcome = self.frame(now - last);
            last = now;
            flush(&self, &mut report);
            outcome.map_err(|problem| error(problem.to_string()))?;
            ran += 1;
        };
        let diagnostics = Arc::clone(&self.diagnostics);
        let names = std::mem::take(&mut self.names);
        let setup = Rc::clone(&self.setup);
        // Teardown removes the contexts, so the names its diagnostics need are kept apart.
        let stopped = self.stop();
        for diagnostic in diagnostics.drain() {
            report(&describe(&setup, &names, &diagnostic));
        }
        stopped.map_err(l3i::Error::from)?;
        Ok(status)
    }
}

/// `diagnostic` as a line for people: the context it happened in by name (the application's
/// `<package>:<target>` when none), then phase, named event, and source.
fn describe(setup: &Setup, names: &BTreeMap<ContextId, String>, diagnostic: &Diagnostic) -> String {
    let place = diagnostic
        .attribution
        .context
        .and_then(|context| names.get(&context).cloned())
        .unwrap_or_else(|| format!("{}:{}", setup.package, setup.target));
    let phase = diagnostic.phase.map(|phase| format!(" during {phase}")).unwrap_or_default();
    let event = diagnostic
        .event
        .as_ref()
        .map(|event| format!(" '{event}'"))
        .unwrap_or_default();
    let source = diagnostic
        .source
        .as_ref()
        .map(|source| format!(" ({source})"))
        .unwrap_or_default();
    format!(
        "{} in {place}{phase}{event}{source}: {}",
        diagnostic.severity, diagnostic.message
    )
}
