//! `jess.audio`: the native half of audio for scripts, under `@jess/audio`.
//!
//! Luau reads and writes file formats (`@jess/audio` parses WAV, and a package parses whatever
//! else) and hands the engine samples: `pcm` turns interleaved bytes into a `Pcm` every voice
//! can share. A context plays voices on its sample clock; every command takes an optional `at`,
//! an output frame, and is carried out on exactly that frame. `offline` makes a context the
//! script renders itself, into a buffer of interleaved 32-bit floats: the same engine a device
//! runs, for tests, exports, and audio as data. Nothing here is a callback: the engine never
//! calls into Luau.
//!
//! Frames, not seconds, are the unit everywhere: they are what the clock counts, and converting
//! (and rounding) is the caller's choice. `@jess/audio` has the helpers.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::audio::{AudioConfig, AudioError, LoopRegion, OfflineContext, Pcm, PlaySpec, SampleFormat, VoiceKey, VoiceParam};
use crate::lua::plan::Functions;
use crate::lua::{Bytes, Text, error, kind, push_table};
use l3i::Runtime;
use l3i::bind::{Call, StackResults};
use l3i::convert::NewBuffer;
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::stack::{Frame, Scope, TableView, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::Table;

/// The largest frame number a Luau number holds exactly.
const MAX_FRAME: f64 = 9_007_199_254_740_992.0;

const PCM_KEYS: &[&str] = &["channels", "format", "rate"];
const CONTEXT_KEYS: &[&str] = &["channels", "commands", "rate", "voices"];
const PLAY_KEYS: &[&str] = &["at", "gain", "loop", "loopEnd", "loopStart", "offset", "pan", "rate"];

/// Decoded audio, shared by every voice that plays it.
#[derive(Clone)]
pub struct LuaAudioPcm(Arc<Pcm>);

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioPcm {
    const NAME: &'static str = "jess.audio.Pcm";
}

/// A context the script renders.
#[derive(Clone)]
pub struct LuaAudioContext(Rc<RefCell<OfflineContext>>);

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioContext {
    const NAME: &'static str = "jess.audio.Context";
}

/// One voice of a context.
#[derive(Clone)]
pub struct LuaAudioVoice {
    context: Rc<RefCell<OfflineContext>>,
    key: VoiceKey,
}

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioVoice {
    const NAME: &'static str = "jess.audio.Voice";
}

impl From<AudioError> for l3i::Error {
    fn from(problem: AudioError) -> Self {
        error(problem.to_string())
    }
}

/// A whole, non-negative number a Luau number holds exactly, as a count or frame.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Checked whole and in range.
fn whole(value: f64, what: &str) -> l3i::Result<u64> {
    if value.fract() != 0.0 || !(0.0..=MAX_FRAME).contains(&value) {
        return Err(error(format!("{what} is a whole number of frames from 0, not {value}")));
    }
    Ok(value as u64)
}

fn count(value: f64, what: &str) -> l3i::Result<usize> {
    usize::try_from(whole(value, what)?).map_err(|_| error(format!("{what} is too large: {value}")))
}

#[allow(clippy::cast_precision_loss)] // Frames below 2^53, which `whole` guarantees going in.
const fn number(frames: u64) -> f64 {
    frames as f64
}

fn frame_option(at: Option<f64>) -> l3i::Result<Option<u64>> {
    at.map(|at| whole(at, "at")).transpose()
}

/// The options table a function was given, checked for keys it doesn't take; `None` for none.
fn table<'v>(
    frame: &Frame<'_>,
    options: Option<ValueView<'v>>,
    function: &str,
    keys: &[&str],
) -> l3i::Result<Option<TableView<'v>>> {
    let Some(options) = options.filter(|options| !options.is_nil()) else {
        return Ok(None);
    };
    if !options.is_table() {
        return Err(error(format!("{function} takes a table of options, not {}", kind(options))));
    }
    crate::lua::check_options(frame, options, function, keys)?;
    Ok(Some(options.as_table()?))
}

fn optional<T: for<'a> l3i::convert::FromView<'a>>(
    frame: &Frame<'_>,
    table: Option<&TableView<'_>>,
    key: &str,
    function: &str,
) -> l3i::Result<Option<T>> {
    table.map_or_else(|| Ok(None), |table| table.raw_get_optional(frame, key, function))
}

/// `audio.pcm(data, { format, channels, rate })`.
fn pcm(call: &Call<'_>, data: &Bytes, options: Option<ValueView<'_>>) -> l3i::Result<Owned<LuaAudioPcm>> {
    const FUNCTION: &str = "jess.audio.pcm";
    let (format, channels, rate) = call.with_frame(|frame| {
        let Some(table) = table(frame, options, FUNCTION, PCM_KEYS)? else {
            return Err(error(format!("{FUNCTION} needs {{ format, channels, rate }}")));
        };
        let format: Option<String> = optional(frame, Some(&table), "format", FUNCTION)?;
        let channels: Option<f64> = optional(frame, Some(&table), "channels", FUNCTION)?;
        let rate: Option<f64> = optional(frame, Some(&table), "rate", FUNCTION)?;
        match (format, channels, rate) {
            (Some(format), Some(channels), Some(rate)) => Ok((format, channels, rate)),
            _ => Err(error(format!("{FUNCTION} needs format, channels, and rate"))),
        }
    })?;
    let format: SampleFormat = format.parse().map_err(l3i::Error::from)?;
    let rate = u32::try_from(whole(rate, "rate")?).map_err(|_| l3i::Error::from(AudioError::Rate(u32::MAX)))?;
    let pcm = Pcm::from_interleaved(&data.0, format, count(channels, "channels")?, rate).map_err(l3i::Error::from)?;
    Ok(Owned(LuaAudioPcm(Arc::new(pcm))))
}

/// `audio.offline({ rate, channels, voices, commands })`.
fn offline(call: &Call<'_>, options: Option<ValueView<'_>>) -> l3i::Result<Owned<LuaAudioContext>> {
    const FUNCTION: &str = "jess.audio.offline";
    let mut config = AudioConfig::default();
    call.with_frame(|frame| {
        let table = table(frame, options, FUNCTION, CONTEXT_KEYS)?;
        if let Some(rate) = optional::<f64>(frame, table.as_ref(), "rate", FUNCTION)? {
            config.rate = u32::try_from(whole(rate, "rate")?).map_err(|_| l3i::Error::from(AudioError::Rate(u32::MAX)))?;
        }
        if let Some(channels) = optional::<f64>(frame, table.as_ref(), "channels", FUNCTION)? {
            config.channels = count(channels, "channels")?;
        }
        if let Some(voices) = optional::<f64>(frame, table.as_ref(), "voices", FUNCTION)? {
            config.voices = count(voices, "voices")?;
        }
        if let Some(commands) = optional::<f64>(frame, table.as_ref(), "commands", FUNCTION)? {
            config.commands = count(commands, "commands")?;
        }
        Ok(())
    })?;
    let context = OfflineContext::new(&config).map_err(l3i::Error::from)?;
    Ok(Owned(LuaAudioContext(Rc::new(RefCell::new(context)))))
}

/// `context:play(pcm, { at, gain, pan, rate, offset, loop, loopStart, loopEnd })`.
#[allow(clippy::cast_possible_truncation)] // Gain and pan are f32 in the engine; range is checked there.
fn play_spec(call: &Call<'_>, pcm: &Pcm, options: Option<ValueView<'_>>) -> l3i::Result<PlaySpec> {
    const FUNCTION: &str = "AudioContext:play";
    let mut spec = PlaySpec::default();
    call.with_frame(|frame| {
        let table = table(frame, options, FUNCTION, PLAY_KEYS)?;
        let table = table.as_ref();
        spec.at = frame_option(optional(frame, table, "at", FUNCTION)?)?;
        if let Some(gain) = optional::<f64>(frame, table, "gain", FUNCTION)? {
            spec.gain = VoiceParam::Gain.check(gain).map_err(l3i::Error::from)? as f32;
        }
        if let Some(pan) = optional::<f64>(frame, table, "pan", FUNCTION)? {
            spec.pan = VoiceParam::Pan.check(pan).map_err(l3i::Error::from)? as f32;
        }
        if let Some(rate) = optional::<f64>(frame, table, "rate", FUNCTION)? {
            spec.rate = rate;
        }
        if let Some(offset) = optional::<f64>(frame, table, "offset", FUNCTION)? {
            spec.offset = whole(offset, "offset")?;
        }
        let looping = optional::<bool>(frame, table, "loop", FUNCTION)?.unwrap_or(false);
        let start = optional::<f64>(frame, table, "loopStart", FUNCTION)?;
        let end = optional::<f64>(frame, table, "loopEnd", FUNCTION)?;
        if !looping && (start.is_some() || end.is_some()) {
            return Err(error(format!("{FUNCTION}: loopStart and loopEnd need loop = true")));
        }
        if looping {
            spec.looping = Some(LoopRegion {
                start: start.map_or(Ok(0), |start| whole(start, "loopStart"))?,
                end: end.map_or_else(|| Ok(pcm.frames() as u64), |end| whole(end, "loopEnd"))?,
            });
        }
        Ok(())
    })?;
    Ok(spec)
}

fn interleaved_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|sample| sample.to_le_bytes()).collect()
}

pub fn module(runtime: &Runtime) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "audio")?;
    module.function("pcm", |call: &Call<'_>, data: Bytes, options: Option<ValueView<'_>>| {
        pcm(call, &data, options)
    })?;
    module.function("offline", |call: &Call<'_>, options: Option<ValueView<'_>>| {
        offline(call, options)
    })?;
    module.finish()
}

/// Declares the PCM, context, and voice types.
#[allow(clippy::too_many_lines)] // one method per operation
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut pcm = d.userdata::<LuaAudioPcm>(LuaAudioPcm::NAME);
    pcm.tag(TagPolicy::Never)
        .doc("Decoded audio: planar float channels at one rate, shared by every voice that plays it.");
    pcm.method("rate", |this: &LuaAudioPcm| f64::from(this.0.rate())).untyped();
    pcm.method("channels", |this: &LuaAudioPcm| number(this.0.channels() as u64))
        .untyped();
    pcm.method("frames", |this: &LuaAudioPcm| number(this.0.frames() as u64))
        .untyped();

    let mut context = d.userdata::<LuaAudioContext>(LuaAudioContext::NAME);
    context
        .tag(TagPolicy::Never)
        .doc("An audio context: voices on a sample clock, rendered by the script.");
    context
        .method("rate", |this: &LuaAudioContext| {
            f64::from(this.0.borrow_mut().control().rate())
        })
        .untyped();
    context
        .method("channels", |this: &LuaAudioContext| {
            number(this.0.borrow_mut().control().channels() as u64)
        })
        .untyped();
    context
        .method("now", |this: &LuaAudioContext| number(this.0.borrow_mut().control().now()))
        .untyped();
    context
        .method(
            "play",
            |this: &LuaAudioContext,
             call: &Call<'_>,
             source: ValueView<'_>,
             options: Option<ValueView<'_>>|
             -> l3i::Result<Owned<LuaAudioVoice>> {
                let Some(source) = l3i::userdata::receiver::<LuaAudioPcm>(source) else {
                    return Err(error(format!(
                        "AudioContext:play takes a Pcm from jess.audio.pcm, not {}",
                        kind(source)
                    )));
                };
                let source = Arc::clone(&source.0);
                let spec = play_spec(call, &source, options)?;
                let key = this.0.borrow_mut().control().play(&source, &spec).map_err(l3i::Error::from)?;
                Ok(Owned(LuaAudioVoice {
                    context: Rc::clone(&this.0),
                    key,
                }))
            },
        )
        .untyped();
    context
        .method("render", |this: &LuaAudioContext, frames: f64| -> l3i::Result<NewBuffer> {
            let frames = count(frames, "frames")?;
            let samples = this.0.borrow_mut().render(frames);
            Ok(NewBuffer(interleaved_bytes(&samples)))
        })
        .untyped();
    context
        .method(
            "stats",
            |this: &LuaAudioContext, call: &Call<'_>| -> l3i::Result<StackResults> {
                let stats = this.0.borrow_mut().control().stats();
                push_table(call, 0, 6, |frame, table| {
                    table.raw_set_value(frame, "frames", &number(stats.frames))?;
                    table.raw_set_value(frame, "active", &number(stats.active))?;
                    table.raw_set_value(frame, "allocated", &number(stats.allocated))?;
                    table.raw_set_value(frame, "late", &number(stats.late))?;
                    table.raw_set_value(frame, "nonfinite", &number(stats.nonfinite))?;
                    table.raw_set_value(frame, "pending", &number(stats.pending))
                })?;
                Ok(StackResults)
            },
        )
        .untyped();

    let mut voice = d.userdata::<LuaAudioVoice>(LuaAudioVoice::NAME);
    voice
        .tag(TagPolicy::Never)
        .doc("One playback of a Pcm in a context; once it ends, the handle names nothing.");
    voice
        .method("stop", |this: &LuaAudioVoice, at: Option<f64>| -> l3i::Result<bool> {
            let at = frame_option(at)?;
            this.context
                .borrow_mut()
                .control()
                .stop(this.key, at)
                .map_err(l3i::Error::from)
        })
        .untyped();
    voice
        .method(
            "set",
            |this: &LuaAudioVoice, name: Text, value: f64, at: Option<f64>| -> l3i::Result<bool> {
                let Some(param) = VoiceParam::named(&name.0) else {
                    return Err(error(format!("AudioVoice:set takes gain, pan, or rate, not {:?}", name.0)));
                };
                let at = frame_option(at)?;
                this.context
                    .borrow_mut()
                    .control()
                    .set(this.key, param, value, at)
                    .map_err(l3i::Error::from)
            },
        )
        .untyped();
    voice
        .method("state", |this: &LuaAudioVoice| {
            this.context.borrow_mut().control().state(this.key).name()
        })
        .untyped();
}
