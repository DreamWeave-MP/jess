//! `jess.input`: the action graph and an input runtime, for scripts.
//!
//! Luau writes the binding document and the native side owns it: the authoring functions
//! (`key`, `chord`, `tap`, `vector2`, ...) return plain node tables, `graph` compiles a document
//! of them into an `InputGraph` the crate has validated, and a `runtime` installs a graph, takes
//! synthetic events, and reports action state. Nothing here is a callback: a script asks for the
//! state after the tick, and no device event ever calls into Luau.
//!
//! Which runtime `jess.input.runtime()` hands out depends on where the script runs. Under a
//! `ScriptDomain` that carries a `Mutex<jess::input::InputService>` service (`jess::host` attaches
//! one), it is the application's live input: a platform backend feeds it, the host ticks it in
//! the input phase and delivers that tick's action events and intents as the phase payload, and
//! the script configures it (installs the graph, activates sets, routes intents) and reads it
//! after the tick. `tick` and `advance` are the host's there, and refused. Anywhere else (a plain
//! `jess run`, a domain without the service, a worker not given it) the runtime is a fresh
//! headless one with a synthetic source, ticked by the script: what tests and replays need.
//!
//! Every runtime method checks a capability under a domain (`jess::input::capability`): reading
//! action state, events, and intents is `input.actions`; device descriptors are `input.observe`;
//! installing, activating, routing, and synthetic events are `input.configure`. Outside any
//! domain nothing holds authority and the calls succeed ([`crate::lua::guard`]).
//!
//! Durations are seconds in every function and millisecond fields in the document, which is what
//! `jess::input` stores. Action values are booleans, numbers, and `vector`s with a zero `z`.

use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::input::{
    ActionEventKind, ActionGraph, ActionState, BindingLocation, Conflict, Control, DeviceDescriptor, DeviceId, GraphSpec,
    InputService, IntentScope, NodeSpec, Owner, Presentation, PresentationFamily, Timestamp, Value, capability, conflicts,
    presentations,
};
use l3i::Runtime;
use l3i::bind::{Call, StackResults};
use l3i::convert::Vector3;
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::stack::{Frame, Scope, TableView, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::Table;
use serde_json::{Value as Json, json};

use crate::lua::convert;
use crate::lua::plan::Functions;
use crate::lua::{Jess, Text, error, guard, kind, lua, push_table};

/// A compiled, validated action graph.
#[derive(Clone)]
pub struct LuaInputGraph {
    graph: Rc<ActionGraph>,
    jess: Rc<Jess>,
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

/// A document as a script writes it: `version` may be left out.
#[derive(serde::Serialize, serde::Deserialize)]
struct SpecInput {
    #[serde(default = "current_version")]
    version: u32,
    actions: Vec<crate::input::ActionSpec>,
    #[serde(default)]
    sets: Vec<crate::input::SetSpec>,
    #[serde(default)]
    layers: Vec<crate::input::LayerSpec>,
}

const fn current_version() -> u32 {
    crate::input::FORMAT_VERSION
}

fn control(text: &Text) -> l3i::Result<Control> {
    text.0.parse().map_err(error)
}

/// A class of control by its bare name: `input.key('KeyA')` is `key:KeyA`.
fn named_control(class: &str, name: &Text) -> l3i::Result<Control> {
    format!("{class}:{}", name.0).parse().map_err(error)
}

fn millis(function: &str, seconds: f64) -> l3i::Result<u32> {
    if !(seconds.is_finite() && seconds >= 0.0) {
        return Err(error(format!("{function} takes a duration in seconds, not {seconds}")));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Checked finite and positive; clamped.
    Ok((seconds * 1000.0).round().min(f64::from(u32::MAX)) as u32)
}

fn seconds(timestamp: Timestamp) -> f64 {
    Duration::from_nanos(timestamp.nanos()).as_secs_f64()
}

/// Reads a node table a script passed, naming the argument in errors.
fn node(frame: &Frame<'_>, jess: &Jess, value: ValueView<'_>, what: &str) -> l3i::Result<NodeSpec> {
    if !value.is_table() {
        return Err(error(format!("{what} must be a node table, not {}", kind(value))));
    }
    convert::from_lua::<NodeSpec>(frame, jess, value, None)
        .map_err(|problem| error(format!("{what} isn't a node: {}", crate::lua::error_message(&problem))))
}

fn nodes(frame: &Frame<'_>, jess: &Jess, list: ValueView<'_>, what: &str) -> l3i::Result<Vec<NodeSpec>> {
    if !list.is_table() {
        return Err(error(format!("{what} must be a list of node tables, not {}", kind(list))));
    }
    let mut members = Vec::new();
    list.as_table()?.for_each_array(frame, |frame, index, item| {
        members.push(node(frame, jess, item, &format!("{what}[{index}]"))?);
        Ok(())
    })?;
    Ok(members)
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

fn compile(jess: &Rc<Jess>, spec: GraphSpec) -> l3i::Result<Owned<LuaInputGraph>> {
    let graph = ActionGraph::compile(spec).map_err(compile_error)?;
    Ok(Owned(LuaInputGraph {
        graph: Rc::new(graph),
        jess: jess.clone(),
    }))
}

/// A document from a script's table, `version` filled in when left out.
fn spec_from_table(frame: &Frame<'_>, jess: &Jess, value: ValueView<'_>, function: &str) -> l3i::Result<GraphSpec> {
    if !value.is_table() {
        return Err(error(format!(
            "{function} takes a document table with actions and sets, not {}",
            kind(value)
        )));
    }
    let input: SpecInput = convert::from_lua(frame, jess, value, None)
        .map_err(|problem| error(format!("{function}: {}", crate::lua::error_message(&problem))))?;
    Ok(GraphSpec {
        version: input.version,
        actions: input.actions,
        sets: input.sets,
        layers: input.layers,
    })
}

fn parse_event_kind(text: &Text) -> l3i::Result<ActionEventKind> {
    match text.0.as_str() {
        "pressed" => Ok(ActionEventKind::Pressed),
        "released" => Ok(ActionEventKind::Released),
        "changed" => Ok(ActionEventKind::Changed),
        other => Err(error(format!(
            "an action event kind is pressed, released, or changed, not {other:?}"
        ))),
    }
}

const fn event_kind_name(kind: ActionEventKind) -> &'static str {
    match kind {
        ActionEventKind::Pressed => "pressed",
        ActionEventKind::Released => "released",
        ActionEventKind::Changed => "changed",
    }
}

fn parse_family(text: &Text) -> l3i::Result<PresentationFamily> {
    PresentationFamily::from_name(&text.0).ok_or_else(|| {
        let names: Vec<&str> = PresentationFamily::ALL.iter().map(|family| family.name()).collect();
        error(format!(
            "a presentation family is one of {}, not {:?}",
            names.join(", "),
            text.0
        ))
    })
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

fn location_json(location: &BindingLocation) -> Json {
    match &location.owner {
        Owner::Set(name) => json!({ "set": name, "index": location.index }),
        Owner::Layer(name) => json!({ "layer": name, "index": location.index }),
    }
}

fn presentation_json(shape: &Presentation) -> Json {
    let part = |input: &Presentation| presentation_json(input);
    match shape {
        Presentation::Control(control) => json!({
            "kind": "control",
            "control": control.control.to_string(),
            "family": control.family.name(),
            "label": control.label,
        }),
        Presentation::Chord(parts) => json!({ "kind": "chord", "parts": parts.iter().map(part).collect::<Vec<_>>() }),
        Presentation::AnyOf(parts) => json!({ "kind": "anyOf", "parts": parts.iter().map(part).collect::<Vec<_>>() }),
        Presentation::Tap { input, within_ms } => {
            json!({ "kind": "tap", "input": part(input), "within": f64::from(*within_ms) / 1000.0 })
        }
        Presentation::Hold { input, duration_ms } => {
            json!({ "kind": "hold", "input": part(input), "duration": f64::from(*duration_ms) / 1000.0 })
        }
        Presentation::Axis { negative, positive } => {
            json!({ "kind": "axis", "negative": part(negative), "positive": part(positive) })
        }
        Presentation::Vector2 { x, y } => json!({ "kind": "vector2", "x": part(x), "y": part(y) }),
        Presentation::While { input, condition } => {
            json!({ "kind": "while", "input": part(input), "condition": part(condition) })
        }
        Presentation::Action(name) => json!({ "kind": "action", "action": name }),
        Presentation::Constant => json!({ "kind": "constant" }),
    }
}

fn conflict_json(conflict: &Conflict) -> Json {
    let kind = match conflict.kind {
        crate::input::ConflictKind::Press => "press",
        crate::input::ConflictKind::Tap => "tap",
        crate::input::ConflictKind::Hold => "hold",
        crate::input::ConflictKind::Axis => "axis",
    };
    json!({
        "owner": location_json(&BindingLocation { owner: conflict.owner.clone(), index: 0 })
            .as_object()
            .map(|owner| owner.iter().filter(|(key, _)| *key != "index").map(|(key, value)| (key.clone(), value.clone())).collect::<serde_json::Map<_, _>>()),
        "kind": kind,
        "controls": conflict.controls.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "actions": conflict.actions,
        "bindings": conflict.bindings.iter().map(location_json).collect::<Vec<_>>(),
    })
}

/// The module's authoring functions, each returning a node table.
#[allow(clippy::too_many_lines)] // one function per node type
fn authoring(module: &Functions<'_>, jess_state: &Rc<Jess>) -> l3i::Result<()> {
    // Each function gets the call, the runtime's state as `$jess`, and its arguments, and its
    // body is the node it makes.
    macro_rules! node_function {
        ($name:literal, |$call:ident, $jess:ident $(, $arg:ident : $ty:ty)*| $body:block) => {{
            let state = jess_state.clone();
            module.function($name, move |$call: &Call<'_>, $($arg: $ty),*| -> l3i::Result<StackResults> {
                let $jess: &Jess = &state;
                let spec: NodeSpec = $body;
                convert::to_lua($call, &state, &spec)?;
                Ok(StackResults)
            })?;
        }};
    }
    node_function!("key", |_call, _jess, name: Text| {
        NodeSpec::button(named_control("key", &name)?)
    });
    node_function!("pointerButton", |_call, _jess, name: Text| {
        NodeSpec::button(named_control("pointerButton", &name)?)
    });
    node_function!("gamepadButton", |_call, _jess, name: Text| {
        NodeSpec::button(named_control("gamepadButton", &name)?)
    });
    node_function!("button", |_call, _jess, text: Text| {
        let control = control(&text)?;
        if !control.is_digital() {
            return Err(error(format!(
                "input.button reads a button, and {control} is an axis; use input.axis"
            )));
        }
        NodeSpec::button(control)
    });
    node_function!("axis", |_call, _jess, text: Text| {
        let control = control(&text)?;
        if control.is_digital() {
            return Err(error(format!(
                "input.axis reads an axis, and {control} is a button; use input.button"
            )));
        }
        NodeSpec::axis(control)
    });
    node_function!("gamepadAxis", |_call, _jess, name: Text| {
        NodeSpec::axis(named_control("gamepadAxis", &name)?)
    });
    node_function!("pointerAxis", |_call, _jess, name: Text| {
        NodeSpec::axis(named_control("pointerAxis", &name)?)
    });
    node_function!("constant", |_call, _jess, value: f64| {
        #[allow(clippy::cast_possible_truncation)] // A binding constant is a small number.
        NodeSpec::Constant { value: value as f32 }
    });
    node_function!("invert", |call, jess, input: ValueView<'_>| {
        call.with_frame(|frame| Ok(NodeSpec::invert(node(frame, jess, input, "input.invert's input")?)))?
    });
    node_function!("scale", |call, jess, input: ValueView<'_>, factor: f64| {
        #[allow(clippy::cast_possible_truncation)] // A scale factor is a small number.
        call.with_frame(|frame| {
            Ok(NodeSpec::scale(
                node(frame, jess, input, "input.scale's input")?,
                factor as f32,
            ))
        })?
    });
    node_function!("deadzone", |call, jess, input: ValueView<'_>, inner: f64, outer: f64| {
        #[allow(clippy::cast_possible_truncation)] // Deadzone edges are in 0..1.
        call.with_frame(|frame| {
            Ok(NodeSpec::deadzone(
                node(frame, jess, input, "input.deadzone's input")?,
                inner as f32,
                outer as f32,
            ))
        })?
    });
    node_function!("threshold", |call, jess, input: ValueView<'_>, level: f64| {
        #[allow(clippy::cast_possible_truncation)] // A threshold level is in 0..1.
        call.with_frame(|frame| {
            Ok(NodeSpec::threshold(
                node(frame, jess, input, "input.threshold's input")?,
                level as f32,
            ))
        })?
    });
    node_function!("chord", |call, jess, members: ValueView<'_>| {
        call.with_frame(|frame| Ok(NodeSpec::chord(nodes(frame, jess, members, "input.chord's members")?)))?
    });
    node_function!("any", |call, jess, members: ValueView<'_>| {
        call.with_frame(|frame| Ok(NodeSpec::any(nodes(frame, jess, members, "input.any's members")?)))?
    });
    node_function!("gate", |call, jess, input: ValueView<'_>, condition: ValueView<'_>| {
        call.with_frame(|frame| {
            Ok(NodeSpec::gate(
                node(frame, jess, input, "input.gate's input")?,
                node(frame, jess, condition, "input.gate's condition")?,
            ))
        })?
    });
    node_function!("tap", |call, jess, input: ValueView<'_>, within: f64| {
        let within = millis("input.tap", within)?;
        call.with_frame(|frame| Ok(NodeSpec::tap(node(frame, jess, input, "input.tap's input")?, within)))?
    });
    node_function!("hold", |call, jess, input: ValueView<'_>, duration: f64| {
        let duration = millis("input.hold", duration)?;
        call.with_frame(|frame| Ok(NodeSpec::hold(node(frame, jess, input, "input.hold's input")?, duration)))?
    });
    node_function!(
        "axisFromButtons",
        |call, jess, negative: ValueView<'_>, positive: ValueView<'_>| {
            call.with_frame(|frame| {
                Ok(NodeSpec::axis_from_buttons(
                    node(frame, jess, negative, "input.axisFromButtons' negative")?,
                    node(frame, jess, positive, "input.axisFromButtons' positive")?,
                ))
            })?
        }
    );
    node_function!("vector2", |call, jess, x: ValueView<'_>, y: ValueView<'_>| {
        call.with_frame(|frame| {
            Ok(NodeSpec::vector2(
                node(frame, jess, x, "input.vector2's x")?,
                node(frame, jess, y, "input.vector2's y")?,
            ))
        })?
    });
    node_function!("actionActive", |_call, _jess, action: Text| {
        NodeSpec::action_active(action.0)
    });
    node_function!("actionValue", |_call, _jess, action: Text| {
        NodeSpec::ActionValue { action: action.0 }
    });
    Ok(())
}

pub fn module(runtime: &Runtime, jess: &Rc<Jess>) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "input")?;
    authoring(&module, jess)?;
    let state = jess.clone();
    module.function(
        "graph",
        move |call: &Call<'_>, spec: ValueView<'_>| -> l3i::Result<Owned<LuaInputGraph>> {
            let spec = call.with_frame(|frame| spec_from_table(frame, &state, spec, "input.graph"))?;
            compile(&state, spec)
        },
    )?;
    let state = jess.clone();
    module.function("fromJson", move |text: Text| -> l3i::Result<Owned<LuaInputGraph>> {
        compile(&state, GraphSpec::from_json(&text.0).map_err(error)?)
    })?;
    let state = jess.clone();
    module.function("runtime", move || {
        let (service, live) = crate::runtime::service::<SharedInput>().map_or_else(
            || (Arc::new(Mutex::new(InputService::new())), false),
            |service| (service, true),
        );
        Owned(LuaInputRuntime {
            service,
            live,
            jess: state.clone(),
        })
    })?;
    module.finish()
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
        .method("spec", |this: &LuaInputGraph, call: &Call<'_>| -> l3i::Result<StackResults> {
            convert::to_lua(call, &this.jess, this.graph.spec())?;
            Ok(StackResults)
        })
        .untyped();
    graph
        .method(
            "actions",
            |this: &LuaInputGraph, call: &Call<'_>| -> l3i::Result<StackResults> {
                lua::push_list(call, this.graph.actions().iter(), |frame, action| {
                    push_table(frame, 0, 2, |frame, row| {
                        row.raw_set_value(frame, "name", action.name())?;
                        row.raw_set_value(frame, "valueType", action.value_type().name())
                    })
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    graph
        .method(
            "bindings",
            |this: &LuaInputGraph, call: &Call<'_>, action: Text| -> l3i::Result<StackResults> {
                let locations: Vec<Json> = this.graph.spec().bindings_for(&action.0).iter().map(location_json).collect();
                convert::display_to_lua(call, &Json::Array(locations))?;
                Ok(StackResults)
            },
        )
        .untyped();
    graph
        .method(
            "presentations",
            |this: &LuaInputGraph, call: &Call<'_>, action: Text, family: Text| -> l3i::Result<StackResults> {
                let family = parse_family(&family)?;
                let shapes: Vec<Json> = presentations(this.graph.spec(), &action.0, family)
                    .iter()
                    .map(|binding| {
                        json!({
                            "action": binding.action,
                            "location": location_json(&binding.location),
                            "family": binding.family.name(),
                            "shape": presentation_json(&binding.shape),
                        })
                    })
                    .collect();
                convert::display_to_lua(call, &Json::Array(shapes))?;
                Ok(StackResults)
            },
        )
        .untyped();
    graph
        .method(
            "conflicts",
            |this: &LuaInputGraph, call: &Call<'_>| -> l3i::Result<StackResults> {
                let found: Vec<Json> = conflicts(this.graph.spec()).iter().map(conflict_json).collect();
                convert::display_to_lua(call, &Json::Array(found))?;
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
            |this: &LuaInputRuntime, call: &Call<'_>, graph: ValueView<'_>| -> l3i::Result<f64> {
                guard(&this.jess, "jess.input", capability::CONFIGURE)?;
                let spec = match l3i::userdata::receiver::<LuaInputGraph>(graph) {
                    Some(graph) => graph.graph.spec().clone(),
                    None => call.with_frame(|frame| spec_from_table(frame, &this.jess, graph, "InputRuntime:install"))?,
                };
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
                let rows = tick_rows(&mut this.lock());
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
            "route",
            |this: &LuaInputRuntime, action: Text, kind: Text, intent: Text| -> l3i::Result<()> {
                guard(&this.jess, "jess.input", capability::CONFIGURE)?;
                let kind = parse_event_kind(&kind)?;
                this.lock().route(&action.0, kind, &intent.0);
                Ok(())
            },
        )
        .untyped();
    runtime
        .method(
            "intents",
            |this: &LuaInputRuntime, call: &Call<'_>| -> l3i::Result<StackResults> {
                guard(&this.jess, "jess.input", capability::ACTIONS)?;
                let rows = {
                    let service = this.lock();
                    let names = action_names(&service);
                    service
                        .dispatches()
                        .iter()
                        .map(|dispatch| Row {
                            intent: Some(service.intents().name(dispatch.intent).unwrap_or_default().to_owned()),
                            action: names[dispatch.action.index() as usize].clone(),
                            kind: dispatch.kind,
                            value: dispatch.value,
                            time: dispatch.timestamp,
                            device: dispatch.device,
                        })
                        .collect::<Vec<Row>>()
                };
                push_rows(call, &rows)?;
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
                lua::push_list(call, devices.iter(), |frame, (id, descriptor, connected)| {
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

/// One action event or intent dispatch as a script sees it: `{ intent?, action, kind, value,
/// time, device }`, the same row the host puts in the input phase's payload.
struct Row {
    intent: Option<String>,
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
    let tick = service.tick(now, IntentScope::default());
    tick.events
        .iter()
        .map(|event| Row {
            intent: None,
            action: names[event.action.index() as usize].clone(),
            kind: event.kind,
            value: event.value,
            time: event.timestamp,
            device: event.device,
        })
        .collect()
}

fn push_rows(scope: &impl Scope, rows: &[Row]) -> l3i::Result<()> {
    lua::push_list(scope, rows.iter(), |frame, row| {
        push_table(frame, 0, 6, |frame, table| {
            if let Some(intent) = &row.intent {
                table.raw_set_value(frame, "intent", intent.as_str())?;
            }
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
