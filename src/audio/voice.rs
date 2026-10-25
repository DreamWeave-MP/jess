//! Voices: one playback of one [`Pcm`], as the engine renders it.

// Interpolation and mixing are sums of products throughout. `mul_add` is a libm call on a CPU
// built for without FMA, many times slower than the plain expression.
#![allow(clippy::suboptimal_flops)]

use std::f32::consts::FRAC_PI_4;
use std::sync::Arc;

use crate::audio::param::{Curve, Glide, Param};
use crate::audio::spatial::{Emitter, Space};
use crate::audio::stream::Stream;
use crate::audio::{AudioError, BusKey, MAX_CHANNELS, MAX_PLAYBACK_RATE, MAX_VOICE_SENDS, Pcm, QUANTUM};

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
    /// Buses it also sends to, each at a level relative to what it plays into `bus`: a shared
    /// reverb, say. Fixed when the voice starts; the levels can change.
    pub sends: [Option<VoiceSend>; MAX_VOICE_SENDS],
    /// Frames to fade in over, from silence to `gain`.
    pub fade: u64,
    /// Where it is, if it's in space: then distance, direction, and motion set its level, pan,
    /// and pitch, and `pan` is not used.
    pub spatial: Option<Emitter>,
    /// How much it matters when there are more voices than the context mixes: higher keeps its
    /// place over lower, whatever their levels.
    pub priority: f32,
    /// What happens to it while it isn't mixed.
    pub culled: Culled,
}

/// What a voice does while the context has more voices than it mixes and it isn't among them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Culled {
    /// Keeps time, silently: when it is mixed again it is where it would have been. A looping
    /// machine's hum doesn't restart for having been out of earshot.
    #[default]
    Advance,
    /// Holds its place until it is mixed again.
    Pause,
    /// Ends: a one-shot that didn't make the cut is gone.
    Stop,
}

impl Culled {
    /// The name scripts use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Culled::Advance => "advance",
            Culled::Pause => "pause",
            Culled::Stop => "stop",
        }
    }

    /// The behaviour called `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Culled> {
        [Culled::Advance, Culled::Pause, Culled::Stop]
            .into_iter()
            .find(|culled| culled.name() == name)
    }
}

/// A voice's send: the bus, and the level relative to the voice's own gain.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct VoiceSend {
    /// The bus sent to.
    pub bus: BusKey,
    /// Linear level.
    pub level: f32,
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
            sends: [None; MAX_VOICE_SENDS],
            fade: 0,
            spatial: None,
            priority: 0.0,
            culled: Culled::Advance,
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
        self.check_levels()?;
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

    /// Checks the spec against `stream` and an output of `channels`: a stream plays once, from
    /// where its writer is, at up to four times its rate.
    pub(crate) fn check_stream(&self, stream: &Stream, channels: usize) -> Result<(), AudioError> {
        self.check_levels()?;
        if self.offset != 0 || self.looping.is_some() {
            return Err(AudioError::Parameter {
                name: "offset",
                value: to_f64(self.offset),
                expected: "0 and no loop for a stream, which plays once from where its writer is",
            });
        }
        if self.rate > 4.0 {
            return Err(AudioError::Parameter {
                name: "rate",
                value: self.rate,
                expected: "at most 4 for a stream",
            });
        }
        mixes(stream.channels(), channels)
    }

    /// Checks the gain, pan, rate, sends, and place.
    fn check_levels(&self) -> Result<(), AudioError> {
        VoiceParam::Gain.check(f64::from(self.gain))?;
        VoiceParam::Pan.check(f64::from(self.pan))?;
        VoiceParam::Rate.check(self.rate)?;
        for send in self.sends.iter().flatten() {
            check_level(f64::from(send.level))?;
        }
        if let Some(emitter) = &self.spatial {
            emitter.check()?;
        }
        Ok(())
    }
}

/// A send's level, if it may take it.
pub fn check_level(level: f64) -> Result<f64, AudioError> {
    if level.is_finite() {
        return Ok(level);
    }
    Err(AudioError::Parameter {
        name: "send level",
        value: level,
        expected: "a finite number",
    })
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
    /// Moves `frames` steps along at once, through the loop, without reading anything.
    fn skip(&mut self, frames: u64) {
        self.position += self.step * frames;
        let Some((start, end)) = self.looping else {
            return;
        };
        if self.position >> 32 >= end {
            let (loop_start, length) = (start << 32, (end - start) << 32);
            self.position = loop_start + (self.position - loop_start) % length;
            self.wrapped = true;
        }
    }

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

/// How loud each of a voice's two coefficients is over a block: one number each, or one per
/// frame in an envelope the engine lends (the first coefficient at 0, the second at `QUANTUM`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Levels {
    Steady(f32, f32),
    Varying,
}

/// One level to mix at: `scale`, times a frame's worth of an envelope while there is one.
#[derive(Clone, Copy)]
struct Level<'a> {
    envelope: Option<&'a [f32]>,
    scale: f32,
}

fn add(out: &mut [f32], from: &[f32], level: Level<'_>) {
    let Some(envelope) = level.envelope else {
        for (out, sample) in out.iter_mut().zip(from) {
            *out += sample * level.scale;
        }
        return;
    };
    for ((out, sample), gain) in out.iter_mut().zip(from).zip(envelope) {
        *out += sample * gain * level.scale;
    }
}

/// A voice's two coefficients at `gain` and `pan`, for a source of `source` channels into an
/// output of `output`: left and right for a stereo output, and the one gain otherwise.
fn coefficients(gain: f32, pan: f32, source: usize, output: usize) -> (f32, f32) {
    match (source, output) {
        (1, 2) => {
            let angle = (pan + 1.0) * FRAC_PI_4;
            (gain * angle.cos(), gain * angle.sin())
        }
        (2, 2) => (gain * (1.0 - pan.max(0.0)), gain * (1.0 + pan.min(0.0))),
        (2, 1) => (gain * 0.5, gain * 0.5),
        _ => (gain, gain),
    }
}

/// What space does to a voice, as of the block being mixed.
struct Spatial {
    emitter: Emitter,
    /// The two coefficients distance, cone, and pan give it at this block's start and end; the
    /// block moves from one to the other, so a moving voice glides rather than steps.
    from: (f32, f32),
    to: (f32, f32),
    /// True until the first block has been heard: it starts where it is, not from silence.
    fresh: bool,
    doppler: f64,
    /// The one-pole low-pass's coefficient (1 lets everything through) and memory per channel.
    lowpass: f32,
    memory: [f32; MAX_CHANNELS],
}

impl Spatial {
    const fn new(emitter: Emitter) -> Spatial {
        Spatial {
            emitter,
            from: (0.0, 0.0),
            to: (0.0, 0.0),
            fresh: true,
            doppler: 1.0,
            lowpass: 1.0,
            memory: [0.0; MAX_CHANNELS],
        }
    }
}

/// Frames a stream voice's window holds per channel: a block at the fastest a stream plays, and
/// the frames around it that interpolation reads.
pub const STREAM_WINDOW: usize = QUANTUM * 4 + 8;

/// What a voice plays.
pub enum Source {
    /// Decoded audio, shared.
    Pcm(Arc<Pcm>),
    /// A stream, and the window the voice reads it through: planar, [`STREAM_WINDOW`] frames a
    /// channel, allocated on the control side.
    Stream(Arc<Stream>, Box<[f32]>),
}

impl Source {
    pub(crate) fn channels(&self) -> usize {
        match self {
            Source::Pcm(pcm) => pcm.channels(),
            Source::Stream(stream, _) => stream.channels(),
        }
    }

    pub(crate) fn rate(&self) -> u32 {
        match self {
            Source::Pcm(pcm) => pcm.rate(),
            Source::Stream(stream, _) => stream.rate(),
        }
    }
}

/// Reads `frames` frames of each channel of `pcm` into `scratch` from `cursor`, moving it, with
/// `steps` frame by frame while the rate ramps; how many it read before the source ran out.
fn render_pcm(pcm: &Pcm, cursor: &mut Cursor, scratch: &mut [f32], steps: Option<&[u64]>, frames: usize) -> usize {
    let length = pcm.frames() as u64;
    let start = *cursor;
    let mut produced = 0;
    for channel in 0..pcm.channels() {
        *cursor = start;
        let data = pcm.channel(channel);
        produced = 0;
        for (index, sample) in scratch[channel * QUANTUM..channel * QUANTUM + frames].iter_mut().enumerate() {
            if cursor.looping.is_none() && cursor.position >> 32 >= length {
                break;
            }
            *sample = cursor.read(data);
            produced += 1;
            if let Some(steps) = steps {
                cursor.step = steps[index];
            }
            cursor.advance();
        }
    }
    produced
}

/// A stream voice's reading: the stream, and the window of it around the cursor.
struct Reader<'a> {
    stream: &'a Stream,
    window: &'a mut [f32],
    base: &'a mut u64,
    filled: &'a mut usize,
}

impl Reader<'_> {
    /// Reads `frames` frames as [`render_pcm`] does, from the window, which it first slides up to
    /// the cursor and fills from the stream. Where the stream has nothing yet the window holds
    /// silence, counted as missed; a finished stream ends where its data does.
    fn render(self, cursor: &mut Cursor, scratch: &mut [f32], steps: Option<&[u64]>, frames: usize) -> usize {
        let channels = self.stream.channels();
        // Let go of what the cursor has passed, but the frame interpolation still reads.
        let keep = (cursor.position >> 32).saturating_sub(1);
        if keep > *self.base {
            let passed = usize::try_from(keep - *self.base).unwrap_or(usize::MAX).min(*self.filled);
            for channel in 0..channels {
                let line = &mut self.window[channel * STREAM_WINDOW..(channel + 1) * STREAM_WINDOW];
                line.copy_within(passed..*self.filled, 0);
            }
            *self.base += passed as u64;
            *self.filled -= passed;
        }
        let travel = steps.map_or(cursor.step * frames as u64, |steps| steps.iter().sum());
        let last = ((cursor.position + travel) >> 32) + 3;
        let wanted = usize::try_from(last.saturating_sub(*self.base))
            .unwrap_or(usize::MAX)
            .min(STREAM_WINDOW);
        if wanted > *self.filled {
            *self.filled += self
                .stream
                .take(self.window, STREAM_WINDOW, *self.filled, wanted - *self.filled);
        }
        if *self.filled < wanted && !self.stream.finished() {
            for channel in 0..channels {
                self.window[channel * STREAM_WINDOW + *self.filled..channel * STREAM_WINDOW + wanted].fill(0.0);
            }
            self.stream.miss((wanted - *self.filled) as u64);
            *self.filled = wanted;
        }
        let finished = self.stream.finished();
        let origin = *self.base << 32;
        let start = Cursor {
            position: cursor.position - origin,
            ..*cursor
        };
        let mut relative = start;
        let mut produced = 0;
        for channel in 0..channels {
            relative = start;
            let data = &self.window[channel * STREAM_WINDOW..channel * STREAM_WINDOW + *self.filled];
            produced = 0;
            for (index, sample) in scratch[channel * QUANTUM..channel * QUANTUM + frames].iter_mut().enumerate() {
                // An unfinished stream has silence where it has nothing; a finished one ends.
                if finished && relative.position >> 32 >= data.len() as u64 {
                    break;
                }
                *sample = relative.read(data);
                produced += 1;
                if let Some(steps) = steps {
                    relative.step = steps[index];
                }
                relative.advance();
            }
        }
        cursor.position = relative.position + origin;
        cursor.step = relative.step;
        produced
    }
}

/// A playing voice, owned by the engine.
pub struct Voice {
    pub(crate) generation: u32,
    pub(crate) source: Source,
    /// For a stream, the absolute frame its window starts at, and the frames in it.
    base: u64,
    filled: usize,
    /// The slot of the bus it plays into.
    pub(crate) bus: u32,
    /// Bus slots it sends to, and at what level.
    pub(crate) sends: [Option<(u32, f32)>; MAX_VOICE_SENDS],
    cursor: Cursor,
    gain: Param,
    pan: Param,
    rate: Param,
    /// Frames left of the fade a stop began; the voice ends when it runs out.
    ending: Option<u64>,
    spatial: Option<Spatial>,
    pub(crate) priority: f32,
    pub(crate) culled: Culled,
    /// Mixed this block: the context mixes every voice, or this one made the cut.
    pub(crate) audible: bool,
    /// The output's rate, for working out the step when the rate changes.
    output_rate: u32,
}

impl Voice {
    pub(crate) fn new(generation: u32, source: Source, spec: &PlaySpec, output_rate: u32) -> Voice {
        let mut gain = Param::new(f64::from(spec.gain));
        if spec.fade > 0 {
            gain.set(0.0);
            gain.ramp(
                f64::from(spec.gain),
                Glide {
                    frames: spec.fade,
                    curve: Curve::Linear,
                },
            );
        }
        let mut voice = Voice {
            generation,
            bus: spec.bus.slot,
            sends: spec.sends.map(|send| send.map(|send| (send.bus.slot, send.level))),
            cursor: Cursor {
                position: spec.offset << 32,
                step: 0,
                looping: spec.looping.map(|region| (region.start, region.end)),
                wrapped: false,
            },
            gain,
            pan: Param::new(f64::from(spec.pan)),
            rate: Param::new(spec.rate),
            ending: None,
            spatial: spec.spatial.map(Spatial::new),
            priority: spec.priority,
            culled: spec.culled,
            audible: true,
            output_rate,
            source,
            base: 0,
            filled: 0,
        };
        voice.cursor.step = voice.step(spec.rate);
        voice
    }

    /// How loud it would be mixed: its gain times what space gives it, for ranking voices when
    /// there are more than the context mixes.
    #[allow(clippy::cast_possible_truncation)] // A level.
    pub(crate) fn loudness(&self) -> f32 {
        let gain = (self.gain.value() as f32).abs();
        self.spatial
            .as_ref()
            .map_or(gain, |spatial| gain * spatial.to.0.max(spatial.to.1))
    }

    /// Keeps time for `frames` frames without mixing anything: its ramps, its fade, and its place in
    /// its source all move on. False once it has run out.
    pub(crate) fn skip(&mut self, frames: usize) -> bool {
        // A stream has nothing to skip but what it has been given: culled, it holds its place.
        if matches!(self.source, Source::Stream(..)) {
            return true;
        }
        let frames = frames as u64;
        if let Some(left) = self.ending.as_mut() {
            if *left <= frames {
                return false;
            }
            *left -= frames;
        }
        self.gain.advance(frames);
        self.pan.advance(frames);
        if !self.rate.steady() {
            self.rate.advance(frames);
            self.cursor.step = self.step(self.rate.value());
        }
        self.cursor.skip(frames);
        match &self.source {
            Source::Pcm(pcm) => self.cursor.looping.is_some() || self.cursor.position >> 32 < pcm.frames() as u64,
            Source::Stream(..) => true,
        }
    }

    /// Moves a voice in space; a voice that wasn't in space stays out of it.
    pub(crate) fn place(&mut self, emitter: Emitter) {
        if let Some(spatial) = self.spatial.as_mut() {
            spatial.emitter = emitter;
        }
    }

    /// Works out what `space` does to the voice over the next block: where its level and pan go
    /// to, its Doppler shift, and its low-pass. Nothing for a voice not in space.
    #[allow(clippy::cast_possible_truncation)] // Engine coefficients are f32.
    pub(crate) fn hear(&mut self, space: &Space, output: usize) {
        let source = self.source.channels();
        let Some(spatial) = self.spatial.as_mut() else {
            return;
        };
        let heard = space.hear(&spatial.emitter);
        let target = coefficients(heard.gain as f32, heard.pan as f32, source, output);
        spatial.from = if spatial.fresh { target } else { spatial.to };
        spatial.to = target;
        spatial.fresh = false;
        spatial.doppler = heard.doppler;
        let nyquist = f64::from(self.output_rate) / 2.0;
        spatial.lowpass = if spatial.emitter.lowpass >= nyquist {
            1.0
        } else {
            let omega = std::f64::consts::TAU * spatial.emitter.lowpass / f64::from(self.output_rate);
            (1.0 - (-omega).exp()) as f32
        };
        self.cursor.step = self.step(self.rate.value());
    }

    /// Sets a parameter, at once or over `glide`.
    pub(crate) fn set(&mut self, param: VoiceParam, value: f64, glide: Option<Glide>) {
        let target = match param {
            VoiceParam::Gain => &mut self.gain,
            VoiceParam::Pan => &mut self.pan,
            VoiceParam::Rate => &mut self.rate,
        };
        match glide {
            Some(glide) => target.ramp(value, glide),
            None => target.set(value),
        }
        if param == VoiceParam::Rate {
            self.cursor.step = self.step(self.rate.value());
        }
    }

    /// Begins a stop: fades to silence over `fade` frames and then ends. False when there's no
    /// fade, and the voice should end now.
    pub(crate) fn stop(&mut self, fade: u64) -> bool {
        if fade == 0 {
            return false;
        }
        self.gain.ramp(
            0.0,
            Glide {
                frames: fade,
                curve: Curve::Linear,
            },
        );
        self.ending = Some(self.ending.map_or(fade, |left| left.min(fade)));
        true
    }

    /// Loops over `region` from here on, or stops looping, so playback runs on past the loop's
    /// end: an intro, a loop, and an outro in one source.
    pub(crate) fn set_loop(&mut self, region: Option<LoopRegion>) {
        self.cursor.looping = region.map(|region| (region.start, region.end));
        self.cursor.wrapped = false;
    }

    /// The fixed-point step for a playback rate, shifted by Doppler for a voice in space.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // A positive step below 2^64.
    fn step(&self, rate: f64) -> u64 {
        let doppler = self.spatial.as_ref().map_or(1.0, |spatial| spatial.doppler);
        let mut ratio = rate * doppler * f64::from(self.source.rate()) / f64::from(self.output_rate);
        if matches!(self.source, Source::Stream(..)) {
            // A stream is read through a window a block wide at four times: no faster.
            ratio = ratio.min(4.0);
        }
        ((ratio * 4_294_967_296.0).round() as u64).max(1)
    }

    /// Renders up to `frames` frames of each source channel into `scratch`, channel `c` at
    /// `c * QUANTUM`, using `steps` for a rate that is ramping. Returns how many frames it
    /// rendered: fewer than `frames` when the source ran out or a fade-out finished, and the voice
    /// has ended.
    pub(crate) fn render(&mut self, scratch: &mut [f32], steps: &mut [u64], frames: usize) -> usize {
        let frames = self
            .ending
            .map_or(frames, |left| frames.min(usize::try_from(left).unwrap_or(usize::MAX)));
        let gliding = !self.rate.steady();
        if gliding {
            for step in &mut steps[..frames] {
                *step = self.step(self.rate.value());
                self.rate.advance(1);
            }
        }
        let steps = gliding.then_some(&steps[..frames]);
        let produced = match &mut self.source {
            Source::Pcm(pcm) => render_pcm(pcm, &mut self.cursor, scratch, steps, frames),
            Source::Stream(stream, window) => {
                let reader = Reader {
                    stream,
                    window,
                    base: &mut self.base,
                    filled: &mut self.filled,
                };
                reader.render(&mut self.cursor, scratch, steps, frames)
            }
        };
        if gliding {
            self.cursor.step = self.step(self.rate.value());
        }
        if let Some(spatial) = self.spatial.as_mut()
            && spatial.lowpass < 1.0
        {
            for channel in 0..self.source.channels() {
                let memory = &mut spatial.memory[channel];
                for sample in &mut scratch[channel * QUANTUM..channel * QUANTUM + produced] {
                    *memory += spatial.lowpass * (*sample - *memory);
                    *sample = *memory;
                }
            }
        }
        // A fade that runs out mid-block renders short, which ends the voice; one that runs out
        // on the block's last frame renders nothing the next time round.
        if let Some(left) = self.ending.as_mut() {
            *left -= produced as u64;
        }
        produced
    }

    /// The voice's coefficients for the next `frames` frames into an output of `output`
    /// channels, moving its gain and pan along: steady numbers, or, while either ramps, one per
    /// frame written into `envelope`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)] // Engine levels are f32.
    pub(crate) fn levels(&mut self, frames: usize, envelope: &mut [f32], output: usize) -> Levels {
        let source = self.source.channels();
        if let Some(spatial) = &self.spatial {
            // Space sets the pan; the voice's gain scales what space gives, frame by frame.
            let ((first, second), (last_first, last_second)) = (spatial.from, spatial.to);
            if self.gain.steady() && spatial.from == spatial.to {
                let gain = self.gain.value() as f32;
                return Levels::Steady(gain * first, gain * second);
            }
            let (gains, seconds) = envelope.split_at_mut(QUANTUM);
            self.gain.fill(&mut gains[..frames]);
            for (index, (gain, other)) in gains[..frames].iter_mut().zip(&mut seconds[..frames]).enumerate() {
                let through = (index + 1) as f32 / frames as f32;
                *other = *gain * (second + (last_second - second) * through);
                *gain *= first + (last_first - first) * through;
            }
            return Levels::Varying;
        }
        if self.gain.steady() && self.pan.steady() {
            let (first, second) = coefficients(self.gain.value() as f32, self.pan.value() as f32, source, output);
            return Levels::Steady(first, second);
        }
        let (gains, pans) = envelope.split_at_mut(QUANTUM);
        self.gain.fill(&mut gains[..frames]);
        self.pan.fill(&mut pans[..frames]);
        for (gain, pan) in gains[..frames].iter_mut().zip(&mut pans[..frames]) {
            (*gain, *pan) = coefficients(*gain, *pan, source, output);
        }
        Levels::Varying
    }

    /// Adds `frames` of the rendered source channels in `scratch` into `bus`, `output` channels
    /// of `QUANTUM` each, at `levels` (from [`Voice::levels`], with its `envelope`), scaled by
    /// `scale` (a send's level).
    pub(crate) fn mix(
        &self,
        scratch: &[f32],
        frames: usize,
        bus: &mut [f32],
        output: usize,
        (levels, envelope): (Levels, &[f32]),
        scale: f32,
    ) {
        let (first, second) = match levels {
            Levels::Steady(first, second) => (
                Level {
                    envelope: None,
                    scale: first * scale,
                },
                Level {
                    envelope: None,
                    scale: second * scale,
                },
            ),
            Levels::Varying => (
                Level {
                    envelope: Some(&envelope[..frames]),
                    scale,
                },
                Level {
                    envelope: Some(&envelope[QUANTUM..QUANTUM + frames]),
                    scale,
                },
            ),
        };
        let channel = |index: usize| &scratch[index * QUANTUM..index * QUANTUM + frames];
        let mut out = |to: usize, from: &[f32], level: Level<'_>| {
            add(&mut bus[to * QUANTUM..to * QUANTUM + frames], from, level);
        };
        match (self.source.channels(), output) {
            (1, 2) => {
                out(0, channel(0), first);
                out(1, channel(0), second);
            }
            (2, 2) => {
                out(0, channel(0), first);
                out(1, channel(1), second);
            }
            (2, 1) => {
                out(0, channel(0), first);
                out(0, channel(1), second);
            }
            (1, _) => {
                for to in 0..output {
                    out(to, channel(0), first);
                }
            }
            _ => {
                for to in 0..output {
                    out(to, channel(to), first);
                }
            }
        }
    }

    /// Changes the level of the send to bus slot `target`, if the voice has one.
    pub(crate) fn set_send(&mut self, target: u32, level: f32) {
        for (bus, known) in self.sends.iter_mut().flatten() {
            if *bus == target {
                *known = level;
            }
        }
    }
}

#[allow(clippy::cast_possible_wrap)] // Frame indices are far below 2^63.
const fn to_signed(value: u64) -> i64 {
    value as i64
}
