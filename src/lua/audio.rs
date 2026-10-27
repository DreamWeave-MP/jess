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
use crate::audio::processor::{Setting, Settings, Setup};
use crate::audio::{
    AudioConfig, AudioError, BusSpec, Control, Culled, Curve, Decode, DeviceRequest, Distance, Emitter, Feed, FeedSpec,
    FeedWatch, Glide, Handedness, Listener, LiveContext, LoopRegion, NodeKey, OfflineContext, Pcm, PlaySpec, SampleFormat,
    Tap, VoiceKey, VoiceParam, VoiceSend,
};
use crate::lua::plan::Functions;
use crate::lua::{Bytes, Jess, Text, error, guard, kind, option_value, push_table, read_options};
use crate::runtime::ResourceHandle;
use l3i::Runtime;
use l3i::bind::{Call, StackResults};
use l3i::convert::{NewBuffer, Vector3};
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::stack::{Frame, Scope, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::{Function, Table};

/// The largest frame number a Luau number holds exactly.
const MAX_FRAME: f64 = 9_007_199_254_740_992.0;

const PCM_KEYS: &[&str] = &["channels", "format", "rate"];
const OUTPUT_KEYS: &[&str] = &[
    "channels",
    "commands",
    "connections",
    "device",
    "handedness",
    "nodes",
    "period",
    "physical",
    "rate",
    "speedOfSound",
    "voices",
];
const CONTEXT_KEYS: &[&str] = &[
    "channels",
    "commands",
    "connections",
    "handedness",
    "nodes",
    "physical",
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
const BUS_KEYS: &[&str] = &["channels", "gain", "muted", "parent"];
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

/// A context and the processor kinds the runtime that made it can make by name. It derefs to the
/// context, which is what nearly every use wants.
struct Audio {
    context: RefCell<Context>,
    processors: crate::audio::Registry,
}

impl std::ops::Deref for Audio {
    type Target = RefCell<Context>;

    fn deref(&self) -> &RefCell<Context> {
        &self.context
    }
}

type Shared = Rc<Audio>;

/// The outputs a runtime opened. Each plays until the script closes it or the runtime ends,
/// whether or not the script still holds it: a sound doesn't stop because its handles were
/// collected, and a domain's restart, which drops its runtime, stops every output it opened.
#[derive(Default)]
pub struct Outputs(RefCell<Vec<Shared>>);

impl Outputs {
    fn hold(&self, context: &Shared) {
        let mut outputs = self.0.borrow_mut();
        outputs.retain(|output| !matches!(&*output.borrow(), Context::Live(live) if live.is_closed()));
        outputs.push(Rc::clone(context));
    }
}

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
    let (mut channels, mut rate, mut seconds) = (2.0, 48_000.0, 1.0);
    call.with_frame(|frame| {
        read_table(frame, options, FUNCTION, STREAM_KEYS, |_, name, value| {
            match name {
                b"channels" => channels = option_value(value, "channels", FUNCTION)?,
                b"rate" => rate = option_value(value, "rate", FUNCTION)?,
                b"seconds" => seconds = option_value(value, "seconds", FUNCTION)?,
                _ => return Ok(false),
            }
            Ok(true)
        })
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

/// A compressed file's decoder, from l3i's `@dream/audio`, as a feed reads it.
struct Decoding(l3i::audio::Decoder);

impl Decode for Decoding {
    fn read(&mut self, out: &mut [f32]) -> Result<usize, String> {
        self.0.read(out).map_err(|problem| problem.to_string())
    }

    fn seek(&mut self, frame: u64) -> Result<(), String> {
        self.0.seek(frame).map_err(|problem| problem.to_string())
    }
}

const FEED_KEYS: &[&str] = &["block", "format", "loop", "loopStart", "seconds", "start"];

/// A feed: a compressed file decoded into a stream on a thread of its own.
#[derive(Clone)]
pub struct LuaAudioFeed {
    watch: FeedWatch,
    stream: Arc<Stream>,
}

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioFeed {
    const NAME: &'static str = "jess.audio.Feed";
}

/// The feeds a runtime started. Each runs until it ends by itself or is stopped, whether or not
/// the script still holds it; the runtime's end stops every one and waits for its thread.
#[derive(Default)]
pub struct Feeds(RefCell<Vec<Feed>>);

impl Feeds {
    fn hold(&self, feed: Feed) {
        let mut feeds = self.0.borrow_mut();
        feeds.retain(|feed| !feed.status().done);
        feeds.push(feed);
    }
}

/// `audio.feed(data, { format, seconds, block, start, loop, loopStart })`: a decoder over the
/// file, a stream of its layout, and a feed keeping the one full from the other.
fn feed(jess: &Jess, call: &Call<'_>, data: Bytes, options: Option<ValueView<'_>>) -> l3i::Result<Owned<LuaAudioFeed>> {
    const FUNCTION: &str = "jess.audio.feed";
    let (mut format, mut seconds, mut block, mut start) = (None, 1.0, FeedSpec::default().block, None);
    let (mut looping, mut loop_start) = (false, 0);
    call.with_frame(|frame| {
        read_optional(frame, options, FUNCTION, FEED_KEYS, |_, name, value| {
            match name {
                b"format" => {
                    let name: &str = option_value(value, "format", FUNCTION)?;
                    format = Some(
                        l3i::audio::Format::parse(name)
                            .ok_or_else(|| error(format!("{FUNCTION}'s format is mp3, vorbis, or opus, not {name:?}")))?,
                    );
                }
                b"seconds" => seconds = option_value(value, "seconds", FUNCTION)?,
                b"block" => block = count(option_value(value, "block", FUNCTION)?, "a feed's block")?,
                b"start" => start = Some(whole(option_value(value, "start", FUNCTION)?, "start")?),
                b"loop" => looping = option_value(value, "loop", FUNCTION)?,
                b"loopStart" => loop_start = whole(option_value(value, "loopStart", FUNCTION)?, "loopStart")?,
                _ => return Ok(false),
            }
            Ok(true)
        })
    })?;
    let spec = FeedSpec {
        block,
        looping: looping.then_some(loop_start),
    };
    let mut decoder = l3i::audio::Decoder::new(data.0, format).map_err(|problem| error(format!("{FUNCTION}: {problem}")))?;
    if let Some(start) = start {
        decoder
            .seek(start)
            .map_err(|problem| error(format!("{FUNCTION}: {problem}")))?;
    }
    let stream = Stream::new(decoder.channels(), decoder.rate(), seconds)?;
    let feed = Feed::start(Box::new(Decoding(decoder)), Arc::clone(&stream), spec)?;
    let watch = feed.watch();
    jess.feeds.hold(feed);
    Ok(Owned(LuaAudioFeed { watch, stream }))
}

fn describe_feed(d: &mut ExtensionDescriptor) {
    let mut feed = d.userdata::<LuaAudioFeed>(LuaAudioFeed::NAME);
    feed.tag(TagPolicy::Never)
        .doc("A compressed file decoded into a stream on a thread of its own.");
    feed.method("stream", |this: &LuaAudioFeed| {
        Owned(LuaAudioStream(Arc::clone(&this.stream)))
    })
    .untyped();
    feed.method(
        "status",
        |this: &LuaAudioFeed, call: &Call<'_>| -> l3i::Result<StackResults> {
            let status = this.watch.status();
            push_table(call, 0, 3, |frame, table| {
                table.raw_set_value(frame, "done", &status.done)?;
                table.raw_set_value(frame, "loops", &number(status.loops))?;
                if let Some(problem) = &status.error {
                    table.raw_set_value(frame, "error", problem.as_str())?;
                }
                Ok(())
            })?;
            Ok(StackResults)
        },
    )
    .untyped();
    feed.method("stop", |this: &LuaAudioFeed| this.watch.stop()).untyped();
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

/// One node of a context: a bus, or a processor.
#[derive(Clone)]
pub struct LuaAudioNode {
    context: Shared,
    key: NodeKey,
    kind: &'static str,
}

// SAFETY: shared Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaAudioNode {
    const NAME: &'static str = "jess.audio.Node";
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

/// The options table a function was given, if it was given one, read in one walk.
fn read_optional(
    frame: &Frame<'_>,
    options: Option<ValueView<'_>>,
    function: &str,
    keys: &[&str],
    visit: impl FnMut(&Frame<'_>, &[u8], ValueView<'_>) -> l3i::Result<bool>,
) -> l3i::Result<()> {
    let Some(options) = options.filter(|options| !options.is_nil()) else {
        return Ok(());
    };
    read_table(frame, options, function, keys, visit)
}

/// A table of options that must be there, read in one walk.
fn read_table(
    frame: &Frame<'_>,
    options: ValueView<'_>,
    function: &str,
    keys: &[&str],
    visit: impl FnMut(&Frame<'_>, &[u8], ValueView<'_>) -> l3i::Result<bool>,
) -> l3i::Result<()> {
    if !options.is_table() {
        return Err(error(format!("{function} takes a table of options, not {}", kind(options))));
    }
    read_options(frame, &options.as_table()?, function, keys, visit)
}

/// The node a value is, which must belong to `context`.
fn node_of(value: ValueView<'_>, context: &Shared, what: &str) -> l3i::Result<NodeKey> {
    let Some(node) = l3i::userdata::receiver::<LuaAudioNode>(value) else {
        return Err(error(format!("{what} is an AudioNode, not {}", kind(value))));
    };
    if !Rc::ptr_eq(&node.context, context) {
        return Err(AudioError::StaleNode.into());
    }
    Ok(node.key)
}

fn vector(value: ValueView<'_>, key: &str, function: &str) -> l3i::Result<[f64; 3]> {
    let value: Vector3 = option_value(value, key, function)?;
    Ok([f64::from(value.x), f64::from(value.y), f64::from(value.z)])
}

/// `base` with the fields of a place's options table changed: vectors for position, velocity,
/// and direction, a cone in degrees, a distance model by name, and its numbers.
fn emitter(frame: &Frame<'_>, options: ValueView<'_>, base: Emitter, function: &str) -> l3i::Result<Emitter> {
    let mut emitter = base;
    read_table(frame, options, function, PLACE_KEYS, |frame, name, value| {
        match name {
            b"position" => emitter.position = vector(value, "position", function)?,
            b"velocity" => emitter.velocity = vector(value, "velocity", function)?,
            b"direction" => emitter.direction = Some(vector(value, "direction", function)?),
            b"distance" => {
                let model: &str = option_value(value, "distance", function)?;
                emitter.distance = Distance::named(model).ok_or_else(|| {
                    error(format!(
                        "{function}'s distance is none, inverse, linear, or exponential, not {model:?}"
                    ))
                })?;
            }
            b"reference" => emitter.reference = option_value(value, "reference", function)?,
            b"maximum" => emitter.maximum = option_value(value, "maximum", function)?,
            b"rolloff" => emitter.rolloff = option_value(value, "rolloff", function)?,
            b"doppler" => emitter.doppler = option_value(value, "doppler", function)?,
            b"gain" => emitter.gain = option_value(value, "gain", function)?,
            b"lowpass" => emitter.lowpass = option_value(value, "lowpass", function)?,
            b"cone" => read_table(frame, value, function, CONE_KEYS, |_, name, value| {
                match name {
                    b"inner" => emitter.inner = option_value::<f64>(value, "inner", function)?.to_radians(),
                    b"outer" => emitter.outer = option_value::<f64>(value, "outer", function)?.to_radians(),
                    b"outside" => emitter.outside = option_value(value, "outside", function)?,
                    _ => return Ok(false),
                }
                Ok(true)
            })?,
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(emitter)
}

/// `base` with the fields of a listener's options table changed.
fn listener(frame: &Frame<'_>, options: ValueView<'_>, base: Listener, function: &str) -> l3i::Result<Listener> {
    let mut listener = base;
    read_table(frame, options, function, LISTENER_KEYS, |_, name, value| {
        match name {
            b"position" => listener.position = vector(value, "position", function)?,
            b"forward" => listener.forward = vector(value, "forward", function)?,
            b"up" => listener.up = vector(value, "up", function)?,
            b"velocity" => listener.velocity = vector(value, "velocity", function)?,
            _ => return Ok(false),
        }
        Ok(true)
    })?;
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
    let (mut format, mut channels, mut rate) = (None, None, None);
    call.with_frame(|frame| {
        read_optional(frame, options, FUNCTION, PCM_KEYS, |_, name, value| {
            match name {
                b"format" => format = Some(option_value::<&str>(value, "format", FUNCTION)?.parse::<SampleFormat>()?),
                b"channels" => channels = Some(option_value::<f64>(value, "channels", FUNCTION)?),
                b"rate" => rate = Some(option_value::<f64>(value, "rate", FUNCTION)?),
                _ => return Ok(false),
            }
            Ok(true)
        })
    })?;
    let (Some(format), Some(channels), Some(rate)) = (format, channels, rate) else {
        return Err(error(format!("{FUNCTION} needs format, channels, and rate")));
    };
    let pcm = Pcm::from_interleaved(&data.0, format, count(channels, "channels")?, rate_of(rate)?)?;
    Ok(Owned(LuaAudioPcm(Arc::new(pcm))))
}

/// `audio.offline({ rate, channels, voices, buses, processors, commands, ... })`.
fn offline(jess: &Jess, call: &Call<'_>, options: Option<ValueView<'_>>) -> l3i::Result<Owned<LuaAudioContext>> {
    let (config, _) = context_config(call, options, "jess.audio.offline", CONTEXT_KEYS)?;
    let context = OfflineContext::new(&config)?;
    Ok(Owned(LuaAudioContext(Rc::new(Audio {
        context: RefCell::new(Context::Offline(Box::new(context))),
        processors: jess.processors.borrow().clone(),
    }))))
}

/// `audio.output({ device, rate, period, channels, voices, ... })`: the platform's default output,
/// or the null device, rendering a context the script drives like any other.
fn output(jess: &Jess, call: &Call<'_>, options: Option<ValueView<'_>>) -> l3i::Result<Owned<LuaAudioContext>> {
    const FUNCTION: &str = "jess.audio.output";
    guard(jess, FUNCTION, crate::audio::capability::OUTPUT)?;
    let (config, request) = context_config(call, options, FUNCTION, OUTPUT_KEYS)?;
    let live = request.open(&config)?;
    let context = Rc::new(Audio {
        context: RefCell::new(Context::Live(Box::new(live))),
        processors: jess.processors.borrow().clone(),
    });
    jess.outputs.hold(&context);
    Ok(Owned(LuaAudioContext(context)))
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
        read_optional(frame, options, function, keys, |_, name, value| {
            let number = |key| option_value::<f64>(value, key, function);
            match name {
                b"rate" => {
                    config.rate = rate_of(number("rate")?)?;
                    request.rate = Some(config.rate);
                }
                b"period" if live => {
                    request.period = Some(u32::try_from(whole(number("period")?, "period")?).unwrap_or(u32::MAX));
                }
                b"device" if live => {
                    request.null = match option_value::<&str>(value, "device", function)? {
                        "default" => false,
                        "null" => true,
                        other => return Err(error(format!("{function}'s device is default or null, not {other:?}"))),
                    };
                }
                b"channels" => config.channels = count(number("channels")?, "channels")?,
                b"voices" => config.voices = count(number("voices")?, "voices")?,
                b"nodes" => config.nodes = count(number("nodes")?, "nodes")?,
                b"connections" => config.connections = count(number("connections")?, "connections")?,
                b"commands" => config.commands = count(number("commands")?, "commands")?,
                b"physical" => config.physical = Some(count(number("physical")?, "physical")?),
                b"speedOfSound" => config.speed_of_sound = number("speedOfSound")?,
                b"handedness" => {
                    config.handedness = match option_value::<&str>(value, "handedness", function)? {
                        "right" => Handedness::Right,
                        "left" => Handedness::Left,
                        other => {
                            return Err(error(format!("{function}'s handedness is right or left, not {other:?}")));
                        }
                    };
                }
                _ => return Ok(false),
            }
            Ok(true)
        })
    })?;
    Ok((config, request))
}

/// `context:play(pcm, { at, bus, sends, gain, pan, rate, offset, loop, loopStart, loopEnd })`.
#[allow(clippy::cast_possible_truncation)] // Gain and pan are f32 in the engine; range is checked there.
fn play_spec(call: &Call<'_>, context: &Shared, frames: u64, options: Option<ValueView<'_>>) -> l3i::Result<PlaySpec> {
    const FUNCTION: &str = "AudioContext:play";
    let mut spec = PlaySpec::default();
    let (mut looping, mut start, mut end) = (false, None, None);
    call.with_frame(|frame| {
        read_optional(frame, options, FUNCTION, PLAY_KEYS, |frame, name, value| {
            match name {
                b"at" => spec.at = Some(whole(option_value(value, "at", FUNCTION)?, "at")?),
                b"gain" => spec.gain = VoiceParam::Gain.check(option_value(value, "gain", FUNCTION)?)? as f32,
                b"pan" => spec.pan = VoiceParam::Pan.check(option_value(value, "pan", FUNCTION)?)? as f32,
                b"rate" => spec.rate = option_value(value, "rate", FUNCTION)?,
                b"offset" => spec.offset = whole(option_value(value, "offset", FUNCTION)?, "offset")?,
                b"bus" => spec.bus = node_of(value, context, "bus")?,
                b"sends" => spec.sends = voice_sends(frame, value, context)?,
                b"fade" => spec.fade = whole(option_value(value, "fade", FUNCTION)?, "fade")?,
                b"priority" => {
                    let priority: f64 = option_value(value, "priority", FUNCTION)?;
                    if !priority.is_finite() {
                        return Err(error(format!("{FUNCTION}'s priority is a finite number, not {priority}")));
                    }
                    spec.priority = priority as f32;
                }
                b"culled" => {
                    let culled: &str = option_value(value, "culled", FUNCTION)?;
                    spec.culled = Culled::named(culled)
                        .ok_or_else(|| error(format!("{FUNCTION}'s culled is advance, pause, or stop, not {culled:?}")))?;
                }
                b"place" => spec.spatial = Some(emitter(frame, value, Emitter::default(), "AudioContext:play's place")?),
                b"loop" => looping = option_value(value, "loop", FUNCTION)?,
                b"loopStart" => start = Some(whole(option_value(value, "loopStart", FUNCTION)?, "loopStart")?),
                b"loopEnd" => end = Some(whole(option_value(value, "loopEnd", FUNCTION)?, "loopEnd")?),
                _ => return Ok(false),
            }
            Ok(true)
        })
    })?;
    if !looping && (start.is_some() || end.is_some()) {
        return Err(error(format!("{FUNCTION}: loopStart and loopEnd need loop = true")));
    }
    if looping {
        spec.looping = Some(LoopRegion {
            start: start.unwrap_or(0),
            end: end.unwrap_or(frames),
        });
    }
    Ok(spec)
}

/// `sends = { { bus, level }, ... }` of a play's options.
#[allow(clippy::cast_possible_truncation)] // Levels are f32 in the engine.
fn voice_sends(frame: &Frame<'_>, list: ValueView<'_>, context: &Shared) -> l3i::Result<Vec<VoiceSend>> {
    const FUNCTION: &str = "AudioContext:play's sends";
    let mut sends = Vec::new();
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
            let (mut bus, mut level) = (None, 1.0);
            read_table(frame, entry, FUNCTION, SEND_KEYS, |_, name, value| {
                match name {
                    b"bus" => bus = Some(node_of(value, context, "bus")?),
                    b"level" => level = option_value(value, "level", FUNCTION)?,
                    _ => return Ok(false),
                }
                Ok(true)
            })?;
            let Some(bus) = bus else {
                return Err(error(format!("{FUNCTION}: each send names its bus")));
            };
            sends.push(VoiceSend {
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
}

/// `context:bus({ parent, gain, muted, channels })`.
#[allow(clippy::cast_possible_truncation)] // Gain is f32 in the engine, ranged there.
fn bus_spec(call: &Call<'_>, context: &Shared, options: Option<ValueView<'_>>) -> l3i::Result<BusSpec> {
    const FUNCTION: &str = "AudioContext:bus";
    let mut spec = BusSpec::default();
    call.with_frame(|frame| {
        read_optional(frame, options, FUNCTION, BUS_KEYS, |_, name, value| {
            match name {
                b"parent" => spec.parent = node_of(value, context, "parent")?,
                b"gain" => spec.gain = option_value::<f64>(value, "gain", FUNCTION)? as f32,
                b"muted" => spec.muted = option_value(value, "muted", FUNCTION)?,
                b"channels" => spec.channels = Some(count(option_value(value, "channels", FUNCTION)?, "channels")?),
                _ => return Ok(false),
            }
            Ok(true)
        })
    })?;
    Ok(spec)
}

/// How a script makes a processor: its settings, and where it goes.
struct Making {
    /// Every key of the options table but these, a number or a name.
    settings: Settings,
    /// `index`: its place in an insert chain, from 1 in the table.
    index: Option<usize>,
    /// `channels`: fewer than the context's.
    channels: Option<usize>,
}

/// A processor's settings, from its options table.
fn settings(frame: &Frame<'_>, options: Option<ValueView<'_>>, function: &str) -> l3i::Result<Making> {
    let mut making = Making {
        settings: Settings::new(),
        index: None,
        channels: None,
    };
    let Some(options) = options.filter(|options| !options.is_nil()) else {
        return Ok(making);
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
                making.index = Some(count(position - 1.0, "index")?);
            } else {
                problem = Some(error(format!("{function}'s index is a position in the chain from 1")));
            }
            return Ok(());
        }
        if name == "channels" {
            let channels = if value.is_number() { value.read::<f64>()? } else { 0.0 };
            making.channels = Some(count(channels, "channels")?);
            return Ok(());
        }
        if value.is_number() {
            making.settings.insert(&name, Setting::Number(value.read::<f64>()?));
        } else if value.is_string() {
            let text = String::from_utf8_lossy(value.read::<&[u8]>()?).into_owned();
            making.settings.insert(&name, Setting::Text(text));
        } else {
            problem = Some(error(format!(
                "{function}'s {name} is a number or a name, not {}",
                kind(value)
            )));
        }
        Ok(())
    })?;
    problem.map_or(Ok(making), Err)
}

fn interleaved_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|sample| sample.to_le_bytes()).collect()
}

fn node_handle(context: &Shared, key: NodeKey) -> Owned<LuaAudioNode> {
    let kind = context.borrow_mut().control().describe(key).map_or("", |(kind, _)| kind);
    Owned(LuaAudioNode {
        context: Rc::clone(context),
        key,
        kind,
    })
}

/// A processor made for a script, the channels it asked for, and where in a chain it goes.
type Made = (Box<dyn crate::audio::Processor>, Option<usize>, Option<usize>);

/// Makes a processor of `kind` through the runtime's registry, for a node of `channels` (the
/// context's, unless fewer) in `context`.
fn make(context: &Shared, kind: &str, making: Making) -> l3i::Result<Made> {
    let mut shared = context.borrow_mut();
    let control = shared.control();
    let setup = Setup {
        rate: control.rate(),
        channels: making.channels.unwrap_or_else(|| control.channels()),
    };
    drop(shared);
    let processor = context.processors.create(kind, &setup, making.settings)?;
    Ok((processor, making.channels, making.index))
}

pub fn module(runtime: &Runtime, jess: &Rc<Jess>) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "audio")?;
    module.function("pcm", |call: &Call<'_>, data: Bytes, options: Option<ValueView<'_>>| {
        pcm(call, &data, options)
    })?;
    let rendering = Rc::clone(jess);
    module.function("offline", move |call: &Call<'_>, options: Option<ValueView<'_>>| {
        offline(&rendering, call, options)
    })?;
    let state = Rc::clone(jess);
    module.function("output", move |call: &Call<'_>, options: Option<ValueView<'_>>| {
        output(&state, call, options)
    })?;
    module.function("stream", |call: &Call<'_>, options: ValueView<'_>| stream(call, options))?;
    let feeding = Rc::clone(jess);
    module.function("feed", move |call: &Call<'_>, data: Bytes, options: Option<ValueView<'_>>| {
        feed(&feeding, call, data, options)
    })?;
    let kinds = Rc::clone(jess);
    module.function("processors", move |call: &Call<'_>| -> l3i::Result<StackResults> {
        let registry = kinds.processors.borrow();
        l3i::stack::push_list(call, registry.kinds(), |frame, kind| {
            frame.push(kind)?;
            Ok(())
        })?;
        Ok(StackResults)
    })?;
    module.finish()
}

/// Declares the PCM, context, voice, node, stream, and feed types.
pub fn describe(d: &mut ExtensionDescriptor) {
    describe_pcm(d);
    describe_context(d);
    describe_voice(d);
    describe_node(d);
    describe_stream(d);
    describe_feed(d);
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
        .method("master", |this: &LuaAudioContext| node_handle(&this.0, NodeKey::MASTER))
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
                let (info, stats, closed) = (live.info().clone(), live.stats(), live.is_closed());
                push_table(call, 0, 11, |frame, table| {
                    table.raw_set_value(frame, "backend", info.backend.as_str())?;
                    table.raw_set_value(frame, "rate", &f64::from(info.rate))?;
                    table.raw_set_value(frame, "channels", &number(info.channels as u64))?;
                    table.raw_set_value(frame, "period", &f64::from(info.period))?;
                    table.raw_set_value(frame, "periods", &f64::from(info.periods))?;
                    table.raw_set_value(frame, "latency", &number(info.latency()))?;
                    table.raw_set_value(frame, "stopped", &number(stats.stopped))?;
                    table.raw_set_value(frame, "rerouted", &number(stats.rerouted))?;
                    table.raw_set_value(frame, "interrupted", &number(stats.interrupted))?;
                    table.raw_set_value(frame, "failed", &stats.failed)?;
                    table.raw_set_value(frame, "closed", &closed)
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    context
        .method("close", |this: &LuaAudioContext| -> l3i::Result<()> {
            let Context::Live(live) = &mut *this.0.borrow_mut() else {
                return Err(error(
                    "AudioContext:close closes an output; an offline context goes when nothing holds it",
                ));
            };
            live.close();
            Ok(())
        })
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
            |this: &LuaAudioContext, call: &Call<'_>, options: Option<ValueView<'_>>| -> l3i::Result<Owned<LuaAudioNode>> {
                let spec = bus_spec(call, &this.0, options)?;
                let key = this.0.borrow_mut().control().add_bus(&spec)?;
                Ok(node_handle(&this.0, key))
            },
        )
        .untyped();
    context
        .method(
            "node",
            |this: &LuaAudioContext,
             call: &Call<'_>,
             name: Text,
             options: Option<ValueView<'_>>|
             -> l3i::Result<Owned<LuaAudioNode>> {
                let making = call.with_frame(|frame| settings(frame, options, "AudioContext:node"))?;
                let (processor, channels, _) = make(&this.0, &name.0, making)?;
                let key = this.0.borrow_mut().control().add_node(processor, channels)?;
                Ok(node_handle(&this.0, key))
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
                push_table(call, 0, 9, |frame, table| {
                    table.raw_set_value(frame, "culled", &number(stats.culled))?;
                    table.raw_set_value(frame, "revisions", &number(stats.revisions))?;
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
                let bus = node_of(bus, &this.context, "AudioVoice:send's node")?;
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
fn describe_node(d: &mut ExtensionDescriptor) {
    let mut node = d.userdata::<LuaAudioNode>(LuaAudioNode::NAME);
    node.tag(TagPolicy::Never)
        .doc("A node of a context's graph: a bus, or a processor. Once it is removed, the handle names nothing.");
    node.method("kind", |this: &LuaAudioNode| this.kind).untyped();
    node.method(
        "params",
        |this: &LuaAudioNode, call: &Call<'_>| -> l3i::Result<StackResults> {
            let params = this
                .context
                .borrow_mut()
                .control()
                .describe(this.key)
                .map_or(&[][..], |(_, params)| params);
            l3i::stack::push_list(call, params.iter(), |frame, param| {
                let table = frame.push_table(0, 6)?;
                table.raw_set_value(frame, "name", param.name)?;
                table.raw_set_value(frame, "min", &param.min)?;
                table.raw_set_value(frame, "max", &param.max)?;
                table.raw_set_value(frame, "default", &param.default)?;
                table.raw_set_value(frame, "unit", param.unit)?;
                table.raw_set_value(frame, "rate", param.rate.name())
            })?;
            Ok(StackResults)
        },
    )
    .untyped();
    node.method(
        "set",
        |this: &LuaAudioNode, name: Text, value: f64, at: Option<f64>| -> l3i::Result<bool> {
            let at = frame_option(at)?;
            Ok(this
                .context
                .borrow_mut()
                .control()
                .set_param(this.key, &name.0, value, at, None)?)
        },
    )
    .untyped();
    node.method(
        "ramp",
        |this: &LuaAudioNode,
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
                .set_param(this.key, &name.0, value, at, Some(glide))?)
        },
    )
    .untyped();
    node.method(
        "route",
        |this: &LuaAudioNode, parent: ValueView<'_>, at: Option<f64>| -> l3i::Result<()> {
            let parent = node_of(parent, &this.context, "AudioNode:route's parent")?;
            let at = frame_option(at)?;
            Ok(this.context.borrow_mut().control().route(this.key, parent, at)?)
        },
    )
    .untyped();
    node.method(
        "send",
        |this: &LuaAudioNode, call: &Call<'_>, target: ValueView<'_>, options: Option<ValueView<'_>>| -> l3i::Result<()> {
            const FUNCTION: &str = "AudioNode:send";
            let target = node_of(target, &this.context, "AudioNode:send's target")?;
            let (mut level, mut tap, mut at) = (1.0, Tap::PostFader, None);
            call.with_frame(|frame| {
                read_optional(frame, options, FUNCTION, BUS_SEND_KEYS, |_, name, value| {
                    match name {
                        b"level" => level = option_value(value, "level", FUNCTION)?,
                        b"tap" => tap = tap_of(option_value(value, "tap", FUNCTION)?, FUNCTION)?,
                        b"at" => at = Some(whole(option_value(value, "at", FUNCTION)?, "at")?),
                        _ => return Ok(false),
                    }
                    Ok(true)
                })
            })?;
            Ok(this
                .context
                .borrow_mut()
                .control()
                .send_to(this.key, target, level, tap, at)?)
        },
    )
    .untyped();
    node.method(
        "unsend",
        |this: &LuaAudioNode, target: ValueView<'_>, at: Option<f64>| -> l3i::Result<bool> {
            let target = node_of(target, &this.context, "AudioNode:unsend's target")?;
            let at = frame_option(at)?;
            Ok(this.context.borrow_mut().control().unsend(this.key, target, at)?)
        },
    )
    .untyped();
    node.method(
        "listen",
        |this: &LuaAudioNode, key: Option<ValueView<'_>>, at: Option<f64>| -> l3i::Result<bool> {
            let key = key
                .filter(|key| !key.is_nil())
                .map(|key| node_of(key, &this.context, "AudioNode:listen's key"))
                .transpose()?;
            let at = frame_option(at)?;
            Ok(this.context.borrow_mut().control().listen(this.key, key, at)?)
        },
    )
    .untyped();
    node.method("remove", |this: &LuaAudioNode, at: Option<f64>| -> l3i::Result<bool> {
        let at = frame_option(at)?;
        Ok(this.context.borrow_mut().control().remove(this.key, at)?)
    })
    .untyped();
    node.method("live", |this: &LuaAudioNode| {
        this.context.borrow_mut().control().node_live(this.key)
    })
    .untyped();
    node.method("latency", |this: &LuaAudioNode| -> l3i::Result<f64> {
        Ok(number(this.context.borrow_mut().control().latency(this.key)?))
    })
    .untyped();
    node.method("meter", |this: &LuaAudioNode, call: &Call<'_>| -> l3i::Result<StackResults> {
        let meter = this.context.borrow_mut().control().meter(this.key)?;
        push_table(call, 0, 2, |frame, table| {
            table.raw_set_value(frame, "peak", &f64::from(meter.peak))?;
            table.raw_set_value(frame, "rms", &f64::from(meter.rms))
        })?;
        Ok(StackResults)
    })
    .untyped();
    node.method(
        "insert",
        |this: &LuaAudioNode,
         call: &Call<'_>,
         name: Text,
         options: Option<ValueView<'_>>|
         -> l3i::Result<Owned<LuaAudioNode>> {
            let making = call.with_frame(|frame| settings(frame, options, "AudioNode:insert"))?;
            let (processor, _, index) = make(&this.context, &name.0, making)?;
            let key = this.context.borrow_mut().control().insert(this.key, processor, index)?;
            Ok(node_handle(&this.context, key))
        },
    )
    .untyped();
}

/// A tap by the name scripts use.
fn tap_of(name: &str, function: &str) -> l3i::Result<Tap> {
    match name {
        "post" => Ok(Tap::PostFader),
        "pre" => Ok(Tap::PreFader),
        other => Err(error(format!("{function}'s tap is post or pre, not {other:?}"))),
    }
}
