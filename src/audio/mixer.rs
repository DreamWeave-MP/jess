//! Mixers, the nodes buses are: they sum what reaches them, apply a gain and a mute, and are
//! metered on the way out.
//!
//! Every context has one in slot 0, the master, whose output is the context's. Any other bus is a
//! mixer with a route, a connection from its output to another bus, and whatever sends it has.

use crate::audio::processor::{Io, ParamInfo, Ports, Processor, scale};

/// A node as the application names it: its slot in the engine's node table and the slot's
/// generation when the node was made, so a handle outliving its node names nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeKey {
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

impl NodeKey {
    /// The master bus: the context's output. It has no route and is never removed.
    pub const MASTER: NodeKey = NodeKey { slot: 0, generation: 0 };
}

/// How to make a bus.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BusSpec {
    /// The node it is routed into.
    pub parent: NodeKey,
    /// Linear gain.
    pub gain: f32,
    /// Silent while true.
    pub muted: bool,
    /// Its channels, if fewer than the context's.
    pub channels: Option<usize>,
}

impl Default for BusSpec {
    fn default() -> Self {
        BusSpec {
            parent: NodeKey::MASTER,
            gain: 1.0,
            muted: false,
            channels: None,
        }
    }
}

/// Where a connection from a node taps it: after the node's own processing (a bus's gain and
/// mute, the usual), or before it, so a bus's send keeps its level when the bus is faded or muted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tap {
    /// After the node's own processing.
    #[default]
    PostFader,
    /// Before it, after its insert chain.
    PreFader,
}

/// A bus's processing: a gain, sample by sample while it moves, and a mute.
#[derive(Debug)]
pub struct Mixer {
    gain: f32,
    muted: bool,
}

impl Mixer {
    const PARAMS: &'static [ParamInfo] = &[
        ParamInfo::audio("gain", -64.0, 64.0, 1.0, ""),
        ParamInfo::control("mute", 0.0, 1.0, 0.0, ""),
    ];

    /// A mixer at `gain`, muted or not.
    #[must_use]
    pub const fn new(gain: f32, muted: bool) -> Mixer {
        Mixer { gain, muted }
    }
}

impl Processor for Mixer {
    fn kind(&self) -> &'static str {
        "mixer"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Mixer::PARAMS
    }

    fn ports(&self) -> &'static Ports {
        &Ports::EFFECT
    }

    fn mixer(&self) -> bool {
        true
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, index: usize, value: f64) {
        match index {
            0 => self.gain = value as f32,
            _ => self.muted = value != 0.0,
        }
    }

    fn value(&self, index: usize) -> f64 {
        match index {
            0 => f64::from(self.gain),
            _ => f64::from(u8::from(self.muted)),
        }
    }

    fn process(&mut self, io: &mut Io<'_>) {
        if self.muted {
            let channels = io.channels();
            let mut block = io.main();
            for channel in 0..channels {
                block.channel(channel).fill(0.0);
            }
            return;
        }
        scale(io, 0, self.gain);
    }
}
