//! Buses: where voices and other buses sum before they reach the output.
//!
//! Every context has a master bus, which is the output. Any other bus has a parent it sums into,
//! so buses make a tree under the master. A bus sums whatever is routed to it, applies its gain,
//! and adds the result into its parent.

use crate::audio::{AudioError, MAX_CHAIN, MAX_SENDS};

/// A bus as the application names it: its slot in the engine's table and the slot's generation
/// when it was made, so a handle outliving its bus names nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BusKey {
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

impl BusKey {
    /// The master bus: the context's output. It has no parent and is never removed.
    pub const MASTER: BusKey = BusKey { slot: 0, generation: 0 };
}

/// How to make a bus.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BusSpec {
    /// The bus it sums into.
    pub parent: BusKey,
    /// Linear gain.
    pub gain: f32,
    /// Silent while true.
    pub muted: bool,
}

impl Default for BusSpec {
    fn default() -> Self {
        BusSpec {
            parent: BusKey::MASTER,
            gain: 1.0,
            muted: false,
        }
    }
}

/// A bus parameter a command can change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BusParam {
    /// [`BusSpec::gain`].
    Gain,
    /// [`BusSpec::muted`]: `0` is unmuted, anything else muted.
    Mute,
}

impl BusParam {
    /// The name scripts use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            BusParam::Gain => "gain",
            BusParam::Mute => "mute",
        }
    }

    /// The parameter called `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<BusParam> {
        [BusParam::Gain, BusParam::Mute]
            .into_iter()
            .find(|param| param.name() == name)
    }

    /// `value`, if this parameter may take it.
    pub(crate) fn check(self, value: f64) -> Result<f64, AudioError> {
        if value.is_finite() {
            return Ok(value);
        }
        Err(AudioError::Parameter {
            name: self.name(),
            value,
            expected: "a finite number",
        })
    }
}

/// Where a send taps its bus: after the bus's gain and mute (the usual), or before them, so the
/// send keeps its level when the bus is faded or muted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tap {
    /// After the bus's own gain.
    #[default]
    PostFader,
    /// Before it.
    PreFader,
}

/// One send of a bus, as the engine holds it.
#[derive(Clone, Copy, Debug)]
pub struct BusSend {
    pub target: u32,
    pub level: f32,
    pub tap: Tap,
}

/// A bus as the engine holds it.
pub struct Bus {
    pub generation: u32,
    pub parent: Option<u32>,
    pub sends: [Option<BusSend>; MAX_SENDS],
    /// The processor slots it runs, in order; the first `length`.
    pub chain: [u32; MAX_CHAIN],
    pub length: usize,
    /// Removed, and ringing out: frames left before it goes.
    pub draining: Option<u64>,
    gain: f32,
    muted: bool,
}

impl Bus {
    pub const fn new(generation: u32, parent: Option<u32>, spec: &BusSpec) -> Bus {
        Bus {
            generation,
            parent,
            sends: [None; MAX_SENDS],
            chain: [0; MAX_CHAIN],
            length: 0,
            draining: None,
            gain: spec.gain,
            muted: spec.muted,
        }
    }

    /// The processor slots it runs, in order.
    pub fn chain(&self) -> &[u32] {
        &self.chain[..self.length]
    }

    /// Puts processor slot `processor` at `index` in the chain, or last; does nothing when the
    /// chain is full, which the control side never sends.
    pub fn mount(&mut self, processor: u32, index: usize) {
        if self.length == MAX_CHAIN {
            return;
        }
        let index = index.min(self.length);
        self.chain.copy_within(index..self.length, index + 1);
        self.chain[index] = processor;
        self.length += 1;
    }

    /// Takes processor slot `processor` out of the chain; true if it was there.
    pub fn unmount(&mut self, processor: u32) -> bool {
        let Some(index) = self.chain().iter().position(|known| *known == processor) else {
            return false;
        };
        self.chain.copy_within(index + 1..self.length, index);
        self.length -= 1;
        true
    }

    /// Sends to `target` at `level`, replacing a send already there.
    pub fn send(&mut self, send: BusSend) {
        let existing = self
            .sends
            .iter()
            .position(|known| known.is_some_and(|known| known.target == send.target));
        let free = existing.or_else(|| self.sends.iter().position(Option::is_none));
        if let Some(index) = free {
            self.sends[index] = Some(send);
        }
    }

    pub fn unsend(&mut self, target: u32) {
        for send in &mut self.sends {
            if send.is_some_and(|known| known.target == target) {
                *send = None;
            }
        }
    }

    #[allow(clippy::cast_possible_truncation)] // Checked finite on the control side.
    pub fn set(&mut self, param: BusParam, value: f64) {
        match param {
            BusParam::Gain => self.gain = value as f32,
            BusParam::Mute => self.muted = value != 0.0,
        }
    }

    /// The gain the bus applies to what it summed.
    pub const fn level(&self) -> f32 {
        if self.muted { 0.0 } else { self.gain }
    }
}
