//! `jess.input`: the native half of input for scripts, under `@jess/input`.
//!
//! Luau writes the binding document (`@jess/input`'s builders, in the built-in package) and hands
//! it over as JSON; `graph` compiles it into an `InputGraph` the crate has validated, and a
//! `runtime` installs a graph, takes synthetic events, and reports action state. Labels, binding
//! presentation, conflict checks, and intent routing are Luau too, over the document and the
//! action events. Nothing here is a callback: a script asks for the state after the tick, and no
//! device event ever calls into Luau.
//!
//! Which runtime `jess.input.runtime()` hands out depends on where the script runs. Under a
//! `ScriptDomain` that carries a `Mutex<jess::input::InputService>` service (`jess::host` attaches
//! one), it is the application's live input: a platform backend feeds it, the host ticks it in
//! the input phase and delivers that tick's action events as the phase payload, and the script
//! configures it (installs the graph, activates sets) and reads it after the tick. `tick` and
//! `advance` are the host's there, and refused. Anywhere else (a plain `jess run`, a domain
//! without the service, a worker not given it) the runtime is a fresh headless one with a
//! synthetic source, ticked by the script: what tests and replays need.
//!
//! Every runtime method checks a capability (`jess::input::capability`): reading action state and
//! events is `input.actions`; device descriptors are `input.observe`; installing, activating,
//! and synthetic events are `input.configure` ([`crate::lua::guard`]).
//!
//! Durations are seconds in every function. Action values are booleans, numbers, and `vector`s
//! with a zero `z`.

use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::input::{
    ActionEventKind, ActionGraph, ActionState, Control, DeviceDescriptor, DeviceId, FORMAT_VERSION, GraphSpec, InputService,
    Timestamp, Value, capability,
};
use crate::lua::plan::Functions;
use crate::lua::{Jess, Text, error, guard, push_table};
use l3i::Runtime;
use l3i::bind::{Call, StackResults};
use l3i::convert::Vector3;
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::stack::{Frame, Scope, TableView, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::Table;

/// A compiled, validated action graph.
#[derive(Clone)]
pub struct LuaInputGraph {
    graph: Rc<ActionGraph>,
}

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaInputGraph {
    const NAME: &'static str = "jess.input.Graph";
}

/// The service type a domain carries for its input: the live [`InputService`] `jess::host`
/// attaches, ticks, and `jess.input` binds to.
pub type SharedInput = Mutex<InputService>;

/// An input runtime handle: the domain's live service, or a headless one of the script's own.
#[derive(Clone)]
pub struct LuaInputRuntime {
    service: Arc<SharedInput>,
    /// True for the domain's service: the host ticks it and owns its clock.
    live: bool,
    jess: Rc<Jess>,
    /// The windows' keyboard and pointer, polled into a headless runtime at each of its ticks.
    #[cfg(feature = "window")]
    windows: Option<Rc<std::cell::RefCell<crate::window::WindowInput>>>,
}

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaInputRuntime {
    const NAME: &'static str = "jess.input.Runtime";
}

impl LuaInputRuntime {
    /// The service, locked. A poisoned lock still holds a whole service: whatever panicked
    /// while holding it was a native bug elsewhere, not a half-written tick.
    fn lock(&self) -> MutexGuard<'_, InputService> {
        self.service.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What a live runtime refuses: the host ticks it and moves its clock.
    fn host_owned(&self, method: &str) -> l3i::Result<()> {
        if self.live {
            return Err(error(format!(
                "InputRuntime:{method} is the host's on a live input runtime: it ticks in the input phase; bind a context to that phase and read the payload"
            )));
        }
        Ok(())
    }
}

/// The device a script numbered, in `service`.
fn device(service: &InputService, number: f64) -> l3i::Result<DeviceId> {
    // Device numbers are small whole numbers a script got from `connect` or a device list.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = if number.fract() == 0.0 && number >= 0.0 {
        number as u32
    } else {
        u32::MAX
    };
    service
        .device(index)
        .ok_or_else(|| error(format!("no device {number} is known to this runtime")))
}

fn control(text: &Text) -> l3i::Result<Control> {
    text.0.parse().map_err(error)
}

fn seconds(timestamp: Timestamp) -> f64 {
    Duration::from_nanos(timestamp.nanos()).as_secs_f64()
}

/// Every compile error, one per line, as one Luau error.
fn compile_error(errors: Vec<crate::input::GraphError>) -> l3i::Error {
    let mut message = String::from("the action graph doesn't compile:");
    for problem in errors {
        message.push_str("\n  ");
        message.push_str(&problem.to_string());
    }
    error(message)
}

fn compile(spec: GraphSpec) -> l3i::Result<Owned<LuaInputGraph>> {
    let graph = ActionGraph::compile(spec).map_err(compile_error)?;
    Ok(Owned(LuaInputGraph { graph: Rc::new(graph) }))
}

const fn event_kind_name(kind: ActionEventKind) -> &'static str {
    match kind {
        ActionEventKind::Pressed => "pressed",
        ActionEventKind::Released => "released",
        ActionEventKind::Changed => "changed",
    }
}

fn push_value(frame: &Frame<'_>, value: Value) -> l3i::Result<()> {
    match value {
        Value::Digital(on) => {
            frame.push_boolean(on)?;
        }
        Value::Scalar(number) => {
            frame.push_number(f64::from(number))?;
        }
        Value::Vector2([x, y]) => {
            frame.push(&Vector3 { x, y, z: 0.0 })?;
        }
    }
    Ok(())
}

fn set_value(frame: &Frame<'_>, table: &TableView<'_>, key: &str, value: Value) -> l3i::Result<()> {
    push_value(frame, value)?;
    table.raw_set(frame, key)
}

fn push_action_state(scope: &impl Scope, state: &ActionState) -> l3i::Result<()> {
    push_table(scope, 0, 7, |frame, table| {
        set_value(frame, table, "value", state.value)?;
        table.raw_set_value(frame, "active", &state.active)?;
        table.raw_set_value(frame, "pressed", &state.pressed_this_tick)?;
        table.raw_set_value(frame, "released", &state.released_this_tick)?;
        table.raw_set_value(frame, "changed", &state.changed_this_tick)?;
        table.raw_set_value(frame, "since", &seconds(state.since))?;
        table.raw_set_value(frame, "device", &state.device.map(|device| f64::from(device.index())))
    })
}

pub fn module(runtime: &Runtime, jess: &Rc<Jess>) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "input")?;
    // The document arrives as JSON: `@jess/input` encodes a Luau document, and a saved one is
    // already text.
    module.function("graph", |text: Text| -> l3i::Result<Owned<LuaInputGraph>> {
        compile(GraphSpec::from_json(&text.0).map_err(error)?)
    })?;
    module.set("formatVersion", &f64::from(FORMAT_VERSION))?;
    // The control vocabulary is device truth: a script names a control, and this checks it.
    module.function("control", |text: Text| -> l3i::Result<String> {
        Ok(control(&text)?.to_string())
    })?;
    let state = jess.clone();
    module.function(
        "runtime",
        move |call: &Call<'_>, options: Option<ValueView<'_>>| -> l3i::Result<Owned<LuaInputRuntime>> {
            const FUNCTION: &str = "jess.input.runtime";
            let windows = call.with_frame(|frame| windows_option(frame, FUNCTION, options))?;
            let (service, live) = crate::runtime::service::<SharedInput>().map_or_else(
                || (Arc::new(Mutex::new(InputService::new())), false),
                |service| (service, true),
            );
            if windows && live {
                return Err(error(format!(
                    "{FUNCTION}: the live input runtime takes its windows' input from its host already"
                )));
            }
            Ok(Owned(LuaInputRuntime {
                service,
                live,
                jess: state.clone(),
                #[cfg(feature = "window")]
                windows: windows.then(|| Rc::new(std::cell::RefCell::new(crate::window::WindowInput::new(None)))),
            }))
        },
    )?;
    module.finish()
}

/// Whether `options`, `{ windows }`, ask a headless runtime for its windows' keyboard and pointer.
fn windows_option(frame: &Frame<'_>, function: &str, options: Option<ValueView<'_>>) -> l3i::Result<bool> {
    let Some(options) = options.filter(|options| !options.is_nil()) else {
        return Ok(false);
    };
    crate::lua::check_options(frame, options, function, &["windows"])?;
    let windows = options.as_table()?.raw_get(frame, "windows")?;
    if windows.is_nil() {
        return Ok(false);
    }
    if !windows.is_boolean() {
        return Err(error(format!(
            "{function}: windows is a boolean, not {}",
            crate::lua::kind(windows)
        )));
    }
    let wanted = windows.read::<bool>()?;
    if wanted && !cfg!(feature = "window") {
        return Err(error(format!(
            "{function}: this jess was built without the window feature, so it has no windows"
        )));
    }
    Ok(wanted)
}

/// Declares the graph and runtime types.
#[allow(clippy::too_many_lines)] // one method per operation
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut graph = d.userdata::<LuaInputGraph>(LuaInputGraph::NAME);
    graph
        .tag(TagPolicy::Never)
        .doc("A compiled action graph: actions, sets, layers, and their bindings, validated.");
    graph
        .method("toJson", |this: &LuaInputGraph| this.graph.spec().to_json())
        .untyped();
    graph
        .method(
            "actions",
            |this: &LuaInputGraph, call: &Call<'_>| -> l3i::Result<StackResults> {
                l3i::stack::push_list(call, this.graph.actions().iter(), |frame, action| {
                    push_table(frame, 0, 2, |frame, row| {
                        row.raw_set_value(frame, "name", action.name())?;
                        row.raw_set_value(frame, "valueType", action.value_type().name())
                    })
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    let mut runtime = d.userdata::<LuaInputRuntime>(LuaInputRuntime::NAME);
    runtime.tag(TagPolicy::Never).doc(
        "An input runtime: the application's live one under a host, or a headless one with synthetic devices, ticked by the script.",
    );
    runtime
        .method(
            "install",
            |this: &LuaInputRuntime, graph: ValueView<'_>| -> l3i::Result<f64> {
                guard(&this.jess, "jess.input", capability::CONFIGURE)?;
                let Some(graph) = l3i::userdata::receiver::<LuaInputGraph>(graph) else {
                    return Err(error(
                        "InputRuntime:install takes an InputGraph: compile the document with @jess/input's graph",
                    ));
                };
                let spec = graph.graph.spec().clone();
                let generation = this.lock().install(spec).map_err(compile_error)?;
                #[allow(clippy::cast_precision_loss)] // Generations are counted in small numbers.
                Ok(generation.value() as f64)
            },
        )
        .untyped();
    runtime
        .method("uninstall", |this: &LuaInputRuntime| -> l3i::Result<()> {
            guard(&this.jess, "jess.input", capability::CONFIGURE)?;
            this.lock().uninstall();
            Ok(())
        })
        .untyped();
    runtime
        .method("restoreDefaults", |this: &LuaInputRuntime| -> l3i::Result<()> {
            guard(&this.jess, "jess.input", capability::CONFIGURE)?;
            this.lock().restore_defaults().map_err(error)?;
            Ok(())
        })
        .untyped();
    runtime
        .method(
            "setActive",
            |this: &LuaInputRuntime, set: Text, active: Option<bool>| -> l3i::Result<()> {
                guard(&this.jess, "jess.input", capability::CONFIGURE)?;
                this.lock().set_active(&set.0, active.unwrap_or(true)).map_err(error)
            },
        )
        .untyped();
    runtime
        .method("pushLayer", |this: &LuaInputRuntime, layer: Text| -> l3i::Result<()> {
            guard(&this.jess, "jess.input", capability::CONFIGURE)?;
            this.lock().push_layer(&layer.0).map_err(error)
        })
        .untyped();
    runtime
        .method("popLayer", |this: &LuaInputRuntime, layer: Text| -> l3i::Result<bool> {
            guard(&this.jess, "jess.input", capability::CONFIGURE)?;
            this.lock().pop_layer(&layer.0).map_err(error)
        })
        .untyped();
    runtime
        .method("connect", |this: &LuaInputRuntime, kind: Text| -> l3i::Result<f64> {
            guard(&this.jess, "jess.input", capability::CONFIGURE)?;
            let mut service = this.lock();
            let source = service.synthetic();
            let device = match kind.0.as_str() {
                "keyboard" => source.connect_keyboard(),
                "pointer" => source.connect_pointer(),
                "gamepad" => source.connect_gamepad(),
                "touch" => source.connect_touch(),
                other => {
                    return Err(error(format!(
                        "InputRuntime:connect takes keyboard, pointer, gamepad, or touch, not {other:?}"
                    )));
                }
            };
            drop(service);
            Ok(f64::from(device.index()))
        })
        .untyped();
    runtime
        .method("disconnect", |this: &LuaInputRuntime, number: f64| -> l3i::Result<()> {
            guard(&this.jess, "jess.input", capability::CONFIGURE)?;
            let mut service = this.lock();
            let device = device(&service, number)?;
            service.synthetic().disconnect(device);
            drop(service);
            Ok(())
        })
        .untyped();
    runtime
        .method(
            "press",
            |this: &LuaInputRuntime, number: f64, control_text: Text| -> l3i::Result<()> {
                guard(&this.jess, "jess.input", capability::CONFIGURE)?;
                let mut service = this.lock();
                let (device, control) = (device(&service, number)?, control(&control_text)?);
                service.synthetic().press(device, control);
                drop(service);
                Ok(())
            },
        )
        .untyped();
    runtime
        .method(
            "release",
            |this: &LuaInputRuntime, number: f64, control_text: Text| -> l3i::Result<()> {
                guard(&this.jess, "jess.input", capability::CONFIGURE)?;
                let mut service = this.lock();
                let (device, control) = (device(&service, number)?, control(&control_text)?);
                service.synthetic().release(device, control);
                drop(service);
                Ok(())
            },
        )
        .untyped();
    runtime
        .method(
            "axis",
            |this: &LuaInputRuntime, number: f64, control_text: Text, value: f64| -> l3i::Result<()> {
                guard(&this.jess, "jess.input", capability::CONFIGURE)?;
                let mut service = this.lock();
                let (device, control) = (device(&service, number)?, control(&control_text)?);
                #[allow(clippy::cast_possible_truncation)] // Axis values are normalized.
                service.synthetic().axis(device, control, value as f32);
                drop(service);
                Ok(())
            },
        )
        .untyped();
    runtime
        .method("focus", |this: &LuaInputRuntime, focused: bool| -> l3i::Result<()> {
            guard(&this.jess, "jess.input", capability::CONFIGURE)?;
            this.lock().synthetic().focus(focused);
            Ok(())
        })
        .untyped();
    runtime
        .method("advance", |this: &LuaInputRuntime, seconds: f64| -> l3i::Result<()> {
            guard(&this.jess, "jess.input", capability::CONFIGURE)?;
            this.host_owned("advance")?;
            if !(seconds.is_finite() && seconds >= 0.0) {
                return Err(error(format!(
                    "InputRuntime:advance takes a duration in seconds, not {seconds}"
                )));
            }
            this.lock().advance(Duration::from_secs_f64(seconds));
            Ok(())
        })
        .untyped();
    runtime
        .method("now", |this: &LuaInputRuntime| -> l3i::Result<f64> {
            guard(&this.jess, "jess.input", capability::ACTIONS)?;
            Ok(seconds(this.lock().now()))
        })
        .untyped();
    runtime
        .method(
            "tick",
            |this: &LuaInputRuntime, call: &Call<'_>| -> l3i::Result<StackResults> {
                guard(&this.jess, "jess.input", capability::ACTIONS)?;
                this.host_owned("tick")?;
                let mut service = this.lock();
                #[cfg(feature = "window")]
                if let Some(windows) = &this.windows {
                    use crate::input::Backend as _;
                    let now = service.now();
                    windows
                        .borrow_mut()
                        .poll(&service.sender(), now)
                        .map_err(|problem| error(format!("InputRuntime:tick: the windows' input: {problem}")))?;
                }
                let rows = tick_rows(&mut service);
                drop(service);
                push_rows(call, &rows)?;
                Ok(StackResults)
            },
        )
        .untyped();
    runtime
        .method(
            "action",
            |this: &LuaInputRuntime, call: &Call<'_>, name: Text| -> l3i::Result<StackResults> {
                guard(&this.jess, "jess.input", capability::ACTIONS)?;
                let action = this.lock().runtime().action(&name.0).copied();
                match action {
                    Some(action) => push_action_state(call, &action)?,
                    None => {
                        call.push(&())?;
                    }
                }
                Ok(StackResults)
            },
        )
        .untyped();
    runtime
        .method(
            "devices",
            |this: &LuaInputRuntime, call: &Call<'_>| -> l3i::Result<StackResults> {
                guard(&this.jess, "jess.input", capability::OBSERVE)?;
                let devices: Vec<(DeviceId, DeviceDescriptor, bool)> = this
                    .lock()
                    .runtime()
                    .state()
                    .devices()
                    .map(|device| (device.id(), device.descriptor().clone(), device.is_connected()))
                    .collect();
                l3i::stack::push_list(call, devices.iter(), |frame, (id, descriptor, connected)| {
                    push_table(frame, 0, 5, |frame, row| {
                        row.raw_set_value(frame, "id", &f64::from(id.index()))?;
                        row.raw_set_value(frame, "kind", descriptor.kind.name())?;
                        row.raw_set_value(frame, "name", descriptor.name.as_str())?;
                        row.raw_set_value(frame, "family", descriptor.family.name())?;
                        row.raw_set_value(frame, "connected", connected)
                    })
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    runtime
        .method(
            "stats",
            |this: &LuaInputRuntime, call: &Call<'_>| -> l3i::Result<StackResults> {
                guard(&this.jess, "jess.input", capability::ACTIONS)?;
                let stats = this.lock().runtime().stats();
                push_table(call, 0, 8, |frame, table| {
                    table.raw_set_value(frame, "deviceEvents", &f64::from(stats.device_events))?;
                    #[allow(clippy::cast_precision_loss)] // Counts, not identities.
                    table.raw_set_value(frame, "ticks", &(stats.evaluation.ticks as f64))?;
                    table.raw_set_value(frame, "nodesEvaluated", &f64::from(stats.evaluation.nodes_evaluated))?;
                    table.raw_set_value(frame, "bindingsEvaluated", &f64::from(stats.evaluation.bindings_evaluated))?;
                    table.raw_set_value(frame, "actionsChanged", &f64::from(stats.evaluation.actions_changed))?;
                    table.raw_set_value(frame, "actionEvents", &f64::from(stats.evaluation.action_events))?;
                    table.raw_set_value(frame, "activeSets", &f64::from(stats.evaluation.active_sets))?;
                    table.raw_set_value(frame, "activeLayers", &f64::from(stats.evaluation.active_layers))
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
}

/// One action event as a script sees it: `{ action, kind, value, time, device }`, the same row
/// the host puts in the input phase's payload.
struct Row {
    action: String,
    kind: ActionEventKind,
    value: Value,
    time: Timestamp,
    device: Option<DeviceId>,
}

/// Ticks a script-driven service at its own clock and returns the tick's events as rows.
fn tick_rows(service: &mut InputService) -> Vec<Row> {
    let names = action_names(service);
    let now = service.now();
    let tick = service.tick(now);
    tick.events
        .iter()
        .map(|event| Row {
            action: names[event.action.index() as usize].clone(),
            kind: event.kind,
            value: event.value,
            time: event.timestamp,
            device: event.device,
        })
        .collect()
}

fn push_rows(scope: &impl Scope, rows: &[Row]) -> l3i::Result<()> {
    l3i::stack::push_list(scope, rows.iter(), |frame, row| {
        push_table(frame, 0, 5, |frame, table| {
            table.raw_set_value(frame, "action", row.action.as_str())?;
            table.raw_set_value(frame, "kind", event_kind_name(row.kind))?;
            set_value(frame, table, "value", row.value)?;
            table.raw_set_value(frame, "time", &seconds(row.time))?;
            table.raw_set_value(frame, "device", &row.device.map(|device| f64::from(device.index())))
        })
    })
}

/// The installed graph's action names by id, so a row names its action without borrowing the
/// graph across a tick.
fn action_names(service: &InputService) -> Vec<String> {
    service
        .runtime()
        .graph()
        .map(|graph| graph.actions().iter().map(|action| action.name().to_owned()).collect())
        .unwrap_or_default()
}
