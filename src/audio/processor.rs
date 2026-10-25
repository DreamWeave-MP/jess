//! Processors: signal processing a bus runs over what it summed, in a chain.
//!
//! A processor is made on the control side, with everything it will ever need allocated, and
//! moved into the engine when it is put on a bus; it comes back the same way to be dropped. On
//! the render thread it only processes blocks in place and takes parameter values. It declares
//! its parameters, how far its output lags its input, and how long it keeps sounding once its
//! input stops, so the graph can report latency and a removed bus can ring out.
//!
//! There is no public plugin interface: the built-in processors are the trait's implementations.

use crate::audio::{AudioError, QUANTUM};

/// A processor as the application names it: its slot in the engine's table and the slot's
/// generation when it was made.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ProcessorKey {
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

/// One of a processor's parameters: its name, range, and default.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ParamInfo {
    /// The name scripts use.
    pub name: &'static str,
    /// The lowest value it takes.
    pub min: f64,
    /// The highest value it takes.
    pub max: f64,
    /// What it starts at.
    pub default: f64,
}

/// How long a processor keeps sounding after its input goes silent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tail {
    /// Not at all: silence in, silence out.
    None,
    /// For this many frames at most.
    Frames(u64),
}

/// A block of a bus: `channels` channels of [`QUANTUM`] samples each, channel after channel, of
/// which the first `frames` of each are this block's.
#[derive(Debug)]
pub struct Block<'a> {
    /// The samples.
    pub samples: &'a mut [f32],
    /// The channel count.
    pub channels: usize,
    /// The frames in this block.
    pub frames: usize,
}

impl Block<'_> {
    /// Channel `channel`'s samples in this block.
    pub fn channel(&mut self, channel: usize) -> &mut [f32] {
        &mut self.samples[channel * QUANTUM..channel * QUANTUM + self.frames]
    }
}

/// Signal processing a bus runs in its chain. See the module docs.
pub trait Processor: Send {
    /// What it is, as scripts name it.
    fn kind(&self) -> &'static str;

    /// Its parameters, by index.
    fn params(&self) -> &'static [ParamInfo];

    /// Sets parameter `index` to `value`, which is inside the parameter's range. It applies from
    /// the next sample processed.
    fn set(&mut self, index: usize, value: f64);

    /// How many frames its output lags its input.
    fn latency(&self) -> u64 {
        0
    }

    /// How long it keeps sounding after its input stops.
    fn tail(&self) -> Tail {
        Tail::None
    }

    /// Processes `block` in place. `key` is the block of the bus it listens to, if it has one.
    fn process(&mut self, block: &mut Block<'_>, key: Option<&[f32]>);
}

/// The index of the parameter called `name` among a `kind` processor's `params`.
///
/// # Errors
///
/// [`AudioError::UnknownParameter`] naming the parameters it does have.
pub fn param_index(kind: &'static str, params: &[ParamInfo], name: &str) -> Result<usize, AudioError> {
    params
        .iter()
        .position(|param| param.name == name)
        .ok_or_else(|| AudioError::UnknownParameter {
            kind,
            name: name.to_owned(),
            known: params.iter().map(|param| param.name).collect::<Vec<_>>().join(", "),
        })
}

/// `value`, if `param` takes it.
///
/// # Errors
///
/// [`AudioError::Parameter`] outside the parameter's range or not finite.
pub fn check_param(param: &ParamInfo, value: f64) -> Result<f64, AudioError> {
    if value.is_finite() && value >= param.min && value <= param.max {
        return Ok(value);
    }
    Err(AudioError::Parameter {
        name: param.name,
        value,
        expected: "inside its range",
    })
}

/// Multiplies a bus by a gain: a trim, or a fader that comes before others in the chain.
#[derive(Debug)]
pub struct Gain {
    gain: f32,
}

impl Gain {
    const PARAMS: &'static [ParamInfo] = &[ParamInfo {
        name: "gain",
        min: -64.0,
        max: 64.0,
        default: 1.0,
    }];

    /// A gain of one.
    #[must_use]
    pub const fn new() -> Gain {
        Gain { gain: 1.0 }
    }
}

impl Default for Gain {
    fn default() -> Self {
        Gain::new()
    }
}

impl Processor for Gain {
    fn kind(&self) -> &'static str {
        "gain"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Gain::PARAMS
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, _index: usize, value: f64) {
        self.gain = value as f32;
    }

    fn process(&mut self, block: &mut Block<'_>, _key: Option<&[f32]>) {
        for channel in 0..block.channels {
            for sample in block.channel(channel) {
                *sample *= self.gain;
            }
        }
    }
}
