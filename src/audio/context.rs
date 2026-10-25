//! Contexts: an engine and the control side that drives it.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use crate::audio::bus::BusSend;
use crate::audio::engine::{Capacity, Command, Counters, Engine, Feedback, Stamped};
use crate::audio::graph::Graph;
use crate::audio::pcm::check_layout;
use crate::audio::processor::{ParamInfo, Processor, ProcessorKey, check_param, param_index};
use crate::audio::voice::{VoiceKey, check_level};
use crate::audio::{
    AudioError, BusKey, BusParam, BusSpec, MAX_BUSES, MAX_CHAIN, MAX_PROCESSORS, MAX_VOICE_SENDS, MAX_VOICES, Pcm, PlaySpec,
    Tap, VoiceParam,
};

/// How to make a context.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AudioConfig {
    /// Output frames per second.
    pub rate: u32,
    /// Output channels.
    pub channels: usize,
    /// Voices that can play or wait to start at once.
    pub voices: usize,
    /// Buses, the master included.
    pub buses: usize,
    /// Processors on all buses together.
    pub processors: usize,
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
            buses: 64,
            processors: 256,
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

/// A bus's level over a stretch of output: the peak sample's magnitude and the RMS, linear.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Meter {
    /// The largest sample's magnitude.
    pub peak: f32,
    /// The root mean square of every sample.
    pub rms: f32,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    generation: u32,
    busy: bool,
    start: Option<u64>,
    /// The slot of the bus the voice plays into.
    bus: u32,
    /// The slots of the buses it sends to.
    sends: [Option<u32>; MAX_VOICE_SENDS],
}

impl Slot {
    fn feeds(&self, bus: u32) -> bool {
        self.busy && (self.bus == bus || self.sends.contains(&Some(bus)))
    }
}

/// A processor as the control side knows it: enough to check commands for it without asking the
/// engine, which has the processor itself.
#[derive(Clone, Copy, Debug)]
struct ProcessorSlot {
    generation: u32,
    /// Taken: mounted, or sent away and not yet back.
    busy: bool,
    /// In a bus's chain, as far as this side has sent.
    mounted: bool,
    bus: u32,
    latency: u64,
    kind: &'static str,
    params: &'static [ParamInfo],
}

/// The application side of a context: it starts, stops, and changes voices and buses by sending
/// the engine commands, and hears back what ended. It never touches the engine's state.
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
    graph: Graph,
    processors: Vec<ProcessorSlot>,
    free_processors: Vec<u32>,
    sequence: u64,
    counters: Arc<Counters>,
}

/// An engine and its control side, connected; the engine goes to whatever renders it.
///
/// # Errors
///
/// [`AudioError::Rate`] or [`AudioError::Channels`] out of range, and
/// [`AudioError::Capacity`] for no voices or more than [`MAX_VOICES`], no buses or more than
/// [`MAX_BUSES`], more than [`MAX_PROCESSORS`], or no command room.
pub fn context(config: &AudioConfig) -> Result<(Control, Engine), AudioError> {
    check_layout(config.channels, config.rate)?;
    if config.voices == 0 || config.voices > MAX_VOICES {
        return Err(AudioError::Capacity("a context's voice count", config.voices));
    }
    if config.buses == 0 || config.buses > MAX_BUSES {
        return Err(AudioError::Capacity("a context's bus count", config.buses));
    }
    if config.processors > MAX_PROCESSORS {
        return Err(AudioError::Capacity("a context's processor count", config.processors));
    }
    if config.commands == 0 {
        return Err(AudioError::Capacity("a context's command capacity", 0));
    }
    let capacity = Capacity {
        voices: config.voices,
        buses: config.buses,
        processors: config.processors,
        commands: config.commands,
    };
    let (commands, receive) = sync_channel(config.commands);
    let (send, feedback) = sync_channel(capacity.feedback());
    let counters = Arc::new(Counters::new(config.buses));
    let engine = Engine::new(config.rate, config.channels, &capacity, receive, send, Arc::clone(&counters));
    let processors = vec![
        ProcessorSlot {
            generation: 0,
            busy: false,
            mounted: false,
            bus: 0,
            latency: 0,
            kind: "",
            params: &[],
        };
        config.processors
    ];
    let slots = vec![
        Slot {
            generation: 0,
            busy: false,
            start: None,
            bus: 0,
            sends: [None; MAX_VOICE_SENDS],
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
        graph: Graph::new(config.buses),
        processors,
        free_processors: (0..u32::try_from(config.processors).unwrap_or(u32::MAX)).rev().collect(),
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
    /// [`AudioError::Layout`]), [`AudioError::StaleBus`], [`AudioError::VoicesExhausted`],
    /// [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn play(&mut self, pcm: &Arc<Pcm>, spec: &PlaySpec) -> Result<VoiceKey, AudioError> {
        spec.check(pcm, self.channels)?;
        self.collect();
        self.graph.check_live(spec.bus)?;
        for send in spec.sends.iter().flatten() {
            self.graph.check_live(send.bus)?;
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
        entry.bus = spec.bus.slot;
        entry.sends = spec.sends.map(|send| send.map(|send| send.bus.slot));
        Ok(voice)
    }

    /// Changes the level of a voice's send to `bus` at frame `at`, or at the next frame rendered.
    /// True if the voice hadn't ended and sends there; its sends are fixed when it starts.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`] for a level that isn't finite, [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn set_send(&mut self, voice: VoiceKey, bus: BusKey, level: f64, at: Option<u64>) -> Result<bool, AudioError> {
        let level = check_level(level)?;
        self.collect();
        if !self.is_live(voice) || !self.slots[voice.slot as usize].sends.contains(&Some(bus.slot)) {
            return Ok(false);
        }
        #[allow(clippy::cast_possible_truncation)] // Levels are f32 in the engine.
        let level = level as f32;
        self.send(
            at,
            Command::SendVoice {
                voice,
                target: bus.slot,
                level,
            },
        )?;
        Ok(true)
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
        self.send(at, Command::SetVoice { voice, param, value })?;
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

    /// Makes a bus under `spec.parent`. It exists from the next frame rendered: voices can play
    /// into it at once.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`] for a gain that isn't finite, [`AudioError::StaleBus`] for a
    /// parent that isn't live, [`AudioError::BusesExhausted`], [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn add_bus(&mut self, spec: &BusSpec) -> Result<BusKey, AudioError> {
        BusParam::Gain.check(f64::from(spec.gain))?;
        self.collect();
        self.graph.check_live(spec.parent)?;
        let Some(bus) = self.graph.add(spec.parent) else {
            return Err(AudioError::BusesExhausted(self.graph.capacity()));
        };
        let sent = self.graph.order().and_then(|order| {
            // A bus is never stamped for later: a voice may be sent to it straight away.
            self.send(None, Command::AddBus { bus, spec: *spec, order })
        });
        if let Err(error) = sent {
            self.graph.unadd(bus);
            return Err(error);
        }
        Ok(bus)
    }

    /// Removes a bus at frame `at`, or at the next frame rendered. True if it was live; removing
    /// a removed bus does nothing.
    ///
    /// # Errors
    ///
    /// [`AudioError::Master`] for the master bus, [`AudioError::BusInUse`] while a voice or a
    /// bus is routed into it, [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn remove_bus(&mut self, bus: BusKey, at: Option<u64>) -> Result<bool, AudioError> {
        if bus == BusKey::MASTER {
            return Err(AudioError::Master);
        }
        self.collect();
        if !self.graph.is_live(bus) {
            return Ok(false);
        }
        let voices = self.slots.iter().filter(|slot| slot.feeds(bus.slot)).count();
        let buses = self.graph.children(bus);
        if voices > 0 || buses > 0 {
            return Err(AudioError::BusInUse { voices, buses });
        }
        self.send(at, Command::RemoveBus { bus })?;
        self.graph.mark_removing(bus);
        // Its processors go with it, once it has rung out.
        for slot in &mut self.processors {
            if slot.mounted && slot.bus == bus.slot {
                slot.mounted = false;
            }
        }
        Ok(true)
    }

    /// Routes a bus into `parent` from frame `at`, or from the next frame rendered.
    ///
    /// # Errors
    ///
    /// [`AudioError::Master`] for the master bus, [`AudioError::StaleBus`] for either bus not
    /// live, [`AudioError::Cycle`] when `parent` is fed by `bus`, [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn route_bus(&mut self, bus: BusKey, parent: BusKey, at: Option<u64>) -> Result<(), AudioError> {
        if bus == BusKey::MASTER {
            return Err(AudioError::Master);
        }
        self.collect();
        self.graph.check_live(bus)?;
        self.graph.check_live(parent)?;
        let order = self.graph.route(bus, parent)?;
        self.send(
            at,
            Command::RouteBus {
                bus,
                parent: parent.slot,
                order,
            },
        )
    }

    /// Sets a bus's parameter at frame `at`, or at the next frame rendered. True if the bus was
    /// live.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`], [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn set_bus(&mut self, bus: BusKey, param: BusParam, value: f64, at: Option<u64>) -> Result<bool, AudioError> {
        let value = param.check(value)?;
        self.collect();
        if !self.graph.is_live(bus) {
            return Ok(false);
        }
        self.send(at, Command::SetBus { bus, param, value })?;
        Ok(true)
    }

    /// Sends `bus` to `target` at `level` from frame `at`, or from the next frame rendered,
    /// tapped after the bus's gain or before it. A send already there changes level and tap.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`], [`AudioError::StaleBus`] for either bus not live,
    /// [`AudioError::Cycle`] when `target` feeds `bus`, [`AudioError::SendsExhausted`],
    /// [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn send_bus(
        &mut self,
        bus: BusKey,
        target: BusKey,
        level: f64,
        tap: Tap,
        at: Option<u64>,
    ) -> Result<(), AudioError> {
        let level = check_level(level)?;
        self.collect();
        self.graph.check_live(bus)?;
        self.graph.check_live(target)?;
        if bus == target {
            return Err(AudioError::Cycle);
        }
        let order = self.graph.send(bus, target)?;
        let added = order.is_some();
        #[allow(clippy::cast_possible_truncation)] // Levels are f32 in the engine.
        let send = BusSend {
            target: target.slot,
            level: level as f32,
            tap,
        };
        let result = self.send(at, Command::SendBus { bus, send, order });
        if result.is_err() && added {
            self.graph.unsend(bus, target);
        }
        result
    }

    /// Removes the send from `bus` to `target` at frame `at`, or at the next frame rendered.
    /// True if there was one.
    ///
    /// # Errors
    ///
    /// [`AudioError::QueueFull`] or [`AudioError::Disconnected`].
    pub fn unsend_bus(&mut self, bus: BusKey, target: BusKey, at: Option<u64>) -> Result<bool, AudioError> {
        self.collect();
        if !self.graph.is_live(bus) {
            return Ok(false);
        }
        let Some(order) = self.graph.unsend(bus, target) else {
            return Ok(false);
        };
        let target = target.slot;
        self.send(
            at,
            Command::UnsendBus {
                bus,
                target,
                order: Some(order),
            },
        )?;
        Ok(true)
    }

    /// Puts `processor` in `bus`'s chain at `index` (from 0), or last, from the next frame
    /// rendered. The processor moves to the engine; the key names it from here on.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleBus`], [`AudioError::ChainFull`], [`AudioError::ProcessorsExhausted`],
    /// [`AudioError::QueueFull`], or [`AudioError::Disconnected`].
    pub fn mount(
        &mut self,
        processor: Box<dyn Processor>,
        bus: BusKey,
        index: Option<usize>,
    ) -> Result<ProcessorKey, AudioError> {
        self.collect();
        self.graph.check_live(bus)?;
        let chained = self
            .processors
            .iter()
            .filter(|slot| slot.mounted && slot.bus == bus.slot)
            .count();
        if chained == MAX_CHAIN {
            return Err(AudioError::ChainFull(MAX_CHAIN));
        }
        let Some(slot) = self.free_processors.pop() else {
            return Err(AudioError::ProcessorsExhausted(self.processors.len()));
        };
        let key = ProcessorKey {
            slot,
            generation: self.processors[slot as usize].generation,
        };
        let entry = ProcessorSlot {
            generation: key.generation,
            busy: true,
            mounted: true,
            bus: bus.slot,
            latency: processor.latency(),
            kind: processor.kind(),
            params: processor.params(),
        };
        let command = Command::Mount {
            processor: key,
            instance: processor,
            bus,
            index: index.unwrap_or(MAX_CHAIN),
        };
        if let Err(error) = self.send(None, command) {
            self.free_processors.push(slot);
            return Err(error);
        }
        self.processors[slot as usize] = entry;
        Ok(key)
    }

    /// Takes a processor out of its chain at frame `at`, or at the next frame rendered. True if
    /// it was mounted.
    ///
    /// # Errors
    ///
    /// [`AudioError::QueueFull`] or [`AudioError::Disconnected`].
    pub fn unmount(&mut self, processor: ProcessorKey, at: Option<u64>) -> Result<bool, AudioError> {
        self.collect();
        if !self.processor_live(processor) {
            return Ok(false);
        }
        self.send(at, Command::Unmount { processor })?;
        self.processors[processor.slot as usize].mounted = false;
        Ok(true)
    }

    /// Sets a processor's parameter `name` at frame `at`, or at the next frame rendered. True if
    /// the processor is mounted.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleProcessor`], [`AudioError::UnknownParameter`],
    /// [`AudioError::Parameter`] outside its range, [`AudioError::QueueFull`], or
    /// [`AudioError::Disconnected`].
    pub fn set_processor(
        &mut self,
        processor: ProcessorKey,
        name: &str,
        value: f64,
        at: Option<u64>,
    ) -> Result<bool, AudioError> {
        self.collect();
        if !self.processor_live(processor) {
            return Ok(false);
        }
        let slot = self.processors[processor.slot as usize];
        let index = param_index(slot.kind, slot.params, name)?;
        let value = check_param(&slot.params[index], value)?;
        self.send(at, Command::SetProcessor { processor, index, value })?;
        Ok(true)
    }

    /// True while `processor` is in a chain.
    #[must_use]
    pub fn processor_live(&self, processor: ProcessorKey) -> bool {
        self.processors
            .get(processor.slot as usize)
            .is_some_and(|slot| slot.mounted && slot.generation == processor.generation)
    }

    /// How many frames `bus`'s output lags the voices that reach it, by the longest way round:
    /// the processor latency along the path a voice takes to it.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleBus`].
    pub fn latency(&mut self, bus: BusKey) -> Result<u64, AudioError> {
        self.collect();
        self.graph.check_live(bus)?;
        let processors = &self.processors;
        let latencies = self.graph.latencies(|bus| {
            processors
                .iter()
                .filter(|slot| slot.mounted && slot.bus == bus)
                .map(|slot| slot.latency)
                .sum()
        });
        Ok(latencies[bus.slot as usize])
    }

    /// What `bus` has put out, after its gain, since the last time it was read: the peak sample's
    /// magnitude and the RMS level, across its channels. Reading starts the next measurement.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleBus`].
    pub fn meter(&mut self, bus: BusKey) -> Result<Meter, AudioError> {
        self.collect();
        self.graph.check_live(bus)?;
        let meter = &self.counters.meters[bus.slot as usize];
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

    /// True while `bus` is live: made, and not removed.
    pub fn bus_live(&mut self, bus: BusKey) -> bool {
        self.collect();
        self.graph.is_live(bus)
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

    /// Reads what the engine sent back: ended voices and buses free their slots, and the memory
    /// the engine let go of is dropped here rather than on the render thread.
    pub fn collect(&mut self) {
        while let Ok(feedback) = self.feedback.try_recv() {
            match feedback {
                Feedback::VoiceEnded { voice, pcm } => {
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
                Feedback::BusEnded { bus } => {
                    self.graph.ended(bus);
                    // The next bus in the slot starts its own measurement.
                    let meter = &self.counters.meters[bus.slot as usize];
                    meter.peak.store(0, Ordering::Relaxed);
                    meter.energy.store(0, Ordering::Relaxed);
                    meter.samples.store(0, Ordering::Relaxed);
                }
                Feedback::ProcessorEnded { processor, instance } => {
                    drop(instance);
                    let slot = &mut self.processors[processor.slot as usize];
                    if slot.generation != processor.generation || !slot.busy {
                        continue;
                    }
                    slot.generation = slot.generation.wrapping_add(1);
                    slot.busy = false;
                    slot.mounted = false;
                    self.free_processors.push(processor.slot);
                }
                Feedback::Order(order) => drop(order),
            }
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
