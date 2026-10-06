//! Binding presentation: what a binding is made of, as data a GUI can draw however it likes.
//!
//! Nothing here renders. A [`Presentation`] is the shape of a binding (this control, held, while
//! that chord is down), and a [`ControlPresentation`] is one control with the semantic label its
//! [`PresentationFamily`] gives it (`South` is `A` on an Xbox pad and `Cross` on one of Sony's).
//! Whether that label becomes a glyph, a keycap, or a sentence read aloud is the GUI's business;
//! mapping labels to artwork is a theme's.

use crate::input::graph::spec::{BindingLocation, GraphSpec, NodeSpec, Owner};
use crate::input::{Control, PresentationFamily};

/// One control in a binding, with its family's label.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ControlPresentation {
    /// The control.
    pub control: Control,
    /// The family the label is from.
    pub family: PresentationFamily,
    /// The family's semantic label for the control.
    pub label: String,
}

/// The shape of a binding, reduced to what a person needs to be shown.
///
/// Transforms that don't change what to press (`invert`, `scale`, `deadzone`, `threshold`) are
/// left out: the presentation of a deadzoned stick is the stick.
#[derive(Clone, PartialEq, Debug)]
pub enum Presentation {
    /// One control.
    Control(ControlPresentation),
    /// Every part held together.
    Chord(Vec<Self>),
    /// Any one of the parts.
    AnyOf(Vec<Self>),
    /// A press and release within the threshold.
    Tap {
        /// What to tap.
        input: Box<Self>,
        /// The threshold, in milliseconds.
        within_ms: u32,
    },
    /// A press held for the threshold.
    Hold {
        /// What to hold.
        input: Box<Self>,
        /// The threshold, in milliseconds.
        duration_ms: u32,
    },
    /// Two inputs pulling an axis apart.
    Axis {
        /// Toward `-1`.
        negative: Box<Self>,
        /// Toward `1`.
        positive: Box<Self>,
    },
    /// Two axes as a vector.
    Vector2 {
        /// The horizontal axis.
        x: Box<Self>,
        /// The vertical axis.
        y: Box<Self>,
    },
    /// An input that only counts while a condition holds.
    While {
        /// The input.
        input: Box<Self>,
        /// The condition.
        condition: Box<Self>,
    },
    /// Another action, by name: this binding follows it.
    Action(String),
    /// A fixed value; nothing to press.
    Constant,
}

/// One binding of an action, as presentation data.
#[derive(Clone, PartialEq, Debug)]
pub struct BindingPresentation {
    /// The action's name.
    pub action: String,
    /// Where the binding is in the document, which is also how a remap addresses it.
    pub location: BindingLocation,
    /// The family the labels are from.
    pub family: PresentationFamily,
    /// The binding's shape.
    pub shape: Presentation,
}

/// A node on the conversion stack: entered once to queue its inputs, left once they are built.
enum Visit<'a> {
    Enter(&'a NodeSpec),
    Leave(&'a NodeSpec, usize),
}

/// The presentation of one node tree, and whether every control in it belongs to `family`.
fn present(root: &NodeSpec, family: PresentationFamily) -> (Presentation, bool) {
    let mut visits = vec![Visit::Enter(root)];
    let mut built: Vec<Presentation> = Vec::new();
    let mut covered = true;
    while let Some(visit) = visits.pop() {
        let (spec, count) = match visit {
            Visit::Enter(spec) => {
                let inputs = spec.inputs();
                visits.push(Visit::Leave(spec, inputs.len()));
                visits.extend(inputs.into_iter().rev().map(|(_, input)| Visit::Enter(input)));
                continue;
            }
            Visit::Leave(spec, count) => (spec, count),
        };
        let mut inputs = built.split_off(built.len() - count).into_iter();
        let mut next = || Box::new(inputs.next().expect("a node's inputs were built before it"));
        let shape = match spec {
            NodeSpec::SourceButton { control, .. } | NodeSpec::SourceAxis { control, .. } => {
                covered &= family.covers(control.device_kind());
                Presentation::Control(ControlPresentation {
                    control: *control,
                    family,
                    label: family.label(*control),
                })
            }
            NodeSpec::Constant { .. } => Presentation::Constant,
            NodeSpec::Invert { .. } | NodeSpec::Scale { .. } | NodeSpec::Deadzone { .. } | NodeSpec::Threshold { .. } => {
                *next()
            }
            NodeSpec::Chord { .. } => Presentation::Chord(inputs.collect()),
            NodeSpec::Any { .. } => Presentation::AnyOf(inputs.collect()),
            NodeSpec::Gate { .. } => Presentation::While {
                input: next(),
                condition: next(),
            },
            NodeSpec::Tap { within_ms, .. } => Presentation::Tap {
                input: next(),
                within_ms: *within_ms,
            },
            NodeSpec::Hold { duration_ms, .. } => Presentation::Hold {
                input: next(),
                duration_ms: *duration_ms,
            },
            NodeSpec::AxisFromButtons { .. } => Presentation::Axis {
                negative: next(),
                positive: next(),
            },
            NodeSpec::Vector2 { .. } => Presentation::Vector2 { x: next(), y: next() },
            NodeSpec::ActionActive { action } | NodeSpec::ActionValue { action } => Presentation::Action(action.clone()),
        };
        built.push(shape);
    }
    (built.pop().expect("converting a tree leaves its root"), covered)
}

/// The bindings of `action` a person using `family` can press, as presentation data: every
/// binding whose controls all belong to the family, sets first, each owner's in author order.
///
/// A binding that mixes families (a keyboard modifier with a gamepad button) belongs to neither
/// and is left out; ask for it with [`binding_presentation`].
#[must_use]
pub fn presentations(spec: &GraphSpec, action: &str, family: PresentationFamily) -> Vec<BindingPresentation> {
    spec.bindings_for(action)
        .into_iter()
        .filter_map(|location| {
            let binding = spec.binding(&location)?;
            let (shape, covered) = present(&binding.input, family);
            covered.then(|| BindingPresentation {
                action: action.to_owned(),
                location,
                family,
                shape,
            })
        })
        .collect()
}

/// The presentation of the binding at `location`, labeled by `family` whatever its controls are.
#[must_use]
pub fn binding_presentation(
    spec: &GraphSpec,
    location: &BindingLocation,
    family: PresentationFamily,
) -> Option<BindingPresentation> {
    let binding = spec.binding(location)?;
    Some(BindingPresentation {
        action: binding.action.clone(),
        location: location.clone(),
        family,
        shape: present(&binding.input, family).0,
    })
}

/// What two bindings share that makes them a conflict.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ConflictKind {
    /// Both fire on the same press, or the same chord.
    Press,
    /// Both fire on a tap of the same input.
    Tap,
    /// Both fire on a hold of the same input.
    Hold,
    /// Both read the same axis.
    Axis,
}

/// Two bindings in one set or layer that fire different actions from the same input.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Conflict {
    /// The set or layer both bindings are in.
    pub owner: Owner,
    /// How they collide.
    pub kind: ConflictKind,
    /// The controls both bindings read, sorted.
    pub controls: Vec<Control>,
    /// The two actions.
    pub actions: [String; 2],
    /// The two bindings, in document order.
    pub bindings: [BindingLocation; 2],
}

/// What a binding fires on, when that is simple enough to compare: a press of one button or one
/// chord, a tap or hold of one, or one bare axis. Anything gated, combined, or derived from
/// another action depends on context and has no signature.
fn signature(node: &NodeSpec) -> Option<(ConflictKind, Vec<Control>)> {
    let pressed = |node: &NodeSpec| -> Option<Vec<Control>> {
        match node {
            NodeSpec::SourceButton { control, device: None } => Some(vec![*control]),
            NodeSpec::Chord { members } => {
                let mut controls = Vec::with_capacity(members.len());
                for member in members {
                    let NodeSpec::SourceButton { control, device: None } = member else {
                        return None;
                    };
                    controls.push(*control);
                }
                controls.sort_unstable();
                Some(controls)
            }
            _ => None,
        }
    };
    match node {
        NodeSpec::Tap { input, .. } => pressed(input).map(|controls| (ConflictKind::Tap, controls)),
        NodeSpec::Hold { input, .. } => pressed(input).map(|controls| (ConflictKind::Hold, controls)),
        NodeSpec::SourceAxis { control, device: None } => Some((ConflictKind::Axis, vec![*control])),
        other => pressed(other).map(|controls| (ConflictKind::Press, controls)),
    }
}

/// The plain conflicts in a document: within one set or one layer, two bindings to different
/// actions that fire on the same press, chord, tap, hold, or axis.
///
/// This is the check that is always a mistake. It deliberately says nothing about the rest:
/// `Ctrl+S` beside `S`, a tap beside a hold, the same button in two sets, and anything behind a
/// gate can all be intended, and whether they are depends on which sets and layers an
/// application activates together. The document and [`presentations`] expose the structure a
/// tool needs to judge those itself.
#[must_use]
pub fn conflicts(spec: &GraphSpec) -> Vec<Conflict> {
    let sets = spec.sets.iter().map(|set| (Owner::Set(set.name.clone()), &set.bindings));
    let layers = spec
        .layers
        .iter()
        .map(|layer| (Owner::Layer(layer.name.clone()), &layer.bindings));
    let mut found = Vec::new();
    for (owner, bindings) in sets.chain(layers) {
        let signatures: Vec<_> = bindings.iter().map(|binding| signature(&binding.input)).collect();
        for (first, first_signature) in signatures.iter().enumerate() {
            let Some((kind, controls)) = first_signature else {
                continue;
            };
            for (second, second_signature) in signatures.iter().enumerate().skip(first + 1) {
                if second_signature.as_ref() != Some(&(*kind, controls.clone()))
                    || bindings[first].action == bindings[second].action
                {
                    continue;
                }
                found.push(Conflict {
                    owner: owner.clone(),
                    kind: *kind,
                    controls: controls.clone(),
                    actions: [bindings[first].action.clone(), bindings[second].action.clone()],
                    bindings: [first, second].map(|index| BindingLocation {
                        owner: owner.clone(),
                        index,
                    }),
                });
            }
        }
    }
    found
}
