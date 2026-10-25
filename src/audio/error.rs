//! Why an audio call was refused.

use std::fmt;

use crate::audio::SampleFormat;

/// Why making PCM, configuring a context, or submitting a command failed.
///
/// Nothing in the engine is refused silently: a full queue, a stale voice, and a parameter out
/// of range are each an error the caller sees.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioError {
    /// A sample rate of zero or past [`MAX_RATE`](crate::audio::MAX_RATE).
    Rate(u32),
    /// A channel count of zero or past [`MAX_CHANNELS`](crate::audio::MAX_CHANNELS).
    Channels(usize),
    /// Interleaved bytes that aren't a whole number of frames.
    PartialFrame {
        /// How many bytes there were.
        bytes: usize,
        /// How many bytes one frame takes.
        frame: usize,
        /// The format they were read as.
        format: SampleFormat,
    },
    /// A sample format name nothing reads.
    UnknownFormat(String),
    /// A voice count of zero or past [`MAX_VOICES`](crate::audio::MAX_VOICES), or a queue
    /// capacity of zero.
    Capacity(&'static str, usize),
    /// Every voice slot is taken.
    VoicesExhausted(usize),
    /// The command queue to the engine is full: the engine is behind, or isn't running.
    QueueFull(usize),
    /// The engine is gone (its device closed, or it was dropped).
    Disconnected,
    /// A voice handle from a voice that already ended, or another context.
    StaleVoice,
    /// A bus handle from a bus that was removed, or another context.
    StaleBus,
    /// Every bus slot is taken.
    BusesExhausted(usize),
    /// A bus still being routed to can't be removed.
    BusInUse {
        /// Voices playing into it, or waiting to start there.
        voices: usize,
        /// Buses summing into it.
        buses: usize,
    },
    /// The routing would feed a bus back into itself.
    Cycle,
    /// A bus or voice already has as many sends as it can.
    SendsExhausted(usize),
    /// A group was opened inside another.
    GroupOpen,
    /// Every processor slot is taken.
    ProcessorsExhausted(usize),
    /// A bus already runs as many processors as it can.
    ChainFull(usize),
    /// A processor handle from one that was removed, or another context.
    StaleProcessor,
    /// A processor kind there isn't.
    UnknownProcessor {
        /// The kind asked for.
        kind: String,
        /// The kinds there are.
        known: String,
    },
    /// A processor setting of the wrong type.
    Setting {
        /// The processor's kind.
        kind: &'static str,
        /// The setting.
        name: String,
        /// What it should be.
        expected: &'static str,
    },
    /// A parameter name a processor doesn't have.
    UnknownParameter {
        /// The processor's kind.
        kind: &'static str,
        /// The name asked for.
        name: String,
        /// The names it has.
        known: String,
    },
    /// The master bus is the output: it has no parent and stays.
    Master,
    /// A source whose channels the context can't mix into its output.
    Layout {
        /// The source's channels.
        source: usize,
        /// The output's channels.
        output: usize,
    },
    /// A parameter value out of its range, or not finite.
    Parameter {
        /// The parameter.
        name: &'static str,
        /// The value refused.
        value: f64,
        /// What it may be.
        expected: &'static str,
    },
    /// A loop region outside the source, or empty.
    LoopRegion {
        /// The region's first frame.
        start: u64,
        /// The frame after its last.
        end: u64,
        /// The source's length in frames.
        frames: u64,
    },
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::Rate(rate) => write!(
                f,
                "a sample rate is 1 to {} frames per second, not {rate}",
                crate::audio::MAX_RATE
            ),
            AudioError::Channels(channels) => {
                write!(f, "a channel count is 1 to {}, not {channels}", crate::audio::MAX_CHANNELS)
            }
            AudioError::PartialFrame { bytes, frame, format } => {
                write!(f, "{bytes} bytes of {format} aren't whole frames of {frame} bytes")
            }
            AudioError::UnknownFormat(name) => write!(
                f,
                "no sample format is called {name:?}; there are u8, s16, s24, s32, f32, and f64"
            ),
            AudioError::Capacity(what, value) => write!(f, "{what} can't be {value}"),
            AudioError::VoicesExhausted(voices) => {
                write!(f, "all {voices} voices are playing or scheduled")
            }
            AudioError::QueueFull(capacity) => write!(
                f,
                "the audio command queue is full ({capacity} commands): the engine is behind or not running"
            ),
            AudioError::Disconnected => f.write_str("the audio engine is gone"),
            AudioError::StaleVoice => f.write_str("the voice has ended or belongs to another context"),
            AudioError::StaleBus => f.write_str("the bus was removed or belongs to another context"),
            AudioError::BusesExhausted(buses) => write!(f, "all {buses} buses are in use"),
            AudioError::BusInUse { voices, buses } => write!(
                f,
                "the bus still has {voices} voices and {buses} buses routed into it; stop or reroute them first"
            ),
            AudioError::Cycle => f.write_str("that routing would feed a bus back into itself"),
            AudioError::SendsExhausted(sends) => write!(f, "it already has all {sends} of its sends"),
            AudioError::GroupOpen => f.write_str("a group of commands is already open; groups don't nest"),
            AudioError::ProcessorsExhausted(count) => write!(f, "all {count} processors are in use"),
            AudioError::ChainFull(count) => write!(f, "the bus already runs {count} processors"),
            AudioError::StaleProcessor => f.write_str("the processor was removed or belongs to another context"),
            AudioError::UnknownProcessor { kind, known } => {
                write!(f, "there is no {kind:?} processor; there are {known}")
            }
            AudioError::Setting { kind, name, expected } => {
                write!(f, "a {kind} processor's {name} is {expected}")
            }
            AudioError::UnknownParameter { kind, name, known } => {
                write!(f, "a {kind} processor has no parameter {name:?}; it has {known}")
            }
            AudioError::Master => f.write_str("the master bus is the output: it has no parent and can't be removed"),
            AudioError::Layout { source, output } => write!(
                f,
                "a {source}-channel source doesn't mix into a {output}-channel output; mono goes anywhere, stereo goes to mono, and otherwise the counts must match"
            ),
            AudioError::Parameter { name, value, expected } => {
                write!(f, "{name} must be {expected}, not {value}")
            }
            AudioError::LoopRegion { start, end, frames } => write!(
                f,
                "the loop region {start}..{end} isn't a non-empty range inside the source's {frames} frames"
            ),
        }
    }
}

impl std::error::Error for AudioError {}
