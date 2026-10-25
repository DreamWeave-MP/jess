//! The engine: everything that runs on the render thread.
//!
//! [`Engine::render`] is the whole of it. A device callback calls it for each buffer the device
//! asks for, and an [`OfflineContext`](crate::audio::OfflineContext) calls it in a loop: the same
//! code, so an offline render is the test of what a device plays.
//!
//! Rendering never allocates, frees, locks, or blocks. Everything it needs is allocated when the
//! engine is made; commands arrive on a bounded queue it only polls, and anything holding memory
//! that it is done with (the [`Pcm`] of an ended voice) goes back to the control side on a
//! second queue to be dropped there.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};

use crate::audio::voice::{Voice, VoiceKey};
use crate::audio::{MAX_CHANNELS, Pcm, PlaySpec, QUANTUM, VoiceParam};

/// What the control side asks of the engine.
pub enum Command {
    Start { voice: VoiceKey, pcm: Arc<Pcm>, spec: PlaySpec },
    Stop { voice: VoiceKey },
    Set { voice: VoiceKey, param: VoiceParam, value: f64 },
}

impl Command {
    const fn voice(&self) -> VoiceKey {
        match self {
            Command::Start { voice, .. } | Command::Stop { voice } | Command::Set { voice, .. } => *voice,
        }
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
    /// A voice ended: it ran out, was stopped, or was stopped before it started. Its
    /// slot is free once the control side has read this, and the PCM drops there.
    Ended { voice: VoiceKey, pcm: Arc<Pcm> },
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

/// The render side of a context. See the module docs.
pub struct Engine {
    channels: usize,
    rate: u32,
    commands: Receiver<Stamped>,
    feedback: SyncSender<Feedback>,
    /// Feedback the queue had no room for, sent again before the next render. It can't grow past
    /// the voice count: a slot ends once, and is reused only after its feedback is read.
    unsent: Vec<Feedback>,
    voices: Box<[Option<Voice>]>,
    /// Commands stamped for a later frame, latest first, so the next is at the end.
    pending: Vec<Stamped>,
    bus: Box<[f32]>,
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
        voices: usize,
        pending: usize,
        commands: Receiver<Stamped>,
        feedback: SyncSender<Feedback>,
        counters: Arc<Counters>,
    ) -> Engine {
        Engine {
            channels,
            rate,
            commands,
            feedback,
            unsent: Vec::with_capacity(voices),
            voices: std::iter::repeat_with(|| None).take(voices).collect(),
            pending: Vec::with_capacity(pending),
            bus: vec![0.0; channels * QUANTUM].into_boxed_slice(),
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
        let key = command.voice();
        let slot = key.slot as usize;
        match command {
            Command::Start { voice, pcm, spec } => {
                if let Some(old) = self.voices[slot].take() {
                    let old_key = VoiceKey {
                        slot: voice.slot,
                        generation: old.generation,
                    };
                    self.retire(old_key, old.pcm);
                }
                self.voices[slot] = Some(Voice::new(voice.generation, pcm, &spec, self.rate));
            }
            Command::Stop { voice } => {
                if self.voices[slot]
                    .as_ref()
                    .is_some_and(|playing| playing.generation == voice.generation)
                {
                    if let Some(stopped) = self.voices[slot].take() {
                        self.retire(voice, stopped.pcm);
                    }
                    return;
                }
                // Stopped before its start: the start never happens.
                if let Some(index) = self.pending_start(voice)
                    && let Command::Start { pcm, .. } = self.pending.remove(index).command
                {
                    self.retire(voice, pcm);
                }
            }
            Command::Set { voice, param, value } => {
                if let Some(playing) = self.voices[slot].as_mut()
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
        }
    }

    fn pending_start(&self, voice: VoiceKey) -> Option<usize> {
        self.pending
            .iter()
            .position(|waiting| matches!(waiting.command, Command::Start { voice: start, .. } if start == voice))
    }

    fn retire(&mut self, voice: VoiceKey, pcm: Arc<Pcm>) {
        let ended = Feedback::Ended { voice, pcm };
        if let Err(TrySendError::Full(ended) | TrySendError::Disconnected(ended)) = self.feedback.try_send(ended) {
            self.unsent.push(ended);
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

    /// Mixes `frames` frames of every voice into the bus.
    fn mix(&mut self, frames: usize) {
        for channel in 0..self.channels {
            self.bus[channel * QUANTUM..channel * QUANTUM + frames].fill(0.0);
        }
        for slot in 0..self.voices.len() {
            let Some(voice) = self.voices[slot].as_mut() else {
                continue;
            };
            let rendered = voice.render(&mut self.scratch, frames);
            voice.mix(&self.scratch, rendered, &mut self.bus, self.channels);
            if rendered == frames {
                continue;
            }
            if let Some(ended) = self.voices[slot].take() {
                let key = VoiceKey {
                    slot: u32::try_from(slot).unwrap_or(u32::MAX),
                    generation: ended.generation,
                };
                self.retire(key, ended.pcm);
            }
        }
    }

    /// Interleaves `frames` of the bus into `out`, writing any sample that isn't finite as
    /// silence: one bad voice must not poison a device.
    fn write(&mut self, out: &mut [f32], frames: usize) {
        for frame in 0..frames {
            for channel in 0..self.channels {
                let sample = self.bus[channel * QUANTUM + frame];
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
