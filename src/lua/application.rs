//! Application targets: the supervisor that runs a package's `kind = "application"` target in a
//! [`Host`] of its own, and `@jess/application`, what the target's code asks of that host.
//!
//! An application is one [`ScriptDomain`]. Its package resolves with its dependencies exactly as a
//! command target's does; then, inside [`ScriptDomain::load`], in the domain's one primary
//! runtime:
//!
//! 1. the plan is resolved, every entrypoint admitted against the target's capabilities, and the
//!    plan mounted, before anything a package holds runs; then its layers' entrypoints run;
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

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use l3i::Runtime;
use l3i::debug::DebugScope;
use l3i::stack::{Frame, ValueView};
use l3i::value::{Function, Table, Value};

use crate::host::{ExitRequest, FrameReport, Host, HostSpec, InputSource};
use crate::lua::package::{self, Inspection, Managed, Search, Target, TargetRequest};
use crate::lua::{error, kind, state};
use crate::runtime::{
    CapabilitySet, ContextId, ContextSpec, Diagnostic, DiagnosticSink, DomainSpec, FixedStep, Generation, Phase,
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
    /// The contexts the last load declared, so the next one can retire those its code dropped.
    declared: RefCell<Vec<String>>,
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
        // From here until they are published, a lookup says the interfaces are being composed:
        // the layers' entrypoints run before any managed module, and are early too.
        crate::runtime::luau::compose_interfaces(runtime);
        package::run_entrypoints(runtime)?;
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
        let mut declaring = Vec::new();
        for (index, package, managed) in order {
            let declared = Self::run_managed(runtime, index, &package, &managed)?;
            declaring.push(declared.context.clone());
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
        // A module the code no longer has leaves no context behind.
        for name in self.declared.replace(declaring) {
            if !self.declared.borrow().contains(&name) {
                domain.remove_context(&name);
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
    diagnostics: Arc<Diagnostics>,
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
/// asks, into a host of its own. The domain's load resolves the package's plan and admits every
/// entrypoint against the target's capabilities before anything a package holds runs. With `input`
/// the host polls it every frame and runs the `input` handlers on what it produces.
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
    input: Option<InputSource>,
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
    let setup = Rc::new(Setup {
        package: inspection.id.clone(),
        target: target.id.clone(),
        entry: target.entry.clone(),
        locations,
        layers,
        capabilities: target.capabilities.clone(),
        search,
        args: request.args.clone(),
        declared: RefCell::new(Vec::new()),
    });
    let capabilities = target.capabilities.clone();
    let runtime_options = options.clone();
    let factory: Arc<dyn RuntimeFactory> =
        Arc::new(move || crate::lua::runtime_with_capabilities(runtime_options.clone(), &capabilities));
    let grant = RootAuthority::new().grant(CapabilitySet::from_names(&target.capabilities).map_err(l3i::Error::from)?);
    let diagnostics = Arc::new(Diagnostics::default());
    let spec = DomainSpec::new(format!("{}:{}", inspection.id, target.id), grant)
        .diagnostics(diagnostics.clone())
        .fixed_step(FixedStep {
            step: FRAME,
            max_catch_up: MAX_CATCH_UP,
        });
    let host_spec = match input {
        Some(source) => HostSpec::new(spec).input(source),
        None => HostSpec::new(spec),
    };
    let host = Host::create(factory, host_spec).map_err(l3i::Error::from)?;
    let loading = Rc::clone(&setup);
    if let Err(problem) = host.domain().load(|runtime| loading.load(runtime)) {
        // Every failure the load reported, each attributed to the context it happened in (a
        // module's load handler) or to the application, and the first one when none was.
        let names = host
            .domain()
            .contexts()
            .into_iter()
            .map(|context| (context.id, context.name))
            .collect::<BTreeMap<_, _>>();
        let mut lines = diagnostics
            .drain(true)
            .iter()
            .map(|held| tell(&setup, &names, held))
            .collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push(message_of(&problem));
        }
        drop(host.stop());
        return Err(error(format!(
            "{} target {} failed to load:\n{}",
            inspection.id,
            target.id,
            lines.join("\n")
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
    /// source line its handler was defined at. A run of identical diagnostics is its first and,
    /// when the run ends (or every few thousand), how many more there were; past a few thousand
    /// undrained, the rest are counted as dropped.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<String> {
        self.diagnostics
            .drain(false)
            .iter()
            .map(|held| tell(&self.setup, &self.names, held))
            .collect()
    }

    /// The diagnostics reported since the last call, as the runtime reported them: each distinct
    /// one, without the counts of its repeats or of what was dropped.
    #[must_use]
    pub fn drain_diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics
            .drain(false)
            .into_iter()
            .filter_map(|held| match held {
                Held::Reported(diagnostic) => Some(diagnostic),
                Held::Repeated { .. } | Held::Dropped(_) => None,
            })
            .collect()
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
    /// However it ends, it ends the same way: the application is stopped, shutdown handlers and
    /// all, and what they report is passed on before the run returns.
    ///
    /// # Errors
    ///
    /// Returns an error if the domain refuses to start or to advance a frame (it isn't in a state
    /// to), or input fails to poll; that failure, not one teardown met after it.
    pub fn run(mut self, frames: Option<u64>, interrupt: &AtomicBool, mut report: impl FnMut(&str)) -> l3i::Result<u8> {
        let outcome = self.drive(frames, interrupt, &mut report);
        let diagnostics = Arc::clone(&self.diagnostics);
        let names = std::mem::take(&mut self.names);
        let setup = Rc::clone(&self.setup);
        // Teardown removes the contexts, so the names its diagnostics need are kept apart.
        let stopped = self.stop();
        for held in diagnostics.drain(true) {
            report(&tell(&setup, &names, &held));
        }
        let status = outcome?;
        stopped.map_err(l3i::Error::from)?;
        Ok(status)
    }

    /// [`Application::run`] up to teardown: startup, then frames until something says stop.
    fn drive(&mut self, frames: Option<u64>, interrupt: &AtomicBool, report: &mut impl FnMut(&str)) -> l3i::Result<u8> {
        let started = self.start();
        self.flush(report);
        started.map_err(l3i::Error::from)?;
        let mut ran = 0u64;
        let mut next = Instant::now();
        let mut last = Instant::now();
        loop {
            if let Some(status) = self.exit_requested() {
                return Ok(status);
            }
            if frames.is_some_and(|frames| ran >= frames) {
                return Ok(0);
            }
            if interrupt.load(Ordering::Acquire) {
                return Ok(130);
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
            self.flush(report);
            outcome.map_err(|problem| error(problem.to_string()))?;
            ran += 1;
        }
    }

    fn flush(&self, report: &mut impl FnMut(&str)) {
        for line in self.diagnostics() {
            report(&line);
        }
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

/// How many diagnostics an application holds for its driver before it counts the rest as dropped.
const HELD: usize = 4_096;

/// How many repeats of one diagnostic go by between reports of how many there have been.
const REPEATS_REPORTED_EVERY: u64 = 3_600;

/// One thing an application reported, as its driver reads it.
enum Held {
    /// A diagnostic, the first of any run of identical ones.
    Reported(Diagnostic),
    /// The diagnostic before it, repeated `times` more times since it was last told.
    Repeated { of: Diagnostic, times: u64 },
    /// Diagnostics past [`HELD`] that nobody drained in time, counted and gone.
    Dropped(u64),
}

#[derive(Default)]
struct Holding {
    held: Vec<Held>,
    dropped: u64,
    /// The last diagnostic reported, kept across drains, and how many times it has repeated
    /// since that was last told.
    last: Option<Diagnostic>,
    repeats: u64,
}

impl Holding {
    fn tell_repeats(&mut self) {
        if self.repeats == 0 {
            return;
        }
        if let Some(last) = &self.last {
            let repeated = Held::Repeated {
                of: last.clone(),
                times: self.repeats,
            };
            self.hold(repeated);
        }
        self.repeats = 0;
    }

    fn hold(&mut self, held: Held) {
        if self.held.len() < HELD {
            self.held.push(held);
        } else {
            self.dropped += 1;
        }
    }
}

/// An application's diagnostics, held for whoever drives it: at most [`HELD`] of them, and a run
/// of identical ones as the first and a count, so a handler that fails every frame says so once
/// and then how often, not sixty times a second.
#[derive(Default)]
struct Diagnostics(Mutex<Holding>);

impl DiagnosticSink for Diagnostics {
    fn report(&self, diagnostic: Diagnostic) {
        let mut holding = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if holding.last.as_ref() == Some(&diagnostic) {
            holding.repeats += 1;
            if holding.repeats >= REPEATS_REPORTED_EVERY {
                holding.tell_repeats();
            }
            return;
        }
        holding.tell_repeats();
        holding.last = Some(diagnostic.clone());
        holding.hold(Held::Reported(diagnostic));
    }
}

impl Diagnostics {
    /// What was held since the last drain; with `ending`, the count of a run of repeats still
    /// going, too, since nothing will end it now.
    fn drain(&self, ending: bool) -> Vec<Held> {
        let (mut held, dropped) = {
            let mut holding = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            if ending {
                holding.tell_repeats();
            }
            (std::mem::take(&mut holding.held), std::mem::take(&mut holding.dropped))
        };
        if dropped > 0 {
            held.push(Held::Dropped(dropped));
        }
        held
    }
}

/// `held` as a line for people.
fn tell(setup: &Setup, names: &BTreeMap<ContextId, String>, held: &Held) -> String {
    match held {
        Held::Reported(diagnostic) => describe(setup, names, diagnostic),
        Held::Repeated { of, times } => {
            format!("{} (repeated {times} more times)", describe(setup, names, of))
        }
        Held::Dropped(count) => format!("{count} more diagnostics were dropped: nothing read them in time"),
    }
}
