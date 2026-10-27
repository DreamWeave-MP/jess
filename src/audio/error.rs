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
    /// Every command the engine can hold for a later frame is waiting for its frame. Commands
    /// for now still go through.
    ScheduleFull(usize),
    /// The engine is gone (its device closed, or it was dropped).
    Disconnected,
    /// A voice handle from a voice that already ended, or another context.
    StaleVoice,
    /// A node handle from a node that was removed, or another context, or a node that can't
    /// take what was asked of it (a voice into a node without an audio input).
    StaleNode,
    /// Every node slot is taken.
    NodesExhausted(usize),
    /// Every connection the context has room for is made.
    ConnectionsExhausted(usize),
    /// A node still fed can't be removed.
    NodeInUse {
        /// Voices playing into it, or waiting to start there.
        voices: usize,
        /// Connections reaching its audio inputs.
        nodes: usize,
    },
    /// The routing would feed a node back into itself without a feedback delay on the way.
    Cycle,
    /// A port or parameter name a node doesn't have.
    UnknownPort {
        /// The node's kind.
        kind: &'static str,
        /// The name asked for.
        name: String,
        /// The names it has.
        known: String,
    },
    /// An inserted processor's output is its host's, and its main input is the chain's.
    Inserted,
    /// The node can't go in an insert chain: it is wired, of another shape or width than a chain
    /// holds, or in a chain already.
    NotInsertable,
    /// The sidechain already hears another connection.
    SidechainTaken,
    /// A revision stamped before one still waiting for its frame: revisions take effect in the
    /// order they are made.
    GraphOrder {
        /// The frame the waiting revision takes effect on.
        pending: u64,
    },
    /// Something the engine doesn't do, and why.
    Unsupported(&'static str),
    /// A processor that can't be a node.
    Shape {
        /// Its kind.
        kind: &'static str,
        /// What is wrong with it.
        problem: &'static str,
    },
    /// A processor kind registered twice.
    KindTaken(String),
    /// A bus or voice already has as many sends as it can.
    SendsExhausted(usize),
    /// The audio device couldn't be opened or started, or this jess has none.
    Device(String),
    /// Another thread is writing the stream: a stream has one writer at a time.
    StreamBusy,
    /// The stream already has a voice playing it.
    StreamClaimed,
    /// A group was opened inside another.
    GroupOpen,
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
    /// The master bus is the output: it has no route and stays.
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
            AudioError::ScheduleFull(capacity) => write!(
                f,
                "all {capacity} commands the engine can hold for later are waiting for their frames"
            ),
            AudioError::Disconnected => f.write_str("the audio engine is gone: its output was closed"),
            AudioError::StaleVoice => f.write_str("the voice has ended or belongs to another context"),
            AudioError::StaleNode => {
                f.write_str("the node was removed, belongs to another context, or has no audio input to play into")
            }
            AudioError::NodesExhausted(nodes) => write!(f, "all {nodes} nodes are in use"),
            AudioError::ConnectionsExhausted(count) => write!(f, "all {count} connections are made"),
            AudioError::NodeInUse { voices, nodes } => write!(
                f,
                "the node still has {voices} voices and {nodes} connections feeding it; stop or reroute them first"
            ),
            AudioError::Cycle => f.write_str(
                "that routing would feed a node back into itself; a loop needs a feedback delay on one of its connections",
            ),
            AudioError::UnknownPort { kind, name, known } => {
                write!(f, "a {kind} node has no port or parameter {name:?}; it has {known}")
            }
            AudioError::Inserted => f.write_str(
                "an inserted processor's output is its host's and its input is the chain's; connect the host instead",
            ),
            AudioError::NotInsertable => f.write_str(
                "only an unconnected processor with one input and one output, as wide as the host, goes in an insert chain",
            ),
            AudioError::SidechainTaken => f.write_str("the sidechain already hears another node; disconnect it first"),
            AudioError::GraphOrder { pending } => write!(
                f,
                "a graph change waits for frame {pending}; changes take effect in the order they're made, so stamp this one for that frame or later"
            ),
            AudioError::Unsupported(what) => write!(f, "unsupported: {what}"),
            AudioError::Shape { kind, problem } => write!(f, "a {kind} processor can't be a node: it has {problem}"),
            AudioError::KindTaken(kind) => write!(f, "a processor kind {kind:?} is already registered"),
            AudioError::SendsExhausted(sends) => write!(f, "it already has all {sends} of its sends"),
            AudioError::Device(problem) => f.write_str(problem),
            AudioError::StreamBusy => f.write_str("another thread is writing the stream; a stream has one writer at a time"),
            AudioError::StreamClaimed => f.write_str("the stream already has a voice playing it; a stream plays once"),
            AudioError::GroupOpen => f.write_str("a group of commands is already open; groups don't nest"),
            AudioError::UnknownProcessor { kind, known } => {
                write!(f, "there is no {kind:?} processor; there are {known}")
            }
            AudioError::Setting { kind, name, expected } => {
                write!(f, "a {kind} processor's {name} is {expected}")
            }
            AudioError::UnknownParameter { kind, name, known } => {
                write!(f, "a {kind} processor has no parameter {name:?}; it has {known}")
            }
            AudioError::Master => f.write_str("the master bus is the output: it has no route and can't be removed"),
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
