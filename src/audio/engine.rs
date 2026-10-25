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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};

use crate::audio::bus::Bus;
use crate::audio::voice::{Voice, VoiceKey};
use crate::audio::{BusKey, BusParam, BusSpec, MAX_CHANNELS, Pcm, PlaySpec, QUANTUM, VoiceParam};

/// What the control side asks of the engine.
pub enum Command {
    Start {
        voice: VoiceKey,
        pcm: Arc<Pcm>,
        spec: PlaySpec,
    },
    Stop {
        voice: VoiceKey,
    },
    SetVoice {
        voice: VoiceKey,
        param: VoiceParam,
        value: f64,
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
    },
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
    VoiceEnded { voice: VoiceKey, pcm: Arc<Pcm> },
    /// A bus was removed; its slot is free once the control side has read this.
    BusEnded { bus: BusKey },
    /// An order the engine replaced, to drop.
    Order(Box<[u32]>),
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
}

/// How big each of the engine's tables is, fixed when it's made.
pub struct Capacity {
    pub voices: usize,
    pub buses: usize,
    pub commands: usize,
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
    /// Bus slots, each before every bus it feeds; slots of removed buses are skipped.
    order: Box<[u32]>,
    /// One block of every bus: `channels × QUANTUM` samples per slot, channel after channel.
    mixes: Box<[f32]>,
    /// Commands stamped for a later frame, latest first, so the next is at the end.
    pending: Vec<Stamped>,
    scratch: Box<[f32]>,
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
            unsent: Vec::with_capacity(capacity.voices + capacity.buses + capacity.commands),
            voices: std::iter::repeat_with(|| None).take(capacity.voices).collect(),
            buses,
            order: Box::new([0]),
            mixes: vec![0.0; capacity.buses * channels * QUANTUM].into_boxed_slice(),
            pending: Vec::with_capacity(capacity.commands),
            scratch: vec![0.0; MAX_CHANNELS * QUANTUM].into_boxed_slice(),
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
            let mut block = QUANTUM.min(frames - done);
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
        while self.pending.len() < self.pending.capacity() {
            let Ok(stamped) = self.commands.try_recv() else {
                return;
            };
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
            Command::Start { voice, pcm, spec } => {
                let slot = voice.slot as usize;
                if let Some(old) = self.voices[slot].take() {
                    let old_key = VoiceKey {
                        slot: voice.slot,
                        generation: old.generation,
                    };
                    self.send(Feedback::VoiceEnded {
                        voice: old_key,
                        pcm: old.pcm,
                    });
                }
                self.voices[slot] = Some(Voice::new(voice.generation, pcm, &spec, self.rate));
            }
            Command::Stop { voice } => self.stop(voice),
            Command::SetVoice { voice, param, value } => self.set_voice(voice, param, value),
            Command::AddBus { bus, spec, order } => {
                self.buses[bus.slot as usize] = Some(Bus::new(bus.generation, Some(spec.parent.slot), &spec));
                self.reorder(order);
            }
            Command::RemoveBus { bus } => {
                if self.bus(bus).is_some() {
                    self.buses[bus.slot as usize] = None;
                    self.send(Feedback::BusEnded { bus });
                }
            }
            Command::RouteBus { bus, parent, order } => {
                if let Some(routed) = self.bus(bus) {
                    routed.parent = Some(parent);
                }
                self.reorder(order);
            }
            Command::SetBus { bus, param, value } => {
                if let Some(changed) = self.bus(bus) {
                    changed.set(param, value);
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

    fn stop(&mut self, voice: VoiceKey) {
        let slot = voice.slot as usize;
        if self.voices[slot]
            .as_ref()
            .is_some_and(|playing| playing.generation == voice.generation)
        {
            if let Some(stopped) = self.voices[slot].take() {
                self.send(Feedback::VoiceEnded { voice, pcm: stopped.pcm });
            }
            return;
        }
        // Stopped before its start: the start never happens.
        if let Some(index) = self.pending_start(voice)
            && let Command::Start { pcm, .. } = self.pending.remove(index).command
        {
            self.send(Feedback::VoiceEnded { voice, pcm });
        }
    }

    fn set_voice(&mut self, voice: VoiceKey, param: VoiceParam, value: f64) {
        if let Some(playing) = self.voices[voice.slot as usize].as_mut()
            && playing.generation == voice.generation
        {
            playing.set(param, value);
            return;
        }
        // Set before its start: it starts that way.
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
        for slot in 0..self.voices.len() {
            let Some(voice) = self.voices[slot].as_mut() else {
                continue;
            };
            let rendered = voice.render(&mut self.scratch, frames);
            let bus = voice.bus as usize;
            voice.mix(
                &self.scratch,
                rendered,
                &mut self.mixes[bus * stride..(bus + 1) * stride],
                self.channels,
            );
            if rendered == frames {
                continue;
            }
            if let Some(ended) = self.voices[slot].take() {
                let voice = VoiceKey {
                    slot: u32::try_from(slot).unwrap_or(u32::MAX),
                    generation: ended.generation,
                };
                self.send(Feedback::VoiceEnded { voice, pcm: ended.pcm });
            }
        }
        for index in 0..self.order.len() {
            let slot = self.order[index] as usize;
            let Some(bus) = self.buses[slot].as_ref() else {
                continue;
            };
            let (level, parent) = (bus.level(), bus.parent);
            let Some(parent) = parent else {
                continue;
            };
            let (from, to) = two(&mut self.mixes, stride, slot, parent as usize);
            for channel in 0..self.channels {
                let range = channel * QUANTUM..channel * QUANTUM + frames;
                for (out, sample) in to[range.clone()].iter_mut().zip(&from[range]) {
                    *out += sample * level;
                }
            }
        }
    }

    /// Interleaves `frames` of the master bus into `out`, at its gain, writing any sample that
    /// isn't finite as silence: one bad voice must not poison a device.
    fn write(&mut self, out: &mut [f32], frames: usize) {
        let level = self.buses[0].as_ref().map_or(1.0, Bus::level);
        for frame in 0..frames {
            for channel in 0..self.channels {
                let sample = self.mixes[channel * QUANTUM + frame] * level;
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
