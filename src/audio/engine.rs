//! The engine: everything that runs on the render thread.
//!
//! [`Engine::render`] is the whole of it. A device callback calls it for each buffer the device
//! asks for, and an [`OfflineContext`](crate::audio::OfflineContext) calls it in a loop: the same
//! code, so an offline render is the test of what a device plays.
//!
//! Rendering never allocates, frees, locks, or blocks. Everything it needs is allocated when the
//! engine is made; commands arrive on a bounded queue it only polls, and anything holding memory
//! that it is done with (the [`Pcm`] of an ended voice, an order it replaced) goes back to the
//! control side on a second queue to be dropped there.
//!
//! Each block, voices add into the buses they play into, then the buses are walked in the order
//! the control side last sent: each applies its gain and adds into its parent, and the master,
//! last, is the output.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};

use crate::audio::bus::{Bus, BusSend};
use crate::audio::param::{Glide, Param};
use crate::audio::processor::{Block, Processor, ProcessorKey, Tail};
use crate::audio::spatial::{Emitter, Listener, Space};
use crate::audio::voice::{Culled, Source, Voice, VoiceKey};
use crate::audio::{BusKey, BusParam, BusSpec, LoopRegion, MAX_CHANNELS, PlaySpec, QUANTUM, Tap, VoiceParam};

/// What the control side asks of the engine.
pub enum Command {
    Start {
        voice: VoiceKey,
        source: Source,
        spec: PlaySpec,
    },
    /// Stops a voice, fading out over `fade` frames first.
    Stop {
        voice: VoiceKey,
        fade: u64,
    },
    SetVoice {
        voice: VoiceKey,
        param: VoiceParam,
        value: f64,
        glide: Option<Glide>,
    },
    /// Loops a voice over a new region, or lets it play on past its loop.
    SetLoop {
        voice: VoiceKey,
        looping: Option<LoopRegion>,
    },
    /// A new bus, and the order with it in.
    AddBus {
        bus: BusKey,
        spec: BusSpec,
        order: Box<[u32]>,
    },
    /// Removes a bus nothing routes to any more.
    RemoveBus {
        bus: BusKey,
    },
    /// Moves a bus to a new parent, and the order that goes with the move.
    RouteBus {
        bus: BusKey,
        parent: u32,
        order: Box<[u32]>,
    },
    SetBus {
        bus: BusKey,
        param: BusParam,
        value: f64,
        glide: Option<Glide>,
    },
    /// Adds or changes a bus's send, with the new order when the send is new.
    SendBus {
        bus: BusKey,
        send: BusSend,
        order: Option<Box<[u32]>>,
    },
    UnsendBus {
        bus: BusKey,
        target: u32,
        order: Option<Box<[u32]>>,
    },
    /// Changes the level of a voice's send.
    SendVoice {
        voice: VoiceKey,
        target: u32,
        level: f32,
    },
    /// Puts a processor in a bus's chain at `index`.
    Mount {
        processor: ProcessorKey,
        instance: Box<dyn Processor>,
        bus: BusKey,
        index: usize,
    },
    Unmount {
        processor: ProcessorKey,
    },
    SetProcessor {
        processor: ProcessorKey,
        index: usize,
        value: f64,
        glide: Option<Glide>,
    },
    /// Commands that arrive together, each with its own stamp.
    Group(Vec<Stamped>),
    /// Moves the listener.
    Listener(Listener),
    /// Moves a voice in space.
    Place {
        voice: VoiceKey,
        emitter: Emitter,
    },
    /// Has a processor listen to a bus, with the order that mixes the bus first; or to its own
    /// bus again, which leaves the order as it is.
    Listen {
        processor: ProcessorKey,
        key: Option<u32>,
        order: Option<Box<[u32]>>,
    },
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
    /// once the control side has read this, and the PCM drops there.
    VoiceEnded { voice: VoiceKey, source: Source },
    /// A bus was removed and has rung out; its slot is free once the control side has read this.
    BusEnded { bus: BusKey },
    /// A processor left its bus, to be dropped.
    ProcessorEnded {
        processor: ProcessorKey,
        instance: Box<dyn Processor>,
    },
    /// An order the engine replaced, to drop.
    Order(Box<[u32]>),
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
    /// Each bus's meter, by slot.
    pub(crate) meters: Box<[BusMeter]>,
}

impl Counters {
    pub fn new(buses: usize) -> Counters {
        Counters {
            meters: std::iter::repeat_with(BusMeter::default).take(buses).collect(),
            ..Counters::default()
        }
    }
}

/// What one bus put out since the control side last read it, after its gain: the largest
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
    pub buses: usize,
    pub processors: usize,
    pub commands: usize,
    /// Voices mixed at once; the rest wait their turn.
    pub physical: usize,
}

impl Capacity {
    /// The feedback the engine can owe at once: each voice, bus, and processor ends once and its
    /// slot is reused only after the control side reads that, and each command replaces at most
    /// one order.
    pub const fn feedback(&self) -> usize {
        // A group sends its spent list back as well as whatever its members do.
        self.voices + self.buses + self.processors + 2 * self.commands
    }
}

/// A processor in a bus's chain.
struct Mounted {
    generation: u32,
    bus: u32,
    instance: Box<dyn Processor>,
    /// Parameters ramping, by index: set at the start of each block of at most
    /// [`CONTROL_QUANTUM`] frames while they move.
    ramps: [Option<(usize, Param)>; MAX_RAMPS],
    /// The slot of the bus it listens to, which the order mixes before its own.
    key: Option<u32>,
}

/// The most of one processor's parameters that ramp at once; a further ramp replaces the oldest.
const MAX_RAMPS: usize = 4;

/// The longest block the engine mixes while a processor parameter ramps: how often a processor
/// hears where its ramp has got to.
pub const CONTROL_QUANTUM: usize = 32;

impl Mounted {
    fn ramp(&mut self, index: usize, value: f64, glide: Option<Glide>) {
        let Some(glide) = glide.filter(|glide| glide.frames > 0) else {
            for ramp in &mut self.ramps {
                if ramp.is_some_and(|(known, _)| known == index) {
                    *ramp = None;
                }
            }
            self.instance.set(index, value);
            return;
        };
        let slot = self
            .ramps
            .iter()
            .position(|ramp| ramp.is_some_and(|(known, _)| known == index))
            .or_else(|| self.ramps.iter().position(Option::is_none))
            .unwrap_or(0);
        // The ramp starts from where the parameter is: its last ramp's value, or its own.
        let start = self.ramps[slot]
            .filter(|(known, _)| *known == index)
            .map_or_else(|| self.current(index), |(_, param)| param.value());
        let mut param = Param::new(start);
        param.ramp(value, glide);
        self.ramps[slot] = Some((index, param));
    }

    /// What parameter `index` was last set to; a processor that keeps its own state reports its
    /// default until set.
    fn current(&self, index: usize) -> f64 {
        self.instance.value(index)
    }

    /// Tells the processor where its ramps are, and moves them `frames` along. True while any
    /// still moves.
    fn step_ramps(&mut self, frames: usize) -> bool {
        let mut moving = false;
        for ramp in &mut self.ramps {
            let Some((index, param)) = ramp.as_mut() else {
                continue;
            };
            // A block hears the value its first frame has; a ramp done by then is let go.
            self.instance.set(*index, param.value());
            if param.steady() {
                *ramp = None;
                continue;
            }
            param.advance(frames as u64);
            moving = true;
        }
        moving
    }
}

/// The render side of a context. See the module docs.
pub struct Engine {
    channels: usize,
    rate: u32,
    commands: Receiver<Stamped>,
    feedback: SyncSender<Feedback>,
    /// Feedback the queue had no room for, sent again before the next render. It holds at most
    /// one entry per voice, bus, and command, which is what it's allocated for.
    unsent: Vec<Feedback>,
    voices: Box<[Option<Voice>]>,
    buses: Box<[Option<Bus>]>,
    processors: Box<[Option<Mounted>]>,
    /// Bus slots, each before every bus it feeds; slots of removed buses are skipped.
    order: Box<[u32]>,
    /// One block of every bus: `channels × QUANTUM` samples per slot, channel after channel.
    mixes: Box<[f32]>,
    /// Commands stamped for a later frame, latest first, so the next is at the end.
    pending: Vec<Stamped>,
    scratch: Box<[f32]>,
    /// A voice's fixed-point steps, frame by frame, while its rate ramps.
    steps: Box<[u64]>,
    /// A voice's or bus's levels, frame by frame, while they ramp: two blocks.
    envelope: Box<[f32]>,
    /// True while a processor parameter ramps, which keeps blocks to [`CONTROL_QUANTUM`].
    ramping: bool,
    /// The listener, and how the world's space works.
    space: Space,
    /// A group held back until its members fit the pending list.
    deferred: Option<Vec<Stamped>>,
    received: u64,
    physical: usize,
    /// Each live voice's priority, loudness, and slot, for choosing which are mixed.
    ranks: Box<[(f32, f32, u32)]>,
    culled: u64,
    stolen: u64,
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
    pub(crate) fn new(
        rate: u32,
        channels: usize,
        capacity: &Capacity,
        space: Space,
        commands: Receiver<Stamped>,
        feedback: SyncSender<Feedback>,
        counters: Arc<Counters>,
    ) -> Engine {
        let mut buses: Box<[Option<Bus>]> = std::iter::repeat_with(|| None).take(capacity.buses).collect();
        buses[0] = Some(Bus::new(0, None, &BusSpec::default()));
        Engine {
            channels,
            rate,
            commands,
            feedback,
            unsent: Vec::with_capacity(capacity.feedback()),
            voices: std::iter::repeat_with(|| None).take(capacity.voices).collect(),
            buses,
            processors: std::iter::repeat_with(|| None).take(capacity.processors).collect(),
            order: Box::new([0]),
            mixes: vec![0.0; capacity.buses * channels * QUANTUM].into_boxed_slice(),
            pending: Vec::with_capacity(capacity.commands),
            scratch: vec![0.0; MAX_CHANNELS * QUANTUM].into_boxed_slice(),
            steps: vec![0; QUANTUM].into_boxed_slice(),
            envelope: vec![0.0; 2 * QUANTUM].into_boxed_slice(),
            ramping: false,
            space,
            deferred: None,
            received: 0,
            physical: capacity.physical.max(1),
            ranks: vec![(0.0, 0.0, 0); capacity.voices].into_boxed_slice(),
            culled: 0,
            stolen: 0,
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
            let longest = if self.ramping { CONTROL_QUANTUM } else { QUANTUM };
            let mut block = longest.min(frames - done);
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

    /// Takes queued commands while there's room to hold the ones not yet due. When the pending
    /// list is full the rest wait in the queue, and once that fills the control side is told.
    fn receive(&mut self) {
        if let Some(group) = self.deferred.take()
            && !self.accept_group(group)
        {
            return;
        }
        while self.pending.len() < self.pending.capacity() {
            let Ok(stamped) = self.commands.try_recv() else {
                return;
            };
            self.received += 1;
            if let Command::Group(members) = stamped.command {
                self.received += members.len() as u64;
                if !self.accept_group(members) {
                    return;
                }
                continue;
            }
            self.accept(stamped);
        }
    }

    /// Takes a group's members, or holds the group back until its stamped members fit the
    /// pending list, so its members are never split. False when held back.
    fn accept_group(&mut self, mut members: Vec<Stamped>) -> bool {
        if self.pending.len() + members.len() > self.pending.capacity() {
            self.deferred = Some(members);
            return false;
        }
        // Drained rather than consumed: the emptied list goes back to be freed off this thread.
        #[allow(clippy::iter_with_drain)]
        for stamped in members.drain(..) {
            self.accept(stamped);
        }
        self.send(Feedback::Spent(members));
        true
    }

    /// Carries out a command now, or keeps it for its frame.
    fn accept(&mut self, stamped: Stamped) {
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

    fn carry_out_due(&mut self) {
        while self.pending.last().is_some_and(|next| next.key().0 <= self.now) {
            let Some(stamped) = self.pending.pop() else {
                return;
            };
            self.carry_out(stamped.command);
        }
    }

    #[allow(clippy::too_many_lines)] // one arm per command
    fn carry_out(&mut self, command: Command) {
        match command {
            Command::Start { voice, source, spec } => {
                let slot = voice.slot as usize;
                if let Some(old) = self.voices[slot].take() {
                    let old_key = VoiceKey {
                        slot: voice.slot,
                        generation: old.generation,
                    };
                    self.send(Feedback::VoiceEnded {
                        voice: old_key,
                        source: old.source,
                    });
                }
                self.voices[slot] = Some(Voice::new(voice.generation, source, &spec, self.rate));
            }
            Command::Stop { voice, fade } => self.stop(voice, fade),
            Command::SetVoice {
                voice,
                param,
                value,
                glide,
            } => self.set_voice(voice, param, value, glide),
            Command::SetLoop { voice, looping } => self.set_loop(voice, looping),
            Command::AddBus { bus, spec, order } => {
                self.buses[bus.slot as usize] = Some(Bus::new(bus.generation, Some(spec.parent.slot), &spec));
                self.reorder(order);
            }
            Command::RemoveBus { bus } => {
                let Some(removed) = self.buses[bus.slot as usize].as_ref() else {
                    return;
                };
                if removed.generation != bus.generation || removed.draining.is_some() {
                    return;
                }
                let tail = removed
                    .chain()
                    .iter()
                    .filter_map(|slot| self.processors[*slot as usize].as_ref())
                    .map(|mounted| match mounted.instance.tail() {
                        Tail::None => 0,
                        Tail::Frames(frames) => frames,
                    })
                    .max()
                    .unwrap_or(0);
                if tail == 0 {
                    self.retire_bus(bus.slot as usize);
                } else if let Some(removed) = self.buses[bus.slot as usize].as_mut() {
                    removed.draining = Some(tail);
                }
            }
            Command::RouteBus { bus, parent, order } => {
                if let Some(routed) = self.bus(bus) {
                    routed.parent = Some(parent);
                }
                self.reorder(order);
            }
            Command::SetBus {
                bus,
                param,
                value,
                glide,
            } => {
                if let Some(changed) = self.bus(bus) {
                    changed.set(param, value, glide);
                }
            }
            Command::SendBus { bus, send, order } => {
                if let Some(sending) = self.bus(bus) {
                    sending.send(send);
                }
                if let Some(order) = order {
                    self.reorder(order);
                }
            }
            Command::UnsendBus { bus, target, order } => {
                if let Some(sending) = self.bus(bus) {
                    sending.unsend(target);
                }
                if let Some(order) = order {
                    self.reorder(order);
                }
            }
            Command::SendVoice { voice, target, level } => self.send_voice(voice, target, level),
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
            Command::Mount {
                processor,
                instance,
                bus,
                index,
            } => {
                let Some(chain) = self.bus(bus) else {
                    self.send(Feedback::ProcessorEnded { processor, instance });
                    return;
                };
                chain.mount(processor.slot, index);
                self.processors[processor.slot as usize] = Some(Mounted {
                    generation: processor.generation,
                    bus: bus.slot,
                    instance,
                    ramps: [None; MAX_RAMPS],
                    key: None,
                });
            }
            Command::Unmount { processor } => {
                let slot = processor.slot as usize;
                if self.processors[slot]
                    .as_ref()
                    .is_none_or(|mounted| mounted.generation != processor.generation)
                {
                    return;
                }
                if let Some(mounted) = self.processors[slot].take() {
                    if let Some(bus) = self.buses[mounted.bus as usize].as_mut() {
                        bus.unmount(processor.slot);
                    }
                    self.send(Feedback::ProcessorEnded {
                        processor,
                        instance: mounted.instance,
                    });
                }
            }
            Command::SetProcessor {
                processor,
                index,
                value,
                glide,
            } => {
                if let Some(mounted) = self.processors[processor.slot as usize].as_mut()
                    && mounted.generation == processor.generation
                {
                    mounted.ramp(index, value, glide);
                    // A ramp starts moving this block; the mix keeps the flag while one moves.
                    self.ramping |= mounted.ramps.iter().any(Option::is_some);
                }
            }
            // The control side never opens a group inside another; one would be taken whole.
            Command::Group(members) => {
                self.accept_group(members);
            }
            Command::Listen { processor, key, order } => {
                if let Some(mounted) = self.processors[processor.slot as usize].as_mut()
                    && mounted.generation == processor.generation
                {
                    mounted.key = key;
                }
                if let Some(order) = order {
                    self.reorder(order);
                }
            }
        }
    }

    /// Lets go of the bus in `slot` and every processor it runs.
    fn retire_bus(&mut self, slot: usize) {
        let Some(bus) = self.buses[slot].take() else {
            return;
        };
        for &processor in bus.chain() {
            if let Some(mounted) = self.processors[processor as usize].take() {
                let processor = ProcessorKey {
                    slot: processor,
                    generation: mounted.generation,
                };
                self.send(Feedback::ProcessorEnded {
                    processor,
                    instance: mounted.instance,
                });
            }
        }
        let bus = BusKey {
            slot: u32::try_from(slot).unwrap_or(u32::MAX),
            generation: bus.generation,
        };
        self.send(Feedback::BusEnded { bus });
    }

    fn send_voice(&mut self, voice: VoiceKey, target: u32, level: f32) {
        if let Some(playing) = self.voices[voice.slot as usize].as_mut()
            && playing.generation == voice.generation
        {
            playing.set_send(target, level);
            return;
        }
        if let Some(index) = self.pending_start(voice)
            && let Command::Start { spec, .. } = &mut self.pending[index].command
        {
            for send in spec.sends.iter_mut().flatten() {
                if send.bus.slot == target {
                    send.level = level;
                }
            }
        }
    }

    fn bus(&mut self, bus: BusKey) -> Option<&mut Bus> {
        self.buses[bus.slot as usize]
            .as_mut()
            .filter(|known| known.generation == bus.generation)
    }

    fn reorder(&mut self, order: Box<[u32]>) {
        let old = std::mem::replace(&mut self.order, order);
        self.send(Feedback::Order(old));
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
            if let Some(stopped) = self.voices[slot].take() {
                self.send(Feedback::VoiceEnded {
                    voice,
                    source: stopped.source,
                });
            }
            return;
        }
        // Stopped before its start: the start never happens.
        if let Some(index) = self.pending_start(voice)
            && let Command::Start { source, .. } = self.pending.remove(index).command
        {
            self.send(Feedback::VoiceEnded { voice, source });
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

    /// Mixes `frames` frames: every voice into its bus, then every bus into its parent.
    fn mix(&mut self, frames: usize) {
        let stride = self.channels * QUANTUM;
        for &slot in &self.order {
            let mix = &mut self.mixes[slot as usize * stride..(slot as usize + 1) * stride];
            for channel in 0..self.channels {
                mix[channel * QUANTUM..channel * QUANTUM + frames].fill(0.0);
            }
        }
        self.choose();
        for slot in 0..self.voices.len() {
            let Some(voice) = self.voices[slot].as_mut() else {
                continue;
            };
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
                if let Some(ended) = self.voices[slot].take() {
                    let voice = VoiceKey {
                        slot: u32::try_from(slot).unwrap_or(u32::MAX),
                        generation: ended.generation,
                    };
                    self.send(Feedback::VoiceEnded {
                        voice,
                        source: ended.source,
                    });
                }
                continue;
            }
            let rendered = voice.render(&mut self.scratch, &mut self.steps, frames);
            let levels = voice.levels(rendered, &mut self.envelope, self.channels);
            let bus = voice.bus as usize;
            voice.mix(
                &self.scratch,
                rendered,
                &mut self.mixes[bus * stride..(bus + 1) * stride],
                self.channels,
                (levels, &self.envelope),
                1.0,
            );
            for (target, level) in voice.sends.into_iter().flatten() {
                let target = target as usize;
                voice.mix(
                    &self.scratch,
                    rendered,
                    &mut self.mixes[target * stride..(target + 1) * stride],
                    self.channels,
                    (levels, &self.envelope),
                    level,
                );
            }
            if rendered == frames {
                continue;
            }
            if let Some(ended) = self.voices[slot].take() {
                let voice = VoiceKey {
                    slot: u32::try_from(slot).unwrap_or(u32::MAX),
                    generation: ended.generation,
                };
                self.send(Feedback::VoiceEnded {
                    voice,
                    source: ended.source,
                });
            }
        }
        self.mix_buses(frames);
    }

    /// Mixes `frames` frames of every bus, in order: its processors, its pre-fader sends, its
    /// gain, its meter, its post-fader sends, and its parent. A bus that has rung out goes.
    fn mix_buses(&mut self, frames: usize) {
        let stride = self.channels * QUANTUM;
        let mut ramping = false;
        for index in 0..self.order.len() {
            let slot = self.order[index] as usize;
            let Some((chain, length)) = self.buses[slot].as_ref().map(|bus| (bus.chain, bus.length)) else {
                continue;
            };
            for &processor in &chain[..length] {
                let Some(mounted) = self.processors[processor as usize].as_mut() else {
                    continue;
                };
                ramping |= mounted.step_ramps(frames);
                // A key the order put first, and that is still there, is heard as it left its bus.
                let key = mounted
                    .key
                    .map(|key| key as usize)
                    .filter(|key| *key != slot && self.buses[*key].is_some());
                let (heard, samples) = match key {
                    Some(key) => {
                        let (heard, samples) = two(&mut self.mixes, stride, key, slot);
                        (Some(heard), samples)
                    }
                    None => (None, &mut self.mixes[slot * stride..(slot + 1) * stride]),
                };
                let mut block = Block {
                    samples,
                    channels: self.channels,
                    frames,
                };
                mounted.instance.process(&mut block, heard);
            }
            let Some(bus) = self.buses[slot].as_mut() else {
                continue;
            };
            if let Some(left) = bus.draining.as_mut() {
                *left = left.saturating_sub(frames as u64);
            }
            let (parent, sends) = (bus.parent, bus.sends);
            for send in sends.into_iter().flatten().filter(|send| send.tap == Tap::PreFader) {
                self.add(slot, send.target as usize, send.level, frames);
            }
            if let Some(bus) = self.buses[slot].as_mut() {
                bus.apply(
                    &mut self.mixes[slot * stride..(slot + 1) * stride],
                    self.channels,
                    frames,
                    &mut self.envelope,
                );
            }
            self.meter(slot, frames);
            for send in sends.into_iter().flatten().filter(|send| send.tap == Tap::PostFader) {
                self.add(slot, send.target as usize, send.level, frames);
            }
            if let Some(parent) = parent {
                self.add(slot, parent as usize, 1.0, frames);
            }
        }
        self.ramping = ramping;
        for index in 0..self.order.len() {
            let slot = self.order[index] as usize;
            if self.buses[slot].as_ref().is_some_and(|bus| bus.draining == Some(0)) {
                self.retire_bus(slot);
            }
        }
    }

    /// Works out where every voice in space is, and marks the voices this block mixes: all of
    /// them while they fit the budget, else the ones that matter most, by priority and then by
    /// loudness. A voice already mixed holds its place against one only a little louder, so voices
    /// near the line don't flap in and out.
    fn choose(&mut self) {
        let mut count = 0;
        for (slot, voice) in self.voices.iter_mut().enumerate() {
            let Some(voice) = voice.as_mut() else {
                continue;
            };
            voice.hear(&self.space, self.channels);
            let held = if voice.audible { 1.25 } else { 1.0 };
            self.ranks[count] = (
                voice.priority,
                voice.loudness() * held,
                u32::try_from(slot).unwrap_or(u32::MAX),
            );
            count += 1;
        }
        if count <= self.physical {
            for voice in self.voices.iter_mut().flatten() {
                voice.audible = true;
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
        for voice in self.voices.iter_mut().flatten() {
            voice.audible = false;
        }
        for &(_, _, slot) in &ranks[..self.physical] {
            if let Some(voice) = self.voices[slot as usize].as_mut() {
                voice.audible = true;
            }
        }
        self.culled = (count - self.physical) as u64;
    }

    /// Meters `frames` of bus slot `slot` as it leaves the bus, after its gain.
    fn meter(&self, slot: usize, frames: usize) {
        let stride = self.channels * QUANTUM;
        let block = &self.mixes[slot * stride..(slot + 1) * stride];
        let (mut peak, mut energy) = (0.0f32, 0.0f64);
        for channel in 0..self.channels {
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
        self.counters.meters[slot].add(peak, energy, (frames * self.channels) as u64);
    }

    /// Adds `frames` of bus slot `from` into bus slot `to`, at `gain`.
    fn add(&mut self, from: usize, to: usize, gain: f32, frames: usize) {
        let stride = self.channels * QUANTUM;
        let (from, to) = two(&mut self.mixes, stride, from, to);
        for channel in 0..self.channels {
            let range = channel * QUANTUM..channel * QUANTUM + frames;
            for (out, sample) in to[range.clone()].iter_mut().zip(&from[range]) {
                *out += sample * gain;
            }
        }
    }

    /// Interleaves `frames` of the master bus into `out`, writing any sample that isn't finite as
    /// silence: one bad voice must not poison a device.
    fn write(&mut self, out: &mut [f32], frames: usize) {
        for frame in 0..frames {
            for channel in 0..self.channels {
                let sample = self.mixes[channel * QUANTUM + frame];
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
        let active = self.voices.iter().filter(|voice| voice.is_some()).count();
        self.counters.frames.store(self.now, Ordering::Release);
        self.counters.active.store(active as u64, Ordering::Relaxed);
        self.counters.late.store(self.late, Ordering::Relaxed);
        self.counters.nonfinite.store(self.nonfinite, Ordering::Relaxed);
        self.counters.pending.store(self.pending.len() as u64, Ordering::Relaxed);
        self.counters.received.store(self.received, Ordering::Release);
        self.counters.culled.store(self.culled, Ordering::Relaxed);
        self.counters.stolen.store(self.stolen, Ordering::Relaxed);
    }
}

/// The blocks of slots `from` and `to` (which differ) of `mixes`, the first to read and the
/// second to add into.
fn two(mixes: &mut [f32], stride: usize, from: usize, to: usize) -> (&[f32], &mut [f32]) {
    if from < to {
        let (low, high) = mixes.split_at_mut(to * stride);
        (&low[from * stride..(from + 1) * stride], &mut high[..stride])
    } else {
        let (low, high) = mixes.split_at_mut(from * stride);
        (&high[..stride], &mut low[to * stride..(to + 1) * stride])
    }
}
