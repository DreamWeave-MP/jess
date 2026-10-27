//! The engine: everything that runs on the render thread.
//!
//! [`Engine::render`] is the whole of it. A device callback calls it for each buffer the device
//! asks for, and an [`OfflineContext`](crate::audio::OfflineContext) calls it in a loop: the same
//! code, so an offline render is the test of what a device plays.
//!
//! Rendering never allocates, frees, locks, or blocks. Everything it needs is allocated when the
//! engine is made or on the control side; commands arrive on a bounded queue it only polls, and
//! anything holding memory that it is done with (the [`Pcm`] of an ended voice, a plan it
//! replaced, a node that has rung out) goes back to the control side on a second queue to be
//! dropped there.
//!
//! The engine holds two things apart: the node table, every node's processor and automation by
//! slot, which lives as long as each node does; and the [`Plan`] it runs, which the control side
//! compiled and which it only ever swaps whole. Each block it clears the nodes' input buffers,
//! adds every voice into its outputs, runs the plan's steps in order, and writes the master out.
//!
//! [`Pcm`]: crate::audio::Pcm

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};

use crate::audio::mixer::{self, NodeKey};
use crate::audio::param::{Glide, Param};
use crate::audio::plan::{Modulation, Place, Plan, Push, Revision, Step, Target};
use crate::audio::processor::{Io, MAX_PARAMS, ParamInfo, Processor, Rate};
use crate::audio::spatial::{Emitter, Listener, Space};
use crate::audio::voice::{Begin, Culled, Outputs, Source, Voice, VoiceKey, set_level};
use crate::audio::{LoopRegion, MAX_CHANNELS, QUANTUM, VoiceParam};

/// What the control side asks of the engine.
pub enum Command {
    Start {
        voice: VoiceKey,
        source: Source,
        spec: Begin,
        outputs: Outputs,
    },
    /// Stops a voice, fading out over `fade` frames first.
    Stop { voice: VoiceKey, fade: u64 },
    SetVoice {
        voice: VoiceKey,
        param: VoiceParam,
        value: f64,
        glide: Option<Glide>,
    },
    /// Pauses a voice where it is, or plays it on from there.
    Pause { voice: VoiceKey, paused: bool },
    /// Moves a voice to source frame `frame`; for a stream, once its writer has carried out seek
    /// `epoch`.
    Seek { voice: VoiceKey, frame: u64, epoch: u64 },
    /// Loops a voice over a new region, or lets it play on past its loop.
    SetLoop { voice: VoiceKey, looping: Option<LoopRegion> },
    /// Moves a voice to new outputs; the old ones go back to be dropped.
    Route { voice: VoiceKey, outputs: Outputs },
    /// Changes the level of a voice's output into a node.
    SendVoice { voice: VoiceKey, target: u32, level: f32 },
    /// A revision of the graph: its nodes go into the table when the engine takes the command,
    /// and its plan runs from the command's frame.
    Revise(Box<Revision>),
    /// Sets or ramps a node's parameter.
    SetParam {
        node: NodeKey,
        index: u16,
        value: f64,
        glide: Option<Glide>,
    },
    /// Drops a node parameter's changes sent before this (sequence `before`) that still wait for
    /// their frames, and holds the parameter where it is.
    Cancel { node: NodeKey, index: u16, before: u64 },
    /// Commands that arrive together, each with its own stamp.
    Group(Vec<Stamped>),
    /// Moves the listener.
    Listener(Listener),
    /// Moves a voice in space.
    Place { voice: VoiceKey, emitter: Emitter },
}

impl std::fmt::Debug for Stamped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stamped")
            .field("at", &self.at)
            .field("sequence", &self.sequence)
            .finish_non_exhaustive()
    }
}

/// A command and when to carry it out: `at` an output frame, or `None` for the next frame the
/// engine renders. `sequence` orders commands stamped for the same frame as they were sent.
pub struct Stamped {
    pub(crate) at: Option<u64>,
    pub(crate) sequence: u64,
    pub(crate) command: Command,
}

impl Stamped {
    /// The key commands are carried out in: frame, then the order they were sent.
    fn key(&self) -> (u64, u64) {
        (self.at.unwrap_or(0), self.sequence)
    }
}

/// What the engine tells the control side.
pub enum Feedback {
    /// A voice ended: it ran out, was stopped, or was stopped before it started. Its slot is free
    /// once the control side has read this, and its source and outputs drop there.
    VoiceEnded {
        voice: VoiceKey,
        source: Source,
        outputs: Outputs,
    },
    /// A node ended: removed, and rung out if it had a tail. Its slot is free once the control
    /// side has read this, and its processor drops there.
    NodeEnded { node: NodeKey, state: Box<Node> },
    /// A revision the engine carried out, holding the plan it replaced and the outputs of the
    /// voices it moved, to drop.
    Revised(Box<Revision>),
    /// A voice's outputs, replaced, to drop.
    Outputs(Outputs),
    /// A group's list, emptied, to drop.
    Spent(Vec<Stamped>),
}

/// The engine's counters, shared with the control side. Each is written by the engine at the
/// end of a render and read by anyone.
#[derive(Debug, Default)]
pub struct Counters {
    /// Output frames rendered: the audio clock.
    pub(crate) frames: AtomicU64,
    /// Voices playing after the last render.
    pub(crate) active: AtomicU64,
    /// Commands carried out after the frame they were stamped for.
    pub(crate) late: AtomicU64,
    /// Output samples that weren't finite, written as silence.
    pub(crate) nonfinite: AtomicU64,
    /// Commands waiting for their frame.
    pub(crate) pending: AtomicU64,
    /// Commands taken from the queue, counting a group as its members and one more.
    pub(crate) received: AtomicU64,
    /// Voices live but not mixed in the last block.
    pub(crate) culled: AtomicU64,
    /// Voices ended for not being mixed, all told.
    pub(crate) stolen: AtomicU64,
    /// Graph revisions carried out, all told.
    pub(crate) revisions: AtomicU64,
    /// Each mixer's meter, by slot.
    pub(crate) meters: Box<[BusMeter]>,
    /// Each voice's source frame, by slot, with [`PAUSED`] set while it is paused.
    pub(crate) positions: Box<[AtomicU64]>,
}

/// The bit of a voice's published position that says it is paused.
pub const PAUSED: u64 = 1 << 63;

impl Counters {
    pub fn new(nodes: usize, voices: usize) -> Counters {
        Counters {
            meters: std::iter::repeat_with(BusMeter::default).take(nodes).collect(),
            positions: std::iter::repeat_with(AtomicU64::default).take(voices).collect(),
            ..Counters::default()
        }
    }
}

/// What one mixer put out since the control side last read it, after its gain: the largest
/// sample's magnitude, and the sum of squares over the samples counted. The engine only adds;
/// a reader takes the values and leaves zeros.
#[derive(Debug, Default)]
pub struct BusMeter {
    /// An `f32`'s bits; the order of non-negative floats' bits is their order.
    pub(crate) peak: AtomicU32,
    /// An `f64`'s bits.
    pub(crate) energy: AtomicU64,
    pub(crate) samples: AtomicU64,
}

impl BusMeter {
    fn add(&self, peak: f32, energy: f64, samples: u64) {
        self.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
        // One writer, but a reader may swap in a zero between the load and the store.
        let mut bits = self.energy.load(Ordering::Relaxed);
        loop {
            let sum = (f64::from_bits(bits) + energy).to_bits();
            match self
                .energy
                .compare_exchange_weak(bits, sum, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(now) => bits = now,
            }
        }
        self.samples.fetch_add(samples, Ordering::Relaxed);
    }
}

/// How big each of the engine's tables is, fixed when it's made.
pub struct Capacity {
    pub voices: usize,
    pub nodes: usize,
    pub commands: usize,
    /// Voices mixed at once; the rest wait their turn.
    pub physical: usize,
}

impl Capacity {
    /// The feedback the engine can owe at once: each voice and node ends once and its slot is
    /// reused only after the control side reads that, and each command gives back at most two
    /// things (a group's list, and a revision or what a start replaced).
    pub const fn feedback(&self) -> usize {
        self.voices + self.nodes + 2 * self.commands
    }
}

/// How often, in frames of the clock, a processor hears a moving control-rate parameter.
///
/// It hears it on every multiple of this, however the render is cut; while one moves, the
/// engine's blocks break on those frames.
pub const CONTROL_QUANTUM: usize = 32;

/// A node as the engine holds it: its processor, and its parameters' automation. It lives as long
/// as the node, whatever plans come and go.
pub struct Node {
    pub(crate) generation: u32,
    pub(crate) processor: Box<dyn Processor>,
    info: &'static [ParamInfo],
    params: Box<[Param]>,
    /// The parameters that move, a bit each.
    moving: u64,
    /// Its audio inputs after the first, which is its buffer in the engine: a stride each.
    inputs: Box<[f32]>,
    channels: usize,
    /// Removed, and ringing out: frames left before it goes.
    drain: Option<u64>,
    /// The parameters something modulated in the last block, a bit each: once nothing does, the
    /// processor hears its automation again.
    modulated: u64,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("kind", &self.processor.kind())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Node {
    /// A node for `processor` with `channels`, in a context whose buffers are `stride` samples,
    /// made on the control side with everything it will need.
    pub(crate) fn new(generation: u32, processor: Box<dyn Processor>, channels: usize, stride: usize) -> Node {
        let info = processor.params();
        let params = (0..info.len()).map(|index| Param::new(processor.value(index))).collect();
        let extra = processor.ports().audio().count().saturating_sub(1);
        Node {
            generation,
            info,
            params,
            moving: 0,
            inputs: vec![0.0; extra * stride].into_boxed_slice(),
            channels,
            drain: None,
            modulated: 0,
            processor,
        }
    }

    /// Sets parameter `index` at once, or ramps it over `glide`. True for a control-rate ramp,
    /// which keeps blocks to [`CONTROL_QUANTUM`] while it moves.
    fn automate(&mut self, index: usize, value: f64, glide: Option<Glide>) -> bool {
        let Some(param) = self.params.get_mut(index) else {
            return false;
        };
        let bit = 1u64 << index;
        if let Some(glide) = glide.filter(|glide| glide.frames > 0) {
            param.ramp(value, glide);
            self.moving |= bit;
            return self.info[index].rate == Rate::Control;
        }
        param.set(value);
        self.moving &= !bit;
        self.processor.set(index, value);
        false
    }

    /// Stops parameter `index` where it is, ramping or not.
    fn hold(&mut self, index: usize) {
        let Some(param) = self.params.get_mut(index) else {
            return;
        };
        let value = param.value();
        param.set(value);
        self.moving &= !(1u64 << index);
        self.processor.set(index, value);
    }

    /// Moves its automation `frames` along: tells the processor where each control-rate parameter
    /// is if the block starts `on_grid` (on a multiple of [`CONTROL_QUANTUM`]), and writes each
    /// moving audio-rate one's values into `values`, a [`QUANTUM`] a parameter. Returns the
    /// audio-rate parameters written, a bit each, and whether a control-rate one still moves.
    fn advance(&mut self, frames: usize, values: &mut [f32], on_grid: bool) -> (u64, bool) {
        let (mut varying, mut control) = (0, false);
        let mut moving = self.moving;
        while moving != 0 {
            let index = moving.trailing_zeros() as usize;
            let bit = 1u64 << index;
            moving &= !bit;
            let param = &mut self.params[index];
            match self.info[index].rate {
                Rate::Audio => {
                    param.fill(&mut values[index * QUANTUM..index * QUANTUM + frames]);
                    varying |= bit;
                    if param.steady() {
                        self.processor.set(index, param.value());
                        self.moving &= !bit;
                    }
                }
                Rate::Control => {
                    // The grid hears the value its frame has, and a ramp done by then is let go;
                    // between, the ramp moves on unheard.
                    if on_grid {
                        self.processor.set(index, param.value());
                        if param.steady() {
                            self.moving &= !bit;
                            continue;
                        }
                    }
                    param.advance(frames as u64);
                    control = true;
                }
            }
        }
        (varying, control)
    }
}

impl Node {
    /// Moves its automation `frames` along, as [`Node::advance`], and adds what modulates the
    /// parameters set in `modulated` (in `signals`, a [`QUANTUM`] a parameter) on top, clamped
    /// to each parameter's range: every sample for an audio-rate parameter, whose values go in
    /// `values`; for a control-rate one, at the block's start if it starts `on_grid`. A value
    /// that isn't finite leaves the automation as it is. A parameter nothing modulates any more
    /// hears its automation again.
    fn automate_and_modulate(
        &mut self,
        frames: usize,
        values: &mut [f32],
        (signals, modulated): (&[f32], u64),
        on_grid: bool,
    ) -> (u64, bool) {
        if modulated == 0 && self.modulated == 0 {
            if self.moving == 0 {
                return (0, false);
            }
            return self.advance(frames, values, on_grid);
        }
        // A control-rate parameter's block starts where its automation is before it moves on.
        let mut bases = [0.0f64; MAX_PARAMS];
        for index in bits(modulated) {
            bases[index] = self.params[index].value();
        }
        let (mut varying, ramping) = if self.moving == 0 {
            (0, false)
        } else {
            self.advance(frames, values, on_grid)
        };
        for index in bits(modulated) {
            let info = self.info[index];
            let bit = 1u64 << index;
            let signal = &signals[index * QUANTUM..index * QUANTUM + frames];
            if info.rate == Rate::Control {
                if on_grid {
                    self.processor.set(index, bounded(bases[index], f64::from(signal[0]), &info));
                }
                continue;
            }
            let steady = (varying & bit == 0).then(|| self.params[index].value());
            for (value, offset) in values[index * QUANTUM..index * QUANTUM + frames].iter_mut().zip(signal) {
                let base = steady.unwrap_or_else(|| f64::from(*value));
                #[allow(clippy::cast_possible_truncation)] // Engine values are f32.
                let sum = bounded(base, f64::from(*offset), &info) as f32;
                *value = sum;
            }
            varying |= bit;
        }
        for index in bits(self.modulated & !modulated) {
            self.processor.set(index, self.params[index].value());
        }
        self.modulated = modulated;
        (varying, ramping)
    }
}

/// Parameter `index`'s value as its automation holds it.
fn node_params(params: &[Param], index: usize) -> f64 {
    params.get(index).map_or(0.0, Param::value)
}

/// The set bits of `mask`, as indices.
fn bits(mut mask: u64) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        if mask == 0 {
            return None;
        }
        let index = mask.trailing_zeros() as usize;
        mask &= mask - 1;
        Some(index)
    })
}

/// `base` moved by `offset`, inside `param`'s range; `base` where the sum isn't finite.
fn bounded(base: f64, offset: f64, param: &ParamInfo) -> f64 {
    let sum = base + offset;
    if sum.is_finite() {
        return sum.clamp(param.min, param.max);
    }
    base
}

/// The buffers outputs are in, to read.
#[derive(Clone, Copy)]
struct Places<'a> {
    buffers: &'a [f32],
    scratch: &'a [f32],
    stride: usize,
}

impl Places<'_> {
    fn read(&self, place: Place) -> &[f32] {
        read(self.buffers, self.scratch, place, self.stride)
    }
}

fn read_all<'a>(buffers: &'a [f32], scratch: &'a [f32], stride: usize) -> Places<'a> {
    Places {
        buffers,
        scratch,
        stride,
    }
}

/// Sums `frames` of what each of `modulations` hears, mixed to mono and at its depth, into its
/// parameter's quantum of `signals`. Returns the parameters modulated, a bit each.
fn modulate(modulations: &[Modulation], places: Places<'_>, signals: &mut [f32], frames: usize) -> u64 {
    let mut modulated = 0u64;
    for modulation in modulations {
        let index = usize::from(modulation.param);
        let signal = &mut signals[index * QUANTUM..index * QUANTUM + frames];
        if modulated & (1 << index) == 0 {
            signal.fill(0.0);
            modulated |= 1 << index;
        }
        let source = places.read(modulation.from);
        let channels = usize::from(modulation.from_channels);
        #[allow(clippy::cast_precision_loss)] // A channel count.
        let share = modulation.depth / channels as f32;
        for channel in 0..channels {
            for (out, sample) in signal.iter_mut().zip(&source[channel * QUANTUM..channel * QUANTUM + frames]) {
                *out += sample * share;
            }
        }
    }
    modulated
}

/// What a voice needs of the node in a slot, kept apart from the node so the voices' hot path
/// reads a flat table rather than every node's box: whether one is there, its generation, and
/// its channels.
#[derive(Clone, Copy, Default)]
struct Head {
    live: bool,
    generation: u32,
    channels: u8,
}

impl Head {
    fn of(node: &Node) -> Head {
        Head {
            live: true,
            generation: node.generation,
            channels: u8::try_from(node.channels).unwrap_or(u8::MAX),
        }
    }
}

/// The render side of a context. See the module docs.
pub struct Engine {
    channels: usize,
    rate: u32,
    commands: Receiver<Stamped>,
    feedback: SyncSender<Feedback>,
    /// Feedback the queue had no room for, sent again before the next render. It holds at most
    /// what [`Capacity::feedback`] counts, which is what it's allocated for.
    unsent: Vec<Feedback>,
    voices: Box<[Option<Voice>]>,
    /// The slots of the voices in the table, in order: every walk over the voices walks these,
    /// not a table mostly empty.
    live: Vec<u32>,
    nodes: Box<[Option<Box<Node>>]>,
    /// Each slot's node as voices see it, kept with `nodes`.
    heads: Box<[Head]>,
    /// Nodes ringing out after their removal, which the end of a block looks for.
    draining: usize,
    /// One block of every node's first input, by slot: `channels × QUANTUM` samples each,
    /// channel after channel.
    buffers: Box<[f32]>,
    plan: Box<Plan>,
    /// Commands stamped for a later frame, latest first, so the next is at the end.
    pending: Vec<Stamped>,
    scratch: Box<[f32]>,
    /// One block of signal on its way between buffers: delayed, converted, or both.
    passing: Box<[f32]>,
    /// The sidechain of the node running.
    key: Box<[f32]>,
    /// A voice mixed in the context's layout, for an output of another.
    mixed: Box<[f32]>,
    /// The values of the running node's moving audio-rate parameters, a quantum each.
    values: Box<[f32]>,
    /// What modulates the running node's parameters, a quantum each.
    signals: Box<[f32]>,
    /// A voice's fixed-point steps, frame by frame, while its rate ramps.
    steps: Box<[u64]>,
    /// A voice's levels, frame by frame, while they ramp: two blocks.
    envelope: Box<[f32]>,
    /// True while a control-rate parameter moves, which keeps blocks to [`CONTROL_QUANTUM`].
    ramping: bool,
    /// A command changed the voices since they were last chosen: started, stopped, or paused one.
    /// A voice that runs out in the mix waits for the next span, which the cut can't move.
    changed: bool,
    /// The listener, and how the world's space works.
    space: Space,
    /// A command, or a group, held back until what it stamps for later fits the pending list.
    deferred: Option<Stamped>,
    received: u64,
    physical: usize,
    /// Each live voice's priority, loudness, and slot, for choosing which are mixed.
    ranks: Box<[(f32, f32, u32)]>,
    culled: u64,
    stolen: u64,
    revisions: u64,
    now: u64,
    late: u64,
    nonfinite: u64,
    counters: Arc<Counters>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("rate", &self.rate)
            .field("channels", &self.channels)
            .field("now", &self.now)
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl Engine {
    #[allow(clippy::too_many_arguments)] // The two ends of a context, and what it starts with.
    pub(crate) fn new(
        rate: u32,
        channels: usize,
        capacity: &Capacity,
        space: Space,
        commands: Receiver<Stamped>,
        feedback: SyncSender<Feedback>,
        counters: Arc<Counters>,
        master: Box<Node>,
        plan: Box<Plan>,
    ) -> Engine {
        let mut nodes: Box<[Option<Box<Node>>]> = std::iter::repeat_with(|| None).take(capacity.nodes).collect();
        let mut heads = vec![Head::default(); capacity.nodes].into_boxed_slice();
        heads[0] = Head::of(&master);
        nodes[0] = Some(master);
        let block = MAX_CHANNELS * QUANTUM;
        Engine {
            channels,
            rate,
            commands,
            feedback,
            unsent: Vec::with_capacity(capacity.feedback()),
            voices: std::iter::repeat_with(|| None).take(capacity.voices).collect(),
            live: Vec::with_capacity(capacity.voices),
            nodes,
            heads,
            draining: 0,
            buffers: vec![0.0; capacity.nodes * channels * QUANTUM].into_boxed_slice(),
            plan,
            pending: Vec::with_capacity(capacity.commands),
            scratch: vec![0.0; block].into_boxed_slice(),
            passing: vec![0.0; block].into_boxed_slice(),
            key: vec![0.0; block].into_boxed_slice(),
            mixed: vec![0.0; block].into_boxed_slice(),
            values: vec![0.0; MAX_PARAMS * QUANTUM].into_boxed_slice(),
            signals: vec![0.0; MAX_PARAMS * QUANTUM].into_boxed_slice(),
            steps: vec![0; QUANTUM].into_boxed_slice(),
            envelope: vec![0.0; 2 * QUANTUM].into_boxed_slice(),
            ramping: false,
            changed: false,
            space,
            deferred: None,
            received: 0,
            physical: capacity.physical.max(1),
            ranks: vec![(0.0, 0.0, 0); capacity.voices].into_boxed_slice(),
            culled: 0,
            stolen: 0,
            revisions: 0,
            now: 0,
            late: 0,
            nonfinite: 0,
            counters,
        }
    }

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

    /// The next frame to render: frames rendered so far.
    #[must_use]
    pub const fn now(&self) -> u64 {
        self.now
    }

    /// Renders interleaved frames into all of `out`, whose length is a whole number of frames.
    /// Commands are carried out on the frame they're stamped for, whatever block they fall in.
    ///
    /// # Panics
    ///
    /// If `out` isn't a whole number of frames.
    pub fn render(&mut self, out: &mut [f32]) {
        assert!(out.len().is_multiple_of(self.channels), "an output buffer holds whole frames");
        self.resend();
        let frames = out.len() / self.channels;
        let mut done = 0;
        while done < frames {
            self.receive();
            self.carry_out_due();
            // Blocks break on a grid fixed to the clock, not to the calls: whatever lengths a
            // device asks for, the same frames hear control-rate parameters and space.
            let grid = if self.ramping || self.plan.control {
                CONTROL_QUANTUM
            } else {
                QUANTUM
            };
            let into = usize::try_from(self.now % grid as u64).unwrap_or(0);
            let mut block = (grid - into).min(frames - done);
            if let Some(next) = self.pending.last().and_then(|next| next.at) {
                // Anything due was just carried out, so the next command is ahead of now.
                block = block.min(usize::try_from(next - self.now).unwrap_or(usize::MAX));
            }
            self.mix(block);
            self.write(&mut out[done * self.channels..(done + block) * self.channels], block);
            self.now += block as u64;
            done += block;
        }
        self.publish();
    }

    /// Takes every queued command. Only one that stamps more for later than the pending list
    /// has room for waits, and everything behind it with it, so a group is never split and the
    /// order is kept. The control side never sends one (it refuses with
    /// [`AudioError::ScheduleFull`](crate::audio::AudioError::ScheduleFull)), so commands for
    /// now are never stuck behind commands for later.
    fn receive(&mut self) {
        while let Some(stamped) = self.incoming() {
            if self.pending.len() + self.later(&stamped) > self.pending.capacity() {
                self.deferred = Some(stamped);
                return;
            }
            if let Command::Group(members) = stamped.command {
                self.accept_group(members);
                continue;
            }
            self.accept(stamped);
        }
    }

    /// The command held back, or the next one queued.
    fn incoming(&mut self) -> Option<Stamped> {
        if let Some(stamped) = self.deferred.take() {
            return Some(stamped);
        }
        let stamped = self.commands.try_recv().ok()?;
        self.received += 1;
        if let Command::Group(members) = &stamped.command {
            self.received += members.len() as u64;
        }
        Some(stamped)
    }

    /// Takes a group's members, which the pending list has room for.
    fn accept_group(&mut self, mut members: Vec<Stamped>) {
        // Drained rather than consumed: the emptied list goes back to be freed off this thread.
        #[allow(clippy::iter_with_drain)]
        for stamped in members.drain(..) {
            self.accept(stamped);
        }
        self.send(Feedback::Spent(members));
    }

    /// How many commands `stamped` would add to the pending list: a group's members stamped for
    /// a frame not reached yet, or the command itself if it is.
    fn later(&self, stamped: &Stamped) -> usize {
        let later = |stamped: &Stamped| stamped.at.is_some_and(|at| at > self.now);
        match &stamped.command {
            Command::Group(members) => members.iter().filter(|member| later(member)).count(),
            _ => usize::from(later(stamped)),
        }
    }

    /// Carries out a command now, or keeps it for its frame. A revision's nodes go into the table
    /// either way, so a node exists from the moment the engine hears of it.
    fn accept(&mut self, mut stamped: Stamped) {
        if let Command::Revise(revision) = &mut stamped.command {
            self.install(revision);
        }
        match stamped.at {
            Some(at) if at > self.now => {
                let key = stamped.key();
                let index = self.pending.partition_point(|waiting| waiting.key() > key);
                self.pending.insert(index, stamped);
            }
            Some(at) => {
                if at < self.now {
                    self.late += 1;
                }
                self.carry_out(stamped.command);
            }
            None => self.carry_out(stamped.command),
        }
    }

    /// Puts a revision's new nodes in the table.
    fn install(&mut self, revision: &mut Revision) {
        // Drained rather than consumed: the emptied list goes back with the revision.
        #[allow(clippy::iter_with_drain)]
        for (key, node) in revision.added.drain(..) {
            self.heads[key.slot as usize] = Head::of(&node);
            // The control side gives a slot out only once the engine has given it back, so there
            // is never one here; were there, it would still go back to be dropped.
            if let Some(old) = self.nodes[key.slot as usize].replace(node) {
                if old.drain.is_some() {
                    self.draining -= 1;
                }
                let node = NodeKey {
                    slot: key.slot,
                    generation: old.generation,
                };
                self.send(Feedback::NodeEnded { node, state: old });
            }
        }
    }

    fn carry_out_due(&mut self) {
        while self.pending.last().is_some_and(|next| next.key().0 <= self.now) {
            let Some(stamped) = self.pending.pop() else {
                return;
            };
            self.carry_out(stamped.command);
        }
    }

    fn carry_out(&mut self, command: Command) {
        match command {
            Command::Start {
                voice,
                source,
                spec,
                outputs,
            } => {
                let slot = voice.slot as usize;
                if let Some(old) = self.voices[slot].take() {
                    let old_key = VoiceKey {
                        slot: voice.slot,
                        generation: old.generation,
                    };
                    self.send(Feedback::VoiceEnded {
                        voice: old_key,
                        source: old.source,
                        outputs: old.outputs,
                    });
                }
                if let Err(index) = self.live.binary_search(&voice.slot) {
                    self.live.insert(index, voice.slot);
                }
                self.changed = true;
                self.voices[slot] = Some(Voice::new(voice.generation, source, &spec, outputs, self.rate));
            }
            Command::Stop { voice, fade } => self.stop(voice, fade),
            Command::SetVoice {
                voice,
                param,
                value,
                glide,
            } => self.set_voice(voice, param, value, glide),
            Command::SetLoop { voice, looping } => self.set_loop(voice, looping),
            Command::Pause { voice, paused } => {
                if let Some(playing) = self.voices[voice.slot as usize].as_mut()
                    && playing.generation == voice.generation
                {
                    playing.paused = paused;
                    self.changed = true;
                } else if let Some(index) = self.pending_start(voice)
                    && let Command::Start { spec, .. } = &mut self.pending[index].command
                {
                    spec.paused = paused;
                }
            }
            Command::Seek { voice, frame, epoch } => self.seek(voice, frame, epoch),
            Command::SendVoice { voice, target, level } => self.send_voice(voice, target, level),
            Command::Route { voice, mut outputs } => {
                self.route(voice, &mut outputs);
                self.send(Feedback::Outputs(outputs));
            }
            Command::Revise(revision) => self.revise(revision),
            Command::SetParam {
                node,
                index,
                value,
                glide,
            } => {
                if let Some(entry) = self.node(node) {
                    // A ramp starts moving this block; the mix keeps the flag while one moves.
                    self.ramping |= entry.automate(usize::from(index), value, glide);
                }
            }
            Command::Cancel { node, index, before } => self.cancel(node, index, before),
            // The control side never opens a group inside another; one would be taken whole.
            Command::Group(members) => self.accept_group(members),
            Command::Listener(listener) => self.space.listener = listener,
            Command::Place { voice, emitter } => {
                if let Some(playing) = self.voices[voice.slot as usize].as_mut()
                    && playing.generation == voice.generation
                {
                    playing.place(emitter);
                } else if let Some(index) = self.pending_start(voice)
                    && let Command::Start { spec, .. } = &mut self.pending[index].command
                    && spec.spatial.is_some()
                {
                    spec.spatial = Some(emitter);
                }
            }
        }
    }

    /// Drops the changes of node `node`'s parameter `index` sent before sequence `before` that
    /// still wait for their frames, and holds the parameter where it is.
    fn cancel(&mut self, node: NodeKey, index: u16, before: u64) {
        let mut position = 0;
        while position < self.pending.len() {
            let waiting = &self.pending[position];
            let doomed = waiting.sequence < before
                && matches!(waiting.command, Command::SetParam { node: known, index: param, .. }
                    if known == node && param == index);
            if doomed {
                // Nothing in a parameter's change holds memory: it can go here.
                self.pending.remove(position);
                continue;
            }
            position += 1;
        }
        if let Some(entry) = self.node(node) {
            entry.hold(usize::from(index));
        }
    }

    /// Moves a voice, or a voice waiting to start, to source frame `frame`.
    fn seek(&mut self, voice: VoiceKey, frame: u64, epoch: u64) {
        if let Some(playing) = self.voices[voice.slot as usize].as_mut()
            && playing.generation == voice.generation
        {
            playing.seek(frame, epoch);
            return;
        }
        if let Some(index) = self.pending_start(voice)
            && let Command::Start { spec, source, .. } = &mut self.pending[index].command
        {
            spec.offset = frame;
            if let Source::Stream(..) = source {
                spec.seeking = Some(epoch);
            }
        }
    }

    /// Runs a revision's plan from now on, its history taken from the plan it replaces, and ends
    /// the nodes it removed that have nothing to ring out.
    fn revise(&mut self, mut revision: Box<Revision>) {
        revision.plan.delays.inherit(&self.plan.delays);
        revision.plan.rings.inherit(&self.plan.rings);
        std::mem::swap(&mut self.plan, &mut revision.plan);
        self.revisions += 1;
        for index in 0..revision.retire.len() {
            let key = revision.retire[index];
            if self.node(key).is_some() {
                self.retire(key.slot as usize);
            }
        }
        for (voice, outputs) in &mut revision.voices {
            self.route(*voice, outputs);
        }
        self.send(Feedback::Revised(revision));
    }

    /// Swaps a voice's outputs, or a waiting voice's, for `outputs`, which then holds the old.
    fn route(&mut self, voice: VoiceKey, outputs: &mut Outputs) {
        if let Some(playing) = self.voices[voice.slot as usize].as_mut()
            && playing.generation == voice.generation
        {
            std::mem::swap(&mut playing.outputs, outputs);
            return;
        }
        if let Some(index) = self.pending_start(voice)
            && let Command::Start { outputs: waiting, .. } = &mut self.pending[index].command
        {
            std::mem::swap(waiting, outputs);
        }
    }

    /// Lets go of the node in `slot`.
    fn retire(&mut self, slot: usize) {
        let Some(state) = self.nodes[slot].take() else {
            return;
        };
        self.heads[slot].live = false;
        if state.drain.is_some() {
            self.draining -= 1;
        }
        let node = NodeKey {
            slot: u32::try_from(slot).unwrap_or(u32::MAX),
            generation: state.generation,
        };
        self.send(Feedback::NodeEnded { node, state });
    }

    fn node(&mut self, key: NodeKey) -> Option<&mut Node> {
        self.nodes[key.slot as usize]
            .as_deref_mut()
            .filter(|node| node.generation == key.generation)
    }

    fn send_voice(&mut self, voice: VoiceKey, target: u32, level: f32) {
        if let Some(playing) = self.voices[voice.slot as usize].as_mut()
            && playing.generation == voice.generation
        {
            playing.set_send(target, level);
            return;
        }
        if let Some(index) = self.pending_start(voice)
            && let Command::Start { outputs, .. } = &mut self.pending[index].command
        {
            set_level(outputs, target, level);
        }
    }

    fn stop(&mut self, voice: VoiceKey, fade: u64) {
        let slot = voice.slot as usize;
        if let Some(playing) = self.voices[slot].as_mut()
            && playing.generation == voice.generation
        {
            // A fade ends the voice when it runs out, in the mix.
            if playing.stop(fade) {
                return;
            }
            self.end_voice(slot);
            self.changed = true;
            return;
        }
        // Stopped before its start: the start never happens.
        if let Some(index) = self.pending_start(voice)
            && let Command::Start { source, outputs, .. } = self.pending.remove(index).command
        {
            self.send(Feedback::VoiceEnded { voice, source, outputs });
        }
    }

    fn end_voice(&mut self, slot: usize) {
        if let Some(ended) = self.voices[slot].take() {
            if let Ok(index) = self.live.binary_search(&u32::try_from(slot).unwrap_or(u32::MAX)) {
                self.live.remove(index);
            }
            let voice = VoiceKey {
                slot: u32::try_from(slot).unwrap_or(u32::MAX),
                generation: ended.generation,
            };
            self.send(Feedback::VoiceEnded {
                voice,
                source: ended.source,
                outputs: ended.outputs,
            });
        }
    }

    fn set_loop(&mut self, voice: VoiceKey, looping: Option<LoopRegion>) {
        if let Some(playing) = self.voices[voice.slot as usize].as_mut()
            && playing.generation == voice.generation
        {
            playing.set_loop(looping);
            return;
        }
        if let Some(index) = self.pending_start(voice)
            && let Command::Start { spec, .. } = &mut self.pending[index].command
        {
            spec.looping = looping;
        }
    }

    fn set_voice(&mut self, voice: VoiceKey, param: VoiceParam, value: f64, glide: Option<Glide>) {
        if let Some(playing) = self.voices[voice.slot as usize].as_mut()
            && playing.generation == voice.generation
        {
            playing.set(param, value, glide);
            return;
        }
        // Set before its start, ramp or not: it starts there.
        if let Some(index) = self.pending_start(voice)
            && let Command::Start { spec, .. } = &mut self.pending[index].command
        {
            #[allow(clippy::cast_possible_truncation)] // Checked by the control side.
            match param {
                VoiceParam::Gain => spec.gain = value as f32,
                VoiceParam::Pan => spec.pan = value as f32,
                VoiceParam::Rate => spec.rate = value,
            }
        }
    }

    fn pending_start(&self, voice: VoiceKey) -> Option<usize> {
        self.pending
            .iter()
            .position(|waiting| matches!(waiting.command, Command::Start { voice: start, .. } if start == voice))
    }

    fn send(&mut self, feedback: Feedback) {
        if let Err(TrySendError::Full(feedback) | TrySendError::Disconnected(feedback)) = self.feedback.try_send(feedback) {
            self.unsent.push(feedback);
        }
    }

    fn resend(&mut self) {
        while let Some(feedback) = self.unsent.pop() {
            if let Err(TrySendError::Full(feedback) | TrySendError::Disconnected(feedback)) =
                self.feedback.try_send(feedback)
            {
                self.unsent.push(feedback);
                return;
            }
        }
    }

    /// Mixes `frames` frames: every voice into its nodes, then the plan's steps in order.
    fn mix(&mut self, frames: usize) {
        let stride = self.channels * QUANTUM;
        for clear in &self.plan.clear {
            let head = self.heads[clear.slot as usize];
            if !head.live || head.generation != clear.generation {
                continue;
            }
            let buffer = &mut self.buffers[clear.slot as usize * stride..(clear.slot as usize + 1) * stride];
            for channel in 0..usize::from(clear.channels) {
                buffer[channel * QUANTUM..channel * QUANTUM + frames].fill(0.0);
            }
            if clear.more
                && let Some(node) = self.nodes[clear.slot as usize].as_deref_mut()
            {
                node.inputs.fill(0.0);
            }
        }
        self.choose();
        self.mix_voices(frames);
        // What voices added straight into a node waits for what reaches it the latest way round.
        for index in 0..self.plan.entries.len() {
            let (line, target, _) = self.plan.entries[index];
            if let Some(buffer) = input(&mut self.buffers, &mut self.nodes, target, stride) {
                self.plan.delays.line(line).process(buffer, frames);
            }
        }
        for index in 0..self.plan.feedback.len() {
            let entry = self.plan.feedback[index];
            if let Some(buffer) = input(&mut self.buffers, &mut self.nodes, entry.to, stride) {
                self.plan.rings.ring(entry.ring).read(self.now, buffer, frames);
            }
        }
        let mut ramping = false;
        for index in 0..self.plan.steps.len() {
            let step = self.plan.steps[index];
            ramping |= self.run(&step, frames);
        }
        self.ramping = ramping;
        if self.draining == 0 {
            return;
        }
        for index in 0..self.plan.steps.len() {
            let step = self.plan.steps[index];
            if self.nodes[step.slot as usize]
                .as_ref()
                .is_some_and(|node| node.generation == step.generation && node.drain == Some(0))
            {
                self.retire(step.slot as usize);
            }
        }
    }

    /// Adds every voice the budget lets mix into its outputs, and keeps time for the rest.
    fn mix_voices(&mut self, frames: usize) {
        let stride = self.channels * QUANTUM;
        let offset = usize::try_from(self.now % QUANTUM as u64).unwrap_or(0);
        let mut index = 0;
        while let Some(&slot) = self.live.get(index) {
            // A voice that ends leaves the list, and the next takes its place in it.
            index += 1;
            let slot = slot as usize;
            let Some(voice) = self.voices[slot].as_mut() else {
                continue;
            };
            if voice.paused {
                continue;
            }
            if !voice.audible {
                let keeps = match voice.culled {
                    Culled::Advance => voice.skip(frames),
                    Culled::Pause => true,
                    Culled::Stop => false,
                };
                if keeps {
                    continue;
                }
                if voice.culled == Culled::Stop {
                    self.stolen += 1;
                }
                self.end_voice(slot);
                index -= 1;
                continue;
            }
            let rendered = voice.render(&mut self.scratch, &mut self.steps, frames);
            let levels = voice.levels(rendered, offset, &mut self.envelope, self.channels);
            let mut converted = false;
            for output in voice.outputs.iter() {
                let head = self.heads[output.slot as usize];
                if !head.live || head.generation != output.generation {
                    continue;
                }
                let channels = usize::from(head.channels);
                let target = Target {
                    slot: output.slot,
                    generation: output.generation,
                    input: output.input,
                };
                if channels == self.channels {
                    // A node's first input is its buffer, and most voices play into one of those.
                    let slot = output.slot as usize;
                    let buffer = if output.input == 0 {
                        &mut self.buffers[slot * stride..(slot + 1) * stride]
                    } else if let Some(buffer) = input(&mut self.buffers, &mut self.nodes, target, stride) {
                        buffer
                    } else {
                        continue;
                    };
                    voice.mix(
                        &self.scratch,
                        rendered,
                        buffer,
                        self.channels,
                        (levels, &self.envelope),
                        output.level,
                    );
                    continue;
                }
                // Mixed once in the context's layout, then converted for each node of another.
                if !converted {
                    for channel in 0..self.channels {
                        self.mixed[channel * QUANTUM..channel * QUANTUM + rendered].fill(0.0);
                    }
                    voice.mix(
                        &self.scratch,
                        rendered,
                        &mut self.mixed,
                        self.channels,
                        (levels, &self.envelope),
                        1.0,
                    );
                    converted = true;
                }
                let Some(buffer) = input(&mut self.buffers, &mut self.nodes, target, stride) else {
                    continue;
                };
                add_converted(buffer, channels, &self.mixed, self.channels, rendered, output.level);
            }
            if rendered < frames {
                self.end_voice(slot);
                index -= 1;
            }
        }
    }

    /// Runs one step of the plan. True while a control-rate parameter of its node moves.
    #[allow(clippy::too_many_lines)] // The step's phases, in the order they happen.
    fn run(&mut self, step: &Step, frames: usize) -> bool {
        let stride = self.channels * QUANTUM;
        let channels = usize::from(step.channels);
        let Some(node) = self.nodes[step.slot as usize]
            .as_deref_mut()
            .filter(|node| node.generation == step.generation)
        else {
            return false;
        };
        if let Some(tail) = step.drain
            && node.drain.is_none()
        {
            node.drain = Some(tail);
            self.draining += 1;
        }
        // Most nodes most blocks hold every parameter still: nothing to work out.
        let still = step.modulations.start == step.modulations.end && node.moving == 0 && node.modulated == 0;
        let (varying, ramping) = if still {
            (0, false)
        } else {
            let modulated = modulate(
                &self.plan.modulations[step.modulations.range()],
                read_all(&self.buffers, &self.plan.scratch, stride),
                &mut self.signals,
                frames,
            );
            let on_grid = self.now.is_multiple_of(CONTROL_QUANTUM as u64);
            node.automate_and_modulate(frames, &mut self.values, (&self.signals, modulated), on_grid)
        };
        let key = step.key.map(|pull| {
            let source = read(&self.buffers, &self.plan.scratch, pull.from, stride);
            for channel in 0..channels {
                self.key[channel * QUANTUM..channel * QUANTUM + frames].fill(0.0);
            }
            add_converted(&mut self.key, channels, source, usize::from(pull.from_channels), frames, 1.0);
            if let Some(line) = pull.line {
                self.plan.delays.line(line).process(&mut self.key, frames);
            }
        });
        for index in step.pre.range() {
            let push = self.plan.pushes[index];
            self.push(&push, frames);
        }
        let Some(node) = self.nodes[step.slot as usize].as_deref_mut() else {
            return false;
        };
        if let Some(left) = node.drain.as_mut() {
            *left = left.saturating_sub(frames as u64);
        }
        let Node {
            processor,
            inputs,
            params,
            modulated,
            ..
        } = node;
        let modulated = *modulated;
        let buffer = step.buffer as usize * stride;
        let scratch = &mut self.plan.scratch;
        let outputs = &mut scratch[step.outputs.start as usize * stride..step.outputs.end as usize * stride];
        for output in outputs.chunks_exact_mut(stride) {
            for channel in 0..channels {
                output[channel * QUANTUM..channel * QUANTUM + frames].fill(0.0);
            }
        }
        let key = key.map(|()| &self.key[..]);
        if step.metered && !step.separate {
            // A bus: the mixer's own kernel, run straight on the buffer from the automation the
            // engine holds, without going through the processor. A mute something modulates is
            // the processor's to know.
            let gains = (varying & (1 << mixer::GAIN) != 0).then(|| &self.values[..frames]);
            #[allow(clippy::cast_possible_truncation)] // Engine gains are f32.
            let gain = node_params(params, mixer::GAIN) as f32;
            let muted = if modulated & (1 << mixer::MUTE) == 0 {
                node_params(params, mixer::MUTE) != 0.0
            } else {
                processor.value(mixer::MUTE) != 0.0
            };
            mixer::apply(
                &mut self.buffers[buffer..buffer + stride],
                channels,
                frames,
                gains,
                gain,
                muted,
            );
        } else if step.separate {
            let (main, rest) = outputs.split_at_mut(stride);
            let mut io = Io::new(main, channels, frames)
                .with_key(key)
                .with_ports(Some(&self.buffers[buffer..buffer + stride]), inputs, rest)
                .with_params(&self.values, varying);
            processor.process(&mut io);
        } else {
            let main = &mut self.buffers[buffer..buffer + stride];
            let mut io = Io::new(main, channels, frames)
                .with_key(key)
                .with_ports(None, inputs, outputs)
                .with_params(&self.values, varying);
            processor.process(&mut io);
        }
        if step.metered {
            self.meter(step.slot as usize, channels, frames);
        }
        for index in step.post.range() {
            let push = self.plan.pushes[index];
            self.push(&push, frames);
        }
        for index in step.feedback.range() {
            let entry = self.plan.feedback[index];
            let ring = self.plan.rings.ring(entry.ring);
            let source = read(&self.buffers, &self.plan.scratch, entry.from, stride);
            let (from, to) = (usize::from(entry.from_channels), ring.channels());
            if from == to {
                ring.write(self.now, source, frames);
                continue;
            }
            for channel in 0..to {
                self.passing[channel * QUANTUM..channel * QUANTUM + frames].fill(0.0);
            }
            add_converted(&mut self.passing, to, source, from, frames, 1.0);
            ring.write(self.now, &self.passing, frames);
        }
        ramping
    }

    /// Adds `frames` of a push's source into its target, at its level, converted, and through its
    /// compensation line if it has one.
    fn push(&mut self, push: &Push, frames: usize) {
        let stride = self.channels * QUANTUM;
        let (from, to) = (usize::from(push.from_channels), usize::from(push.to_channels));
        if push.line.is_none()
            && let Place::Buffer(source) = push.from
            && push.to.input == 0
            && source != push.to.slot
        {
            let head = self.heads[push.to.slot as usize];
            if !head.live || head.generation != push.to.generation {
                return;
            }
            // The common case, a bus into its parent: straight from one buffer into the other.
            let (source, target) = two(&mut self.buffers, stride, source as usize, push.to.slot as usize);
            add_converted(target, to, source, from, frames, push.level);
            return;
        }
        let source = read(&self.buffers, &self.plan.scratch, push.from, stride);
        for channel in 0..to {
            self.passing[channel * QUANTUM..channel * QUANTUM + frames].fill(0.0);
        }
        add_converted(&mut self.passing, to, source, from, frames, 1.0);
        if let Some(line) = push.line {
            self.plan.delays.line(line).process(&mut self.passing, frames);
        }
        if let Some(target) = input(&mut self.buffers, &mut self.nodes, push.to, stride) {
            add_converted(target, to, &self.passing, to, frames, push.level);
        }
    }

    /// Works out where every voice in space is, and marks the voices this block mixes: all of
    /// them while they fit the budget, else the ones that matter most, by priority and then by
    /// loudness. A voice already mixed holds its place against one only a little louder, so voices
    /// near the line don't flap in and out. A paused voice is neither mixed nor counted.
    ///
    /// Space is heard, and the voices chosen, at the start of each [`QUANTUM`] of the clock, and
    /// chosen again on any frame the voices change, so neither depends on how a render is cut.
    fn choose(&mut self) {
        let span = self.now.is_multiple_of(QUANTUM as u64);
        if !span && !self.changed {
            for &slot in &self.live {
                if let Some(voice) = self.voices[slot as usize].as_mut()
                    && voice.unheard()
                {
                    voice.hear(&self.space, self.channels);
                }
            }
            return;
        }
        self.changed = false;
        let mut count = 0;
        for &slot in &self.live {
            let Some(voice) = self.voices[slot as usize].as_mut() else {
                continue;
            };
            if voice.paused {
                continue;
            }
            if span || voice.unheard() {
                voice.hear(&self.space, self.channels);
            }
            let held = if voice.audible { 1.25 } else { 1.0 };
            self.ranks[count] = (voice.priority, voice.loudness() * held, slot);
            count += 1;
        }
        if count <= self.physical {
            for &slot in &self.live {
                if let Some(voice) = self.voices[slot as usize].as_mut() {
                    voice.audible = true;
                }
            }
            self.culled = 0;
            return;
        }
        let ranks = &mut self.ranks[..count];
        ranks.select_nth_unstable_by(self.physical - 1, |first, second| {
            second
                .0
                .total_cmp(&first.0)
                .then(second.1.total_cmp(&first.1))
                .then(first.2.cmp(&second.2))
        });
        for &slot in &self.live {
            if let Some(voice) = self.voices[slot as usize].as_mut() {
                voice.audible = false;
            }
        }
        for &(_, _, slot) in &ranks[..self.physical] {
            if let Some(voice) = self.voices[slot as usize].as_mut() {
                voice.audible = true;
            }
        }
        self.culled = (count - self.physical) as u64;
    }

    /// Meters `frames` of node slot `slot`'s buffer as it leaves the node, after its gain.
    fn meter(&self, slot: usize, channels: usize, frames: usize) {
        let stride = self.channels * QUANTUM;
        let block = &self.buffers[slot * stride..(slot + 1) * stride];
        let (mut peak, mut energy) = (0.0f32, 0.0f64);
        for channel in 0..channels {
            for sample in &block[channel * QUANTUM..channel * QUANTUM + frames] {
                let sample = f64::from(*sample);
                let square = sample * sample;
                #[allow(clippy::cast_possible_truncation)] // Back to the f32 it came from.
                let magnitude = sample.abs() as f32;
                peak = peak.max(magnitude);
                energy += square;
            }
        }
        // Not-a-number compares false with everything; the meter shows it as nothing.
        self.counters.meters[slot].add(peak, energy, (frames * channels) as u64);
    }

    /// Interleaves `frames` of the master into `out`, writing any sample that isn't finite as
    /// silence: one bad voice must not poison a device.
    fn write(&mut self, out: &mut [f32], frames: usize) {
        for frame in 0..frames {
            for channel in 0..self.channels {
                let sample = self.buffers[channel * QUANTUM + frame];
                out[frame * self.channels + channel] = if sample.is_finite() {
                    sample
                } else {
                    self.nonfinite += 1;
                    0.0
                };
            }
        }
    }

    fn publish(&self) {
        for &slot in &self.live {
            let Some(voice) = self.voices[slot as usize].as_ref() else {
                continue;
            };
            let paused = if voice.paused { PAUSED } else { 0 };
            self.counters.positions[slot as usize].store(voice.position().min(PAUSED - 1) | paused, Ordering::Relaxed);
        }
        self.counters.frames.store(self.now, Ordering::Release);
        self.counters.active.store(self.live.len() as u64, Ordering::Relaxed);
        self.counters.late.store(self.late, Ordering::Relaxed);
        self.counters.nonfinite.store(self.nonfinite, Ordering::Relaxed);
        self.counters.pending.store(self.pending.len() as u64, Ordering::Relaxed);
        self.counters.received.store(self.received, Ordering::Release);
        self.counters.culled.store(self.culled, Ordering::Relaxed);
        self.counters.stolen.store(self.stolen, Ordering::Relaxed);
        self.counters.revisions.store(self.revisions, Ordering::Relaxed);
    }
}

/// The block of a node input: its buffer for the first, the node's own storage for the rest.
fn input<'a>(
    buffers: &'a mut [f32],
    nodes: &'a mut [Option<Box<Node>>],
    target: Target,
    stride: usize,
) -> Option<&'a mut [f32]> {
    let node = nodes[target.slot as usize]
        .as_deref_mut()
        .filter(|node| node.generation == target.generation)?;
    let slot = target.slot as usize;
    match target.input {
        0 => Some(&mut buffers[slot * stride..(slot + 1) * stride]),
        input => {
            let index = usize::from(input) - 1;
            node.inputs.get_mut(index * stride..(index + 1) * stride)
        }
    }
}

/// The block an output is in.
fn read<'a>(buffers: &'a [f32], scratch: &'a [f32], place: Place, stride: usize) -> &'a [f32] {
    match place {
        Place::Buffer(slot) => &buffers[slot as usize * stride..(slot as usize + 1) * stride],
        Place::Scratch(index) => &scratch[index as usize * stride..(index as usize + 1) * stride],
    }
}

/// Adds `frames` of `from`, `sources` channels, into `to`, `targets` channels, at `level`: the
/// same count as it is, mono into every channel, and many channels into mono as their average.
fn add_converted(to: &mut [f32], targets: usize, from: &[f32], sources: usize, frames: usize, level: f32) {
    let add = |to: &mut [f32], from: &[f32], level: f32| {
        for (out, sample) in to.iter_mut().zip(from) {
            *out += sample * level;
        }
    };
    if sources == targets {
        for channel in 0..targets {
            let range = channel * QUANTUM..channel * QUANTUM + frames;
            add(&mut to[range.clone()], &from[range], level);
        }
        return;
    }
    if sources == 1 {
        for channel in 0..targets {
            add(&mut to[channel * QUANTUM..channel * QUANTUM + frames], &from[..frames], level);
        }
        return;
    }
    #[allow(clippy::cast_precision_loss)] // A channel count.
    let share = level / sources as f32;
    for channel in 0..sources {
        add(&mut to[..frames], &from[channel * QUANTUM..channel * QUANTUM + frames], share);
    }
}

/// The blocks of slots `from` and `to` (which differ) of `buffers`, the first to read and the
/// second to add into.
fn two(buffers: &mut [f32], stride: usize, from: usize, to: usize) -> (&[f32], &mut [f32]) {
    if from < to {
        let (low, high) = buffers.split_at_mut(to * stride);
        (&low[from * stride..(from + 1) * stride], &mut high[..stride])
    } else {
        let (low, high) = buffers.split_at_mut(from * stride);
        (&high[..stride], &mut low[to * stride..(to + 1) * stride])
    }
}
