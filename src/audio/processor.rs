//! Processors: what a node of the graph runs.
//!
//! A processor is made on the control side, with everything it will ever need allocated, and
//! moved into the engine's node table when its node is made; it comes back the same way to be
//! dropped. On the render thread it only processes blocks and takes parameter values. It declares
//! its parameters, its ports, whether it works in place, how far its output lags its input, and
//! how long it keeps sounding once its input stops, so the graph can check connections, line up
//! paths of different latency, and let a removed node ring out.
//!
//! The trait is public: a host adds its own kinds to a [`Registry`] and scripts make them by name
//! like the built-in ones. Nothing about a processor is dynamic linking: it is Rust compiled into
//! the host, so it works the same where a platform can't load libraries.

use std::fmt;
use std::sync::Arc;

use crate::audio::dsp::crossfade::Crossfade;
use crate::audio::dsp::delay::Delay;
use crate::audio::dsp::drive::Drive;
use crate::audio::dsp::dynamics::{Compressor, Limiter};
use crate::audio::dsp::filter::{Filter, Shape};
use crate::audio::dsp::generators::{Envelope, Oscillator, Wave};
use crate::audio::dsp::pan::Pan;
use crate::audio::dsp::reverb::Reverb;
use crate::audio::mixer::Mixer;
use crate::audio::{AudioError, QUANTUM};

/// The most inputs, sidechains included, or outputs a processor has.
pub const MAX_PORTS: usize = 8;

/// The most parameters a processor has.
pub const MAX_PARAMS: usize = 64;

/// How often a parameter's value can change inside a block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rate {
    /// Once a control block, of at most [`CONTROL_QUANTUM`](crate::audio::CONTROL_QUANTUM)
    /// frames while it moves: the processor hears it through [`Processor::set`].
    Control,
    /// Every sample: while it moves, the processor reads its values from [`Io::param`], and
    /// otherwise holds the value [`Processor::set`] gave it.
    Audio,
}

impl Rate {
    /// The name scripts use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Rate::Control => "control",
            Rate::Audio => "audio",
        }
    }
}

/// One of a processor's parameters: its name, range, default, unit, and rate.
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
    /// What it counts, for people: `"Hz"`, `"dB"`, `"s"`, or `""` for a plain number.
    pub unit: &'static str,
    /// How often it can change inside a block.
    pub rate: Rate,
}

impl ParamInfo {
    /// A control-rate parameter.
    #[must_use]
    pub const fn control(name: &'static str, min: f64, max: f64, default: f64, unit: &'static str) -> ParamInfo {
        ParamInfo {
            name,
            min,
            max,
            default,
            unit,
            rate: Rate::Control,
        }
    }

    /// An audio-rate parameter.
    #[must_use]
    pub const fn audio(name: &'static str, min: f64, max: f64, default: f64, unit: &'static str) -> ParamInfo {
        ParamInfo {
            name,
            min,
            max,
            default,
            unit,
            rate: Rate::Audio,
        }
    }
}

/// How long a processor keeps sounding after its input goes silent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tail {
    /// Not at all: silence in, silence out.
    None,
    /// For this many frames at most.
    Frames(u64),
}

impl Tail {
    /// Its length in frames.
    #[must_use]
    pub const fn frames(self) -> u64 {
        match self {
            Tail::None => 0,
            Tail::Frames(frames) => frames,
        }
    }
}

/// What an input port carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortKind {
    /// Signal the processor processes: everything connected to it sums.
    Audio,
    /// Signal it listens to and doesn't pass on, as a compressor follows a key: one connection.
    Sidechain,
}

/// One input port.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Port {
    /// The name connections use.
    pub name: &'static str,
    /// What it carries.
    pub kind: PortKind,
}

/// A processor's inputs and outputs, and whether it works in place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ports {
    /// Its inputs, audio and sidechain, in order.
    pub inputs: &'static [Port],
    /// Its outputs' names, in order.
    pub outputs: &'static [&'static str],
    /// True when it turns its first audio input into its first output in the same buffer. One
    /// that doesn't gets its first input to read and a separate first output to write.
    pub in_place: bool,
}

const IN: Port = Port {
    name: "in",
    kind: PortKind::Audio,
};

const KEY: Port = Port {
    name: "key",
    kind: PortKind::Sidechain,
};

impl Ports {
    /// An effect: one input, `in`, processed in place into one output, `out`.
    pub const EFFECT: Ports = Ports {
        inputs: &[IN],
        outputs: &["out"],
        in_place: true,
    };

    /// An effect that can listen to a sidechain, `key`.
    pub const KEYED: Ports = Ports {
        inputs: &[IN, KEY],
        outputs: &["out"],
        in_place: true,
    };

    /// A source: no inputs, one output, `out`, which it writes over the silence its buffer holds.
    pub const SOURCE: Ports = Ports {
        inputs: &[],
        outputs: &["out"],
        in_place: true,
    };

    /// The audio inputs, in order, sidechains left out: what [`Io::input`] counts.
    pub fn audio(&self) -> impl Iterator<Item = &'static Port> {
        self.inputs.iter().filter(|port| port.kind == PortKind::Audio)
    }

    /// The sidechain input, if there is one.
    #[must_use]
    pub fn sidechain(&self) -> Option<&'static Port> {
        self.inputs.iter().find(|port| port.kind == PortKind::Sidechain)
    }

    /// True for the one-input, one-output shape an insert chain can hold.
    #[must_use]
    pub fn insertable(&self) -> bool {
        self.in_place && self.audio().count() == 1 && self.outputs.len() == 1
    }
}

/// A block of one buffer: `channels` channels of [`QUANTUM`] samples each, channel after channel,
/// of which the first `frames` of each are this block's.
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

/// What a processor reads and writes in one block. Every buffer is planar: `channels` channels of
/// [`QUANTUM`] samples, channel after channel, of which the first `frames` are this block's.
pub struct Io<'a> {
    frames: usize,
    channels: usize,
    main: &'a mut [f32],
    input: Option<&'a [f32]>,
    inputs: &'a [f32],
    outputs: &'a mut [f32],
    key: Option<&'a [f32]>,
    params: &'a [f32],
    varying: u64,
}

impl fmt::Debug for Io<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Io")
            .field("frames", &self.frames)
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

impl<'a> Io<'a> {
    /// The I/O of an effect running in place on `main`, with nothing else: for testing a
    /// processor, or running one outside a context.
    #[must_use]
    pub fn new(main: &'a mut [f32], channels: usize, frames: usize) -> Io<'a> {
        Io {
            frames,
            channels,
            main,
            input: None,
            inputs: &[],
            outputs: &mut [],
            key: None,
            params: &[],
            varying: 0,
        }
    }

    /// The same, hearing `key` on its sidechain.
    #[must_use]
    pub const fn with_key(mut self, key: Option<&'a [f32]>) -> Io<'a> {
        self.key = key;
        self
    }

    /// The same, with its other ports: `input`, the first audio input of a processor that
    /// doesn't work in place; `inputs`, the audio inputs after the first, one buffer after
    /// another; and `outputs`, the outputs after the first.
    #[must_use]
    pub const fn with_ports(mut self, input: Option<&'a [f32]>, inputs: &'a [f32], outputs: &'a mut [f32]) -> Io<'a> {
        self.input = input;
        self.inputs = inputs;
        self.outputs = outputs;
        self
    }

    /// The same, with per-sample values for the parameters whose bits are set in `varying`:
    /// parameter `index` at `params[index * QUANTUM..]`.
    #[must_use]
    pub const fn with_params(mut self, params: &'a [f32], varying: u64) -> Io<'a> {
        self.params = params;
        self.varying = varying;
        self
    }

    /// The frames in this block.
    #[must_use]
    pub const fn frames(&self) -> usize {
        self.frames
    }

    /// The channels of every buffer.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// The main buffer: for a processor that works in place, its first audio input summed, which
    /// it turns into its first output; for a source, silence to write over; for one that doesn't
    /// work in place, its first output, silent, to write.
    pub fn main(&mut self) -> Block<'_> {
        Block {
            samples: self.main,
            channels: self.channels,
            frames: self.frames,
        }
    }

    /// The main buffer and the sidechain together, for an effect.
    pub fn effect(&mut self) -> (Block<'_>, Option<&[f32]>) {
        (
            Block {
                samples: self.main,
                channels: self.channels,
                frames: self.frames,
            },
            self.key,
        )
    }

    /// Audio input `index` (from 0, sidechains not counted), summed. A processor that works in
    /// place has its first input in [`Io::main`], and this is `None` for it.
    #[must_use]
    pub fn input(&self, index: usize) -> Option<&[f32]> {
        let stride = self.channels * QUANTUM;
        if index == 0 {
            return self.input;
        }
        self.inputs.get((index - 1) * stride..index * stride)
    }

    /// The main buffer and audio input `index` together: what a processor that works in place
    /// and has more than one input reads and writes.
    pub fn main_and_input(&mut self, index: usize) -> (Block<'_>, Option<&[f32]>) {
        let stride = self.channels * QUANTUM;
        let input = match index {
            0 => self.input,
            _ => self.inputs.get((index - 1) * stride..index * stride),
        };
        let block = Block {
            samples: self.main,
            channels: self.channels,
            frames: self.frames,
        };
        (block, input)
    }

    /// Output `index` after the first (which is [`Io::main`]), silent, to write.
    pub fn output(&mut self, index: usize) -> Option<Block<'_>> {
        let stride = self.channels * QUANTUM;
        let index = index.checked_sub(1)?;
        let samples = self.outputs.get_mut(index * stride..(index + 1) * stride)?;
        Some(Block {
            samples,
            channels: self.channels,
            frames: self.frames,
        })
    }

    /// The sidechain, if something is connected to it.
    #[must_use]
    pub const fn key(&self) -> Option<&[f32]> {
        self.key
    }

    /// The main buffer and audio-rate parameter `index`'s values together, as [`Io::param`] has
    /// them.
    pub fn main_and_param(&mut self, index: usize) -> (Block<'_>, Option<&[f32]>) {
        let values = (index < MAX_PARAMS && self.varying & (1 << index) != 0)
            .then(|| self.params.get(index * QUANTUM..index * QUANTUM + self.frames))
            .flatten();
        let block = Block {
            samples: self.main,
            channels: self.channels,
            frames: self.frames,
        };
        (block, values)
    }

    /// Audio-rate parameter `index`'s value at each frame of this block while it moves (ramped or
    /// modulated); `None` while it holds the value [`Processor::set`] gave it.
    #[must_use]
    pub fn param(&self, index: usize) -> Option<&[f32]> {
        if index >= MAX_PARAMS || self.varying & (1 << index) == 0 {
            return None;
        }
        self.params.get(index * QUANTUM..index * QUANTUM + self.frames)
    }
}

/// What a node runs. See the module docs.
pub trait Processor: Send {
    /// What it is, as scripts name it.
    fn kind(&self) -> &'static str;

    /// Its parameters, by index.
    fn params(&self) -> &'static [ParamInfo];

    /// Its inputs and outputs.
    fn ports(&self) -> &'static Ports {
        &Ports::EFFECT
    }

    /// True for a mixer: its output is metered, and it is a bus.
    fn mixer(&self) -> bool {
        false
    }

    /// Sets parameter `index` to `value`, which is inside the parameter's range. It applies from
    /// the next sample processed.
    fn set(&mut self, index: usize, value: f64);

    /// Parameter `index`'s value: where automation of it starts.
    fn value(&self, index: usize) -> f64;

    /// How many frames its output lags its input. Fixed for the life of the processor.
    fn latency(&self) -> u64 {
        0
    }

    /// How long it keeps sounding after its input stops.
    fn tail(&self) -> Tail {
        Tail::None
    }

    /// Processes one block. It never allocates, frees, locks, or blocks.
    fn process(&mut self, io: &mut Io<'_>);
}

/// Why `processor` can't be a node, if it can't.
///
/// # Errors
///
/// [`AudioError::Shape`] naming what is wrong: too many ports or parameters, more than one
/// sidechain, a default outside its range, no outputs, or no input to read for a processor that
/// doesn't work in place.
pub fn check_shape(processor: &dyn Processor) -> Result<(), AudioError> {
    let kind = processor.kind();
    let ports = processor.ports();
    let problem = if ports.inputs.len() > MAX_PORTS || ports.outputs.len() > MAX_PORTS {
        Some("more than 8 inputs or outputs")
    } else if ports.inputs.iter().filter(|port| port.kind == PortKind::Sidechain).count() > 1 {
        Some("more than one sidechain")
    } else if processor.params().len() > MAX_PARAMS {
        Some("more than 64 parameters")
    } else if processor
        .params()
        .iter()
        .any(|param| !(param.min <= param.default && param.default <= param.max))
    {
        Some("a parameter whose default is outside its range")
    } else if ports.outputs.is_empty() {
        Some("no outputs")
    } else if !ports.in_place && ports.audio().next().is_none() {
        Some("no input to read, though it doesn't work in place")
    } else {
        None
    };
    problem.map_or(Ok(()), |problem| Err(AudioError::Shape { kind, problem }))
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

/// What a processor is made for: the context's rate and the node's channels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Setup {
    /// Frames per second.
    pub rate: u32,
    /// The node's channels.
    pub channels: usize,
}

/// Makes a processor of one kind, taking the settings it is made with from the [`Settings`]; the
/// registry sets what is left as parameters.
pub type Factory = dyn Fn(&Setup, &mut Settings) -> Result<Box<dyn Processor>, AudioError> + Send + Sync;

/// The kinds of processor scripts can make, by name: the built-in ones and whatever a host adds.
#[derive(Clone)]
pub struct Registry {
    kinds: Vec<(String, Arc<Factory>)>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.kinds()).finish()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Registry::builtin()
    }
}

impl Registry {
    /// The built-in kinds.
    #[must_use]
    pub fn builtin() -> Registry {
        let mut registry = Registry { kinds: Vec::new() };
        let builtin: [(&str, Box<Factory>); 12] = [
            ("mixer", Box::new(|_, _| Ok(Box::new(Mixer::new(1.0, false))))),
            ("gain", Box::new(|_, _| Ok(Box::new(Gain::new())))),
            ("pan", Box::new(|_, _| Ok(Box::new(Pan::default())))),
            (
                "filter",
                Box::new(|setup, settings| {
                    let shape = settings
                        .text("filter", "shape")?
                        .map_or(Ok(Shape::Lowpass), |name| name.parse())?;
                    Ok(Box::new(Filter::new(shape, setup.rate)))
                }),
            ),
            (
                "delay",
                Box::new(|setup, settings| {
                    let longest = settings.number("delay", "longest")?.unwrap_or(2.0);
                    if !(longest > 0.0 && longest <= Delay::LONGEST) {
                        return Err(AudioError::Setting {
                            kind: "delay",
                            name: "longest".to_owned(),
                            expected: "a number of seconds above 0 and at most 10",
                        });
                    }
                    Ok(Box::new(Delay::new(setup.rate, setup.channels, longest)))
                }),
            ),
            ("compressor", Box::new(|setup, _| Ok(Box::new(Compressor::new(setup.rate))))),
            (
                "limiter",
                Box::new(|setup, settings| {
                    let lookahead = settings.number("limiter", "lookahead")?.unwrap_or(0.005);
                    if !(0.0..=Limiter::LONGEST).contains(&lookahead) {
                        return Err(AudioError::Setting {
                            kind: "limiter",
                            name: "lookahead".to_owned(),
                            expected: "a number of seconds from 0 to 0.05",
                        });
                    }
                    Ok(Box::new(Limiter::new(setup.rate, setup.channels, lookahead)))
                }),
            ),
            ("reverb", Box::new(|setup, _| Ok(Box::new(Reverb::new(setup.rate))))),
            ("drive", Box::new(|_, _| Ok(Box::new(Drive::new())))),
            ("crossfade", Box::new(|_, _| Ok(Box::new(Crossfade::new())))),
            (
                "oscillator",
                Box::new(|setup, settings| {
                    let wave = settings
                        .text("oscillator", "wave")?
                        .map_or(Ok(Wave::Sine), |name| name.parse())?;
                    Ok(Box::new(Oscillator::new(wave, setup.rate)))
                }),
            ),
            ("envelope", Box::new(|setup, _| Ok(Box::new(Envelope::new(setup.rate))))),
        ];
        for (kind, factory) in builtin {
            registry.kinds.push((kind.to_owned(), Arc::from(factory)));
        }
        registry
    }

    /// Adds a kind, made by `factory`, whose processors' [`Processor::kind`] is `kind`.
    ///
    /// # Errors
    ///
    /// [`AudioError::KindTaken`] for a kind there already is.
    pub fn register(
        &mut self,
        kind: &str,
        factory: impl Fn(&Setup, &mut Settings) -> Result<Box<dyn Processor>, AudioError> + Send + Sync + 'static,
    ) -> Result<(), AudioError> {
        if self.kinds.iter().any(|(known, _)| known == kind) {
            return Err(AudioError::KindTaken(kind.to_owned()));
        }
        self.kinds.push((kind.to_owned(), Arc::new(factory)));
        Ok(())
    }

    /// The kinds it makes, in the order they were added.
    pub fn kinds(&self) -> impl ExactSizeIterator<Item = &str> {
        self.kinds.iter().map(|(kind, _)| kind.as_str())
    }

    /// Makes a processor of `kind` for `setup`, with `settings`: the ones the kind is made with,
    /// and the rest as parameters.
    ///
    /// # Errors
    ///
    /// [`AudioError::UnknownProcessor`] for a kind there isn't, [`AudioError::Shape`] for a
    /// processor whose kind or shape is wrong, and whatever its settings are wrong with.
    pub fn create(&self, kind: &str, setup: &Setup, mut settings: Settings) -> Result<Box<dyn Processor>, AudioError> {
        let Some((_, factory)) = self.kinds.iter().find(|(known, _)| known == kind) else {
            return Err(AudioError::UnknownProcessor {
                kind: kind.to_owned(),
                known: self.kinds().collect::<Vec<_>>().join(", "),
            });
        };
        let mut processor = factory(setup, &mut settings)?;
        if processor.kind() != kind {
            return Err(AudioError::Shape {
                kind: processor.kind(),
                problem: "a kind other than the one it was registered as",
            });
        }
        check_shape(processor.as_ref())?;
        settings.apply(processor.as_mut())?;
        Ok(processor)
    }
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

/// Multiplies its input by a gain, sample by sample while the gain moves: a trim, a fader, or an
/// amplifier an envelope modulates.
#[derive(Debug)]
pub struct Gain {
    gain: f32,
}

impl Gain {
    const PARAMS: &'static [ParamInfo] = &[ParamInfo::audio("gain", -64.0, 64.0, 1.0, "")];

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

    fn process(&mut self, io: &mut Io<'_>) {
        scale(io, 0, self.gain);
    }
}

/// Multiplies `io`'s main buffer by audio-rate parameter `param`: its values frame by frame while
/// it moves, `steady` while it holds.
pub fn scale(io: &mut Io<'_>, param: usize, steady: f32) {
    let (channels, frames) = (io.channels, io.frames);
    // The values are borrowed from the engine, not from `io`, so the main buffer stays free.
    let values = io.params;
    let gains = (param < MAX_PARAMS && io.varying & (1 << param) != 0)
        .then(|| values.get(param * QUANTUM..param * QUANTUM + frames))
        .flatten();
    for channel in 0..channels {
        let samples = &mut io.main[channel * QUANTUM..channel * QUANTUM + frames];
        let Some(gains) = gains else {
            for sample in samples {
                *sample *= steady;
            }
            continue;
        };
        for (sample, gain) in samples.iter_mut().zip(gains) {
            *sample *= gain;
        }
    }
}
