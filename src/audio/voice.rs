//! Voices: one playback of one [`Pcm`], as the engine renders it.

use std::f32::consts::FRAC_PI_4;
use std::sync::Arc;

use crate::audio::{AudioError, BusKey, MAX_PLAYBACK_RATE, Pcm, QUANTUM};

/// One fractional frame in the 32.32 fixed point a voice's position is kept in. Positions are
/// integers so that a render is the same on every machine, however long it runs.
const ONE: u64 = 1 << 32;
const FRACTION: u64 = ONE - 1;

/// A voice as the application names it: its slot in the engine's pool and the slot's
/// generation when the voice was made, so a handle outliving its voice names nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct VoiceKey {
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

/// A loop region, in the source's frames: playback that reaches `end` continues at `start`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoopRegion {
    /// The first frame of the loop.
    pub start: u64,
    /// The frame after the loop's last.
    pub end: u64,
}

/// How to start a voice.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PlaySpec {
    /// The output frame to start on; `None` starts at the next frame the engine renders.
    pub at: Option<u64>,
    /// Linear gain.
    pub gain: f32,
    /// `-1.0` (left) to `1.0` (right). A mono source pans with equal power; a stereo one
    /// balances, attenuating the far side.
    pub pan: f32,
    /// Playback rate: `2.0` is twice as fast and an octave up. Pitch moves with it.
    pub rate: f64,
    /// The source frame to start from.
    pub offset: u64,
    /// Where to loop, if anywhere.
    pub looping: Option<LoopRegion>,
    /// The bus it plays into.
    pub bus: BusKey,
}

impl Default for PlaySpec {
    fn default() -> Self {
        PlaySpec {
            at: None,
            gain: 1.0,
            pan: 0.0,
            rate: 1.0,
            offset: 0,
            looping: None,
            bus: BusKey::MASTER,
        }
    }
}

/// A voice parameter a command can change while it plays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceParam {
    /// [`PlaySpec::gain`].
    Gain,
    /// [`PlaySpec::pan`].
    Pan,
    /// [`PlaySpec::rate`].
    Rate,
}

impl VoiceParam {
    /// The name scripts use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            VoiceParam::Gain => "gain",
            VoiceParam::Pan => "pan",
            VoiceParam::Rate => "rate",
        }
    }

    /// The parameter called `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<VoiceParam> {
        [VoiceParam::Gain, VoiceParam::Pan, VoiceParam::Rate]
            .into_iter()
            .find(|param| param.name() == name)
    }

    /// `value`, if this parameter may take it.
    pub(crate) fn check(self, value: f64) -> Result<f64, AudioError> {
        let (fits, expected) = match self {
            VoiceParam::Gain => (value.is_finite(), "a finite number"),
            VoiceParam::Pan => ((-1.0..=1.0).contains(&value), "from -1 to 1"),
            VoiceParam::Rate => (value > 0.0 && value <= MAX_PLAYBACK_RATE, "above 0 and at most 64"),
        };
        if fits {
            return Ok(value);
        }
        Err(AudioError::Parameter {
            name: self.name(),
            value,
            expected,
        })
    }
}

impl PlaySpec {
    /// Checks the spec against `pcm` and an output of `channels`.
    pub(crate) fn check(&self, pcm: &Pcm, channels: usize) -> Result<(), AudioError> {
        VoiceParam::Gain.check(f64::from(self.gain))?;
        VoiceParam::Pan.check(f64::from(self.pan))?;
        VoiceParam::Rate.check(self.rate)?;
        let frames = pcm.frames() as u64;
        if self.offset > frames {
            return Err(AudioError::Parameter {
                name: "offset",
                value: to_f64(self.offset),
                expected: "inside the source",
            });
        }
        if let Some(region) = self.looping
            && (region.start >= region.end || region.end > frames)
        {
            return Err(AudioError::LoopRegion {
                start: region.start,
                end: region.end,
                frames,
            });
        }
        mixes(pcm.channels(), channels)
    }
}

/// Whether a source of `source` channels mixes into an output of `output`.
pub const fn mixes(source: usize, output: usize) -> Result<(), AudioError> {
    if source == output || source == 1 || (source == 2 && output == 1) {
        return Ok(());
    }
    Err(AudioError::Layout { source, output })
}

#[allow(clippy::cast_precision_loss)] // Frame counts in messages.
const fn to_f64(value: u64) -> f64 {
    value as f64
}

/// Where a voice is in its source, in the 32.32 fixed point `ONE` describes, and how it moves.
#[derive(Clone, Copy)]
struct Cursor {
    position: u64,
    step: u64,
    looping: Option<(u64, u64)>,
    /// True once playback has wrapped from the loop's end to its start, after which the frames
    /// before the start are the loop's end for interpolation.
    wrapped: bool,
}

impl Cursor {
    fn advance(&mut self) {
        self.position += self.step;
        let Some((start, end)) = self.looping else {
            return;
        };
        while self.position >> 32 >= end {
            self.position -= (end - start) << 32;
            self.wrapped = true;
        }
    }

    /// The sample under the cursor: on a whole frame the frame itself, so a voice at its source's
    /// rate copies exactly, and between frames Catmull-Rom through the four around it.
    #[allow(clippy::cast_precision_loss)]
    // A 32-bit fraction in an f32 loses only the far bits.
    // `mul_add` is a libm call on a CPU built for without FMA, many times slower than this.
    #[allow(clippy::suboptimal_flops)]
    fn read(&self, data: &[f32]) -> f32 {
        let index = to_signed(self.position >> 32);
        let fraction = self.position & FRACTION;
        if fraction == 0 {
            return self.sample(data, index);
        }
        let t = fraction as f32 / 4_294_967_296.0;
        let (before, at, after, beyond) = (
            self.sample(data, index - 1),
            self.sample(data, index),
            self.sample(data, index + 1),
            self.sample(data, index + 2),
        );
        let slope = 0.5 * (after - before);
        let curve = before - 2.5 * at + 2.0 * after - 0.5 * beyond;
        let cubic = 0.5 * (beyond - before) + 1.5 * (at - after);
        ((cubic * t + curve) * t + slope) * t + at
    }

    /// The sample at frame `index`, through the loop: past its end is its start again, and
    /// before its start, once it has wrapped, is its end. Outside the source is silence.
    fn sample(&self, data: &[f32], index: i64) -> f32 {
        let index = match self.looping {
            Some((start, end)) => {
                let (start, end) = (to_signed(start), to_signed(end));
                if index >= end {
                    start + (index - end) % (end - start)
                } else if self.wrapped && index < start {
                    end - (start - index)
                } else {
                    index
                }
            }
            None => index,
        };
        usize::try_from(index)
            .ok()
            .and_then(|index| data.get(index))
            .copied()
            .unwrap_or(0.0)
    }
}

/// A playing voice, owned by the engine.
pub struct Voice {
    pub(crate) generation: u32,
    pub(crate) pcm: Arc<Pcm>,
    /// The slot of the bus it plays into.
    pub(crate) bus: u32,
    cursor: Cursor,
    gain: f32,
    pan: f32,
    /// The output's rate, for working out the step when the rate changes.
    output_rate: u32,
}

impl Voice {
    pub(crate) fn new(generation: u32, pcm: Arc<Pcm>, spec: &PlaySpec, output_rate: u32) -> Voice {
        let mut voice = Voice {
            generation,
            bus: spec.bus.slot,
            cursor: Cursor {
                position: spec.offset << 32,
                step: 0,
                looping: spec.looping.map(|region| (region.start, region.end)),
                wrapped: false,
            },
            gain: spec.gain,
            pan: spec.pan,
            output_rate,
            pcm,
        };
        voice.set_rate(spec.rate);
        voice
    }

    #[allow(clippy::cast_possible_truncation)] // Parameters are checked before they get here.
    pub(crate) fn set(&mut self, param: VoiceParam, value: f64) {
        match param {
            VoiceParam::Gain => self.gain = value as f32,
            VoiceParam::Pan => self.pan = value as f32,
            VoiceParam::Rate => self.set_rate(value),
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // A positive step below 2^64.
    fn set_rate(&mut self, rate: f64) {
        let ratio = rate * f64::from(self.pcm.rate()) / f64::from(self.output_rate);
        self.cursor.step = ((ratio * 4_294_967_296.0).round() as u64).max(1);
    }

    /// Renders up to `frames` frames of each source channel into `scratch`, channel `c` at
    /// `c * QUANTUM`. Returns how many frames it rendered: fewer than `frames` when the source
    /// ran out, and the voice has ended.
    pub(crate) fn render(&mut self, scratch: &mut [f32], frames: usize) -> usize {
        let length = self.pcm.frames() as u64;
        let mut produced = 0;
        let mut cursor = self.cursor;
        for channel in 0..self.pcm.channels() {
            cursor = self.cursor;
            let data = self.pcm.channel(channel);
            produced = 0;
            for sample in &mut scratch[channel * QUANTUM..channel * QUANTUM + frames] {
                if cursor.looping.is_none() && cursor.position >> 32 >= length {
                    break;
                }
                *sample = cursor.read(data);
                produced += 1;
                cursor.advance();
            }
        }
        self.cursor = cursor;
        produced
    }

    /// Adds `frames` of the rendered source channels in `scratch` into `bus`, `output` channels
    /// of `QUANTUM` each, at `offset` frames into each.
    pub(crate) fn mix(&self, scratch: &[f32], frames: usize, bus: &mut [f32], output: usize) {
        let source = self.pcm.channels();
        let channel = |index: usize| &scratch[index * QUANTUM..index * QUANTUM + frames];
        let mut add = |to: usize, from: &[f32], gain: f32| {
            for (out, sample) in bus[to * QUANTUM..to * QUANTUM + frames].iter_mut().zip(from) {
                *out += sample * gain;
            }
        };
        match (source, output) {
            (1, 2) => {
                let angle = (self.pan + 1.0) * FRAC_PI_4;
                add(0, channel(0), self.gain * angle.cos());
                add(1, channel(0), self.gain * angle.sin());
            }
            (2, 2) => {
                add(0, channel(0), self.gain * (1.0 - self.pan.max(0.0)));
                add(1, channel(1), self.gain * (1.0 + self.pan.min(0.0)));
            }
            (2, 1) => {
                add(0, channel(0), self.gain * 0.5);
                add(0, channel(1), self.gain * 0.5);
            }
            (1, _) => {
                for to in 0..output {
                    add(to, channel(0), self.gain);
                }
            }
            _ => {
                for to in 0..output {
                    add(to, channel(to), self.gain);
                }
            }
        }
    }
}

#[allow(clippy::cast_possible_wrap)] // Frame indices are far below 2^63.
const fn to_signed(value: u64) -> i64 {
    value as i64
}
