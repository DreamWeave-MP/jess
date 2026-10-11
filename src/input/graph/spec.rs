//! The binding document: what an author writes, what gets saved, and what remapping edits.
//!
//! A [`GraphSpec`] declares actions, and binds them in sets and layers. Each binding is a tree of
//! [`NodeSpec`]s: sources at the leaves, transforms and activators above them, and the action at
//! the root. The document is plain data. It serializes deterministically (fields in declaration
//! order, lists in author order), carries a format version, and refuses to load anything it
//! doesn't understand: an unknown node type or an unknown field is an error naming it, never a
//! binding quietly dropped.
//!
//! The document is not what runs. [`crate::input::ActionGraph::compile`] checks it and flattens it into
//! the arena the evaluator walks.

use std::fmt;

use crate::input::{Control, DeviceId};

/// The binding format version this crate writes, and the only one it reads.
pub const FORMAT_VERSION: u32 = 1;

/// The shape of an action's value, and of every node's output.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueType {
    /// On or off.
    Digital,
    /// One number.
    Scalar,
    /// Two numbers, `x` and `y`.
    Vector2,
}

impl ValueType {
    /// The type's name as it serializes.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Digital => "digital",
            Self::Scalar => "scalar",
            Self::Vector2 => "vector2",
        }
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Where in an author's source something was declared, for diagnostics.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    /// The source file, as the author's runtime names it.
    pub file: String,
    /// The 1-based line.
    pub line: u32,
    /// The 1-based column, or 0 when the frontend only knows the line.
    pub column: u32,
}

impl fmt::Display for Origin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.file, self.line)?;
        if self.column > 0 {
            write!(formatter, ":{}", self.column)?;
        }
        Ok(())
    }
}

/// A semantic action's declaration: its name and the shape of its value.
///
/// Names are the stable identity. `jess::input` reserves none and reads no meaning into them;
/// `ui.accept` and `game.move` are conventions of the packages that declare them.
#[derive(Clone, PartialEq, Eq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSpec {
    /// The action's name, unique in the graph.
    pub name: String,
    /// The shape of its value. Every binding to the action must produce this type.
    pub value_type: ValueType,
    /// Where it was declared.
    #[serde(default)]
    pub origin: Option<Origin>,
}

/// One binding: an action, and the node tree whose output is the action's value.
#[derive(Clone, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingSpec {
    /// The action bound, by name.
    pub action: String,
    /// The node tree producing the action's value.
    pub input: NodeSpec,
}

/// An action set: the bindings of one coherent mode, such as gameplay or a menu.
///
/// Any number of sets can be active at once. An action bound more than once, in one set or
/// across active sets, takes the strongest of its bindings' values (see
/// [`crate::input::Evaluator::evaluate`]).
#[derive(Clone, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSpec {
    /// The set's name, unique among sets.
    pub name: String,
    /// The package that declared it, for diagnostics.
    pub owner: String,
    /// The set's bindings, in author order.
    pub bindings: Vec<BindingSpec>,
    /// Where it was declared.
    #[serde(default)]
    pub origin: Option<Origin>,
}

/// An action layer: a temporary change laid over the active sets.
///
/// For each action it names, an active layer replaces whatever is below it: its own bindings to
/// the action take the place of every set's and every lower layer's, and an action in `masks`
/// produces nothing at all. Actions a layer doesn't name pass through untouched. When several
/// layers are active, the one pushed last is on top and decides first.
#[derive(Clone, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
    /// The layer's name, unique among layers.
    pub name: String,
    /// The package that declared it, for diagnostics.
    pub owner: String,
    /// The bindings that replace lower ones, in author order.
    pub bindings: Vec<BindingSpec>,
    /// Actions the layer silences.
    #[serde(default)]
    pub masks: Vec<String>,
    /// Where it was declared.
    #[serde(default)]
    pub origin: Option<Origin>,
}

/// One node of a binding's tree.
///
/// Serialized as a table tagged by `type`: `{ "type": "chord", "members": [...] }`. Every node
/// produces one [`ValueType`], and every input is checked against what the node takes; nothing
/// is coerced. Durations are whole milliseconds on the monotonic clock.
#[derive(Clone, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum NodeSpec {
    /// A button: digital. On any connected device that has the control, or on `device` alone.
    SourceButton {
        /// The button.
        control: Control,
        /// A single device to read, or every device of the control's kind.
        #[serde(default)]
        device: Option<DeviceId>,
    },
    /// An axis: scalar, in the control's range.
    SourceAxis {
        /// The axis.
        control: Control,
        /// A single device to read, or every device of the control's kind.
        #[serde(default)]
        device: Option<DeviceId>,
    },
    /// A fixed scalar.
    Constant {
        /// The value.
        value: f32,
    },
    /// Digital: not. Scalar and vector: negated.
    Invert {
        /// The input, of any type.
        input: Box<Self>,
    },
    /// A scalar or vector multiplied by `factor`.
    Scale {
        /// The input.
        input: Box<Self>,
        /// The multiplier.
        factor: f32,
    },
    /// A deadzone. Magnitudes at or below `inner` are zero, at or above `outer` are one, and the
    /// band between is rescaled to `0..1`, so the output has no jump at the inner edge. On a
    /// vector the magnitude is the length, which keeps a stick's deadzone round.
    Deadzone {
        /// The input, scalar or vector.
        input: Box<Self>,
        /// The inner edge, `0 <= inner < outer`.
        inner: f32,
        /// The outer edge, `inner < outer <= 1`.
        outer: f32,
    },
    /// A scalar as a button: on while its magnitude is at least `level`.
    Threshold {
        /// The scalar input.
        input: Box<Self>,
        /// The magnitude to reach, above zero.
        level: f32,
    },
    /// Every member held at once. The chord turns on when its last member does and off when any
    /// member does; the order the members went down in doesn't matter. Members aren't consumed:
    /// a plain binding to one of them still fires.
    Chord {
        /// Two or more digital inputs.
        members: Vec<Self>,
    },
    /// Any member held.
    Any {
        /// One or more digital inputs.
        members: Vec<Self>,
    },
    /// The input while `condition` is on, and the input type's zero while it is off.
    Gate {
        /// The input, of any type.
        input: Box<Self>,
        /// The digital condition.
        condition: Box<Self>,
    },
    /// A tap: on for exactly one tick, the tick the input is released, if it was held for at
    /// most `within_ms`.
    Tap {
        /// The digital input.
        input: Box<Self>,
        /// The longest hold that still counts as a tap, in milliseconds, above zero.
        within_ms: u32,
    },
    /// A hold: on from the moment the input has been held for `duration_ms` until it is
    /// released. The action's press edge is the threshold crossing, once.
    Hold {
        /// The digital input.
        input: Box<Self>,
        /// How long the input must be held, in milliseconds, above zero.
        duration_ms: u32,
    },
    /// Two buttons as an axis: `-1` while only `negative` is held, `1` while only `positive` is,
    /// `0` while neither or both are.
    AxisFromButtons {
        /// The digital input for `-1`.
        negative: Box<Self>,
        /// The digital input for `1`.
        positive: Box<Self>,
    },
    /// Two scalars as a vector.
    Vector2 {
        /// The scalar `x`.
        x: Box<Self>,
        /// The scalar `y`.
        y: Box<Self>,
    },
    /// Whether another action is active, this tick: digital. The dependency is structural, so a
    /// cycle of these is a compile error.
    ActionActive {
        /// The other action, by name.
        action: String,
    },
    /// Another action's value, this tick, with that action's type.
    ActionValue {
        /// The other action, by name.
        action: String,
    },
}

impl NodeSpec {
    /// A button on any device.
    #[must_use]
    pub const fn button(control: Control) -> Self {
        Self::SourceButton { control, device: None }
    }

    /// An axis on any device.
    #[must_use]
    pub const fn axis(control: Control) -> Self {
        Self::SourceAxis { control, device: None }
    }

    /// A chord of `members`.
    #[must_use]
    pub fn chord(members: impl IntoIterator<Item = Self>) -> Self {
        Self::Chord {
            members: members.into_iter().collect(),
        }
    }

    /// Any of `members`.
    #[must_use]
    pub fn any(members: impl IntoIterator<Item = Self>) -> Self {
        Self::Any {
            members: members.into_iter().collect(),
        }
    }

    /// `input` while `condition` is on.
    #[must_use]
    pub fn gate(input: Self, condition: Self) -> Self {
        Self::Gate {
            input: Box::new(input),
            condition: Box::new(condition),
        }
    }

    /// A tap of `input` within `within_ms` milliseconds.
    #[must_use]
    pub fn tap(input: Self, within_ms: u32) -> Self {
        Self::Tap {
            input: Box::new(input),
            within_ms,
        }
    }

    /// A hold of `input` for `duration_ms` milliseconds.
    #[must_use]
    pub fn hold(input: Self, duration_ms: u32) -> Self {
        Self::Hold {
            input: Box::new(input),
            duration_ms,
        }
    }

    /// `negative` and `positive` as an axis.
    #[must_use]
    pub fn axis_from_buttons(negative: Self, positive: Self) -> Self {
        Self::AxisFromButtons {
            negative: Box::new(negative),
            positive: Box::new(positive),
        }
    }

    /// `x` and `y` as a vector.
    #[must_use]
    pub fn vector2(x: Self, y: Self) -> Self {
        Self::Vector2 {
            x: Box::new(x),
            y: Box::new(y),
        }
    }

    /// `input` with a deadzone from `inner` to `outer`.
    #[must_use]
    pub fn deadzone(input: Self, inner: f32, outer: f32) -> Self {
        Self::Deadzone {
            input: Box::new(input),
            inner,
            outer,
        }
    }

    /// `input` inverted.
    #[must_use]
    pub fn invert(input: Self) -> Self {
        Self::Invert { input: Box::new(input) }
    }

    /// `input` times `factor`.
    #[must_use]
    pub fn scale(input: Self, factor: f32) -> Self {
        Self::Scale {
            input: Box::new(input),
            factor,
        }
    }

    /// `input` as a button at `level`.
    #[must_use]
    pub fn threshold(input: Self, level: f32) -> Self {
        Self::Threshold {
            input: Box::new(input),
            level,
        }
    }

    /// Whether the action named `action` is active.
    #[must_use]
    pub fn action_active(action: impl Into<String>) -> Self {
        Self::ActionActive { action: action.into() }
    }

    /// The node's type name as it serializes (`chord`, `axisFromButtons`, ...).
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::SourceButton { .. } => "sourceButton",
            Self::SourceAxis { .. } => "sourceAxis",
            Self::Constant { .. } => "constant",
            Self::Invert { .. } => "invert",
            Self::Scale { .. } => "scale",
            Self::Deadzone { .. } => "deadzone",
            Self::Threshold { .. } => "threshold",
            Self::Chord { .. } => "chord",
            Self::Any { .. } => "any",
            Self::Gate { .. } => "gate",
            Self::Tap { .. } => "tap",
            Self::Hold { .. } => "hold",
            Self::AxisFromButtons { .. } => "axisFromButtons",
            Self::Vector2 { .. } => "vector2",
            Self::ActionActive { .. } => "actionActive",
            Self::ActionValue { .. } => "actionValue",
        }
    }

    /// The node's inputs, each with the field it sits in (`input`, `members[1]`, `x`), in order.
    #[must_use]
    pub fn inputs(&self) -> Vec<(String, &Self)> {
        match self {
            Self::SourceButton { .. }
            | Self::SourceAxis { .. }
            | Self::Constant { .. }
            | Self::ActionActive { .. }
            | Self::ActionValue { .. } => Vec::new(),
            Self::Invert { input }
            | Self::Scale { input, .. }
            | Self::Deadzone { input, .. }
            | Self::Threshold { input, .. }
            | Self::Tap { input, .. }
            | Self::Hold { input, .. } => vec![("input".to_owned(), &**input)],
            Self::Chord { members } | Self::Any { members } => members
                .iter()
                .enumerate()
                .map(|(index, member)| (format!("members[{index}]"), member))
                .collect(),
            Self::Gate { input, condition } => {
                vec![("input".to_owned(), &**input), ("condition".to_owned(), &**condition)]
            }
            Self::AxisFromButtons { negative, positive } => {
                vec![("negative".to_owned(), &**negative), ("positive".to_owned(), &**positive)]
            }
            Self::Vector2 { x, y } => vec![("x".to_owned(), &**x), ("y".to_owned(), &**y)],
        }
    }
}

/// A set or a layer, by name: who owns a binding.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Owner {
    /// The action set of this name.
    Set(String),
    /// The action layer of this name.
    Layer(String),
}

impl fmt::Display for Owner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Set(name) => write!(formatter, "set {name:?}"),
            Self::Layer(name) => write!(formatter, "layer {name:?}"),
        }
    }
}

/// One binding's place in a document: its owner, and its position in the owner's list.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BindingLocation {
    /// The set or layer.
    pub owner: Owner,
    /// The 0-based position in the owner's `bindings`.
    pub index: usize,
}

/// Why a document didn't load, or an edit to one didn't apply.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SpecError {
    /// The text isn't a binding document: bad JSON, an unknown node type, an unknown field, or a
    /// value of the wrong type. The message names the line and column.
    Malformed(String),
    /// The document's format version isn't the one this crate reads.
    UnsupportedVersion {
        /// The version the document says it is.
        found: u32,
    },
    /// No set or layer of this name.
    UnknownOwner(Owner),
    /// The owner has no binding at this position.
    NoSuchBinding(BindingLocation),
    /// The action isn't declared.
    UnknownAction(String),
    /// A saved document declares different actions than the defaults it would replace.
    IncompatibleActions(Vec<String>),
}

impl fmt::Display for SpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(message) => write!(formatter, "the binding document doesn't parse: {message}"),
            Self::UnsupportedVersion { found } => write!(
                formatter,
                "the binding document is format version {found}, and this jess::input reads version {FORMAT_VERSION}"
            ),
            Self::UnknownOwner(owner) => write!(formatter, "the binding document has no {owner}"),
            Self::NoSuchBinding(location) => {
                write!(formatter, "{} has no binding {}", location.owner, location.index)
            }
            Self::UnknownAction(name) => write!(formatter, "the binding document declares no action {name:?}"),
            Self::IncompatibleActions(problems) => write!(
                formatter,
                "the saved bindings don't fit the installed actions: {}",
                problems.join("; ")
            ),
        }
    }
}

impl std::error::Error for SpecError {}

/// A binding document: the actions, and the sets and layers that bind them.
#[derive(Clone, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSpec {
    /// The format version, [`FORMAT_VERSION`].
    pub version: u32,
    /// The actions, in declaration order.
    pub actions: Vec<ActionSpec>,
    /// The action sets, in declaration order.
    pub sets: Vec<SetSpec>,
    /// The action layers, in declaration order.
    #[serde(default)]
    pub layers: Vec<LayerSpec>,
}

impl Default for GraphSpec {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphSpec {
    /// An empty document at the current format version.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            version: FORMAT_VERSION,
            actions: Vec::new(),
            sets: Vec::new(),
            layers: Vec::new(),
        }
    }

    /// Declares an action.
    pub fn action(&mut self, name: impl Into<String>, value_type: ValueType) -> &mut Self {
        self.actions.push(ActionSpec {
            name: name.into(),
            value_type,
            origin: None,
        });
        self
    }

    /// Declares an empty set.
    pub fn set(&mut self, name: impl Into<String>, owner: impl Into<String>) -> &mut Self {
        self.sets.push(SetSpec {
            name: name.into(),
            owner: owner.into(),
            bindings: Vec::new(),
            origin: None,
        });
        self
    }

    /// Declares an empty layer.
    pub fn layer(&mut self, name: impl Into<String>, owner: impl Into<String>) -> &mut Self {
        self.layers.push(LayerSpec {
            name: name.into(),
            owner: owner.into(),
            bindings: Vec::new(),
            masks: Vec::new(),
            origin: None,
        });
        self
    }

    /// The document as JSON: pretty-printed, fields in declaration order, lists in author order,
    /// ending in a newline. The same document always gives the same text.
    ///
    /// # Panics
    ///
    /// If a number in the document isn't finite, which [`crate::input::ActionGraph::compile`] rejects
    /// first.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("a binding document holds only finite data and strings");
        text.push('\n');
        text
    }

    /// Reads a document from JSON.
    ///
    /// # Errors
    ///
    /// [`SpecError::Malformed`] for anything that isn't a document, naming the line and column,
    /// and [`SpecError::UnsupportedVersion`] for a document of another format version. Nothing
    /// unrecognized is skipped.
    pub fn from_json(text: &str) -> Result<Self, SpecError> {
        // The version is read on its own first, so a newer format says it is newer instead of
        // failing on whichever node it added.
        #[derive(serde::Deserialize)]
        struct Versioned {
            version: u32,
        }
        let versioned: Versioned = serde_json::from_str(text).map_err(|error| SpecError::Malformed(error.to_string()))?;
        if versioned.version != FORMAT_VERSION {
            return Err(SpecError::UnsupportedVersion {
                found: versioned.version,
            });
        }
        serde_json::from_str(text).map_err(|error| SpecError::Malformed(error.to_string()))
    }

    fn bindings_of_owner(&self, owner: &Owner) -> Option<&Vec<BindingSpec>> {
        match owner {
            Owner::Set(name) => self.sets.iter().find(|set| set.name == *name).map(|set| &set.bindings),
            Owner::Layer(name) => self
                .layers
                .iter()
                .find(|layer| layer.name == *name)
                .map(|layer| &layer.bindings),
        }
    }

    fn bindings_of_owner_mut(&mut self, owner: &Owner) -> Result<&mut Vec<BindingSpec>, SpecError> {
        match owner {
            Owner::Set(name) => self
                .sets
                .iter_mut()
                .find(|set| set.name == *name)
                .map(|set| &mut set.bindings),
            Owner::Layer(name) => self
                .layers
                .iter_mut()
                .find(|layer| layer.name == *name)
                .map(|layer| &mut layer.bindings),
        }
        .ok_or_else(|| SpecError::UnknownOwner(owner.clone()))
    }

    /// Every binding to `action`, sets first, each in author order.
    #[must_use]
    pub fn bindings_for(&self, action: &str) -> Vec<BindingLocation> {
        let sets = self.sets.iter().map(|set| (Owner::Set(set.name.clone()), &set.bindings));
        let layers = self
            .layers
            .iter()
            .map(|layer| (Owner::Layer(layer.name.clone()), &layer.bindings));
        let mut found = Vec::new();
        for (owner, bindings) in sets.chain(layers) {
            for (index, binding) in bindings.iter().enumerate() {
                if binding.action == action {
                    found.push(BindingLocation {
                        owner: owner.clone(),
                        index,
                    });
                }
            }
        }
        found
    }

    /// The binding at `location`.
    #[must_use]
    pub fn binding(&self, location: &BindingLocation) -> Option<&BindingSpec> {
        self.bindings_of_owner(&location.owner)?.get(location.index)
    }

    /// Adds a binding to `action` at the end of `owner`'s list: an alternate, if the owner
    /// already binds the action.
    ///
    /// # Errors
    ///
    /// The owner or the action doesn't exist. The new binding's type is checked when the
    /// document is next compiled.
    pub fn add_binding(&mut self, owner: &Owner, action: &str, input: NodeSpec) -> Result<BindingLocation, SpecError> {
        if !self.actions.iter().any(|declared| declared.name == action) {
            return Err(SpecError::UnknownAction(action.to_owned()));
        }
        let bindings = self.bindings_of_owner_mut(owner)?;
        bindings.push(BindingSpec {
            action: action.to_owned(),
            input,
        });
        Ok(BindingLocation {
            owner: owner.clone(),
            index: bindings.len() - 1,
        })
    }

    /// Replaces the node tree of the binding at `location`, returning the old one.
    ///
    /// # Errors
    ///
    /// The owner or the binding doesn't exist.
    pub fn replace_binding(&mut self, location: &BindingLocation, input: NodeSpec) -> Result<NodeSpec, SpecError> {
        let bindings = self.bindings_of_owner_mut(&location.owner)?;
        let binding = bindings
            .get_mut(location.index)
            .ok_or_else(|| SpecError::NoSuchBinding(location.clone()))?;
        Ok(std::mem::replace(&mut binding.input, input))
    }

    /// Removes the binding at `location`, returning it. Later bindings of the same owner move up
    /// one position.
    ///
    /// # Errors
    ///
    /// The owner or the binding doesn't exist.
    pub fn remove_binding(&mut self, location: &BindingLocation) -> Result<BindingSpec, SpecError> {
        let bindings = self.bindings_of_owner_mut(&location.owner)?;
        if location.index >= bindings.len() {
            return Err(SpecError::NoSuchBinding(location.clone()));
        }
        Ok(bindings.remove(location.index))
    }

    /// Checks that this document declares the same actions, by name and type, as `defaults`:
    /// what a user's saved bindings must do before they replace a package's.
    ///
    /// # Errors
    ///
    /// [`SpecError::IncompatibleActions`], listing every action that is missing, extra, or of
    /// another type.
    pub fn check_compatible(&self, defaults: &Self) -> Result<(), SpecError> {
        let mut problems = Vec::new();
        for default in &defaults.actions {
            match self.actions.iter().find(|action| action.name == default.name) {
                None => problems.push(format!("{:?} is missing", default.name)),
                Some(action) if action.value_type != default.value_type => problems.push(format!(
                    "{:?} is {} and the installed action is {}",
                    default.name, action.value_type, default.value_type
                )),
                Some(_) => {}
            }
        }
        for action in &self.actions {
            if !defaults.actions.iter().any(|default| default.name == action.name) {
                problems.push(format!("{:?} is not an installed action", action.name));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(SpecError::IncompatibleActions(problems))
        }
    }
}
