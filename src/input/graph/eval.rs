//! Evaluating a compiled graph against current device state.
//!
//! An [`Evaluator`] is the per-graph state that changes every tick: each node's value, when it
//! last rose, which sets are active, which layers are pushed, and every action's state. All of it
//! is allocated when the evaluator is made. [`Evaluator::evaluate`] walks arrays and allocates
//! nothing, so the cost of a tick is the number of live nodes and nothing else.

use std::time::{Duration, Instant};

use crate::input::graph::compile::{ActionGraph, Op};
use crate::input::graph::spec::ValueType;
use crate::input::id::BindingId;
use crate::input::{ActionId, DeviceId, Generation, InputState, LayerId, SetId, Timestamp};

/// A node's or an action's value.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Value {
    /// On or off.
    Digital(bool),
    /// One number.
    Scalar(f32),
    /// `[x, y]`.
    Vector2([f32; 2]),
}

impl Value {
    /// The resting value of a type: off, `0.0`, or `[0.0, 0.0]`.
    #[must_use]
    pub const fn zero(value_type: ValueType) -> Self {
        match value_type {
            ValueType::Digital => Self::Digital(false),
            ValueType::Scalar => Self::Scalar(0.0),
            ValueType::Vector2 => Self::Vector2([0.0, 0.0]),
        }
    }

    /// The value's type.
    #[must_use]
    pub const fn value_type(self) -> ValueType {
        match self {
            Self::Digital(_) => ValueType::Digital,
            Self::Scalar(_) => ValueType::Scalar,
            Self::Vector2(_) => ValueType::Vector2,
        }
    }

    /// How far the value is from rest: `0.0` or `1.0` for a digital value, the absolute value of
    /// a scalar, the length of a vector.
    #[must_use]
    pub fn magnitude(self) -> f32 {
        match self {
            Self::Digital(on) => f32::from(u8::from(on)),
            Self::Scalar(value) => value.abs(),
            Self::Vector2([x, y]) => x.hypot(y),
        }
    }

    /// Whether the value is away from rest.
    #[must_use]
    pub fn is_active(self) -> bool {
        self.magnitude() > 0.0
    }

    /// The value, if it is digital.
    #[must_use]
    pub const fn as_digital(self) -> Option<bool> {
        match self {
            Self::Digital(on) => Some(on),
            Self::Scalar(_) | Self::Vector2(_) => None,
        }
    }

    /// The value, if it is a scalar.
    #[must_use]
    pub const fn as_scalar(self) -> Option<f32> {
        match self {
            Self::Scalar(value) => Some(value),
            Self::Digital(_) | Self::Vector2(_) => None,
        }
    }

    /// The value, if it is a vector.
    #[must_use]
    pub const fn as_vector2(self) -> Option<[f32; 2]> {
        match self {
            Self::Vector2(value) => Some(value),
            Self::Digital(_) | Self::Scalar(_) => None,
        }
    }

    const fn on(self) -> bool {
        matches!(self, Self::Digital(true))
    }

    const fn scalar(self) -> f32 {
        match self {
            Self::Scalar(value) => value,
            Self::Digital(_) | Self::Vector2(_) => 0.0,
        }
    }
}

/// One action's state after a tick. Edges are computed here, so nobody diffs frames by hand.
#[derive(Clone, Copy, PartialEq, Debug)]
#[allow(clippy::struct_excessive_bools)] // A snapshot of four independent facts about one tick.
pub struct ActionState {
    /// The action's value: the strongest of its live bindings' values, or the type's zero.
    pub value: Value,
    /// Whether the value is away from rest.
    pub active: bool,
    /// Whether the action became active this tick.
    pub pressed_this_tick: bool,
    /// Whether the action stopped being active this tick.
    pub released_this_tick: bool,
    /// Whether the value is different from last tick's.
    pub changed_this_tick: bool,
    /// When the action last became active or stopped being active.
    pub since: Timestamp,
    /// The binding that produced the value, when one is active: why the action fired.
    pub binding: Option<BindingId>,
    /// The device behind that binding's value, when it traces to one.
    pub device: Option<DeviceId>,
}

/// What happened to an action.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ActionEventKind {
    /// The action became active.
    Pressed,
    /// The action stopped being active.
    Released,
    /// A scalar or vector action's value changed. Digital actions only press and release.
    Changed,
}

/// A semantic action event: one edge of one action, in one tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ActionEvent {
    /// The action.
    pub action: ActionId,
    /// What happened.
    pub kind: ActionEventKind,
    /// The action's value after the tick.
    pub value: Value,
    /// The tick's time.
    pub timestamp: Timestamp,
    /// The graph generation that produced the event. A consumer holding events across a graph
    /// replacement drops the ones from an older generation.
    pub generation: Generation,
    /// The binding that produced the value.
    pub binding: Option<BindingId>,
    /// The device behind it.
    pub device: Option<DeviceId>,
}

/// What one tick cost, and what it did. The numbers a profiler attributes to input.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct EvaluationStats {
    /// How many ticks the evaluator has run.
    pub ticks: u64,
    /// Nodes evaluated in the last tick.
    pub nodes_evaluated: u32,
    /// Bindings evaluated in the last tick.
    pub bindings_evaluated: u32,
    /// Actions whose value changed in the last tick.
    pub actions_changed: u32,
    /// Action events the last tick produced.
    pub action_events: u32,
    /// Sets active during the last tick.
    pub active_sets: u32,
    /// Layers pushed during the last tick.
    pub active_layers: u32,
    /// Wall time the last tick's evaluation took.
    pub evaluation_time: Duration,
    /// Wall time every tick's evaluation has taken, added up.
    pub total_evaluation_time: Duration,
}

#[derive(Clone, Copy, Debug)]
struct NodeRuntime {
    value: Value,
    previous: Value,
    /// When a digital node last turned on.
    rose_at: Timestamp,
    /// When a digital node last turned on or off.
    changed_at: Timestamp,
    device: Option<DeviceId>,
}

/// The changing half of an action graph: node values, activation, and action state.
///
/// An evaluator belongs to the [`ActionGraph`] it was made for and refuses any other. It holds
/// no reference to the graph, so the two can live side by side in one struct; pass the same graph
/// to every call.
///
/// # What a tick sees
///
/// A tick reads [`InputState`] as it is when [`Evaluator::evaluate`] runs. A press and its
/// release that both land between two ticks are invisible to the graph, so tick at least as
/// often as the shortest input you care about. Tap and hold measure with the devices' own event
/// timestamps where a source has them, so their thresholds don't drift with the tick rate.
///
/// # Going live
///
/// A binding is live while its owner decides its action: its set is active and no pushed layer
/// names the action, or its layer is the topmost pushed layer that does. A binding that goes
/// live starts from rest, and a button that was already held when the sets or layers last
/// changed stays invisible to it until released. Opening a menu with the key that also accepts
/// in the menu does not accept; a key pressed after the menu opened does.
#[derive(Clone, Debug)]
pub struct Evaluator {
    graph_instance: u64,
    generation: Generation,
    nodes: Vec<NodeRuntime>,
    actions: Vec<ActionState>,
    /// Indexed by binding: whether it was live in the last tick.
    live: Vec<bool>,
    /// Indexed by binding: the tick before the sets or layers last changed, as of when the
    /// binding last went live. A button pressed before it was held when the binding went live,
    /// and the binding ignores it until it is released.
    live_since: Vec<Timestamp>,
    /// Indexed by set.
    set_active: Vec<bool>,
    /// Bottom to top. Never longer than the graph's layer count.
    layers: Vec<LayerId>,
    events: Vec<ActionEvent>,
    stats: EvaluationStats,
    /// The `now` of the last tick.
    last_tick: Timestamp,
    /// The last tick before the sets or layers last changed: a button pressed before it was held
    /// when they changed, and a binding that goes live ignores it until it is released.
    activation: Timestamp,
}

impl Evaluator {
    /// An evaluator for `graph`, with no set active and no layer pushed, stamping what it
    /// produces with `generation`.
    #[must_use]
    pub fn new(graph: &ActionGraph, generation: Generation) -> Self {
        let rest = |value_type| NodeRuntime {
            value: Value::zero(value_type),
            previous: Value::zero(value_type),
            rose_at: Timestamp::ZERO,
            changed_at: Timestamp::ZERO,
            device: None,
        };
        Self {
            graph_instance: graph.instance,
            generation,
            nodes: graph.nodes.iter().map(|node| rest(node.value_type)).collect(),
            actions: graph
                .actions()
                .iter()
                .map(|action| ActionState {
                    value: Value::zero(action.value_type()),
                    active: false,
                    pressed_this_tick: false,
                    released_this_tick: false,
                    changed_this_tick: false,
                    since: Timestamp::ZERO,
                    binding: None,
                    device: None,
                })
                .collect(),
            live: vec![false; graph.bindings().len()],
            live_since: vec![Timestamp::ZERO; graph.bindings().len()],
            set_active: vec![false; graph.set_count()],
            layers: Vec::with_capacity(graph.layer_count()),
            // A tick produces at most a press or release and a change per action.
            events: Vec::with_capacity(graph.actions().len() * 2),
            stats: EvaluationStats::default(),
            last_tick: Timestamp::ZERO,
            activation: Timestamp::ZERO,
        }
    }

    /// The generation this evaluator stamps on what it produces.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Dates the evaluator's birth to `now`, the last tick of the graph it replaces: a button
    /// held since before then is invisible to every binding until released, as it would be to a
    /// binding that went live at a set change.
    pub(crate) const fn begin_at(&mut self, now: Timestamp) {
        self.last_tick = now;
        self.activation = now;
    }

    /// Activates or deactivates a set. Takes effect at the next tick.
    ///
    /// # Panics
    ///
    /// If `set` isn't one of the graph's.
    pub fn set_active(&mut self, set: SetId, active: bool) {
        self.set_active[set.position()] = active;
        self.activation = self.last_tick;
    }

    /// Whether a set is active.
    #[must_use]
    pub fn is_set_active(&self, set: SetId) -> bool {
        self.set_active.get(set.position()).copied().unwrap_or(false)
    }

    /// The active sets, in declaration order.
    pub fn active_sets(&self) -> impl Iterator<Item = SetId> + '_ {
        self.set_active
            .iter()
            .enumerate()
            .filter(|(_, active)| **active)
            .map(|(position, _)| SetId::from_position(position))
    }

    /// Pushes a layer on top of the stack. A layer already pushed moves to the top. Takes effect
    /// at the next tick.
    pub fn push_layer(&mut self, layer: LayerId) {
        self.layers.retain(|pushed| *pushed != layer);
        self.layers.push(layer);
        self.activation = self.last_tick;
    }

    /// Removes a layer from the stack, wherever it is. Returns whether it was pushed.
    pub fn pop_layer(&mut self, layer: LayerId) -> bool {
        let before = self.layers.len();
        self.layers.retain(|pushed| *pushed != layer);
        self.activation = self.last_tick;
        self.layers.len() != before
    }

    /// The pushed layers, bottom to top.
    #[must_use]
    pub fn layers(&self) -> &[LayerId] {
        &self.layers
    }

    /// An action's state after the last tick.
    #[must_use]
    pub fn action(&self, action: ActionId) -> Option<&ActionState> {
        self.actions.get(action.position())
    }

    /// Every action's state after the last tick, indexed by action id.
    #[must_use]
    pub fn actions(&self) -> &[ActionState] {
        &self.actions
    }

    /// The last tick's action events, in evaluation order.
    #[must_use]
    pub fn events(&self) -> &[ActionEvent] {
        &self.events
    }

    /// What the last tick cost and did.
    #[must_use]
    pub const fn stats(&self) -> &EvaluationStats {
        &self.stats
    }

    /// Runs one tick: evaluates every live binding against `state` at time `now`, updates every
    /// action's state and edges, and returns the tick's action events.
    ///
    /// Actions are evaluated in the graph's order, so an action that reads another sees that
    /// action's value from this tick. An action with several live bindings takes the value
    /// furthest from rest, the earliest binding winning a tie; that binding is the state's
    /// `binding`. An action with no live binding, or one a layer masks, is at rest.
    ///
    /// Allocates nothing. Attribute its time to input: [`Evaluator::stats`] has the numbers.
    ///
    /// # Panics
    ///
    /// If `graph` isn't the graph this evaluator was made for.
    pub fn evaluate(&mut self, graph: &ActionGraph, state: &InputState, now: Timestamp) -> &[ActionEvent] {
        assert!(
            self.graph_instance == graph.instance,
            "this evaluator was made for another action graph; make a new one when the graph is recompiled"
        );
        let started = Instant::now();
        self.events.clear();
        let (mut nodes_evaluated, mut bindings_evaluated, mut actions_changed) = (0u32, 0u32, 0u32);
        for &action_id in &graph.order {
            let (best, nodes, bindings) = self.evaluate_bindings(graph, state, now, action_id);
            nodes_evaluated += nodes;
            bindings_evaluated += bindings;
            actions_changed += u32::from(self.settle_action(graph, now, action_id, best));
        }

        self.last_tick = now;
        let elapsed = started.elapsed();
        self.stats = EvaluationStats {
            ticks: self.stats.ticks + 1,
            nodes_evaluated,
            bindings_evaluated,
            actions_changed,
            action_events: u32::try_from(self.events.len()).unwrap_or(u32::MAX),
            active_sets: u32::try_from(self.set_active.iter().filter(|active| **active).count()).unwrap_or(u32::MAX),
            active_layers: u32::try_from(self.layers.len()).unwrap_or(u32::MAX),
            evaluation_time: elapsed,
            total_evaluation_time: self.stats.total_evaluation_time + elapsed,
        };
        &self.events
    }
}

/// The strongest live binding's value, and whose it is.
type Best = Option<(BindingId, Value, Option<DeviceId>)>;

impl Evaluator {
    /// Evaluates every live binding of one action: the strongest result, and how many nodes and
    /// bindings that took.
    fn evaluate_bindings(
        &mut self,
        graph: &ActionGraph,
        state: &InputState,
        now: Timestamp,
        action_id: ActionId,
    ) -> (Best, u32, u32) {
        let action = &graph.actions()[action_id.position()];
        let position = action_id.position();
        // The topmost pushed layer that names the action decides it; otherwise the sets do.
        let deciding = self
            .layers
            .iter()
            .rev()
            .copied()
            .find(|layer| graph.layers[layer.position()].touches[position]);
        let masked = deciding.is_some_and(|layer| graph.layers[layer.position()].masks[position]);

        let mut best: Best = None;
        let (mut nodes_evaluated, mut bindings_evaluated) = (0u32, 0u32);
        let sets = action
            .set_bindings
            .iter()
            .map(|&(set, binding)| (deciding.is_none() && self.set_active[set.position()], binding));
        let layers = action
            .layer_bindings
            .iter()
            .map(|&(layer, binding)| (!masked && deciding == Some(layer), binding));
        for (live, binding_id) in sets.chain(layers) {
            let was_live = std::mem::replace(&mut self.live[binding_id.position()], live);
            if !live {
                continue;
            }
            let binding = &graph.bindings()[binding_id.position()];
            let (first, root) = (binding.first.position(), binding.root.position());
            if !was_live {
                self.live_since[binding_id.position()] = self.activation;
            }
            let tick = Tick {
                graph,
                state,
                actions: &self.actions,
                now,
                fresh: !was_live,
                live_since: self.live_since[binding_id.position()],
            };
            for node in first..=root {
                evaluate_node(&tick, &mut self.nodes, node);
            }
            nodes_evaluated += binding.node_count();
            bindings_evaluated += 1;
            let result = &self.nodes[root];
            let stronger = best.is_none_or(|(_, value, _)| result.value.magnitude() > value.magnitude());
            if result.value.is_active() && stronger {
                best = Some((binding_id, result.value, result.device));
            }
        }
        (best, nodes_evaluated, bindings_evaluated)
    }

    /// Records an action's new state and edges, emitting its events. Returns whether the value
    /// changed.
    fn settle_action(&mut self, graph: &ActionGraph, now: Timestamp, action_id: ActionId, best: Best) -> bool {
        let action = &graph.actions()[action_id.position()];
        let position = action_id.position();
        let previous = self.actions[position];
        let (binding, value, device) = best.map_or_else(
            || (None, Value::zero(action.value_type()), None),
            |(binding, value, device)| (Some(binding), value, device),
        );
        let active = value.is_active();
        let (pressed, released) = (active && !previous.active, !active && previous.active);
        let changed = value != previous.value;
        self.actions[position] = ActionState {
            value,
            active,
            pressed_this_tick: pressed,
            released_this_tick: released,
            changed_this_tick: changed,
            since: if pressed || released { now } else { previous.since },
            binding,
            device,
        };
        let mut emit = |kind| {
            self.events.push(ActionEvent {
                action: action_id,
                kind,
                value,
                timestamp: now,
                generation: self.generation,
                // A release is attributed to what was holding the action.
                binding: binding.or(previous.binding),
                device: device.or(previous.device),
            });
        };
        if pressed {
            emit(ActionEventKind::Pressed);
        }
        if released {
            emit(ActionEventKind::Released);
        }
        if changed && action.value_type() != ValueType::Digital {
            emit(ActionEventKind::Changed);
        }
        changed
    }
}

/// A deadzone over a magnitude: zero at or below `inner`, one at or above `outer`, and the band
/// between rescaled to `0..1`.
fn deadzone(magnitude: f32, inner: f32, outer: f32) -> f32 {
    if magnitude <= inner {
        0.0
    } else if magnitude >= outer {
        1.0
    } else {
        (magnitude - inner) / (outer - inner)
    }
}

/// What one binding's nodes are evaluated against.
struct Tick<'a> {
    graph: &'a ActionGraph,
    state: &'a InputState,
    /// Every action's state, the ones evaluated earlier this tick already updated.
    actions: &'a [ActionState],
    now: Timestamp,
    /// The binding's first tick live: its nodes start from rest.
    fresh: bool,
    /// The tick before the sets or layers last changed, as of when the binding went live. A
    /// button pressed before it is ignored until released.
    live_since: Timestamp,
}

/// Evaluates the node at `index`, whose inputs are all before it.
#[allow(clippy::too_many_lines)] // one arm per node type
fn evaluate_node(tick: &Tick<'_>, nodes: &mut [NodeRuntime], index: usize) {
    let Tick {
        graph,
        state,
        actions,
        now,
        fresh,
        live_since,
    } = *tick;
    let node = &graph.nodes[index];
    let (before, rest) = nodes.split_at_mut(index);
    let runtime = &mut rest[0];
    let input = |node: crate::input::NodeId| &before[node.position()];
    if fresh {
        *runtime = NodeRuntime {
            value: Value::zero(node.value_type),
            previous: Value::zero(node.value_type),
            rose_at: now,
            changed_at: now,
            device: None,
        };
    }
    let previous = runtime.value;
    // When a digital node turns on or off, and because of which input: its own clock by
    // default, a source's or an input's where that is more exact.
    let mut rise = now;
    let mut change = now;
    let (value, device) = match &node.op {
        Op::SourceButton { control, device } => {
            let found = device.map_or_else(
                || state.button_on_any(*control),
                |device| {
                    state
                        .device(device)
                        .filter(|state| state.is_connected())
                        .and_then(|state| state.button(*control))
                        .map(|button| (device, button))
                },
            );
            // Held since before the binding went live: not this binding's press.
            let on = found.is_some_and(|(_, button)| button.pressed && button.since >= live_since);
            if let Some((_, button)) = found {
                rise = button.since;
                change = button.since;
            }
            (Value::Digital(on), found.filter(|_| on).map(|(device, _)| device))
        }
        Op::SourceAxis { control, device } => {
            let found = device.map_or_else(
                || state.axis_on_any(*control),
                |device| {
                    state
                        .device(device)
                        .filter(|state| state.is_connected())
                        .and_then(|state| state.axis(*control))
                        .map(|axis| (device, axis))
                },
            );
            let value = found.map_or(0.0, |(_, axis)| axis.value);
            (
                Value::Scalar(value),
                found.filter(|_| value.abs() > 0.0).map(|(device, _)| device),
            )
        }
        Op::Constant(value) => (Value::Scalar(*value), None),
        Op::Invert(node) => {
            let input = input(*node);
            let value = match input.value {
                Value::Digital(on) => Value::Digital(!on),
                Value::Scalar(value) => Value::Scalar(-value),
                Value::Vector2([x, y]) => Value::Vector2([-x, -y]),
            };
            change = input.changed_at;
            rise = input.changed_at;
            (value, input.device)
        }
        Op::Scale { input: node, factor } => {
            let input = input(*node);
            let value = match input.value {
                Value::Vector2([x, y]) => Value::Vector2([x * factor, y * factor]),
                other => Value::Scalar(other.scalar() * factor),
            };
            (value, input.device)
        }
        Op::Deadzone {
            input: node,
            inner,
            outer,
        } => {
            let input = input(*node);
            let value = match input.value {
                Value::Vector2([x, y]) => {
                    let length = x.hypot(y);
                    let scaled = deadzone(length, *inner, *outer);
                    if scaled > 0.0 {
                        Value::Vector2([x / length * scaled, y / length * scaled])
                    } else {
                        Value::Vector2([0.0, 0.0])
                    }
                }
                other => {
                    let value = other.scalar();
                    Value::Scalar(deadzone(value.abs(), *inner, *outer).copysign(value))
                }
            };
            (value, input.device.filter(|_| value.is_active()))
        }
        Op::Threshold { input: node, level } => {
            let input = input(*node);
            let on = input.value.scalar().abs() >= *level;
            (Value::Digital(on), input.device.filter(|_| on))
        }
        Op::Chord(members) => {
            let on = members.iter().all(|member| input(*member).value.on());
            // The chord completes when its last member goes down, and breaks when the first
            // member comes up.
            let mut device = None;
            if on {
                rise = Timestamp::ZERO;
                for member in members.iter().map(|member| input(*member)) {
                    if member.rose_at >= rise {
                        rise = member.rose_at;
                        device = member.device;
                    }
                }
            } else if let Some(released) = members
                .iter()
                .map(|member| input(*member))
                .filter(|member| !member.value.on() && member.previous.on())
                .map(|member| member.changed_at)
                .min()
            {
                change = released;
            }
            (Value::Digital(on), device)
        }
        Op::Any(members) => {
            let first = members.iter().map(|member| input(*member)).find(|member| member.value.on());
            if let Some(member) = first {
                rise = member.rose_at;
            }
            (Value::Digital(first.is_some()), first.and_then(|member| member.device))
        }
        Op::Gate { input: node, condition } => {
            let (input, condition) = (input(*node), input(*condition));
            if condition.value.on() {
                rise = input.rose_at.max(condition.rose_at);
                (input.value, input.device)
            } else {
                (Value::zero(node_type(graph, *node)), None)
            }
        }
        Op::Tap { input: node, within } => {
            let input = input(*node);
            let released = input.previous.on() && !input.value.on();
            let on = released && input.changed_at.since(input.rose_at) <= *within;
            (Value::Digital(on), input.device)
        }
        Op::Hold { input: node, duration } => {
            let input = input(*node);
            let on = input.value.on() && now.since(input.rose_at) >= *duration;
            (Value::Digital(on), input.device.filter(|_| on))
        }
        Op::AxisFromButtons { negative, positive } => {
            let (negative, positive) = (input(*negative), input(*positive));
            let value = f32::from(u8::from(positive.value.on())) - f32::from(u8::from(negative.value.on()));
            let device = if positive.value.on() {
                positive.device
            } else {
                negative.device
            };
            (Value::Scalar(value), device)
        }
        Op::Vector2 { x, y } => {
            let (x, y) = (input(*x), input(*y));
            (Value::Vector2([x.value.scalar(), y.value.scalar()]), x.device.or(y.device))
        }
        Op::ActionActive(action) => {
            let action = &actions[action.position()];
            rise = action.since;
            change = action.since;
            (Value::Digital(action.active), action.device)
        }
        Op::ActionValue(action) => {
            let action = &actions[action.position()];
            rise = action.since;
            change = action.since;
            (action.value, action.device)
        }
    };
    runtime.previous = previous;
    runtime.value = value;
    runtime.device = device;
    if value.on() != previous.on() {
        runtime.changed_at = change.min(now);
        if value.on() {
            runtime.rose_at = rise.min(now);
        }
    }
}

fn node_type(graph: &ActionGraph, node: crate::input::NodeId) -> ValueType {
    graph.nodes[node.position()].value_type
}
