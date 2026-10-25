//! Contexts: an engine and the control side that drives it.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use crate::audio::engine::{Command, Counters, Engine, Feedback, Stamped};
use crate::audio::pcm::check_layout;
use crate::audio::voice::VoiceKey;
use crate::audio::{AudioError, MAX_VOICES, Pcm, PlaySpec, VoiceParam};

/// How to make a context.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AudioConfig {
    /// Output frames per second.
    pub rate: u32,
    /// Output channels.
    pub channels: usize,
    /// Voices that can play or wait to start at once.
    pub voices: usize,
    /// Commands that can wait for the engine at once, in the queue and again in the engine's
    /// list of commands stamped for later.
    pub commands: usize,
}

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig {
            rate: 48_000,
            channels: 2,
            voices: 256,
            commands: 4096,
        }
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
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    generation: u32,
    busy: bool,
    start: Option<u64>,
}

/// The application side of a context: it starts, stops, and changes voices by sending the
/// engine commands, and hears back which voices ended. It never touches the engine's state.
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
    sequence: u64,
    counters: Arc<Counters>,
}

/// An engine and its control side, connected; the engine goes to whatever renders it.
///
/// # Errors
///
/// [`AudioError::Rate`] or [`AudioError::Channels`] out of range, and
/// [`AudioError::Capacity`] for no voices, more than [`MAX_VOICES`], or no command room.
pub fn context(config: &AudioConfig) -> Result<(Control, Engine), AudioError> {
    check_layout(config.channels, config.rate)?;
    if config.voices == 0 || config.voices > MAX_VOICES {
        return Err(AudioError::Capacity("a context's voice count", config.voices));
    }
    if config.commands == 0 {
        return Err(AudioError::Capacity("a context's command capacity", 0));
    }
    let (commands, receive) = sync_channel(config.commands);
    // Every voice ends once and its slot is reused only after this is read, so a queue the size of
    // the pool never fills.
    let (send, feedback) = sync_channel(config.voices);
    let counters = Arc::new(Counters::default());
    let engine = Engine::new(
        config.rate,
        config.channels,
        config.voices,
        config.commands,
        receive,
        send,
        Arc::clone(&counters),
    );
    let slots = (0..config.voices)
        .map(|_| Slot {
            generation: 0,
            busy: false,
            start: None,
        })
        .collect();
    let free = (0..u32::try_from(config.voices).unwrap_or(u32::MAX)).rev().collect();
    let control = Control {
        rate: config.rate,
        channels: config.channels,
        capacity: config.commands,
        commands,
        feedback,
        slots,
        free,
        sequence: 0,
        counters,
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
    /// [`AudioError::Layout`]), [`AudioError::VoicesExhausted`], [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn play(&mut self, pcm: &Arc<Pcm>, spec: &PlaySpec) -> Result<VoiceKey, AudioError> {
        spec.check(pcm, self.channels)?;
        self.collect();
        let Some(slot) = self.free.pop() else {
            return Err(AudioError::VoicesExhausted(self.slots.len()));
        };
        let voice = VoiceKey {
            slot,
            generation: self.slots[slot as usize].generation,
        };
        let command = Command::Start {
            voice,
            pcm: Arc::clone(pcm),
            spec: *spec,
        };
        if let Err(error) = self.send(spec.at, command) {
            self.free.push(slot);
            return Err(error);
        }
        let entry = &mut self.slots[slot as usize];
        entry.busy = true;
        entry.start = spec.at;
        Ok(voice)
    }

    /// Stops a voice at frame `at`, or at the next frame rendered. True if the voice hadn't
    /// ended as far as this side knows; stopping an ended voice does nothing.
    ///
    /// # Errors
    ///
    /// [`AudioError::QueueFull`] or [`AudioError::Disconnected`].
    pub fn stop(&mut self, voice: VoiceKey, at: Option<u64>) -> Result<bool, AudioError> {
        self.collect();
        if !self.is_live(voice) {
            return Ok(false);
        }
        self.send(at, Command::Stop { voice })?;
        Ok(true)
    }

    /// Sets a voice's parameter at frame `at`, or at the next frame rendered. A voice not yet
    /// started starts with it. True if the voice hadn't ended as far as this side knows.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`] for a value the parameter can't take, [`AudioError::QueueFull`],
    /// or [`AudioError::Disconnected`].
    pub fn set(&mut self, voice: VoiceKey, param: VoiceParam, value: f64, at: Option<u64>) -> Result<bool, AudioError> {
        let value = param.check(value)?;
        self.collect();
        if !self.is_live(voice) {
            return Ok(false);
        }
        self.send(at, Command::Set { voice, param, value })?;
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
        }
    }

    /// Reads what the engine sent back: ended voices free their slots, and their PCM is dropped
    /// here rather than on the render thread.
    pub fn collect(&mut self) {
        while let Ok(Feedback::Ended { voice, pcm, .. }) = self.feedback.try_recv() {
            drop(pcm);
            let slot = &mut self.slots[voice.slot as usize];
            if slot.generation != voice.generation || !slot.busy {
                continue;
            }
            slot.generation = slot.generation.wrapping_add(1);
            slot.busy = false;
            slot.start = None;
            self.free.push(voice.slot);
        }
    }

    fn is_live(&self, voice: VoiceKey) -> bool {
        self.slots
            .get(voice.slot as usize)
            .is_some_and(|slot| slot.busy && slot.generation == voice.generation)
    }

    fn send(&mut self, at: Option<u64>, command: Command) -> Result<(), AudioError> {
        self.sequence += 1;
        let stamped = Stamped {
            at,
            sequence: self.sequence,
            command,
        };
        match self.commands.try_send(stamped) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(AudioError::QueueFull(self.capacity)),
            Err(TrySendError::Disconnected(_)) => Err(AudioError::Disconnected),
        }
    }
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
