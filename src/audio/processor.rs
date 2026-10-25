//! Processors: signal processing a bus runs over what it summed, in a chain.
//!
//! A processor is made on the control side, with everything it will ever need allocated, and
//! moved into the engine when it is put on a bus; it comes back the same way to be dropped. On
//! the render thread it only processes blocks in place and takes parameter values. It declares
//! its parameters, how far its output lags its input, and how long it keeps sounding once its
//! input stops, so the graph can report latency and a removed bus can ring out.
//!
//! There is no public plugin interface: the built-in processors are the trait's implementations.

use crate::audio::dsp::delay::Delay;
use crate::audio::dsp::dynamics::Compressor;
use crate::audio::dsp::filter::{Filter, Shape};
use crate::audio::dsp::pan::Pan;
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

    /// Parameter `index`'s value: where a ramp of it starts.
    fn value(&self, index: usize) -> f64;

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

/// A setting a processor is made with: a number, or a name for a choice (a filter's shape).
#[derive(Clone, PartialEq, Debug)]
pub enum Setting {
    /// A number.
    Number(f64),
    /// A choice, by name.
    Text(String),
}

/// The settings a processor is made with, by name. A kind takes the ones fixed at creation
/// (a delay's longest time); every number left over sets the parameter of that name.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Settings {
    entries: Vec<(String, Setting)>,
}

impl Settings {
    /// No settings.
    #[must_use]
    pub const fn new() -> Settings {
        Settings { entries: Vec::new() }
    }

    /// Adds a setting.
    pub fn insert(&mut self, name: &str, setting: Setting) {
        self.entries.push((name.to_owned(), setting));
    }

    /// Takes the number called `name`, if there is one.
    ///
    /// # Errors
    ///
    /// [`AudioError::Setting`] when it's a name rather than a number.
    pub fn number(&mut self, kind: &'static str, name: &str) -> Result<Option<f64>, AudioError> {
        match self.take(name) {
            None => Ok(None),
            Some(Setting::Number(value)) => Ok(Some(value)),
            Some(Setting::Text(_)) => Err(AudioError::Setting {
                kind,
                name: name.to_owned(),
                expected: "a number",
            }),
        }
    }

    /// Takes the name called `name`, if there is one.
    ///
    /// # Errors
    ///
    /// [`AudioError::Setting`] when it's a number rather than a name.
    pub fn text(&mut self, kind: &'static str, name: &str) -> Result<Option<String>, AudioError> {
        match self.take(name) {
            None => Ok(None),
            Some(Setting::Text(value)) => Ok(Some(value)),
            Some(Setting::Number(_)) => Err(AudioError::Setting {
                kind,
                name: name.to_owned(),
                expected: "a name",
            }),
        }
    }

    fn take(&mut self, name: &str) -> Option<Setting> {
        let index = self.entries.iter().position(|(known, _)| known == name)?;
        Some(self.entries.remove(index).1)
    }

    /// Sets every setting left as the parameter of its name.
    ///
    /// # Errors
    ///
    /// [`AudioError::UnknownParameter`], [`AudioError::Parameter`] outside its range, or
    /// [`AudioError::Setting`] for a name where a number goes.
    pub fn apply(self, processor: &mut dyn Processor) -> Result<(), AudioError> {
        for (name, setting) in self.entries {
            let index = param_index(processor.kind(), processor.params(), &name)?;
            let Setting::Number(value) = setting else {
                return Err(AudioError::Setting {
                    kind: processor.kind(),
                    name,
                    expected: "a number",
                });
            };
            let value = check_param(&processor.params()[index], value)?;
            processor.set(index, value);
        }
        Ok(())
    }
}

/// The kinds of processor [`create`] makes.
pub const KINDS: &[&str] = &["gain", "pan", "filter", "delay", "compressor"];

/// Makes a processor of `kind` for a context of `rate` and `channels`, with `settings`: the
/// ones the kind is made with, and the rest as parameters.
///
/// # Errors
///
/// [`AudioError::UnknownProcessor`] for a kind there isn't, and whatever its settings are wrong
/// with.
pub fn create(kind: &str, rate: u32, channels: usize, mut settings: Settings) -> Result<Box<dyn Processor>, AudioError> {
    let mut processor: Box<dyn Processor> = match kind {
        "gain" => Box::new(Gain::new()),
        "pan" => Box::new(Pan::default()),
        "filter" => {
            let shape = settings
                .text("filter", "shape")?
                .map_or(Ok(Shape::Lowpass), |name| name.parse())?;
            Box::new(Filter::new(shape, rate))
        }
        "delay" => {
            let longest = settings.number("delay", "longest")?.unwrap_or(2.0);
            if !(longest > 0.0 && longest <= Delay::LONGEST) {
                return Err(AudioError::Setting {
                    kind: "delay",
                    name: "longest".to_owned(),
                    expected: "a number of seconds above 0 and at most 10",
                });
            }
            Box::new(Delay::new(rate, channels, longest))
        }
        "compressor" => Box::new(Compressor::new(rate)),
        other => {
            return Err(AudioError::UnknownProcessor {
                kind: other.to_owned(),
                known: KINDS.join(", "),
            });
        }
    };
    settings.apply(processor.as_mut())?;
    Ok(processor)
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

    fn value(&self, _index: usize) -> f64 {
        f64::from(self.gain)
    }

    fn process(&mut self, block: &mut Block<'_>, _key: Option<&[f32]>) {
        for channel in 0..block.channels {
            for sample in block.channel(channel) {
                *sample *= self.gain;
            }
        }
    }
}
