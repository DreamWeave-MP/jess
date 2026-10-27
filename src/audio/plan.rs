//! Plans: what the graph compiles to, and what the engine runs.
//!
//! A plan is a list of steps, one for each node that runs, in an order where each comes after
//! everything it hears. Each step names its node by slot and generation, the buffer it runs in,
//! the sidechain it pulls before it runs, and the connections it pushes after. A plan owns no
//! node: the engine's node table does, so replacing a plan leaves every processor and its
//! automation where it was. What a plan does own is what belongs to its connections (compensation
//! lines, feedback rings) and the scratch its nodes' other outputs share.
//!
//! The control side builds a plan whole ([`Definition::compile`]) and sends it; the engine only
//! swaps it in, moving the old plan's delay history into the new one, and sends the old one back.
//!
//! [`Definition::compile`]: crate::audio::graph::Definition::compile

use crate::audio::delays::{Delays, Rings};
use crate::audio::mixer::NodeKey;

/// Where an output's block lives while a block renders.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    /// The buffer of the node in this slot: a node that works in place leaves its first output
    /// there, and an insert chain runs in its host's.
    Buffer(u32),
    /// This stride of the plan's scratch.
    Scratch(u32),
}

/// A node input: its slot and generation, and which audio input (from 0, sidechains not
/// counted) it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    pub slot: u32,
    pub generation: u32,
    pub input: u8,
}

/// An audio connection: after its step, the output at `from` is added into `to` at `level`,
/// converted between channel counts and through compensation line `line` if it has one.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Push {
    pub from: Place,
    pub from_channels: u8,
    pub to: Target,
    pub to_channels: u8,
    pub level: f32,
    pub line: Option<u32>,
}

/// A sidechain: before its step, the output at `from` is read, converted to the node's channels,
/// through compensation line `line` if it has one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pull {
    pub from: Place,
    pub from_channels: u8,
    pub line: Option<u32>,
}

/// A feedback connection's ends: after its source's step, the output at `from` is written into
/// ring `ring`; at the start of every block, what is due is read from it into `to`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Feedback {
    pub ring: u32,
    pub from: Place,
    pub from_channels: u8,
    pub to: Target,
}

/// A range of a plan's lists, as `start..end`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

/// One node's run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Step {
    pub slot: u32,
    pub generation: u32,
    /// The node buffer it runs in: its own, or its host's for an inserted processor.
    pub buffer: u32,
    pub channels: u8,
    /// True for a node that doesn't work in place: it reads its buffer and writes its first output
    /// into scratch.
    pub separate: bool,
    /// The scratch strides its outputs other than the buffer occupy, one after another.
    pub outputs: Span,
    /// Pushes tapped before the node's own processing, and after it.
    pub pre: Span,
    pub post: Span,
    /// Feedback connections leaving it.
    pub feedback: Span,
    pub key: Option<Pull>,
    /// Removed, ringing out for at most this many frames before it goes.
    pub drain: Option<u64>,
    /// A mixer: its output is metered.
    pub metered: bool,
}

/// A node buffer cleared at the start of every block, with the node's other audio inputs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Clear {
    pub slot: u32,
    pub generation: u32,
    pub channels: u8,
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct Plan {
    pub steps: Box<[Step]>,
    pub pushes: Box<[Push]>,
    pub feedback: Box<[Feedback]>,
    pub clear: Box<[Clear]>,
    /// Compensation lines on node inputs' own entries, delaying what voices added there: each the
    /// line and the input it delays.
    pub entries: Box<[(u32, Target, u8)]>,
    pub delays: Delays,
    pub rings: Rings,
    /// Strides of scratch for outputs that aren't a node's buffer, shared by outputs that are
    /// never needed at once.
    pub scratch: Box<[f32]>,
}

/// A revision of the graph as the engine takes it: the plan, the processors of the nodes it made,
/// and the nodes it removed that have nothing to ring out.
pub struct Revision {
    pub plan: Box<Plan>,
    pub added: Vec<(NodeKey, Box<crate::audio::engine::Node>)>,
    pub retire: Box<[NodeKey]>,
}

impl std::fmt::Debug for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Revision")
            .field("steps", &self.plan.steps.len())
            .field("added", &self.added.len())
            .field("retire", &self.retire)
            .finish()
    }
}
