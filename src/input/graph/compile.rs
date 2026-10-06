//! Compiling a binding document into the graph the evaluator walks.
//!
//! [`ActionGraph::compile`] checks everything a document can get wrong and reports all of it at
//! once, each error naming the set or layer, the action, the binding, and the node. What comes
//! out is an arena: every binding's nodes laid out children first, so evaluating a binding is one
//! pass over a contiguous range, and the actions sorted so an action that reads another
//! (`actionActive`, `actionValue`) is evaluated after it.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::input::graph::spec::{FORMAT_VERSION, GraphSpec, NodeSpec, Origin, Owner, ValueType};
use crate::input::id::BindingId;
use crate::input::{ActionId, Control, DeviceId, LayerId, NodeId, SetId};

/// What is wrong with a binding document.
#[derive(Clone, PartialEq, Debug)]
pub enum GraphErrorKind {
    /// The document's format version isn't the one this crate reads.
    UnsupportedVersion {
        /// The version the document says it is.
        found: u32,
    },
    /// An action, set, or layer has an empty name.
    EmptyName {
        /// What has no name: `action`, `set`, or `layer`.
        what: &'static str,
    },
    /// Two declarations share a name.
    Duplicate {
        /// What is declared twice: `action`, `set`, or `layer`.
        what: &'static str,
        /// The name.
        name: String,
    },
    /// A binding, a mask, or a node names an action the document doesn't declare.
    UnknownAction {
        /// The name.
        name: String,
    },
    /// A `sourceButton` names an axis.
    NotAButton {
        /// The control.
        control: Control,
    },
    /// A `sourceAxis` names a button.
    NotAnAxis {
        /// The control.
        control: Control,
    },
    /// A node's input has a type the node doesn't take.
    InputType {
        /// The node's type name.
        node: &'static str,
        /// The input field at fault (`input`, `condition`, `members[1]`, ...).
        input: String,
        /// What the node takes there.
        expected: &'static str,
        /// What the input produces.
        found: ValueType,
    },
    /// A binding produces a different type than its action declares.
    BindingType {
        /// The action's declared type.
        expected: ValueType,
        /// What the binding produces.
        found: ValueType,
    },
    /// A `chord` has fewer than two members, or an `any` has none.
    TooFewMembers {
        /// The node's type name.
        node: &'static str,
        /// How many it needs.
        minimum: usize,
        /// How many it has.
        found: usize,
    },
    /// A number isn't finite.
    NotFinite {
        /// The node's type name.
        node: &'static str,
        /// The field.
        field: &'static str,
    },
    /// A `tap` or `hold` threshold is zero.
    ZeroDuration {
        /// The node's type name.
        node: &'static str,
        /// The field.
        field: &'static str,
    },
    /// A deadzone's edges aren't `0 <= inner < outer <= 1`.
    InvalidDeadzone {
        /// The inner edge.
        inner: f32,
        /// The outer edge.
        outer: f32,
    },
    /// A threshold's level isn't above zero.
    InvalidLevel {
        /// The level.
        level: f32,
    },
    /// Actions read each other in a circle, through `actionActive` or `actionValue`.
    Cycle {
        /// The actions on the cycle, in order, the first repeated at the end.
        actions: Vec<String>,
    },
}

impl fmt::Display for GraphErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => write!(
                formatter,
                "the binding document is format version {found}, and this jess::input reads version {FORMAT_VERSION}"
            ),
            Self::EmptyName { what } => write!(formatter, "{what} has an empty name"),
            Self::Duplicate { what, name } => write!(formatter, "{what} {name:?} is declared twice"),
            Self::UnknownAction { name } => write!(formatter, "no action {name:?} is declared"),
            Self::NotAButton { control } => write!(
                formatter,
                "sourceButton reads a button, and {control} is an axis; read it with sourceAxis, and make a button of it with threshold"
            ),
            Self::NotAnAxis { control } => write!(
                formatter,
                "sourceAxis reads an axis, and {control} is a button; read it with sourceButton"
            ),
            Self::InputType {
                node,
                input,
                expected,
                found,
            } => write!(formatter, "{node}'s {input} must be {expected}, and it is {found}"),
            Self::BindingType { expected, found } => {
                write!(formatter, "the action is {expected}, and this binding produces {found}")
            }
            Self::TooFewMembers { node, minimum, found } => {
                write!(formatter, "{node} needs at least {minimum} members, and has {found}")
            }
            Self::NotFinite { node, field } => write!(formatter, "{node}'s {field} isn't a finite number"),
            Self::ZeroDuration { node, field } => {
                write!(formatter, "{node}'s {field} is 0 ms; a threshold must be at least 1 ms")
            }
            Self::InvalidDeadzone { inner, outer } => write!(
                formatter,
                "deadzone edges must be 0 <= inner < outer <= 1, and they are inner {inner}, outer {outer}"
            ),
            Self::InvalidLevel { level } => {
                write!(formatter, "threshold's level must be above 0, and it is {level}")
            }
            Self::Cycle { actions } => write!(formatter, "actions read each other in a cycle: {}", actions.join(" -> ")),
        }
    }
}

/// One thing wrong with a binding document, and where.
///
/// `Display` leads with the place, in Jess terms: `set "gameplay" (from @jess/example), action
/// "quickSave", binding 0, at chord.members[1]: ...`. Every part is also a field, so a settings
/// screen can point at the binding instead of printing a sentence.
#[derive(Clone, PartialEq, Debug)]
pub struct GraphError {
    /// What is wrong.
    pub kind: GraphErrorKind,
    /// The set or layer the problem is in, if it is in one.
    pub owner: Option<Owner>,
    /// The package that declared that set or layer.
    pub package: Option<String>,
    /// The action the problem concerns.
    pub action: Option<String>,
    /// The binding's 0-based position in its owner's list.
    pub binding: Option<usize>,
    /// The node at fault, as a path from the binding's root (`chord.members[1]`).
    pub path: Option<String>,
    /// Where the owner, or the action, was declared.
    pub origin: Option<Origin>,
}

impl GraphError {
    fn bare(kind: GraphErrorKind) -> Self {
        Self {
            kind,
            owner: None,
            package: None,
            action: None,
            binding: None,
            path: None,
            origin: None,
        }
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut place = String::new();
        let mut part = |text: String| {
            if !place.is_empty() {
                place.push_str(", ");
            }
            place.push_str(&text);
        };
        if let Some(origin) = &self.origin {
            part(origin.to_string());
        }
        if let Some(owner) = &self.owner {
            part(
                self.package
                    .as_ref()
                    .map_or_else(|| owner.to_string(), |package| format!("{owner} (from {package})")),
            );
        }
        if let Some(action) = &self.action {
            part(format!("action {action:?}"));
        }
        if let Some(binding) = self.binding {
            part(format!("binding {binding}"));
        }
        if let Some(path) = &self.path {
            part(format!("at {path}"));
        }
        if place.is_empty() {
            write!(formatter, "{}", self.kind)
        } else {
            write!(formatter, "{place}: {}", self.kind)
        }
    }
}

impl std::error::Error for GraphError {}

/// A compiled node's operation. Inputs are nodes earlier in the same binding's range.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    SourceButton { control: Control, device: Option<DeviceId> },
    SourceAxis { control: Control, device: Option<DeviceId> },
    Constant(f32),
    Invert(NodeId),
    Scale { input: NodeId, factor: f32 },
    Deadzone { input: NodeId, inner: f32, outer: f32 },
    Threshold { input: NodeId, level: f32 },
    Chord(Box<[NodeId]>),
    Any(Box<[NodeId]>),
    Gate { input: NodeId, condition: NodeId },
    Tap { input: NodeId, within: Duration },
    Hold { input: NodeId, duration: Duration },
    AxisFromButtons { negative: NodeId, positive: NodeId },
    Vector2 { x: NodeId, y: NodeId },
    ActionActive(ActionId),
    ActionValue(ActionId),
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) op: Op,
    pub(crate) value_type: ValueType,
}

/// Who owns a compiled binding.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OwnerId {
    /// An action set.
    Set(SetId),
    /// An action layer.
    Layer(LayerId),
}

/// A compiled action.
#[derive(Clone, Debug)]
pub struct Action {
    name: String,
    value_type: ValueType,
    pub(crate) set_bindings: Vec<(SetId, BindingId)>,
    pub(crate) layer_bindings: Vec<(LayerId, BindingId)>,
}

impl Action {
    /// The action's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The shape of the action's value.
    #[must_use]
    pub const fn value_type(&self) -> ValueType {
        self.value_type
    }
}

/// A compiled binding: one node tree, and whose it is.
#[derive(Clone, Debug)]
pub struct Binding {
    action: ActionId,
    owner: OwnerId,
    index: usize,
    pub(crate) first: NodeId,
    pub(crate) root: NodeId,
}

impl Binding {
    /// The action bound.
    #[must_use]
    pub const fn action(&self) -> ActionId {
        self.action
    }

    /// The set or layer the binding belongs to.
    #[must_use]
    pub const fn owner(&self) -> OwnerId {
        self.owner
    }

    /// The binding's 0-based position in its owner's list, as [`GraphSpec`] holds it.
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    /// How many nodes the binding compiled to.
    #[must_use]
    pub const fn node_count(&self) -> u32 {
        self.root.index() - self.first.index() + 1
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Layer {
    pub(crate) name: String,
    /// Indexed by action: whether the layer silences it.
    pub(crate) masks: Vec<bool>,
    /// Indexed by action: whether the layer binds or masks it, and so decides it.
    pub(crate) touches: Vec<bool>,
}

static NEXT_GRAPH: AtomicU64 = AtomicU64::new(1);

/// A compiled action graph: a checked [`GraphSpec`], flattened for evaluation.
///
/// The graph is immutable. Remapping edits the [`GraphSpec`] and compiles again; compiling is a
/// walk over a few dozen nodes, and an immutable graph is what lets an [`crate::input::Evaluator`] hold
/// its per-node state in plain arrays. The document the graph was compiled from stays available
/// ([`ActionGraph::spec`]) for serialization, presentation, and conflict checks.
#[derive(Clone, Debug)]
pub struct ActionGraph {
    /// Which compile this graph came from, so an evaluator can refuse another graph's state.
    pub(crate) instance: u64,
    spec: GraphSpec,
    actions: Vec<Action>,
    action_index: HashMap<String, ActionId>,
    set_names: Vec<String>,
    pub(crate) layers: Vec<Layer>,
    pub(crate) nodes: Vec<Node>,
    bindings: Vec<Binding>,
    /// The actions, each after every action it reads.
    pub(crate) order: Vec<ActionId>,
}

/// A node on the flattening stack: entered once to queue its inputs, left once they are built.
enum Visit<'a> {
    Enter(&'a NodeSpec, String),
    Leave(&'a NodeSpec, String, usize),
}

struct Compiler<'a> {
    spec: &'a GraphSpec,
    action_index: HashMap<String, ActionId>,
    nodes: Vec<Node>,
    errors: Vec<GraphError>,
    /// For each action, the actions its bindings read.
    reads: Vec<Vec<ActionId>>,
}

/// Where a binding is, for the errors found inside it.
struct Place<'a> {
    owner: Owner,
    package: &'a str,
    origin: Option<&'a Origin>,
    action: &'a str,
    binding: usize,
}

impl Compiler<'_> {
    fn fail(&mut self, place: &Place<'_>, path: Option<&str>, kind: GraphErrorKind) {
        self.errors.push(GraphError {
            kind,
            owner: Some(place.owner.clone()),
            package: Some(place.package.to_owned()),
            action: Some(place.action.to_owned()),
            binding: Some(place.binding),
            path: path.map(str::to_owned),
            origin: place.origin.cloned(),
        });
    }

    fn type_of(&self, node: NodeId) -> ValueType {
        self.nodes[node.position()].value_type
    }

    /// Checks one input's type, reporting a mismatch at the node.
    fn expect(
        &mut self,
        place: &Place<'_>,
        path: &str,
        node: &'static str,
        (field, input): (&str, NodeId),
        (expected, accepted): (&'static str, &[ValueType]),
    ) {
        let found = self.type_of(input);
        if !accepted.contains(&found) {
            self.fail(
                place,
                Some(path),
                GraphErrorKind::InputType {
                    node,
                    input: field.to_owned(),
                    expected,
                    found,
                },
            );
        }
    }

    /// Flattens one binding's tree, children before parents, and returns its root.
    fn flatten(&mut self, place: &Place<'_>, reader: Option<ActionId>, root: &NodeSpec) -> NodeId {
        let mut visits = vec![Visit::Enter(root, root.type_name().to_owned())];
        let mut built: Vec<NodeId> = Vec::new();
        while let Some(visit) = visits.pop() {
            match visit {
                Visit::Enter(spec, path) => {
                    let inputs = spec.inputs();
                    visits.push(Visit::Leave(spec, path.clone(), inputs.len()));
                    for (field, input) in inputs.into_iter().rev() {
                        visits.push(Visit::Enter(input, format!("{path}.{field}")));
                    }
                }
                Visit::Leave(spec, path, count) => {
                    let inputs = built.split_off(built.len() - count);
                    let node = self.build(place, reader, spec, &path, &inputs);
                    self.nodes.push(node);
                    built.push(NodeId::from_position(self.nodes.len() - 1));
                }
            }
        }
        built.pop().expect("flattening a tree leaves its root")
    }

    /// Checks one node against its built inputs and makes its compiled form. A node that fails a
    /// check is still built, with the type it would have had, so one mistake is one error.
    #[allow(clippy::too_many_lines)] // one arm per node type
    fn build(
        &mut self,
        place: &Place<'_>,
        reader: Option<ActionId>,
        spec: &NodeSpec,
        path: &str,
        inputs: &[NodeId],
    ) -> Node {
        const DIGITAL: (&str, &[ValueType]) = ("digital", &[ValueType::Digital]);
        const SCALAR: (&str, &[ValueType]) = ("scalar", &[ValueType::Scalar]);
        const NUMERIC: (&str, &[ValueType]) = ("scalar or vector2", &[ValueType::Scalar, ValueType::Vector2]);
        let name = spec.type_name();
        let finite = |compiler: &mut Self, field: &'static str, value: f32| {
            if !value.is_finite() {
                compiler.fail(place, Some(path), GraphErrorKind::NotFinite { node: name, field });
            }
        };
        let millis = |compiler: &mut Self, field: &'static str, value: u32| {
            if value == 0 {
                compiler.fail(place, Some(path), GraphErrorKind::ZeroDuration { node: name, field });
            }
            Duration::from_millis(u64::from(value))
        };
        match spec {
            NodeSpec::SourceButton { control, device } => {
                if !control.is_digital() {
                    self.fail(place, Some(path), GraphErrorKind::NotAButton { control: *control });
                }
                Node {
                    op: Op::SourceButton {
                        control: *control,
                        device: *device,
                    },
                    value_type: ValueType::Digital,
                }
            }
            NodeSpec::SourceAxis { control, device } => {
                if control.is_digital() {
                    self.fail(place, Some(path), GraphErrorKind::NotAnAxis { control: *control });
                }
                Node {
                    op: Op::SourceAxis {
                        control: *control,
                        device: *device,
                    },
                    value_type: ValueType::Scalar,
                }
            }
            NodeSpec::Constant { value } => {
                finite(self, "value", *value);
                Node {
                    op: Op::Constant(*value),
                    value_type: ValueType::Scalar,
                }
            }
            NodeSpec::Invert { .. } => Node {
                op: Op::Invert(inputs[0]),
                value_type: self.type_of(inputs[0]),
            },
            NodeSpec::Scale { factor, .. } => {
                finite(self, "factor", *factor);
                self.expect(place, path, name, ("input", inputs[0]), NUMERIC);
                Node {
                    op: Op::Scale {
                        input: inputs[0],
                        factor: *factor,
                    },
                    value_type: self.numeric_type(inputs[0]),
                }
            }
            NodeSpec::Deadzone { inner, outer, .. } => {
                // Written so a NaN edge fails the check too.
                if !(*inner >= 0.0 && inner < outer && *outer <= 1.0) {
                    self.fail(
                        place,
                        Some(path),
                        GraphErrorKind::InvalidDeadzone {
                            inner: *inner,
                            outer: *outer,
                        },
                    );
                }
                self.expect(place, path, name, ("input", inputs[0]), NUMERIC);
                Node {
                    op: Op::Deadzone {
                        input: inputs[0],
                        inner: *inner,
                        outer: *outer,
                    },
                    value_type: self.numeric_type(inputs[0]),
                }
            }
            NodeSpec::Threshold { level, .. } => {
                if !(level.is_finite() && *level > 0.0) {
                    self.fail(place, Some(path), GraphErrorKind::InvalidLevel { level: *level });
                }
                self.expect(place, path, name, ("input", inputs[0]), SCALAR);
                Node {
                    op: Op::Threshold {
                        input: inputs[0],
                        level: *level,
                    },
                    value_type: ValueType::Digital,
                }
            }
            NodeSpec::Chord { .. } | NodeSpec::Any { .. } => {
                let minimum = if matches!(spec, NodeSpec::Chord { .. }) { 2 } else { 1 };
                if inputs.len() < minimum {
                    self.fail(
                        place,
                        Some(path),
                        GraphErrorKind::TooFewMembers {
                            node: name,
                            minimum,
                            found: inputs.len(),
                        },
                    );
                }
                for (index, member) in inputs.iter().enumerate() {
                    self.expect(place, path, name, (&format!("members[{index}]"), *member), DIGITAL);
                }
                let members = inputs.to_vec().into_boxed_slice();
                Node {
                    op: if minimum == 2 { Op::Chord(members) } else { Op::Any(members) },
                    value_type: ValueType::Digital,
                }
            }
            NodeSpec::Gate { .. } => {
                self.expect(place, path, name, ("condition", inputs[1]), DIGITAL);
                Node {
                    op: Op::Gate {
                        input: inputs[0],
                        condition: inputs[1],
                    },
                    value_type: self.type_of(inputs[0]),
                }
            }
            NodeSpec::Tap { within_ms, .. } => {
                let within = millis(self, "within_ms", *within_ms);
                self.expect(place, path, name, ("input", inputs[0]), DIGITAL);
                Node {
                    op: Op::Tap {
                        input: inputs[0],
                        within,
                    },
                    value_type: ValueType::Digital,
                }
            }
            NodeSpec::Hold { duration_ms, .. } => {
                let duration = millis(self, "duration_ms", *duration_ms);
                self.expect(place, path, name, ("input", inputs[0]), DIGITAL);
                Node {
                    op: Op::Hold {
                        input: inputs[0],
                        duration,
                    },
                    value_type: ValueType::Digital,
                }
            }
            NodeSpec::AxisFromButtons { .. } => {
                self.expect(place, path, name, ("negative", inputs[0]), DIGITAL);
                self.expect(place, path, name, ("positive", inputs[1]), DIGITAL);
                Node {
                    op: Op::AxisFromButtons {
                        negative: inputs[0],
                        positive: inputs[1],
                    },
                    value_type: ValueType::Scalar,
                }
            }
            NodeSpec::Vector2 { .. } => {
                self.expect(place, path, name, ("x", inputs[0]), SCALAR);
                self.expect(place, path, name, ("y", inputs[1]), SCALAR);
                Node {
                    op: Op::Vector2 {
                        x: inputs[0],
                        y: inputs[1],
                    },
                    value_type: ValueType::Vector2,
                }
            }
            NodeSpec::ActionActive { action } | NodeSpec::ActionValue { action } => {
                let active = matches!(spec, NodeSpec::ActionActive { .. });
                let Some(&read) = self.action_index.get(action) else {
                    self.fail(place, Some(path), GraphErrorKind::UnknownAction { name: action.clone() });
                    // Stands in for the missing action so the rest of the tree is still checked.
                    return Node {
                        op: Op::Constant(0.0),
                        value_type: if active { ValueType::Digital } else { ValueType::Scalar },
                    };
                };
                if let Some(reader) = reader
                    && !self.reads[reader.position()].contains(&read)
                {
                    self.reads[reader.position()].push(read);
                }
                if active {
                    Node {
                        op: Op::ActionActive(read),
                        value_type: ValueType::Digital,
                    }
                } else {
                    Node {
                        op: Op::ActionValue(read),
                        value_type: self.spec.actions[read.position()].value_type,
                    }
                }
            }
        }
    }

    /// The type a scale or deadzone passes through: its input's, or scalar when the input is
    /// the wrong type and already reported.
    fn numeric_type(&self, input: NodeId) -> ValueType {
        match self.type_of(input) {
            ValueType::Vector2 => ValueType::Vector2,
            ValueType::Digital | ValueType::Scalar => ValueType::Scalar,
        }
    }
}

/// Checks that every name in `names` is present and unique, reporting what isn't.
fn check_names<'a>(
    what: &'static str,
    names: impl Iterator<Item = (&'a str, Option<&'a Origin>)>,
    errors: &mut Vec<GraphError>,
) {
    let mut seen: Vec<&str> = Vec::new();
    for (name, origin) in names {
        let kind = if name.is_empty() {
            GraphErrorKind::EmptyName { what }
        } else if seen.contains(&name) {
            GraphErrorKind::Duplicate {
                what,
                name: name.to_owned(),
            }
        } else {
            seen.push(name);
            continue;
        };
        errors.push(GraphError {
            origin: origin.cloned(),
            ..GraphError::bare(kind)
        });
    }
}

/// The evaluation order: every action after the actions it reads. `Err` is one cycle, as action
/// positions, the first repeated at the end.
fn evaluation_order(reads: &[Vec<ActionId>]) -> Result<Vec<ActionId>, Vec<usize>> {
    let count = reads.len();
    let mut waiting: Vec<usize> = reads.iter().map(Vec::len).collect();
    let mut order = Vec::with_capacity(count);
    let mut placed = vec![false; count];
    // Declaration order among the actions that are ready, so the order is the same every time.
    while order.len() < count {
        let Some(next) = (0..count).find(|&action| !placed[action] && waiting[action] == 0) else {
            break;
        };
        placed[next] = true;
        order.push(ActionId::from_position(next));
        for (reader, read) in reads.iter().enumerate() {
            if read.iter().any(|action| action.position() == next) {
                waiting[reader] -= 1;
            }
        }
    }
    if order.len() == count {
        return Ok(order);
    }
    // Every action left reads another action left, so following reads from any of them must
    // come back round.
    let start = (0..count).find(|&action| !placed[action]).expect("an unplaced action");
    let mut path = vec![start];
    loop {
        let current = *path.last().expect("the path starts with an action");
        let next = reads[current]
            .iter()
            .map(|action| action.position())
            .find(|&action| !placed[action])
            .expect("an action left unplaced reads another one");
        if let Some(position) = path.iter().position(|&action| action == next) {
            let mut cycle = path.split_off(position);
            cycle.push(next);
            return Err(cycle);
        }
        path.push(next);
    }
}

impl ActionGraph {
    /// Checks `spec` and compiles it.
    ///
    /// # Errors
    ///
    /// Every problem in the document, in document order: unknown and duplicate names, inputs of
    /// the wrong type, bindings that don't produce their action's type, invalid thresholds and
    /// deadzones, and cycles between actions. A document with any error compiles to nothing; a
    /// broken binding is never dropped to let the rest through.
    #[allow(clippy::too_many_lines)] // the document's sections, in order
    pub fn compile(spec: GraphSpec) -> Result<Self, Vec<GraphError>> {
        let mut errors = Vec::new();
        if spec.version != FORMAT_VERSION {
            return Err(vec![GraphError::bare(GraphErrorKind::UnsupportedVersion {
                found: spec.version,
            })]);
        }
        check_names(
            "action",
            spec.actions
                .iter()
                .map(|action| (action.name.as_str(), action.origin.as_ref())),
            &mut errors,
        );
        check_names(
            "set",
            spec.sets.iter().map(|set| (set.name.as_str(), set.origin.as_ref())),
            &mut errors,
        );
        check_names(
            "layer",
            spec.layers.iter().map(|layer| (layer.name.as_str(), layer.origin.as_ref())),
            &mut errors,
        );

        let mut action_index = HashMap::with_capacity(spec.actions.len());
        for (position, action) in spec.actions.iter().enumerate() {
            action_index
                .entry(action.name.clone())
                .or_insert_with(|| ActionId::from_position(position));
        }
        let mut actions: Vec<Action> = spec
            .actions
            .iter()
            .map(|action| Action {
                name: action.name.clone(),
                value_type: action.value_type,
                set_bindings: Vec::new(),
                layer_bindings: Vec::new(),
            })
            .collect();

        let mut compiler = Compiler {
            spec: &spec,
            action_index,
            nodes: Vec::new(),
            errors,
            reads: vec![Vec::new(); spec.actions.len()],
        };
        let mut bindings: Vec<Binding> = Vec::new();
        let sets = spec.sets.iter().enumerate().map(|(position, set)| {
            let owner = OwnerId::Set(SetId::from_position(position));
            (owner, Owner::Set(set.name.clone()), &set.owner, &set.origin, &set.bindings)
        });
        let layers = spec.layers.iter().enumerate().map(|(position, layer)| {
            let owner = OwnerId::Layer(LayerId::from_position(position));
            (
                owner,
                Owner::Layer(layer.name.clone()),
                &layer.owner,
                &layer.origin,
                &layer.bindings,
            )
        });
        for (owner_id, owner, package, origin, owner_bindings) in sets.chain(layers) {
            for (index, binding) in owner_bindings.iter().enumerate() {
                let place = Place {
                    owner: owner.clone(),
                    package,
                    origin: origin.as_ref(),
                    action: &binding.action,
                    binding: index,
                };
                let action = compiler.action_index.get(&binding.action).copied();
                if action.is_none() {
                    compiler.fail(
                        &place,
                        None,
                        GraphErrorKind::UnknownAction {
                            name: binding.action.clone(),
                        },
                    );
                }
                let first = NodeId::from_position(compiler.nodes.len());
                let root = compiler.flatten(&place, action, &binding.input);
                let Some(action) = action else {
                    continue;
                };
                let (expected, found) = (spec.actions[action.position()].value_type, compiler.type_of(root));
                if expected != found {
                    compiler.fail(&place, None, GraphErrorKind::BindingType { expected, found });
                }
                let id = BindingId::from_position(bindings.len());
                bindings.push(Binding {
                    action,
                    owner: owner_id,
                    index,
                    first,
                    root,
                });
                match owner_id {
                    OwnerId::Set(set) => actions[action.position()].set_bindings.push((set, id)),
                    OwnerId::Layer(layer) => actions[action.position()].layer_bindings.push((layer, id)),
                }
            }
        }

        let mut layers = Vec::with_capacity(spec.layers.len());
        for layer in &spec.layers {
            let mut masks = vec![false; spec.actions.len()];
            let mut touches = vec![false; spec.actions.len()];
            for mask in &layer.masks {
                match compiler.action_index.get(mask) {
                    Some(action) => {
                        masks[action.position()] = true;
                        touches[action.position()] = true;
                    }
                    None => compiler.errors.push(GraphError {
                        owner: Some(Owner::Layer(layer.name.clone())),
                        package: Some(layer.owner.clone()),
                        origin: layer.origin.clone(),
                        ..GraphError::bare(GraphErrorKind::UnknownAction { name: mask.clone() })
                    }),
                }
            }
            for binding in &layer.bindings {
                if let Some(action) = compiler.action_index.get(&binding.action) {
                    touches[action.position()] = true;
                }
            }
            layers.push(Layer {
                name: layer.name.clone(),
                masks,
                touches,
            });
        }

        let order = match evaluation_order(&compiler.reads) {
            Ok(order) => order,
            Err(cycle) => {
                let first = cycle[0];
                compiler.errors.push(GraphError {
                    action: Some(spec.actions[first].name.clone()),
                    origin: spec.actions[first].origin.clone(),
                    ..GraphError::bare(GraphErrorKind::Cycle {
                        actions: cycle.iter().map(|&action| spec.actions[action].name.clone()).collect(),
                    })
                });
                Vec::new()
            }
        };

        let Compiler {
            action_index,
            nodes,
            errors,
            ..
        } = compiler;
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Self {
            instance: NEXT_GRAPH.fetch_add(1, Ordering::Relaxed),
            set_names: spec.sets.iter().map(|set| set.name.clone()).collect(),
            spec,
            actions,
            action_index,
            layers,
            nodes,
            bindings,
            order,
        })
    }

    /// The document this graph was compiled from.
    #[must_use]
    pub const fn spec(&self) -> &GraphSpec {
        &self.spec
    }

    /// The actions, in declaration order; an action's position is its id.
    #[must_use]
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// An action by id.
    #[must_use]
    pub fn action(&self, action: ActionId) -> Option<&Action> {
        self.actions.get(action.position())
    }

    /// The id of the action named `name`.
    #[must_use]
    pub fn action_id(&self, name: &str) -> Option<ActionId> {
        self.action_index.get(name).copied()
    }

    /// The id of the set named `name`.
    #[must_use]
    pub fn set_id(&self, name: &str) -> Option<SetId> {
        self.set_names.iter().position(|set| set == name).map(SetId::from_position)
    }

    /// A set's name.
    #[must_use]
    pub fn set_name(&self, set: SetId) -> Option<&str> {
        self.set_names.get(set.position()).map(String::as_str)
    }

    /// How many sets the graph has.
    #[must_use]
    pub fn set_count(&self) -> usize {
        self.set_names.len()
    }

    /// The id of the layer named `name`.
    #[must_use]
    pub fn layer_id(&self, name: &str) -> Option<LayerId> {
        self.layers
            .iter()
            .position(|layer| layer.name == name)
            .map(LayerId::from_position)
    }

    /// A layer's name.
    #[must_use]
    pub fn layer_name(&self, layer: LayerId) -> Option<&str> {
        self.layers.get(layer.position()).map(|layer| layer.name.as_str())
    }

    /// How many layers the graph has.
    #[must_use]
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// The compiled bindings, sets first, each owner's in author order; a binding's position is
    /// its id.
    #[must_use]
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// A binding by id.
    #[must_use]
    pub fn binding(&self, binding: BindingId) -> Option<&Binding> {
        self.bindings.get(binding.position())
    }

    /// The document's form of a compiled binding: its owner by name and its node tree.
    #[must_use]
    pub fn binding_spec(&self, binding: BindingId) -> Option<(Owner, &NodeSpec)> {
        let compiled = self.bindings.get(binding.position())?;
        Some(match compiled.owner {
            OwnerId::Set(set) => {
                let set = &self.spec.sets[set.position()];
                (Owner::Set(set.name.clone()), &set.bindings[compiled.index].input)
            }
            OwnerId::Layer(layer) => {
                let layer = &self.spec.layers[layer.position()];
                (Owner::Layer(layer.name.clone()), &layer.bindings[compiled.index].input)
            }
        })
    }

    /// How many nodes the graph compiled to.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The order actions are evaluated in: every action after the actions it reads.
    #[must_use]
    pub fn evaluation_order(&self) -> &[ActionId] {
        &self.order
    }
}
