//! `jess.audio`: the native half of audio for scripts, under `@jess/audio`.
//!
//! Luau reads and writes file formats (`@jess/audio` parses WAV, and a package parses whatever
//! else) and hands the engine samples: `pcm` turns interleaved bytes into a `Pcm` every voice
//! can share. A context plays voices on its sample clock through a tree of buses that run
//! processors; every command takes an optional `at`, an output frame, and is carried out on
//! exactly that frame. `offline` makes a context the script renders itself, into a buffer of
//! interleaved 32-bit floats: the same engine a device runs, for tests, exports, and audio as
//! data. Nothing here is a callback: the engine never calls into Luau.
//!
//! Frames, not seconds, are the unit everywhere: they are what the clock counts, and converting
//! (and rounding) is the caller's choice. `@jess/audio` has the helpers.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::audio::Stream;
use crate::audio::processor::{Setting, Settings};
use crate::audio::{
    AudioConfig, AudioError, BusKey, BusParam, BusSpec, Control, Culled, Curve, DeviceRequest, Distance, Emitter, Glide,
    Handedness, Listener, LiveContext, LoopRegion, MAX_VOICE_SENDS, OfflineContext, Pcm, PlaySpec, ProcessorKey,
    SampleFormat, Tap, VoiceKey, VoiceParam, VoiceSend,
};
use crate::lua::plan::Functions;
use crate::lua::{Bytes, Jess, Text, error, guard, kind, push_table};
use crate::runtime::ResourceHandle;
use l3i::Runtime;
use l3i::bind::{Call, StackResults};
use l3i::convert::{NewBuffer, Vector3};
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::stack::{Frame, Scope, TableView, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::{Function, Table};

/// The largest frame number a Luau number holds exactly.
const MAX_FRAME: f64 = 9_007_199_254_740_992.0;

const PCM_KEYS: &[&str] = &["channels", "format", "rate"];
const OUTPUT_KEYS: &[&str] = &[
    "buses",
    "channels",
    "commands",
    "device",
    "handedness",
    "period",
    "physical",
    "processors",
    "rate",
    "speedOfSound",
    "voices",
];
const CONTEXT_KEYS: &[&str] = &[
    "buses",
    "channels",
    "commands",
    "handedness",
    "physical",
    "processors",
    "rate",
    "speedOfSound",
    "voices",
];
const PLACE_KEYS: &[&str] = &[
    "cone",
    "direction",
    "distance",
    "doppler",
    "gain",
    "lowpass",
    "maximum",
    "position",
    "reference",
    "rolloff",
    "velocity",
];
const CONE_KEYS: &[&str] = &["inner", "outer", "outside"];
const LISTENER_KEYS: &[&str] = &["forward", "position", "up", "velocity"];
const STREAM_KEYS: &[&str] = &["channels", "rate", "seconds"];
const PLAY_KEYS: &[&str] = &[
    "at",
    "bus",
    "culled",
    "fade",
    "gain",
    "loop",
    "loopEnd",
    "loopStart",
    "offset",
    "pan",
    "place",
    "priority",
    "rate",
    "sends",
];
const SEND_KEYS: &[&str] = &["bus", "level"];
const BUS_KEYS: &[&str] = &["gain", "muted", "parent"];
const BUS_SEND_KEYS: &[&str] = &["at", "level", "tap"];

/// A context a script holds: one it renders, or one a device does.
enum Context {
    Offline(Box<OfflineContext>),
    Live(Box<LiveContext>),
}

impl Context {
    const fn control(&mut self) -> &mut Control {
        match self {
            Context::Offline(context) => context.control(),
            Context::Live(context) => context.control(),
        }
    }
}

type Shared = Rc<RefCell<Context>>;

/// A stream: audio written while it plays.
#[derive(Clone)]
pub struct LuaAudioStream(Arc<Stream>);

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioStream {
    const NAME: &'static str = "jess.audio.Stream";
}

/// The kind of handle a stream crosses to a worker as.
const STREAM_HANDLE: &str = "audio.stream";

/// `audio.stream({ channels, rate, seconds })`, or `audio.stream(handle)` for a stream another
/// worker made.
fn stream(call: &Call<'_>, options: ValueView<'_>) -> l3i::Result<Owned<LuaAudioStream>> {
    const FUNCTION: &str = "jess.audio.stream";
    if let Some(handle) = l3i::userdata::receiver::<ResourceHandle>(options) {
        if handle.kind != STREAM_HANDLE {
            return Err(error(format!(
                "{FUNCTION} opens an {STREAM_HANDLE} handle, not a {}",
                handle.kind
            )));
        }
        let found = Stream::find(handle.id).ok_or_else(|| error(format!("{FUNCTION}: the stream is gone")))?;
        return Ok(Owned(LuaAudioStream(found)));
    }
    let (channels, rate, seconds) = call.with_frame(|frame| {
        let table = table_of(frame, options, FUNCTION, STREAM_KEYS)?;
        let channels = optional::<f64>(frame, Some(&table), "channels", FUNCTION)?.unwrap_or(2.0);
        let rate = optional::<f64>(frame, Some(&table), "rate", FUNCTION)?.unwrap_or(48_000.0);
        let seconds = optional::<f64>(frame, Some(&table), "seconds", FUNCTION)?.unwrap_or(1.0);
        Ok((channels, rate, seconds))
    })?;
    let stream = Stream::new(count(channels, "channels")?, rate_of(rate)?, seconds)?;
    Ok(Owned(LuaAudioStream(stream)))
}

/// Interleaved 32-bit floats from a buffer, whole samples of it.
fn floats(data: &[u8]) -> Vec<f32> {
    let (samples, _) = data.as_chunks::<4>();
    samples.iter().map(|bytes| f32::from_le_bytes(*bytes)).collect()
}

fn describe_stream(d: &mut ExtensionDescriptor) {
    let mut stream = d.userdata::<LuaAudioStream>(LuaAudioStream::NAME);
    stream
        .tag(TagPolicy::Never)
        .doc("A stream: a bounded ring of frames one writer fills while one voice plays it.");
    stream
        .method("channels", |this: &LuaAudioStream| number(this.0.channels() as u64))
        .untyped();
    stream
        .method("rate", |this: &LuaAudioStream| f64::from(this.0.rate()))
        .untyped();
    stream.method("room", |this: &LuaAudioStream| number(this.0.room())).untyped();
    stream
        .method(
            "write",
            |this: &LuaAudioStream, samples: Bytes, frames: Option<f64>| -> l3i::Result<f64> {
                let mut data = &samples.0[..];
                if let Some(frames) = frames {
                    // The first `frames` frames only: a decoder or generator that filled part of
                    // a reused buffer writes just that part.
                    let size = count(frames, "AudioStream:write's frames")?.saturating_mul(this.0.channels() * 4);
                    if size > data.len() {
                        return Err(error(format!(
                            "AudioStream:write: {frames} frames are more than the buffer holds"
                        )));
                    }
                    data = &data[..size];
                }
                if !data.len().is_multiple_of(4) {
                    return Err(error(
                        "AudioStream:write takes interleaved 32-bit floats: a whole number of them",
                    ));
                }
                Ok(number(this.0.write(&floats(data))? as u64))
            },
        )
        .untyped();
    stream
        .method(
            "wait",
            |this: &LuaAudioStream, frames: f64, timeout: Option<f64>| -> l3i::Result<bool> {
                let frames = whole(frames, "frames")?;
                let timeout = timeout.unwrap_or(f64::INFINITY);
                if timeout.is_nan() || timeout < 0.0 {
                    return Err(error(format!(
                        "AudioStream:wait takes a timeout in seconds from 0, not {timeout}"
                    )));
                }
                let timeout = Duration::try_from_secs_f64(timeout).unwrap_or(Duration::MAX);
                Ok(this.0.wait(frames, timeout))
            },
        )
        .untyped();
    stream.method("finish", |this: &LuaAudioStream| this.0.finish()).untyped();
    stream
        .method(
            "stats",
            |this: &LuaAudioStream, call: &Call<'_>| -> l3i::Result<StackResults> {
                let stats = this.0.stats();
                push_table(call, 0, 5, |frame, table| {
                    table.raw_set_value(frame, "written", &number(stats.written))?;
                    table.raw_set_value(frame, "read", &number(stats.read))?;
                    table.raw_set_value(frame, "room", &number(stats.room))?;
                    table.raw_set_value(frame, "missed", &number(stats.missed))?;
                    table.raw_set_value(frame, "finished", &stats.finished)
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    stream
        .method("handle", |this: &LuaAudioStream| {
            Owned(ResourceHandle {
                kind: STREAM_HANDLE.to_owned(),
                id: this.0.id(),
            })
        })
        .untyped();
}

/// Decoded audio, shared by every voice that plays it.
#[derive(Clone)]
pub struct LuaAudioPcm(Arc<Pcm>);

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioPcm {
    const NAME: &'static str = "jess.audio.Pcm";
}

/// A context the script renders.
#[derive(Clone)]
pub struct LuaAudioContext(Shared);

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioContext {
    const NAME: &'static str = "jess.audio.Context";
}

/// One voice of a context.
#[derive(Clone)]
pub struct LuaAudioVoice {
    context: Shared,
    key: VoiceKey,
}

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioVoice {
    const NAME: &'static str = "jess.audio.Voice";
}

/// One bus of a context.
#[derive(Clone)]
pub struct LuaAudioBus {
    context: Shared,
    key: BusKey,
}

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioBus {
    const NAME: &'static str = "jess.audio.Bus";
}

/// One processor in a bus's chain.
#[derive(Clone)]
pub struct LuaAudioProcessor {
    context: Shared,
    key: ProcessorKey,
    kind: &'static str,
}

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioProcessor {
    const NAME: &'static str = "jess.audio.Processor";
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

/// The bus a value is, which must belong to `context`.
fn bus_of(value: ValueView<'_>, context: &Shared, what: &str) -> l3i::Result<BusKey> {
    let Some(bus) = l3i::userdata::receiver::<LuaAudioBus>(value) else {
        return Err(error(format!("{what} is an AudioBus, not {}", kind(value))));
    };
    if !Rc::ptr_eq(&bus.context, context) {
        return Err(AudioError::StaleBus.into());
    }
    Ok(bus.key)
}

/// The bus in field `key` of `table`, if there is one.
fn bus_field(frame: &Frame<'_>, table: Option<&TableView<'_>>, key: &str, context: &Shared) -> l3i::Result<Option<BusKey>> {
    let Some(table) = table else {
        return Ok(None);
    };
    frame.with_frame(|frame| {
        let value = table.raw_get(frame, key)?;
        if value.is_nil() {
            return Ok(None);
        }
        bus_of(value, context, key).map(Some)
    })
}

fn vector(frame: &Frame<'_>, table: &TableView<'_>, key: &str, function: &str) -> l3i::Result<Option<[f64; 3]>> {
    let value: Option<Vector3> = table.raw_get_optional(frame, key, function)?;
    Ok(value.map(|value| [f64::from(value.x), f64::from(value.y), f64::from(value.z)]))
}

/// `base` with the fields of a place's options table changed: vectors for position, velocity,
/// and direction, a cone in degrees, a distance model by name, and its numbers.
fn emitter(frame: &Frame<'_>, options: ValueView<'_>, base: Emitter, function: &str) -> l3i::Result<Emitter> {
    let place = table_of(frame, options, function, PLACE_KEYS)?;
    let mut emitter = base;
    if let Some(position) = vector(frame, &place, "position", function)? {
        emitter.position = position;
    }
    if let Some(velocity) = vector(frame, &place, "velocity", function)? {
        emitter.velocity = velocity;
    }
    if let Some(direction) = vector(frame, &place, "direction", function)? {
        emitter.direction = Some(direction);
    }
    if let Some(model) = optional::<String>(frame, Some(&place), "distance", function)? {
        emitter.distance = Distance::named(&model).ok_or_else(|| {
            error(format!(
                "{function}'s distance is none, inverse, linear, or exponential, not {model:?}"
            ))
        })?;
    }
    for (key, field) in [
        ("reference", &mut emitter.reference),
        ("maximum", &mut emitter.maximum),
        ("rolloff", &mut emitter.rolloff),
        ("doppler", &mut emitter.doppler),
        ("gain", &mut emitter.gain),
        ("lowpass", &mut emitter.lowpass),
    ] {
        if let Some(value) = optional::<f64>(frame, Some(&place), key, function)? {
            *field = value;
        }
    }
    frame.with_frame(|frame| {
        let cone = place.raw_get(frame, "cone")?;
        if cone.is_nil() {
            return Ok(());
        }
        let cone = table_of(frame, cone, function, CONE_KEYS)?;
        for (key, field) in [("inner", &mut emitter.inner), ("outer", &mut emitter.outer)] {
            if let Some(degrees) = optional::<f64>(frame, Some(&cone), key, function)? {
                *field = degrees.to_radians();
            }
        }
        if let Some(outside) = optional::<f64>(frame, Some(&cone), "outside", function)? {
            emitter.outside = outside;
        }
        Ok(())
    })?;
    Ok(emitter)
}

/// `base` with the fields of a listener's options table changed.
fn listener(frame: &Frame<'_>, options: ValueView<'_>, base: Listener, function: &str) -> l3i::Result<Listener> {
    let table = table_of(frame, options, function, LISTENER_KEYS)?;
    let mut listener = base;
    for (key, field) in [
        ("position", &mut listener.position),
        ("forward", &mut listener.forward),
        ("up", &mut listener.up),
        ("velocity", &mut listener.velocity),
    ] {
        if let Some(value) = vector(frame, &table, key, function)? {
            *field = value;
        }
    }
    Ok(listener)
}

fn voice_param(name: &Text, function: &str) -> l3i::Result<VoiceParam> {
    VoiceParam::named(&name.0).ok_or_else(|| error(format!("{function} takes gain, pan, or rate, not {:?}", name.0)))
}

/// A ramp of `frames` frames, `linear` unless `curve` names another.
fn glide_of(frames: f64, curve: Option<&Text>) -> l3i::Result<Glide> {
    let curve = match curve {
        None => Curve::Linear,
        Some(name) => Curve::named(&name.0)
            .ok_or_else(|| error(format!("a ramp's curve is linear or exponential, not {:?}", name.0)))?,
    };
    Ok(Glide {
        frames: whole(frames, "a ramp's length")?,
        curve,
    })
}

fn rate_of(rate: f64) -> l3i::Result<u32> {
    u32::try_from(whole(rate, "rate")?).map_err(|_| AudioError::Rate(u32::MAX).into())
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
    let format: SampleFormat = format.parse()?;
    let pcm = Pcm::from_interleaved(&data.0, format, count(channels, "channels")?, rate_of(rate)?)?;
    Ok(Owned(LuaAudioPcm(Arc::new(pcm))))
}

/// `audio.offline({ rate, channels, voices, buses, processors, commands, ... })`.
fn offline(call: &Call<'_>, options: Option<ValueView<'_>>) -> l3i::Result<Owned<LuaAudioContext>> {
    let (config, _) = context_config(call, options, "jess.audio.offline", CONTEXT_KEYS)?;
    let context = OfflineContext::new(&config)?;
    Ok(Owned(LuaAudioContext(Rc::new(RefCell::new(Context::Offline(Box::new(
        context,
    )))))))
}

/// `audio.output({ device, rate, period, channels, voices, ... })`: the platform's default output,
/// or the null device, rendering a context the script drives like any other.
fn output(jess: &Jess, call: &Call<'_>, options: Option<ValueView<'_>>) -> l3i::Result<Owned<LuaAudioContext>> {
    const FUNCTION: &str = "jess.audio.output";
    guard(jess, FUNCTION, crate::audio::capability::OUTPUT)?;
    let (config, request) = context_config(call, options, FUNCTION, OUTPUT_KEYS)?;
    let live = request.open(&config)?;
    Ok(Owned(LuaAudioContext(Rc::new(RefCell::new(Context::Live(Box::new(live)))))))
}

/// A context's options, and for an output the device's: its rate (the device's own unless given),
/// its period, and whether it is the null device.
fn context_config(
    call: &Call<'_>,
    options: Option<ValueView<'_>>,
    function: &str,
    keys: &[&str],
) -> l3i::Result<(AudioConfig, DeviceRequest)> {
    let mut config = AudioConfig::default();
    let mut request = DeviceRequest::default();
    let live = keys.contains(&"device");
    call.with_frame(|frame| {
        let table = table(frame, options, function, keys)?;
        let table = table.as_ref();
        if let Some(rate) = optional::<f64>(frame, table, "rate", function)? {
            config.rate = rate_of(rate)?;
            request.rate = Some(config.rate);
        }
        if live {
            if let Some(period) = optional::<f64>(frame, table, "period", function)? {
                request.period = Some(u32::try_from(whole(period, "period")?).unwrap_or(u32::MAX));
            }
            request.null = match optional::<String>(frame, table, "device", function)?.as_deref() {
                None | Some("default") => false,
                Some("null") => true,
                Some(other) => return Err(error(format!("{function}'s device is default or null, not {other:?}"))),
            };
        }
        for (key, field) in [
            ("channels", &mut config.channels),
            ("voices", &mut config.voices),
            ("buses", &mut config.buses),
            ("processors", &mut config.processors),
            ("commands", &mut config.commands),
        ] {
            if let Some(value) = optional::<f64>(frame, table, key, function)? {
                *field = count(value, key)?;
            }
        }
        if let Some(physical) = optional::<f64>(frame, table, "physical", function)? {
            config.physical = Some(count(physical, "physical")?);
        }
        if let Some(speed) = optional::<f64>(frame, table, "speedOfSound", function)? {
            config.speed_of_sound = speed;
        }
        config.handedness = match optional::<String>(frame, table, "handedness", function)?.as_deref() {
            None | Some("right") => Handedness::Right,
            Some("left") => Handedness::Left,
            Some(other) => return Err(error(format!("{function}'s handedness is right or left, not {other:?}"))),
        };
        Ok(())
    })?;
    Ok((config, request))
}

/// `context:play(pcm, { at, bus, sends, gain, pan, rate, offset, loop, loopStart, loopEnd })`.
#[allow(clippy::cast_possible_truncation)] // Gain and pan are f32 in the engine; range is checked there.
fn play_spec(call: &Call<'_>, context: &Shared, frames: u64, options: Option<ValueView<'_>>) -> l3i::Result<PlaySpec> {
    const FUNCTION: &str = "AudioContext:play";
    let mut spec = PlaySpec::default();
    call.with_frame(|frame| {
        let table = table(frame, options, FUNCTION, PLAY_KEYS)?;
        let table = table.as_ref();
        spec.at = frame_option(optional(frame, table, "at", FUNCTION)?)?;
        if let Some(gain) = optional::<f64>(frame, table, "gain", FUNCTION)? {
            spec.gain = VoiceParam::Gain.check(gain)? as f32;
        }
        if let Some(pan) = optional::<f64>(frame, table, "pan", FUNCTION)? {
            spec.pan = VoiceParam::Pan.check(pan)? as f32;
        }
        if let Some(rate) = optional::<f64>(frame, table, "rate", FUNCTION)? {
            spec.rate = rate;
        }
        if let Some(offset) = optional::<f64>(frame, table, "offset", FUNCTION)? {
            spec.offset = whole(offset, "offset")?;
        }
        if let Some(bus) = bus_field(frame, table, "bus", context)? {
            spec.bus = bus;
        }
        spec.sends = voice_sends(frame, table, context)?;
        if let Some(fade) = optional::<f64>(frame, table, "fade", FUNCTION)? {
            spec.fade = whole(fade, "fade")?;
        }
        if let Some(priority) = optional::<f64>(frame, table, "priority", FUNCTION)? {
            if !priority.is_finite() {
                return Err(error(format!("{FUNCTION}'s priority is a finite number, not {priority}")));
            }
            spec.priority = priority as f32;
        }
        if let Some(culled) = optional::<String>(frame, table, "culled", FUNCTION)? {
            spec.culled = Culled::named(&culled)
                .ok_or_else(|| error(format!("{FUNCTION}'s culled is advance, pause, or stop, not {culled:?}")))?;
        }
        if let Some(table) = table {
            spec.spatial = frame.with_frame(|frame| {
                let place = table.raw_get(frame, "place")?;
                if place.is_nil() {
                    return Ok(None);
                }
                emitter(frame, place, Emitter::default(), "AudioContext:play's place").map(Some)
            })?;
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
                end: end.map_or_else(|| Ok(frames), |end| whole(end, "loopEnd"))?,
            });
        }
        Ok(())
    })?;
    Ok(spec)
}

/// `sends = { { bus, level }, ... }` of a play's options.
#[allow(clippy::cast_possible_truncation)] // Levels are f32 in the engine.
fn voice_sends(
    frame: &Frame<'_>,
    table: Option<&TableView<'_>>,
    context: &Shared,
) -> l3i::Result<[Option<VoiceSend>; MAX_VOICE_SENDS]> {
    const FUNCTION: &str = "AudioContext:play's sends";
    let mut sends = [None; MAX_VOICE_SENDS];
    let Some(table) = table else {
        return Ok(sends);
    };
    frame.with_frame(|frame| {
        let list = table.raw_get(frame, "sends")?;
        if list.is_nil() {
            return Ok(sends);
        }
        if !list.is_table() {
            return Err(error(format!("{FUNCTION} is a list of {{ bus, level }}, not {}", kind(list))));
        }
        let list = list.as_table()?;
        for index in 1.. {
            let more = frame.with_frame(|frame| {
                let entry = list.raw_get_index(frame, index)?;
                if entry.is_nil() {
                    return Ok(false);
                }
                let slot = usize::try_from(index - 1).unwrap_or(usize::MAX);
                if slot >= MAX_VOICE_SENDS {
                    return Err(AudioError::SendsExhausted(MAX_VOICE_SENDS).into());
                }
                let send = table_of(frame, entry, FUNCTION, SEND_KEYS)?;
                let Some(bus) = bus_field(frame, Some(&send), "bus", context)? else {
                    return Err(error(format!("{FUNCTION}: each send names its bus")));
                };
                let level = optional::<f64>(frame, Some(&send), "level", FUNCTION)?.unwrap_or(1.0);
                sends[slot] = Some(VoiceSend {
                    bus,
                    level: level as f32,
                });
                Ok(true)
            })?;
            if !more {
                break;
            }
        }
        Ok(sends)
    })
}

/// A table that must be there.
fn table_of<'v>(frame: &Frame<'_>, value: ValueView<'v>, function: &str, keys: &[&str]) -> l3i::Result<TableView<'v>> {
    table(frame, Some(value), function, keys)?.ok_or_else(|| error(format!("{function} takes a table")))
}

/// `context:bus({ parent, gain, muted })`.
#[allow(clippy::cast_possible_truncation)] // Gain is f32 in the engine.
fn bus_spec(call: &Call<'_>, context: &Shared, options: Option<ValueView<'_>>) -> l3i::Result<BusSpec> {
    const FUNCTION: &str = "AudioContext:bus";
    let mut spec = BusSpec::default();
    call.with_frame(|frame| {
        let table = table(frame, options, FUNCTION, BUS_KEYS)?;
        let table = table.as_ref();
        if let Some(parent) = bus_field(frame, table, "parent", context)? {
            spec.parent = parent;
        }
        if let Some(gain) = optional::<f64>(frame, table, "gain", FUNCTION)? {
            spec.gain = BusParam::Gain.check(gain)? as f32;
        }
        spec.muted = optional::<bool>(frame, table, "muted", FUNCTION)?.unwrap_or(false);
        Ok(())
    })?;
    Ok(spec)
}

/// A processor's settings: every key of its options table but `index`, a number or a name.
fn settings(frame: &Frame<'_>, options: Option<ValueView<'_>>, function: &str) -> l3i::Result<(Settings, Option<usize>)> {
    let mut settings = Settings::new();
    let mut index = None;
    let Some(options) = options.filter(|options| !options.is_nil()) else {
        return Ok((settings, index));
    };
    if !options.is_table() {
        return Err(error(format!("{function} takes a table of settings, not {}", kind(options))));
    }
    let mut problem = None;
    options.as_table()?.for_each(frame, |_, key, value| {
        if problem.is_some() {
            return Ok(());
        }
        let name = if key.is_string() {
            String::from_utf8_lossy(key.read::<&[u8]>()?).into_owned()
        } else {
            problem = Some(error(format!("{function}'s settings are named")));
            return Ok(());
        };
        if name == "index" {
            let position = if value.is_number() { value.read::<f64>()? } else { 0.0 };
            if position >= 1.0 {
                index = Some(count(position - 1.0, "index")?);
            } else {
                problem = Some(error(format!("{function}'s index is a position in the chain from 1")));
            }
            return Ok(());
        }
        if value.is_number() {
            settings.insert(&name, Setting::Number(value.read::<f64>()?));
        } else if value.is_string() {
            let text = String::from_utf8_lossy(value.read::<&[u8]>()?).into_owned();
            settings.insert(&name, Setting::Text(text));
        } else {
            problem = Some(error(format!(
                "{function}'s {name} is a number or a name, not {}",
                kind(value)
            )));
        }
        Ok(())
    })?;
    problem.map_or(Ok((settings, index)), Err)
}

fn interleaved_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|sample| sample.to_le_bytes()).collect()
}

fn bus_handle(context: &Shared, key: BusKey) -> Owned<LuaAudioBus> {
    Owned(LuaAudioBus {
        context: Rc::clone(context),
        key,
    })
}

pub fn module(runtime: &Runtime, jess: &Rc<Jess>) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "audio")?;
    module.function("pcm", |call: &Call<'_>, data: Bytes, options: Option<ValueView<'_>>| {
        pcm(call, &data, options)
    })?;
    module.function("offline", |call: &Call<'_>, options: Option<ValueView<'_>>| {
        offline(call, options)
    })?;
    let state = Rc::clone(jess);
    module.function("output", move |call: &Call<'_>, options: Option<ValueView<'_>>| {
        output(&state, call, options)
    })?;
    module.function("stream", |call: &Call<'_>, options: ValueView<'_>| stream(call, options))?;
    module.function("processors", |call: &Call<'_>| -> l3i::Result<StackResults> {
        l3i::stack::push_list(call, crate::audio::processor::KINDS.iter(), |frame, kind| {
            frame.push(*kind)?;
            Ok(())
        })?;
        Ok(StackResults)
    })?;
    module.finish()
}

/// Declares the PCM, context, voice, bus, and processor types.
pub fn describe(d: &mut ExtensionDescriptor) {
    describe_pcm(d);
    describe_context(d);
    describe_voice(d);
    describe_bus(d);
    describe_processor(d);
    describe_stream(d);
}

fn describe_pcm(d: &mut ExtensionDescriptor) {
    let mut pcm = d.userdata::<LuaAudioPcm>(LuaAudioPcm::NAME);
    pcm.tag(TagPolicy::Never)
        .doc("Decoded audio: planar float channels at one rate, shared by every voice that plays it.");
    pcm.method("rate", |this: &LuaAudioPcm| f64::from(this.0.rate())).untyped();
    pcm.method("channels", |this: &LuaAudioPcm| number(this.0.channels() as u64))
        .untyped();
    pcm.method("frames", |this: &LuaAudioPcm| number(this.0.frames() as u64))
        .untyped();
}

#[allow(clippy::too_many_lines)] // one method per operation
fn describe_context(d: &mut ExtensionDescriptor) {
    let mut context = d.userdata::<LuaAudioContext>(LuaAudioContext::NAME);
    context
        .tag(TagPolicy::Never)
        .doc("An audio context: voices on a sample clock through a tree of buses, rendered by the script.");
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
                if let Some(stream) = l3i::userdata::receiver::<LuaAudioStream>(source) {
                    let stream = Arc::clone(&stream.0);
                    let spec = play_spec(call, &this.0, 0, options)?;
                    let key = this.0.borrow_mut().control().play_stream(&stream, &spec)?;
                    return Ok(Owned(LuaAudioVoice {
                        context: Rc::clone(&this.0),
                        key,
                    }));
                }
                let Some(source) = l3i::userdata::receiver::<LuaAudioPcm>(source) else {
                    return Err(error(format!(
                        "AudioContext:play takes a Pcm from jess.audio.pcm or a Stream from jess.audio.stream, not {}",
                        kind(source)
                    )));
                };
                let source = Arc::clone(&source.0);
                let spec = play_spec(call, &this.0, source.frames() as u64, options)?;
                let key = this.0.borrow_mut().control().play(&source, &spec)?;
                Ok(Owned(LuaAudioVoice {
                    context: Rc::clone(&this.0),
                    key,
                }))
            },
        )
        .untyped();
    context
        .method("master", |this: &LuaAudioContext| bus_handle(&this.0, BusKey::MASTER))
        .untyped();
    context
        .method(
            "listener",
            |this: &LuaAudioContext, call: &Call<'_>, options: ValueView<'_>, at: Option<f64>| -> l3i::Result<()> {
                let at = frame_option(at)?;
                let base = this.0.borrow_mut().control().listener();
                let moved = call.with_frame(|frame| listener(frame, options, base, "AudioContext:listener"))?;
                Ok(this.0.borrow_mut().control().set_listener(moved, at)?)
            },
        )
        .untyped();
    context
        .method(
            "device",
            |this: &LuaAudioContext, call: &Call<'_>| -> l3i::Result<StackResults> {
                let Context::Live(live) = &*this.0.borrow() else {
                    call.push(&())?;
                    return Ok(StackResults);
                };
                let (info, stats) = (live.info().clone(), live.stats());
                push_table(call, 0, 10, |frame, table| {
                    table.raw_set_value(frame, "backend", info.backend.as_str())?;
                    table.raw_set_value(frame, "rate", &f64::from(info.rate))?;
                    table.raw_set_value(frame, "channels", &number(info.channels as u64))?;
                    table.raw_set_value(frame, "period", &f64::from(info.period))?;
                    table.raw_set_value(frame, "periods", &f64::from(info.periods))?;
                    table.raw_set_value(frame, "latency", &number(info.latency()))?;
                    table.raw_set_value(frame, "stopped", &number(stats.stopped))?;
                    table.raw_set_value(frame, "rerouted", &number(stats.rerouted))?;
                    table.raw_set_value(frame, "interrupted", &number(stats.interrupted))?;
                    table.raw_set_value(frame, "failed", &stats.failed)
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    context
        .method("room", |this: &LuaAudioContext| {
            number(this.0.borrow_mut().control().room() as u64)
        })
        .untyped();
    context
        .method(
            "group",
            |this: &LuaAudioContext, call: &Call<'_>, body: Function| -> l3i::Result<()> {
                this.0.borrow_mut().control().begin_group()?;
                // The body runs with no borrow held: it plays and sets through this context.
                let ran = body.invoke::<(), _>(call, ());
                // What the body sent before an error still goes, so the control side's record of
                // voices and buses stays what the engine is told.
                let sent = this.0.borrow_mut().control().end_group();
                ran?;
                Ok(sent?)
            },
        )
        .untyped();
    context
        .method(
            "bus",
            |this: &LuaAudioContext, call: &Call<'_>, options: Option<ValueView<'_>>| -> l3i::Result<Owned<LuaAudioBus>> {
                let spec = bus_spec(call, &this.0, options)?;
                let key = this.0.borrow_mut().control().add_bus(&spec)?;
                Ok(bus_handle(&this.0, key))
            },
        )
        .untyped();
    context
        .method("render", |this: &LuaAudioContext, frames: f64| -> l3i::Result<NewBuffer> {
            let frames = count(frames, "frames")?;
            let Context::Offline(context) = &mut *this.0.borrow_mut() else {
                return Err(error(
                    "AudioContext:render renders an offline context; a device renders an output",
                ));
            };
            Ok(NewBuffer(interleaved_bytes(&context.render(frames))))
        })
        .untyped();
    context
        .method(
            "stats",
            |this: &LuaAudioContext, call: &Call<'_>| -> l3i::Result<StackResults> {
                let stats = this.0.borrow_mut().control().stats();
                push_table(call, 0, 8, |frame, table| {
                    table.raw_set_value(frame, "culled", &number(stats.culled))?;
                    table.raw_set_value(frame, "stolen", &number(stats.stolen))?;
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
}

fn describe_voice(d: &mut ExtensionDescriptor) {
    let mut voice = d.userdata::<LuaAudioVoice>(LuaAudioVoice::NAME);
    voice
        .tag(TagPolicy::Never)
        .doc("One playback of a Pcm in a context; once it ends, the handle names nothing.");
    voice
        .method(
            "stop",
            |this: &LuaAudioVoice, at: Option<f64>, fade: Option<f64>| -> l3i::Result<bool> {
                let at = frame_option(at)?;
                let fade = fade.map_or(Ok(0), |fade| whole(fade, "fade"))?;
                Ok(this.context.borrow_mut().control().stop(this.key, at, fade)?)
            },
        )
        .untyped();
    voice
        .method(
            "set",
            |this: &LuaAudioVoice, name: Text, value: f64, at: Option<f64>| -> l3i::Result<bool> {
                let param = voice_param(&name, "AudioVoice:set")?;
                let at = frame_option(at)?;
                Ok(this.context.borrow_mut().control().set(this.key, param, value, at, None)?)
            },
        )
        .untyped();
    voice
        .method(
            "ramp",
            |this: &LuaAudioVoice,
             name: Text,
             value: f64,
             frames: f64,
             at: Option<f64>,
             curve: Option<Text>|
             -> l3i::Result<bool> {
                let param = voice_param(&name, "AudioVoice:ramp")?;
                let (at, glide) = (frame_option(at)?, glide_of(frames, curve.as_ref())?);
                Ok(this
                    .context
                    .borrow_mut()
                    .control()
                    .set(this.key, param, value, at, Some(glide))?)
            },
        )
        .untyped();
    voice
        .method(
            "loop",
            |this: &LuaAudioVoice, start: f64, stop: f64, at: Option<f64>| -> l3i::Result<bool> {
                let region = LoopRegion {
                    start: whole(start, "the loop's start")?,
                    end: whole(stop, "the loop's stop")?,
                };
                let at = frame_option(at)?;
                Ok(this.context.borrow_mut().control().set_loop(this.key, Some(region), at)?)
            },
        )
        .untyped();
    voice
        .method(
            "place",
            |this: &LuaAudioVoice, call: &Call<'_>, place: ValueView<'_>, at: Option<f64>| -> l3i::Result<bool> {
                let at = frame_option(at)?;
                let Some(base) = this.context.borrow_mut().control().emitter(this.key) else {
                    return Ok(false);
                };
                let moved = call.with_frame(|frame| emitter(frame, place, base, "AudioVoice:place"))?;
                Ok(this.context.borrow_mut().control().place(this.key, moved, at)?)
            },
        )
        .untyped();
    voice
        .method("release", |this: &LuaAudioVoice, at: Option<f64>| -> l3i::Result<bool> {
            let at = frame_option(at)?;
            Ok(this.context.borrow_mut().control().set_loop(this.key, None, at)?)
        })
        .untyped();
    voice
        .method(
            "send",
            |this: &LuaAudioVoice, bus: ValueView<'_>, level: f64, at: Option<f64>| -> l3i::Result<bool> {
                let bus = bus_of(bus, &this.context, "AudioVoice:send's bus")?;
                let at = frame_option(at)?;
                Ok(this.context.borrow_mut().control().set_send(this.key, bus, level, at)?)
            },
        )
        .untyped();
    voice
        .method("state", |this: &LuaAudioVoice| {
            this.context.borrow_mut().control().state(this.key).name()
        })
        .untyped();
}

#[allow(clippy::too_many_lines)] // one method per operation
fn describe_bus(d: &mut ExtensionDescriptor) {
    let mut bus = d.userdata::<LuaAudioBus>(LuaAudioBus::NAME);
    bus.tag(TagPolicy::Never)
        .doc("A bus: where voices and buses sum, runs its processors, applies its gain, and feeds its parent.");
    bus.method(
        "set",
        |this: &LuaAudioBus, name: Text, value: f64, at: Option<f64>| -> l3i::Result<bool> {
            let Some(param) = BusParam::named(&name.0) else {
                return Err(error(format!("AudioBus:set takes gain or mute, not {:?}", name.0)));
            };
            let at = frame_option(at)?;
            Ok(this
                .context
                .borrow_mut()
                .control()
                .set_bus(this.key, param, value, at, None)?)
        },
    )
    .untyped();
    bus.method(
        "ramp",
        |this: &LuaAudioBus,
         name: Text,
         value: f64,
         frames: f64,
         at: Option<f64>,
         curve: Option<Text>|
         -> l3i::Result<bool> {
            if name.0 != "gain" {
                return Err(error(format!("AudioBus:ramp ramps gain, not {:?}", name.0)));
            }
            let (at, glide) = (frame_option(at)?, glide_of(frames, curve.as_ref())?);
            Ok(this
                .context
                .borrow_mut()
                .control()
                .set_bus(this.key, BusParam::Gain, value, at, Some(glide))?)
        },
    )
    .untyped();
    bus.method(
        "route",
        |this: &LuaAudioBus, parent: ValueView<'_>, at: Option<f64>| -> l3i::Result<()> {
            let parent = bus_of(parent, &this.context, "AudioBus:route's parent")?;
            let at = frame_option(at)?;
            Ok(this.context.borrow_mut().control().route_bus(this.key, parent, at)?)
        },
    )
    .untyped();
    bus.method(
        "send",
        |this: &LuaAudioBus, call: &Call<'_>, target: ValueView<'_>, options: Option<ValueView<'_>>| -> l3i::Result<()> {
            const FUNCTION: &str = "AudioBus:send";
            let target = bus_of(target, &this.context, "AudioBus:send's target")?;
            let (level, tap, at) = call.with_frame(|frame| {
                let table = table(frame, options, FUNCTION, BUS_SEND_KEYS)?;
                let table = table.as_ref();
                let level = optional::<f64>(frame, table, "level", FUNCTION)?.unwrap_or(1.0);
                let tap = match optional::<String>(frame, table, "tap", FUNCTION)?.as_deref() {
                    None | Some("post") => Tap::PostFader,
                    Some("pre") => Tap::PreFader,
                    Some(other) => {
                        return Err(error(format!("{FUNCTION}'s tap is post or pre, not {other:?}")));
                    }
                };
                let at = frame_option(optional(frame, table, "at", FUNCTION)?)?;
                Ok((level, tap, at))
            })?;
            Ok(this
                .context
                .borrow_mut()
                .control()
                .send_bus(this.key, target, level, tap, at)?)
        },
    )
    .untyped();
    bus.method(
        "unsend",
        |this: &LuaAudioBus, target: ValueView<'_>, at: Option<f64>| -> l3i::Result<bool> {
            let target = bus_of(target, &this.context, "AudioBus:unsend's target")?;
            let at = frame_option(at)?;
            Ok(this.context.borrow_mut().control().unsend_bus(this.key, target, at)?)
        },
    )
    .untyped();
    bus.method("remove", |this: &LuaAudioBus, at: Option<f64>| -> l3i::Result<bool> {
        let at = frame_option(at)?;
        Ok(this.context.borrow_mut().control().remove_bus(this.key, at)?)
    })
    .untyped();
    bus.method("live", |this: &LuaAudioBus| {
        this.context.borrow_mut().control().bus_live(this.key)
    })
    .untyped();
    bus.method("latency", |this: &LuaAudioBus| -> l3i::Result<f64> {
        Ok(number(this.context.borrow_mut().control().latency(this.key)?))
    })
    .untyped();
    bus.method("meter", |this: &LuaAudioBus, call: &Call<'_>| -> l3i::Result<StackResults> {
        let meter = this.context.borrow_mut().control().meter(this.key)?;
        push_table(call, 0, 2, |frame, table| {
            table.raw_set_value(frame, "peak", &f64::from(meter.peak))?;
            table.raw_set_value(frame, "rms", &f64::from(meter.rms))
        })?;
        Ok(StackResults)
    })
    .untyped();
    bus.method(
        "insert",
        |this: &LuaAudioBus,
         call: &Call<'_>,
         name: Text,
         options: Option<ValueView<'_>>|
         -> l3i::Result<Owned<LuaAudioProcessor>> {
            let (settings, index) = call.with_frame(|frame| settings(frame, options, "AudioBus:insert"))?;
            let mut context = this.context.borrow_mut();
            let control = context.control();
            let processor = crate::audio::processor::create(&name.0, control.rate(), control.channels(), settings)?;
            let kind = processor.kind();
            let key = control.mount(processor, this.key, index)?;
            drop(context);
            Ok(Owned(LuaAudioProcessor {
                context: Rc::clone(&this.context),
                key,
                kind,
            }))
        },
    )
    .untyped();
}

fn describe_processor(d: &mut ExtensionDescriptor) {
    let mut processor = d.userdata::<LuaAudioProcessor>(LuaAudioProcessor::NAME);
    processor
        .tag(TagPolicy::Never)
        .doc("A processor in a bus's chain; once removed, the handle names nothing.");
    processor.method("kind", |this: &LuaAudioProcessor| this.kind).untyped();
    processor
        .method(
            "set",
            |this: &LuaAudioProcessor, name: Text, value: f64, at: Option<f64>| -> l3i::Result<bool> {
                let at = frame_option(at)?;
                Ok(this
                    .context
                    .borrow_mut()
                    .control()
                    .set_processor(this.key, &name.0, value, at, None)?)
            },
        )
        .untyped();
    processor
        .method(
            "ramp",
            |this: &LuaAudioProcessor,
             name: Text,
             value: f64,
             frames: f64,
             at: Option<f64>,
             curve: Option<Text>|
             -> l3i::Result<bool> {
                let (at, glide) = (frame_option(at)?, glide_of(frames, curve.as_ref())?);
                Ok(this
                    .context
                    .borrow_mut()
                    .control()
                    .set_processor(this.key, &name.0, value, at, Some(glide))?)
            },
        )
        .untyped();
    processor
        .method(
            "listen",
            |this: &LuaAudioProcessor, key: Option<ValueView<'_>>, at: Option<f64>| -> l3i::Result<bool> {
                let key = key
                    .filter(|key| !key.is_nil())
                    .map(|key| bus_of(key, &this.context, "AudioProcessor:listen's bus"))
                    .transpose()?;
                let at = frame_option(at)?;
                Ok(this.context.borrow_mut().control().listen(this.key, key, at)?)
            },
        )
        .untyped();
    processor
        .method("remove", |this: &LuaAudioProcessor, at: Option<f64>| -> l3i::Result<bool> {
            let at = frame_option(at)?;
            Ok(this.context.borrow_mut().control().unmount(this.key, at)?)
        })
        .untyped();
    processor
        .method("live", |this: &LuaAudioProcessor| {
            this.context.borrow_mut().control().processor_live(this.key)
        })
        .untyped();
}
