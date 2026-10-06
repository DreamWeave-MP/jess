//! The input runtime: the queue, the device state, and the installed graph, ticked as one.

use std::fmt;

use crate::input::graph::compile::{ActionGraph, GraphError};
use crate::input::graph::eval::{ActionEvent, ActionState, EvaluationStats, Evaluator};
use crate::input::graph::spec::{GraphSpec, SpecError};
use crate::input::{EventSender, Generation, InputEvent, InputQueue, InputState, SyntheticSource, Timestamp};

/// Why a runtime call didn't happen.
#[derive(Clone, PartialEq, Debug)]
pub enum RuntimeError {
    /// No graph is installed.
    NoGraph,
    /// The installed graph has no set of this name.
    UnknownSet(String),
    /// The installed graph has no layer of this name.
    UnknownLayer(String),
    /// An edit to the binding document didn't apply.
    Spec(SpecError),
    /// The edited document doesn't compile. The installed graph is unchanged.
    Graph(Vec<GraphError>),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoGraph => formatter.write_str("no action graph is installed"),
            Self::UnknownSet(name) => write!(formatter, "the installed action graph has no set {name:?}"),
            Self::UnknownLayer(name) => write!(formatter, "the installed action graph has no layer {name:?}"),
            Self::Spec(error) => write!(formatter, "{error}"),
            Self::Graph(errors) => {
                formatter.write_str("the bindings don't compile")?;
                for error in errors {
                    write!(formatter, "\n  {error}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<SpecError> for RuntimeError {
    fn from(error: SpecError) -> Self {
        Self::Spec(error)
    }
}

/// What one tick did, for a profiler.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RuntimeStats {
    /// Device events the last tick drained and applied.
    pub device_events: u32,
    /// Device events applied since the runtime was made.
    pub total_device_events: u64,
    /// The last tick's graph evaluation, or zeroes with no graph installed.
    pub evaluation: EvaluationStats,
}

#[derive(Debug)]
struct Installed {
    /// The document as its package installed it, before any remap.
    defaults: GraphSpec,
    graph: ActionGraph,
    evaluator: Evaluator,
}

/// One application's input: a queue backends push to, the device state those events add up to,
/// and the action graph evaluated against it.
///
/// # Scheduling
///
/// Nothing happens between ticks. Backends and synthetic sources push to the queue from any
/// thread; [`InputRuntime::tick`], called by the host at its input phase, drains the queue,
/// applies the events to state in order, evaluates the graph once, and returns the action events.
/// No callback is ever invoked from here, and no script is ever entered: a script context reads
/// the results after the tick, on its own schedule.
///
/// # Generations
///
/// Every install, remap, restore, and uninstall replaces the graph whole and bumps
/// [`InputRuntime::generation`]. Action ids, set ids, and binding ids belong to one generation's
/// graph; hold names across a replacement, and drop events stamped with an older generation.
/// A document that doesn't compile replaces nothing: the last good graph keeps running.
///
/// # Capabilities
///
/// The runtime enforces none; the script boundary does (`jess.input`, under a script domain).
/// The calls are grouped so it can: reading [`InputRuntime::state`] and
/// [`InputRuntime::device_events`] is [`crate::input::capability::OBSERVE`], reading action state and
/// events is [`crate::input::capability::ACTIONS`], and installing, remapping, and activating is
/// [`crate::input::capability::CONFIGURE`]. A host that shares one runtime between its own ticking and
/// a script's configuration wraps it in an [`crate::input::InputService`].
#[derive(Debug)]
pub struct InputRuntime {
    queue: InputQueue,
    state: InputState,
    device_events: Vec<InputEvent>,
    installed: Option<Installed>,
    generation: Generation,
    total_device_events: u64,
    /// The `now` of the last tick: when a replacement graph's bindings go live.
    last_tick: Timestamp,
}

impl Default for InputRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl InputRuntime {
    /// A runtime with its own queue, no devices, and no graph.
    #[must_use]
    pub fn new() -> Self {
        Self::with_queue(InputQueue::new())
    }

    /// A runtime draining `queue`.
    #[must_use]
    pub const fn with_queue(queue: InputQueue) -> Self {
        Self {
            queue,
            state: InputState::new(),
            device_events: Vec::new(),
            installed: None,
            generation: Generation::INITIAL,
            total_device_events: 0,
            last_tick: Timestamp::ZERO,
        }
    }

    /// The queue this runtime drains.
    #[must_use]
    pub const fn queue(&self) -> &InputQueue {
        &self.queue
    }

    /// A sender into this runtime's queue, for a backend.
    #[must_use]
    pub fn sender(&self) -> EventSender {
        self.queue.sender()
    }

    /// A synthetic source pushing into this runtime's queue.
    #[must_use]
    pub fn synthetic(&self) -> SyntheticSource {
        self.queue.synthetic()
    }

    /// The current generation.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Compiles `spec` and swaps it in, carrying over which sets are active and which layers are
    /// pushed, by name.
    fn replace(&mut self, defaults: GraphSpec, spec: GraphSpec) -> Result<Generation, Vec<GraphError>> {
        let graph = ActionGraph::compile(spec)?;
        let generation = self.generation.next();
        let mut evaluator = Evaluator::new(&graph, generation);
        evaluator.begin_at(self.last_tick);
        if let Some(old) = &self.installed {
            for set in old.evaluator.active_sets() {
                if let Some(set) = old.graph.set_name(set).and_then(|name| graph.set_id(name)) {
                    evaluator.set_active(set, true);
                }
            }
            for layer in old.evaluator.layers() {
                if let Some(layer) = old.graph.layer_name(*layer).and_then(|name| graph.layer_id(name)) {
                    evaluator.push_layer(layer);
                }
            }
        }
        self.installed = Some(Installed {
            defaults,
            graph,
            evaluator,
        });
        self.generation = generation;
        Ok(generation)
    }

    /// Installs a package's binding document as the new defaults, replacing whatever was
    /// installed and any remaps made to it.
    ///
    /// Sets that were active and layers that were pushed stay so where the new document still
    /// has them by name; every action starts from rest, and a button held across the swap is
    /// ignored until released.
    ///
    /// # Errors
    ///
    /// Everything wrong with the document. The installed graph, if any, keeps running.
    pub fn install(&mut self, spec: GraphSpec) -> Result<Generation, Vec<GraphError>> {
        self.replace(spec.clone(), spec)
    }

    /// Removes the graph. Every action is gone; device state stays.
    pub fn uninstall(&mut self) -> Generation {
        self.installed = None;
        self.generation = self.generation.next();
        self.generation
    }

    /// Edits a copy of the installed document and installs the result, keeping the defaults: a
    /// remap.
    ///
    /// # Errors
    ///
    /// No graph is installed, the edit failed, or the edited document doesn't compile. In every
    /// case the installed graph is unchanged.
    pub fn remap(&mut self, edit: impl FnOnce(&mut GraphSpec) -> Result<(), SpecError>) -> Result<Generation, RuntimeError> {
        let installed = self.installed.as_ref().ok_or(RuntimeError::NoGraph)?;
        let mut spec = installed.graph.spec().clone();
        edit(&mut spec)?;
        let defaults = installed.defaults.clone();
        self.replace(defaults, spec).map_err(RuntimeError::Graph)
    }

    /// Installs a user's saved bindings over the defaults, if they declare the same actions.
    ///
    /// # Errors
    ///
    /// No graph is installed, the saved document declares different actions than the defaults
    /// ([`SpecError::IncompatibleActions`]), or it doesn't compile.
    pub fn apply_bindings(&mut self, saved: GraphSpec) -> Result<Generation, RuntimeError> {
        let installed = self.installed.as_ref().ok_or(RuntimeError::NoGraph)?;
        saved.check_compatible(&installed.defaults)?;
        let defaults = installed.defaults.clone();
        self.replace(defaults, saved).map_err(RuntimeError::Graph)
    }

    /// Puts the defaults back, dropping every remap.
    ///
    /// # Errors
    ///
    /// No graph is installed.
    pub fn restore_defaults(&mut self) -> Result<Generation, RuntimeError> {
        let defaults = self.installed.as_ref().ok_or(RuntimeError::NoGraph)?.defaults.clone();
        self.replace(defaults.clone(), defaults).map_err(RuntimeError::Graph)
    }

    /// The installed graph.
    #[must_use]
    pub fn graph(&self) -> Option<&ActionGraph> {
        self.installed.as_ref().map(|installed| &installed.graph)
    }

    /// The document as its package installed it, before any remap.
    #[must_use]
    pub fn defaults(&self) -> Option<&GraphSpec> {
        self.installed.as_ref().map(|installed| &installed.defaults)
    }

    /// The installed graph's evaluator: action state, active sets, pushed layers, and stats.
    #[must_use]
    pub fn evaluator(&self) -> Option<&Evaluator> {
        self.installed.as_ref().map(|installed| &installed.evaluator)
    }

    /// Activates or deactivates a set, by name, from the next tick.
    ///
    /// # Errors
    ///
    /// No graph is installed, or it has no such set.
    pub fn set_active(&mut self, set: &str, active: bool) -> Result<(), RuntimeError> {
        let installed = self.installed.as_mut().ok_or(RuntimeError::NoGraph)?;
        let id = installed
            .graph
            .set_id(set)
            .ok_or_else(|| RuntimeError::UnknownSet(set.to_owned()))?;
        installed.evaluator.set_active(id, active);
        Ok(())
    }

    /// Pushes a layer, by name, on top of the stack, from the next tick.
    ///
    /// # Errors
    ///
    /// No graph is installed, or it has no such layer.
    pub fn push_layer(&mut self, layer: &str) -> Result<(), RuntimeError> {
        let installed = self.installed.as_mut().ok_or(RuntimeError::NoGraph)?;
        let id = installed
            .graph
            .layer_id(layer)
            .ok_or_else(|| RuntimeError::UnknownLayer(layer.to_owned()))?;
        installed.evaluator.push_layer(id);
        Ok(())
    }

    /// Removes a layer, by name, from the stack. Returns whether it was pushed.
    ///
    /// # Errors
    ///
    /// No graph is installed, or it has no such layer.
    pub fn pop_layer(&mut self, layer: &str) -> Result<bool, RuntimeError> {
        let installed = self.installed.as_mut().ok_or(RuntimeError::NoGraph)?;
        let id = installed
            .graph
            .layer_id(layer)
            .ok_or_else(|| RuntimeError::UnknownLayer(layer.to_owned()))?;
        Ok(installed.evaluator.pop_layer(id))
    }

    /// Runs one tick at time `now`: drains the queue, applies the events to device state in
    /// order, evaluates the installed graph once, and returns its action events.
    ///
    /// `now` is the host's reading of the clock the events were stamped from; hold thresholds
    /// are crossed against it. With no graph installed the tick still updates device state.
    pub fn tick(&mut self, now: Timestamp) -> &[ActionEvent] {
        self.last_tick = now;
        self.state.end_tick();
        self.device_events.clear();
        self.queue.drain(&mut self.device_events);
        for event in &self.device_events {
            self.state.apply(event);
        }
        self.total_device_events += self.device_events.len() as u64;
        match &mut self.installed {
            Some(installed) => installed.evaluator.evaluate(&installed.graph, &self.state, now),
            None => &[],
        }
    }

    /// Current device state, as of the last tick.
    #[must_use]
    pub const fn state(&self) -> &InputState {
        &self.state
    }

    /// The device events the last tick applied, in order.
    #[must_use]
    pub fn device_events(&self) -> &[InputEvent] {
        &self.device_events
    }

    /// The last tick's action events.
    #[must_use]
    pub fn action_events(&self) -> &[ActionEvent] {
        self.installed.as_ref().map_or(&[], |installed| installed.evaluator.events())
    }

    /// An action's state after the last tick, by name.
    #[must_use]
    pub fn action(&self, name: &str) -> Option<&ActionState> {
        let installed = self.installed.as_ref()?;
        installed.evaluator.action(installed.graph.action_id(name)?)
    }

    /// What the last tick did.
    #[must_use]
    pub fn stats(&self) -> RuntimeStats {
        RuntimeStats {
            device_events: u32::try_from(self.device_events.len()).unwrap_or(u32::MAX),
            total_device_events: self.total_device_events,
            evaluation: self
                .installed
                .as_ref()
                .map(|installed| *installed.evaluator.stats())
                .unwrap_or_default(),
        }
    }
}
