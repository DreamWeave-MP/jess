//! Contexts: an engine and the control side that drives it.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use crate::audio::engine::{Capacity, Command, Counters, Engine, Feedback, Node, Stamped};
use crate::audio::graph::{Connection, Definition, Shape};
use crate::audio::mixer::{BusSpec, Mixer, NodeKey, Tap};
use crate::audio::param::Glide;
use crate::audio::pcm::check_layout;
use crate::audio::plan::Revision;
use crate::audio::processor::{ParamInfo, Ports, Processor, check_param, check_shape, param_index};
use crate::audio::spatial::{Emitter, Handedness, Listener, Space};
use crate::audio::stream::Stream;
use crate::audio::voice::{Output, Outputs, STREAM_WINDOW, Source, VoiceKey, check_level};
use crate::audio::{AudioError, LoopRegion, MAX_NODES, MAX_VOICE_OUTPUTS, MAX_VOICES, Pcm, PlaySpec, QUANTUM, VoiceParam};

/// How to make a context.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AudioConfig {
    /// Output frames per second.
    pub rate: u32,
    /// Output channels.
    pub channels: usize,
    /// Voices that can play or wait to start at once.
    pub voices: usize,
    /// Voices mixed at once, if fewer than `voices`: the rest are culled, by priority and then by
    /// loudness, until there is room for them.
    pub physical: Option<usize>,
    /// Nodes, buses and processors alike, the master included.
    pub nodes: usize,
    /// Connections between nodes.
    pub connections: usize,
    /// Commands that can wait for the engine at once, in the queue and again in the engine's
    /// list of commands stamped for later. A command for later past that list's room is refused
    /// with [`AudioError::ScheduleFull`]; a command for now never waits for room in it.
    pub commands: usize,
    /// The speed of sound in the world's units a second, for Doppler: 343 for metres.
    pub speed_of_sound: f64,
    /// Which way the world's axes turn.
    pub handedness: Handedness,
}

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig {
            speed_of_sound: 343.0,
            handedness: Handedness::Right,
            rate: 48_000,
            channels: 2,
            voices: 256,
            physical: None,
            nodes: 256,
            connections: 1024,
            commands: 4096,
        }
    }
}

impl AudioConfig {
    /// Why a context can't be made this way, if it can't: as [`context`].
    fn check(&self) -> Result<(), AudioError> {
        check_layout(self.channels, self.rate)?;
        if self.physical.is_some_and(|physical| physical == 0 || physical > self.voices) {
            return Err(AudioError::Capacity(
                "a context's physical voice count",
                self.physical.unwrap_or(0),
            ));
        }
        if self.voices == 0 || self.voices > MAX_VOICES {
            return Err(AudioError::Capacity("a context's voice count", self.voices));
        }
        if self.nodes == 0 || self.nodes > MAX_NODES {
            return Err(AudioError::Capacity("a context's node count", self.nodes));
        }
        if self.connections > crate::audio::MAX_CONNECTIONS {
            return Err(AudioError::Capacity("a context's connection count", self.connections));
        }
        if self.commands == 0 {
            return Err(AudioError::Capacity("a context's command capacity", 0));
        }
        if !(self.speed_of_sound > 0.0 && self.speed_of_sound.is_finite()) {
            return Err(AudioError::Parameter {
                name: "speed of sound",
                value: self.speed_of_sound,
                expected: "a finite speed above 0",
            });
        }
        Ok(())
    }
}

/// Where a voice is, as the control side last heard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceState {
    /// Stamped for a frame the engine hasn't reached.
    Scheduled,
    /// Started, and not known to have ended.
    Playing,
    /// Ran out or was stopped; its handle names nothing now.
    Ended,
}

impl VoiceState {
    /// The name scripts use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            VoiceState::Scheduled => "scheduled",
            VoiceState::Playing => "playing",
            VoiceState::Ended => "ended",
        }
    }
}

/// What the engine has done, as of its last render.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AudioStats {
    /// Output frames rendered: the audio clock.
    pub frames: u64,
    /// Voices that were playing.
    pub active: u64,
    /// Voices playing or waiting to start, from this side.
    pub allocated: u64,
    /// Commands carried out after the frame they were stamped for.
    pub late: u64,
    /// Output samples that weren't finite and were written as silence.
    pub nonfinite: u64,
    /// Commands the engine holds for a later frame.
    pub pending: u64,
    /// Voices live but not mixed in the last block, for want of room.
    pub culled: u64,
    /// Voices that ended for not being mixed, all told.
    pub stolen: u64,
    /// Graph revisions the engine carried out, all told. Playing, stopping, and changing voices
    /// revises nothing.
    pub revisions: u64,
}

/// A bus's level over a stretch of output: the peak sample's magnitude and the RMS, linear.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Meter {
    /// The largest sample's magnitude.
    pub peak: f32,
    /// The root mean square of every sample.
    pub rms: f32,
}

/// One change to the graph, for [`Control::patch`]. Ports and parameters are named as the
/// processor names them.
#[derive(Clone, PartialEq, Debug)]
pub enum Edit {
    /// Connects output `output` of `from` to input `input` of `to`: an input port, or a parameter
    /// to modulate. A connection already there takes the new level, tap, and feedback.
    Connect {
        /// The node the signal leaves.
        from: NodeKey,
        /// Which of its outputs.
        output: String,
        /// The node it reaches.
        to: NodeKey,
        /// Which of its inputs or parameters.
        input: String,
        /// Its level, tap, and feedback delay.
        connection: Connection,
    },
    /// Takes a connection away; nothing happens if there was none.
    Disconnect {
        /// The node the signal leaves.
        from: NodeKey,
        /// Which of its outputs.
        output: String,
        /// The node it reaches.
        to: NodeKey,
        /// Which of its inputs or parameters.
        input: String,
    },
    /// Routes a node into `parent`, replacing its route connection.
    Route {
        /// The node routed.
        node: NodeKey,
        /// Where it goes.
        parent: NodeKey,
    },
    /// Puts `node` in `host`'s insert chain at `index` (from 0), or last.
    Insert {
        /// The node whose chain it goes in.
        host: NodeKey,
        /// The processor inserted.
        node: NodeKey,
        /// Its place in the chain.
        index: Option<usize>,
    },
    /// Removes a node once nothing feeds it.
    Remove(NodeKey),
    /// Plays a voice into `to`'s audio input `input` (its first without one) at `level` as well
    /// as wherever it plays, or changes the level it plays there at.
    Send {
        /// The voice.
        voice: VoiceKey,
        /// The node it plays into.
        to: NodeKey,
        /// Which of the node's audio inputs.
        input: Option<String>,
        /// Its level there.
        level: f32,
    },
    /// Stops a voice playing into `to`; nothing happens if it didn't.
    Unsend {
        /// The voice.
        voice: VoiceKey,
        /// The node it stops playing into.
        to: NodeKey,
    },
}

#[derive(Clone, Debug)]
struct Slot {
    generation: u32,
    busy: bool,
    start: Option<u64>,
    /// The node inputs it plays into, as last sent: the first where it was played, the rest its
    /// sends.
    outputs: Vec<Output>,
    /// Its source's length, for checking a new loop region.
    frames: u64,
    /// Where it is in space, as last sent, if it's there.
    emitter: Option<Emitter>,
}

/// The application side of a context: it starts, stops, and changes voices, and revises the
/// graph, by sending the engine commands, and hears back what ended. It never touches the
/// engine's state.
///
/// Every command stamped for a frame the engine hasn't reached can also be refused with
/// [`AudioError::ScheduleFull`].
#[derive(Debug)]
pub struct Control {
    rate: u32,
    channels: usize,
    capacity: usize,
    commands: SyncSender<Stamped>,
    feedback: Receiver<Feedback>,
    slots: Vec<Slot>,
    /// Free slots, the next one at the end, the most recently freed first: a script that plays the
    /// same voices gets the same slots, and so the same mix order.
    free: Vec<u32>,
    graph: Definition,
    /// The latest frame a revision sent for later takes effect on: no revision may be stamped
    /// before it while it waits.
    revised: Option<u64>,
    /// The open group's commands, and the room the queue had when it opened.
    group: Option<(Vec<Stamped>, usize)>,
    /// Commands sent, counting a group as its members and one more: what the engine's count of
    /// commands received catches up to.
    sent: u64,
    sequence: u64,
    /// The frames of the commands sent for later that the engine may still be holding, earliest
    /// on top. There are never more than its list for them holds, so it never has to leave one
    /// in the queue, and a command for now is never stuck behind it.
    scheduled: BinaryHeap<Reverse<u64>>,
    counters: Arc<Counters>,
    listener: Listener,
}

/// An engine and its control side, connected; the engine goes to whatever renders it.
///
/// # Errors
///
/// [`AudioError::Rate`] or [`AudioError::Channels`] out of range, and
/// [`AudioError::Capacity`] for no voices or more than [`MAX_VOICES`], no nodes or more than
/// [`MAX_NODES`], too many connections, or no command room.
pub fn context(config: &AudioConfig) -> Result<(Control, Engine), AudioError> {
    config.check()?;
    // Built here, the first time, so no engine builds it while rendering.
    crate::audio::resample::table();
    let capacity = Capacity {
        voices: config.voices,
        nodes: config.nodes,
        commands: config.commands,
        physical: config.physical.unwrap_or(config.voices),
    };
    let (commands, receive) = sync_channel(config.commands);
    let (send, feedback) = sync_channel(capacity.feedback());
    let counters = Arc::new(Counters::new(config.nodes));
    let space = Space {
        speed: config.speed_of_sound,
        handedness: config.handedness,
        ..Space::default()
    };
    let master: Box<dyn Processor> = Box::new(Mixer::new(1.0, false));
    let graph = Definition::new(config.nodes, config.connections, config.channels, Shape::of(master.as_ref()));
    let plan = Box::new(graph.compile()?);
    let master = Box::new(Node::new(0, master, config.channels, config.channels * QUANTUM));
    let engine = Engine::new(
        config.rate,
        config.channels,
        &capacity,
        space,
        receive,
        send,
        Arc::clone(&counters),
        master,
        plan,
    );
    let slots = vec![
        Slot {
            generation: 0,
            busy: false,
            start: None,
            outputs: Vec::new(),
            frames: 0,
            emitter: None,
        };
        config.voices
    ];
    let free = (0..u32::try_from(config.voices).unwrap_or(u32::MAX)).rev().collect();
    let control = Control {
        rate: config.rate,
        channels: config.channels,
        capacity: config.commands,
        commands,
        feedback,
        slots,
        free,
        graph,
        revised: None,
        group: None,
        sent: 0,
        sequence: 0,
        scheduled: BinaryHeap::with_capacity(config.commands),
        counters,
        listener: Listener::default(),
    };
    Ok((control, engine))
}

impl Control {
    /// Output frames per second.
    #[must_use]
    pub const fn rate(&self) -> u32 {
        self.rate
    }

    /// Output channels.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// The audio clock: frames the engine has rendered. A command stamped for this frame or later
    /// is carried out on that frame exactly.
    #[must_use]
    pub fn now(&self) -> u64 {
        self.counters.frames.load(Ordering::Acquire)
    }

    /// Starts a voice playing `pcm`.
    ///
    /// # Errors
    ///
    /// A spec the source can't play ([`AudioError::Parameter`], [`AudioError::LoopRegion`],
    /// [`AudioError::Layout`]), [`AudioError::StaleNode`] for a node that isn't live or takes no
    /// voices, [`AudioError::SendsExhausted`], [`AudioError::VoicesExhausted`],
    /// [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn play(&mut self, pcm: &Arc<Pcm>, spec: &PlaySpec) -> Result<VoiceKey, AudioError> {
        spec.check(pcm, self.channels)?;
        self.start(Source::Pcm(Arc::clone(pcm)), spec, pcm.frames() as u64)
    }

    /// Starts a voice playing `stream`, which no other voice may play; it plays as its writer
    /// writes, silence where the writer is behind, and ends once the writer finishes and it has
    /// played everything written.
    ///
    /// # Errors
    ///
    /// As [`Control::play`], and [`AudioError::StreamClaimed`] for a stream a voice already plays.
    pub fn play_stream(&mut self, stream: &Arc<Stream>, spec: &PlaySpec) -> Result<VoiceKey, AudioError> {
        spec.check_stream(stream, self.channels)?;
        if !stream.claim() {
            return Err(AudioError::StreamClaimed);
        }
        let window = vec![0.0; stream.channels() * STREAM_WINDOW].into_boxed_slice();
        let started = self.start(Source::Stream(Arc::clone(stream), window), spec, 0);
        if started.is_err() {
            stream.release();
        }
        started
    }

    /// The output into `node`'s first audio input at `level`, if voices can play there.
    fn output(&self, node: NodeKey, level: f32) -> Result<Output, AudioError> {
        voice_output(&self.graph, node, None, level)
    }

    fn start(&mut self, source: Source, spec: &PlaySpec, frames: u64) -> Result<VoiceKey, AudioError> {
        self.collect();
        let mut outputs = Vec::with_capacity(1 + spec.sends.len());
        outputs.push(self.output(spec.bus, 1.0)?);
        for send in &spec.sends {
            outputs.push(self.output(send.bus, send.level)?);
        }
        let Some(slot) = self.free.pop() else {
            return Err(AudioError::VoicesExhausted(self.slots.len()));
        };
        let voice = VoiceKey {
            slot,
            generation: self.slots[slot as usize].generation,
        };
        let command = Command::Start {
            voice,
            source,
            spec: spec.begin(),
            outputs: Outputs::new(&outputs),
        };
        if let Err(error) = self.send(spec.at, command) {
            self.free.push(slot);
            return Err(error);
        }
        let entry = &mut self.slots[slot as usize];
        entry.busy = true;
        entry.start = spec.at;
        entry.outputs.clear();
        entry.outputs.extend_from_slice(&outputs);
        entry.frames = frames;
        entry.emitter = spec.spatial;
        Ok(voice)
    }

    /// Sends a voice to `node` at `level` from frame `at`, or from the next frame rendered, or
    /// changes the level of the send there. True if the voice hadn't ended.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`] for a level that isn't finite, [`AudioError::StaleNode`] for a
    /// node voices can't play into, [`AudioError::SendsExhausted`], [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn set_send(&mut self, voice: VoiceKey, node: NodeKey, level: f64, at: Option<u64>) -> Result<bool, AudioError> {
        let level = check_level(level)?;
        self.collect();
        if !self.is_live(voice) {
            return Ok(false);
        }
        #[allow(clippy::cast_possible_truncation)] // Levels are f32 in the engine.
        let level = level as f32;
        let output = self.output(node, level)?;
        let outputs = &self.slots[voice.slot as usize].outputs;
        if let Some(index) = outputs.iter().position(|known| same_input(known, &output)) {
            self.send(
                at,
                Command::SendVoice {
                    voice,
                    target: node.slot,
                    level,
                },
            )?;
            self.slots[voice.slot as usize].outputs[index].level = level;
            return Ok(true);
        }
        let mut outputs = outputs.clone();
        outputs.push(output);
        self.reroute(voice, outputs, at)
    }

    /// Stops a voice sending to `node` from frame `at`, or from the next frame rendered. True if it
    /// sent there.
    ///
    /// # Errors
    ///
    /// [`AudioError::QueueFull`] or [`AudioError::Disconnected`].
    pub fn unsend_voice(&mut self, voice: VoiceKey, node: NodeKey, at: Option<u64>) -> Result<bool, AudioError> {
        self.collect();
        if !self.is_live(voice) {
            return Ok(false);
        }
        let mut outputs = self.slots[voice.slot as usize].outputs.clone();
        let before = outputs.len();
        outputs.retain(|output| (output.slot, output.generation) != (node.slot, node.generation));
        if outputs.len() == before {
            return Ok(false);
        }
        self.reroute(voice, outputs, at)
    }

    /// Plays a voice into `node` from frame `at`, or from the next frame rendered, instead of
    /// where it plays now, keeping its sends. True if the voice hadn't ended.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleNode`] for a node voices can't play into, [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn route_voice(&mut self, voice: VoiceKey, node: NodeKey, at: Option<u64>) -> Result<bool, AudioError> {
        self.collect();
        if !self.is_live(voice) {
            return Ok(false);
        }
        let output = self.output(node, 1.0)?;
        // The new place replaces the first; a send there already is taken in by it.
        let sends = self.slots[voice.slot as usize].outputs.iter().skip(1);
        let outputs = std::iter::once(output)
            .chain(sends.filter(|known| !same_input(known, &output)).copied())
            .collect();
        self.reroute(voice, outputs, at)
    }

    /// Sends a voice its new outputs from frame `at`.
    fn reroute(&mut self, voice: VoiceKey, outputs: Vec<Output>, at: Option<u64>) -> Result<bool, AudioError> {
        if outputs.len() > MAX_VOICE_OUTPUTS {
            return Err(AudioError::SendsExhausted(MAX_VOICE_OUTPUTS - 1));
        }
        self.send(
            at,
            Command::Route {
                voice,
                outputs: Outputs::new(&outputs),
            },
        )?;
        self.slots[voice.slot as usize].outputs = outputs;
        Ok(true)
    }

    /// Where `voice` plays as last sent: each node, and the level it plays there.
    #[must_use]
    pub fn voice_outputs(&self, voice: VoiceKey) -> Vec<(NodeKey, f32)> {
        if !self.is_live(voice) {
            return Vec::new();
        }
        self.slots[voice.slot as usize]
            .outputs
            .iter()
            .map(|output| {
                let node = NodeKey {
                    slot: output.slot,
                    generation: output.generation,
                };
                (node, output.level)
            })
            .collect()
    }

    /// Stops a voice at frame `at`, or at the next frame rendered, fading it out over `fade`
    /// frames first. True if the voice hadn't ended as far as this side knows; stopping an ended
    /// voice does nothing.
    ///
    /// # Errors
    ///
    /// [`AudioError::QueueFull`] or [`AudioError::Disconnected`].
    pub fn stop(&mut self, voice: VoiceKey, at: Option<u64>, fade: u64) -> Result<bool, AudioError> {
        self.collect();
        if !self.is_live(voice) {
            return Ok(false);
        }
        self.send(at, Command::Stop { voice, fade })?;
        Ok(true)
    }

    /// Sets a voice's parameter at frame `at`, or at the next frame rendered, at once or over
    /// `glide`. A voice not yet started starts with it. True if the voice hadn't ended as far as
    /// this side knows.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`] for a value the parameter, or the ramp, can't take,
    /// [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn set(
        &mut self,
        voice: VoiceKey,
        param: VoiceParam,
        value: f64,
        at: Option<u64>,
        glide: Option<Glide>,
    ) -> Result<bool, AudioError> {
        let value = param.check(value)?;
        if let Some(glide) = glide {
            glide.check(param.name(), value)?;
        }
        self.collect();
        if !self.is_live(voice) {
            return Ok(false);
        }
        self.send(
            at,
            Command::SetVoice {
                voice,
                param,
                value,
                glide,
            },
        )?;
        Ok(true)
    }

    /// Loops a voice over `looping` from frame `at`, or from the next frame rendered; `None` lets
    /// it play on past its loop to the end of its source. True if the voice hadn't ended.
    ///
    /// # Errors
    ///
    /// [`AudioError::LoopRegion`] outside its source, [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn set_loop(&mut self, voice: VoiceKey, looping: Option<LoopRegion>, at: Option<u64>) -> Result<bool, AudioError> {
        self.collect();
        if !self.is_live(voice) {
            return Ok(false);
        }
        let frames = self.slots[voice.slot as usize].frames;
        if let Some(region) = looping
            && (region.start >= region.end || region.end > frames)
        {
            return Err(AudioError::LoopRegion {
                start: region.start,
                end: region.end,
                frames,
            });
        }
        self.send(at, Command::SetLoop { voice, looping })?;
        Ok(true)
    }

    /// Where `voice` is.
    pub fn state(&mut self, voice: VoiceKey) -> VoiceState {
        self.collect();
        if !self.is_live(voice) {
            return VoiceState::Ended;
        }
        match self.slots[voice.slot as usize].start {
            Some(start) if start > self.now() => VoiceState::Scheduled,
            _ => VoiceState::Playing,
        }
    }

    /// Refuses a revision stamped `at` while one stamped later is still waiting: revisions take
    /// effect in the order they were made, each plan compiled on top of the last.
    fn check_order(&self, at: Option<u64>) -> Result<(), AudioError> {
        let now = self.now();
        let Some(pending) = self.revised.filter(|pending| *pending > now) else {
            return Ok(());
        };
        if at.unwrap_or(now) < pending {
            return Err(AudioError::GraphOrder { pending });
        }
        Ok(())
    }

    /// Sends the revision that makes the graph `next` from frame `at`, with the nodes it made and
    /// the ones it removed that end at once, and makes `next` the graph, or leaves the graph as
    /// it was and says why not.
    fn commit(
        &mut self,
        at: Option<u64>,
        next: Definition,
        added: Vec<(NodeKey, Box<Node>)>,
        retire: Vec<NodeKey>,
    ) -> Result<(), AudioError> {
        self.commit_moving(at, next, added, retire, Box::default())
    }

    /// [`Control::commit`], moving `voices` to their new outputs with it.
    fn commit_moving(
        &mut self,
        at: Option<u64>,
        next: Definition,
        added: Vec<(NodeKey, Box<Node>)>,
        retire: Vec<NodeKey>,
        voices: Box<[(VoiceKey, Outputs)]>,
    ) -> Result<(), AudioError> {
        self.check_order(at)?;
        let plan = Box::new(next.compile()?);
        let revision = Box::new(Revision {
            plan,
            added,
            retire: retire.into_boxed_slice(),
            voices,
        });
        self.send(at, Command::Revise(revision))?;
        self.graph = next;
        if let Some(at) = at.filter(|at| *at > self.now()) {
            self.revised = Some(self.revised.map_or(at, |pending| pending.max(at)));
        }
        Ok(())
    }

    /// The engine's node for `processor`, made for a node `key` of `channels`.
    fn engine_node(&self, key: NodeKey, processor: Box<dyn Processor>, channels: usize) -> Node {
        Node::new(key.generation, processor, channels, self.channels * QUANTUM)
    }

    /// Makes a bus routed into `spec.parent`. It exists from the next frame rendered: voices can
    /// play into it at once.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`] for a gain out of range, [`AudioError::StaleNode`] for a parent
    /// that isn't live, [`AudioError::NodesExhausted`], [`AudioError::GraphOrder`],
    /// [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn add_bus(&mut self, spec: &BusSpec) -> Result<NodeKey, AudioError> {
        let mixer = Mixer::new(spec.gain, spec.muted);
        check_param(&mixer.params()[0], f64::from(spec.gain))?;
        self.collect();
        let mut next = self.graph.clone();
        let key = next.add(Shape::of(&mixer), spec.channels)?;
        next.route(key, spec.parent)?;
        let node = Box::new(self.engine_node(key, Box::new(mixer), next.channels(key.slot)));
        self.commit(None, next, vec![(key, node)], Vec::new())?;
        Ok(key)
    }

    /// Makes a node running `processor`, with `channels` if fewer than the context's, connected
    /// to nothing. It exists from the next frame rendered.
    ///
    /// # Errors
    ///
    /// [`AudioError::Shape`], [`AudioError::Channels`], [`AudioError::NodesExhausted`],
    /// [`AudioError::GraphOrder`], [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn add_node(&mut self, processor: Box<dyn Processor>, channels: Option<usize>) -> Result<NodeKey, AudioError> {
        check_shape(processor.as_ref())?;
        self.collect();
        let mut next = self.graph.clone();
        let key = next.add(Shape::of(processor.as_ref()), channels)?;
        let node = Box::new(self.engine_node(key, processor, next.channels(key.slot)));
        self.commit(None, next, vec![(key, node)], Vec::new())?;
        Ok(key)
    }

    /// Makes a node running `processor` in `host`'s insert chain at `index` (from 0), or last,
    /// from the next frame rendered.
    ///
    /// # Errors
    ///
    /// [`AudioError::Shape`], [`AudioError::StaleNode`], [`AudioError::NotInsertable`],
    /// [`AudioError::NodesExhausted`], [`AudioError::GraphOrder`], [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn insert(
        &mut self,
        host: NodeKey,
        processor: Box<dyn Processor>,
        index: Option<usize>,
    ) -> Result<NodeKey, AudioError> {
        check_shape(processor.as_ref())?;
        self.collect();
        self.graph.check_live(host)?;
        let mut next = self.graph.clone();
        let key = next.add(Shape::of(processor.as_ref()), Some(next.channels(host.slot)))?;
        next.insert(host, key, index)?;
        let node = Box::new(self.engine_node(key, processor, next.channels(key.slot)));
        self.commit(None, next, vec![(key, node)], Vec::new())?;
        Ok(key)
    }

    /// Applies every edit together from frame `at`, or from the next frame rendered: the graph
    /// takes them all on that frame, or, when any is wrong, none of them.
    ///
    /// # Errors
    ///
    /// Whatever the first wrong edit is wrong with ([`AudioError::StaleNode`],
    /// [`AudioError::UnknownPort`], [`AudioError::Layout`], [`AudioError::NodeInUse`], ...),
    /// [`AudioError::Cycle`] when they make a loop without a feedback delay,
    /// [`AudioError::GraphOrder`], [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn patch(&mut self, edits: &[Edit], at: Option<u64>) -> Result<(), AudioError> {
        self.collect();
        let mut next = self.graph.clone();
        let mut retire = Vec::new();
        // Each voice the patch moves, and where it plays once it has.
        let mut voices: Vec<(VoiceKey, Vec<Output>)> = Vec::new();
        for edit in edits {
            match edit {
                Edit::Connect {
                    from,
                    output,
                    to,
                    input,
                    connection,
                } => next.connect(*from, output, *to, input, *connection)?,
                Edit::Disconnect { from, output, to, input } => {
                    next.disconnect(*from, output, *to, input)?;
                }
                Edit::Route { node, parent } => next.route(*node, *parent)?,
                Edit::Insert { host, node, index } => next.insert(*host, *node, *index)?,
                Edit::Remove(node) => {
                    self.check_unfed(*node, &voices)?;
                    retire.extend(next.remove(*node)?);
                }
                Edit::Send { voice, to, input, level } => {
                    check_level(f64::from(*level))?;
                    let output = voice_output(&next, *to, input.as_deref(), *level)?;
                    let outputs = self.moving(&mut voices, *voice)?;
                    if let Some(known) = outputs.iter_mut().find(|known| same_input(known, &output)) {
                        known.level = output.level;
                    } else if outputs.len() < MAX_VOICE_OUTPUTS {
                        outputs.push(output);
                    } else {
                        return Err(AudioError::SendsExhausted(MAX_VOICE_OUTPUTS - 1));
                    }
                }
                Edit::Unsend { voice, to } => {
                    let outputs = self.moving(&mut voices, *voice)?;
                    outputs.retain(|output| (output.slot, output.generation) != (to.slot, to.generation));
                }
            }
        }
        let moved = voices
            .iter()
            .map(|(voice, outputs)| (*voice, Outputs::new(outputs)))
            .collect();
        self.commit_moving(at, next, Vec::new(), retire, moved)?;
        for (voice, outputs) in voices {
            self.slots[voice.slot as usize].outputs = outputs;
        }
        Ok(())
    }

    /// Where `voice` plays as a patch has it so far, from where it plays now.
    fn moving<'a>(
        &self,
        voices: &'a mut Vec<(VoiceKey, Vec<Output>)>,
        voice: VoiceKey,
    ) -> Result<&'a mut Vec<Output>, AudioError> {
        if !self.is_live(voice) {
            return Err(AudioError::StaleVoice);
        }
        let found = voices.iter().position(|(known, _)| *known == voice);
        let index = found.unwrap_or_else(|| {
            voices.push((voice, self.slots[voice.slot as usize].outputs.clone()));
            voices.len() - 1
        });
        Ok(&mut voices[index].1)
    }

    /// Refuses removing a node voices still play into, as a patch has moved them so far.
    fn check_unfed(&self, node: NodeKey, moved: &[(VoiceKey, Vec<Output>)]) -> Result<(), AudioError> {
        let voices = self
            .slots
            .iter()
            .enumerate()
            .filter(|(slot, entry)| {
                let outputs = moved
                    .iter()
                    .find(|(voice, _)| voice.slot as usize == *slot)
                    .map_or(&entry.outputs[..], |(_, outputs)| &outputs[..]);
                entry.busy && outputs.iter().any(|output| output.slot == node.slot)
            })
            .count();
        if voices > 0 {
            return Err(AudioError::NodeInUse {
                voices,
                nodes: self.graph.feeds(node),
            });
        }
        Ok(())
    }

    /// Removes a node at frame `at`, or at the next frame rendered, with its insert chain; one
    /// with a tail rings out first. True if it was live; removing a removed node does nothing.
    ///
    /// # Errors
    ///
    /// [`AudioError::Master`] for the master, [`AudioError::NodeInUse`] while a voice or a
    /// connection feeds it, [`AudioError::GraphOrder`], [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn remove(&mut self, node: NodeKey, at: Option<u64>) -> Result<bool, AudioError> {
        if node == NodeKey::MASTER {
            return Err(AudioError::Master);
        }
        self.collect();
        if !self.graph.is_live(node) {
            return Ok(false);
        }
        self.patch(&[Edit::Remove(node)], at)?;
        Ok(true)
    }

    /// Routes a node into `parent` from frame `at`, or from the next frame rendered.
    ///
    /// # Errors
    ///
    /// [`AudioError::Master`] for the master, [`AudioError::StaleNode`] for either node not live,
    /// [`AudioError::Cycle`] when `parent` is fed by `node`, [`AudioError::GraphOrder`],
    /// [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn route(&mut self, node: NodeKey, parent: NodeKey, at: Option<u64>) -> Result<(), AudioError> {
        self.patch(&[Edit::Route { node, parent }], at)
    }

    /// The first output and audio input of `from` and `to`: what a send connects.
    fn mains(&self, from: NodeKey, to: NodeKey) -> Result<(String, String), AudioError> {
        let output = self.graph.shape(from).ok_or(AudioError::StaleNode)?.ports.outputs[0];
        let input = self
            .graph
            .shape(to)
            .ok_or(AudioError::StaleNode)?
            .ports
            .audio()
            .next()
            .ok_or(AudioError::StaleNode)?
            .name;
        Ok((output.to_owned(), input.to_owned()))
    }

    /// Sends `node` to `target` at `level` from frame `at`, or from the next frame rendered,
    /// tapped after the node's processing or before it. A send already there changes level and
    /// tap.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`], [`AudioError::StaleNode`] for either node not live,
    /// [`AudioError::Cycle`] when `target` feeds `node`, and as [`Control::patch`].
    pub fn send_to(
        &mut self,
        node: NodeKey,
        target: NodeKey,
        level: f64,
        tap: Tap,
        at: Option<u64>,
    ) -> Result<(), AudioError> {
        let level = check_level(level)?;
        self.collect();
        if node == target {
            return Err(AudioError::Cycle);
        }
        let (output, input) = self.mains(node, target)?;
        #[allow(clippy::cast_possible_truncation)] // Levels are f32 in the engine.
        let connection = Connection {
            level: level as f32,
            tap,
            feedback: None,
        };
        self.patch(
            &[Edit::Connect {
                from: node,
                output,
                to: target,
                input,
                connection,
            }],
            at,
        )
    }

    /// Removes the send from `node` to `target` at frame `at`, or at the next frame rendered.
    /// True if there was one; a node that was removed has none, whatever node has its slot now.
    ///
    /// # Errors
    ///
    /// As [`Control::patch`].
    pub fn unsend(&mut self, node: NodeKey, target: NodeKey, at: Option<u64>) -> Result<bool, AudioError> {
        self.collect();
        if !self.graph.is_live(node) || !self.graph.is_live(target) {
            return Ok(false);
        }
        let (output, input) = self.mains(node, target)?;
        let mut next = self.graph.clone();
        if !next.disconnect(node, &output, target, &input)? {
            return Ok(false);
        }
        self.commit(at, next, Vec::new(), Vec::new())?;
        Ok(true)
    }

    /// Has `node` listen to `key` on its sidechain from frame `at`, or to nothing with `None`.
    /// The key is run before the node, so a key the node feeds is a loop, and refused. True if
    /// the node is live.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleNode`], [`AudioError::Unsupported`] for a node without a sidechain,
    /// [`AudioError::Cycle`], and as [`Control::patch`].
    pub fn listen(&mut self, node: NodeKey, key: Option<NodeKey>, at: Option<u64>) -> Result<bool, AudioError> {
        self.collect();
        let Some(shape) = self.graph.shape(node) else {
            return Ok(false);
        };
        let Some(port) = shape.ports.sidechain() else {
            return Err(AudioError::Unsupported("listening needs a processor with a sidechain"));
        };
        let mut next = self.graph.clone();
        next.unlisten(node);
        if let Some(key) = key {
            let output = next.shape(key).ok_or(AudioError::StaleNode)?.ports.outputs[0];
            next.connect(key, output, node, port.name, Connection::default())?;
        }
        self.commit(at, next, Vec::new(), Vec::new())?;
        Ok(true)
    }

    /// Sets node `node`'s parameter `name` at frame `at`, or at the next frame rendered, at once
    /// or over `glide`. A control-rate parameter moves every
    /// [`CONTROL_QUANTUM`](crate::audio::CONTROL_QUANTUM) frames while it ramps, an audio-rate
    /// one every frame. True if the node is live.
    ///
    /// # Errors
    ///
    /// [`AudioError::UnknownParameter`], [`AudioError::Parameter`] outside its range,
    /// [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn set_param(
        &mut self,
        node: NodeKey,
        name: &str,
        value: f64,
        at: Option<u64>,
        glide: Option<Glide>,
    ) -> Result<bool, AudioError> {
        self.collect();
        let Some(shape) = self.graph.shape(node) else {
            return Ok(false);
        };
        let index = param_index(shape.kind, shape.params, name)?;
        let value = check_param(&shape.params[index], value)?;
        if let Some(glide) = glide {
            glide.check(shape.params[index].name, value)?;
        }
        self.send(
            at,
            Command::SetParam {
                node,
                index: u16::try_from(index).unwrap_or(u16::MAX),
                value,
                glide,
            },
        )?;
        Ok(true)
    }

    /// True while `node` is live: made, and not removed.
    pub fn node_live(&mut self, node: NodeKey) -> bool {
        self.collect();
        self.graph.is_live(node)
    }

    /// The kind and parameters of `node`, while it is live.
    #[must_use]
    pub fn describe(&self, node: NodeKey) -> Option<(&'static str, &'static [ParamInfo])> {
        self.graph.shape(node).map(|shape| (shape.kind, shape.params))
    }

    /// The ports of `node`, while it is live.
    #[must_use]
    pub fn ports(&self, node: NodeKey) -> Option<&'static Ports> {
        self.graph.shape(node).map(|shape| shape.ports)
    }

    /// How many frames `node`'s output lags the voices that reach it, by the longest way round:
    /// the processor latency along the path a voice takes to it.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleNode`].
    pub fn latency(&mut self, node: NodeKey) -> Result<u64, AudioError> {
        self.collect();
        self.graph.latency(node)
    }

    /// What the bus `node` has put out, after its gain, since the last time it was read: the peak
    /// sample's magnitude and the RMS level, across its channels. Reading starts the next
    /// measurement.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleNode`], or [`AudioError::Unsupported`] for a node that isn't a bus.
    pub fn meter(&mut self, node: NodeKey) -> Result<Meter, AudioError> {
        self.collect();
        let shape = self.graph.shape(node).ok_or(AudioError::StaleNode)?;
        if !shape.mixer {
            return Err(AudioError::Unsupported("a bus is metered; another node isn't"));
        }
        let meter = &self.counters.meters[node.slot as usize];
        let peak = f32::from_bits(meter.peak.swap(0, Ordering::Relaxed));
        let energy = f64::from_bits(meter.energy.swap(0, Ordering::Relaxed));
        let samples = meter.samples.swap(0, Ordering::Relaxed);
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // A level.
        let rms = if samples == 0 {
            0.0
        } else {
            (energy / samples as f64).sqrt() as f32
        };
        Ok(Meter { peak, rms })
    }

    /// Moves the listener from frame `at`, or from the next frame rendered. Voices in space glide
    /// to where it puts them over the block after.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`] for vectors that aren't finite, [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn set_listener(&mut self, listener: Listener, at: Option<u64>) -> Result<(), AudioError> {
        let vectors = [listener.position, listener.forward, listener.up, listener.velocity];
        if !vectors.iter().flatten().all(|value| value.is_finite()) {
            return Err(AudioError::Parameter {
                name: "listener",
                value: f64::NAN,
                expected: "finite vectors for position, forward, up, and velocity",
            });
        }
        self.collect();
        self.send(at, Command::Listener(listener))?;
        self.listener = listener;
        Ok(())
    }

    /// The listener as last sent.
    #[must_use]
    pub const fn listener(&self) -> Listener {
        self.listener
    }

    /// Where `voice` is in space as last sent, if it's live and in space.
    #[must_use]
    pub fn emitter(&self, voice: VoiceKey) -> Option<Emitter> {
        if !self.is_live(voice) {
            return None;
        }
        self.slots[voice.slot as usize].emitter
    }

    /// Moves a voice in space from frame `at`, or from the next frame rendered. True if the voice
    /// hadn't ended; a voice that started out of space stays out of it.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`] for an emitter the engine can't hear, [`AudioError::QueueFull`],
    /// or [`AudioError::Disconnected`].
    pub fn place(&mut self, voice: VoiceKey, emitter: Emitter, at: Option<u64>) -> Result<bool, AudioError> {
        emitter.check()?;
        self.collect();
        if !self.is_live(voice) {
            return Ok(false);
        }
        let entry = &self.slots[voice.slot as usize];
        if entry.emitter.is_none() {
            return Ok(false);
        }
        self.send(at, Command::Place { voice, emitter })?;
        self.slots[voice.slot as usize].emitter = Some(emitter);
        Ok(true)
    }

    /// The engine's counters as of its last render.
    pub fn stats(&mut self) -> AudioStats {
        self.collect();
        AudioStats {
            frames: self.now(),
            active: self.counters.active.load(Ordering::Relaxed),
            allocated: (self.slots.len() - self.free.len()) as u64,
            late: self.counters.late.load(Ordering::Relaxed),
            nonfinite: self.counters.nonfinite.load(Ordering::Relaxed),
            pending: self.counters.pending.load(Ordering::Relaxed),
            culled: self.counters.culled.load(Ordering::Relaxed),
            stolen: self.counters.stolen.load(Ordering::Relaxed),
            revisions: self.counters.revisions.load(Ordering::Relaxed),
        }
    }

    /// Reads what the engine sent back: ended voices and nodes free their slots, and the memory
    /// the engine let go of is dropped here rather than on the render thread.
    pub fn collect(&mut self) {
        while let Ok(feedback) = self.feedback.try_recv() {
            match feedback {
                Feedback::VoiceEnded { voice, source, outputs } => {
                    drop((source, outputs));
                    let slot = &mut self.slots[voice.slot as usize];
                    if slot.generation != voice.generation || !slot.busy {
                        continue;
                    }
                    slot.generation = slot.generation.wrapping_add(1);
                    slot.busy = false;
                    slot.start = None;
                    self.free.push(voice.slot);
                }
                Feedback::NodeEnded { node, state } => {
                    drop(state);
                    self.graph.ended(node);
                    // The next node in the slot starts its own measurement.
                    let meter = &self.counters.meters[node.slot as usize];
                    meter.peak.store(0, Ordering::Relaxed);
                    meter.energy.store(0, Ordering::Relaxed);
                    meter.samples.store(0, Ordering::Relaxed);
                }
                Feedback::Revised(revision) => drop(revision),
                Feedback::Outputs(outputs) => drop(outputs),
                Feedback::Spent(group) => drop(group),
            }
        }
    }

    /// How many more commands the queue to the engine has room for, as of the engine's last
    /// render: never more than it really has.
    #[must_use]
    pub fn room(&self) -> usize {
        let received = self.counters.received.load(Ordering::Acquire);
        let waiting = usize::try_from(self.sent.saturating_sub(received)).unwrap_or(usize::MAX);
        self.capacity.saturating_sub(waiting)
    }

    /// Opens a group: every command until [`Control::end_group`] reaches the engine together, in
    /// one entry of its queue, so the ones not stamped for later are carried out on the same
    /// frame, and no render falls between them. A group can't half-arrive: there is room for it
    /// when it opens, and nothing else is sent until it closes.
    ///
    /// # Errors
    ///
    /// [`AudioError::GroupOpen`] inside another group, or [`AudioError::QueueFull`].
    pub fn begin_group(&mut self) -> Result<(), AudioError> {
        if self.group.is_some() {
            return Err(AudioError::GroupOpen);
        }
        self.collect();
        let room = self.room();
        if room == 0 {
            return Err(AudioError::QueueFull(self.capacity));
        }
        self.group = Some((Vec::with_capacity(16), room));
        Ok(())
    }

    /// Sends the group [`Control::begin_group`] opened.
    ///
    /// # Errors
    ///
    /// [`AudioError::Disconnected`]; with no group open, nothing happens.
    pub fn end_group(&mut self) -> Result<(), AudioError> {
        let Some((members, _)) = self.group.take() else {
            return Ok(());
        };
        if members.is_empty() {
            return Ok(());
        }
        let count = members.len() as u64 + 1;
        let stamped = Stamped {
            at: None,
            sequence: self.sequence,
            command: Command::Group(members),
        };
        match self.commands.try_send(stamped) {
            Ok(()) => {
                self.sent += count;
                Ok(())
            }
            // Room was there when the group opened, and nothing else has been sent since.
            Err(TrySendError::Full(_)) => Err(AudioError::QueueFull(self.capacity)),
            Err(TrySendError::Disconnected(_)) => Err(AudioError::Disconnected),
        }
    }

    fn is_live(&self, voice: VoiceKey) -> bool {
        self.slots
            .get(voice.slot as usize)
            .is_some_and(|slot| slot.busy && slot.generation == voice.generation)
    }

    fn send(&mut self, at: Option<u64>, command: Command) -> Result<(), AudioError> {
        let now = self.now();
        let later = at.filter(|at| *at > now);
        if later.is_some() {
            // The engine carries out everything stamped before the frame it last rendered up to.
            while self.scheduled.peek().is_some_and(|Reverse(at)| *at < now) {
                self.scheduled.pop();
            }
            if self.scheduled.len() >= self.capacity {
                return Err(AudioError::ScheduleFull(self.capacity));
            }
        }
        self.sequence += 1;
        let stamped = Stamped {
            at,
            sequence: self.sequence,
            command,
        };
        if let Some((members, room)) = self.group.as_mut() {
            // The group and its members all count against the room it opened with.
            if members.len() + 2 > *room {
                return Err(AudioError::QueueFull(self.capacity));
            }
            members.push(stamped);
            self.scheduled.extend(later.map(Reverse));
            return Ok(());
        }
        match self.commands.try_send(stamped) {
            Ok(()) => {
                self.sent += 1;
                self.scheduled.extend(later.map(Reverse));
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(AudioError::QueueFull(self.capacity)),
            Err(TrySendError::Disconnected(_)) => Err(AudioError::Disconnected),
        }
    }
}

/// A voice's output into `node`'s audio input `input` (its first without one) at `level`, if
/// voices can play there in `graph`.
fn voice_output(graph: &Definition, node: NodeKey, input: Option<&str>, level: f32) -> Result<Output, AudioError> {
    let input = graph.voice_input(node, input)?;
    Ok(Output {
        slot: node.slot,
        generation: node.generation,
        input,
        level,
    })
}

/// True for two outputs into the same node input.
fn same_input(first: &Output, second: &Output) -> bool {
    (first.slot, first.generation, first.input) == (second.slot, second.generation, second.input)
}

/// A context rendered on demand rather than by a device: for tests, exports, and anything that
/// wants audio as data. It runs the same engine a device does.
#[derive(Debug)]
pub struct OfflineContext {
    control: Control,
    engine: Engine,
}

impl OfflineContext {
    /// A context for `config`.
    ///
    /// # Errors
    ///
    /// As [`context`].
    pub fn new(config: &AudioConfig) -> Result<OfflineContext, AudioError> {
        let (control, engine) = context(config)?;
        Ok(OfflineContext { control, engine })
    }

    /// The control side, to play and schedule on.
    pub const fn control(&mut self) -> &mut Control {
        &mut self.control
    }

    #[cfg(test)]
    pub(crate) const fn parts(&mut self) -> (&mut Control, &mut Engine) {
        (&mut self.control, &mut self.engine)
    }

    /// Renders `frames` frames, interleaved.
    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * self.engine.channels()];
        self.render_into(&mut out);
        out
    }

    /// Renders into all of `out`, interleaved; its length is a whole number of frames.
    pub fn render_into(&mut self, out: &mut [f32]) {
        self.engine.render(out);
        self.control.collect();
    }
}
