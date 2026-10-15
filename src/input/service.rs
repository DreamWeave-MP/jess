//! One application's input as one shareable unit: the runtime and a synthetic side.
//!
//! An [`InputRuntime`] is the evaluation engine; an [`InputService`] is what a host owns and a
//! script configures. The host ticks it at its input phase and delivers the tick's action
//! events; a script installs the graph and activates sets, and turns those events into intents
//! itself (`@jess/input`'s router, in Luau); a test presses keys on the synthetic side. Behind a
//! `Mutex`, one service is the application's input, wherever it is reached from, and the crate
//! that ticks it and the crate that binds it to a script need to agree on nothing but this type.

use std::time::Duration;

use crate::input::graph::compile::GraphError;
use crate::input::graph::eval::ActionEvent;
use crate::input::graph::spec::{GraphSpec, SpecError};
use crate::input::runtime::RuntimeError;
use crate::input::{DeviceId, EventSender, Generation, InputRuntime, SyntheticSource, Timestamp};

/// What one tick produced: the action events of the tick just run.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tick<'a> {
    /// The tick's time.
    pub now: Timestamp,
    /// The graph generation the events came from.
    pub generation: Generation,
    /// The action edges, in evaluation order.
    pub events: &'a [ActionEvent],
}

/// One application's input: an [`InputRuntime`] and a synthetic source into its queue. See the
/// module docs.
#[derive(Debug)]
pub struct InputService {
    runtime: InputRuntime,
    synthetic: SyntheticSource,
    last_tick: Timestamp,
}

impl Default for InputService {
    fn default() -> Self {
        Self::new()
    }
}

impl InputService {
    /// A service over a new runtime: no devices, no graph, its clock at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::over(InputRuntime::new())
    }

    /// A service over `runtime`, keeping whatever it has installed.
    #[must_use]
    pub fn over(runtime: InputRuntime) -> Self {
        let synthetic = runtime.synthetic();
        Self {
            runtime,
            synthetic,
            last_tick: Timestamp::ZERO,
        }
    }

    /// The runtime: device state, the installed graph, action state, stats.
    #[must_use]
    pub const fn runtime(&self) -> &InputRuntime {
        &self.runtime
    }

    /// A sender into the runtime's queue, for a backend.
    #[must_use]
    pub fn sender(&self) -> EventSender {
        self.runtime.sender()
    }

    /// The synthetic side: devices and events that exist because a test or a script says so,
    /// stamped from the synthetic clock, which [`InputService::advance`] moves and every tick
    /// catches up to the tick's time.
    pub const fn synthetic(&mut self) -> &mut SyntheticSource {
        &mut self.synthetic
    }

    /// Moves the synthetic clock forward, for a host whose time is the synthetic side's.
    pub fn advance(&mut self, duration: Duration) {
        self.synthetic.advance(duration);
    }

    /// The synthetic clock's time: the later of the last tick and the last `advance`.
    #[must_use]
    pub const fn now(&self) -> Timestamp {
        self.synthetic.now()
    }

    /// The time of the last tick.
    #[must_use]
    pub const fn last_tick(&self) -> Timestamp {
        self.last_tick
    }

    /// The device with id number `index`, if one was ever announced to the runtime's queue.
    #[must_use]
    pub fn device(&self, index: u32) -> Option<DeviceId> {
        self.runtime.queue().device(index)
    }

    // The graph, through the runtime.

    /// [`InputRuntime::install`].
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn install(&mut self, spec: GraphSpec) -> Result<Generation, Vec<GraphError>> {
        self.runtime.install(spec)
    }

    /// [`InputRuntime::uninstall`].
    pub fn uninstall(&mut self) -> Generation {
        self.runtime.uninstall()
    }

    /// [`InputRuntime::remap`].
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn remap(&mut self, edit: impl FnOnce(&mut GraphSpec) -> Result<(), SpecError>) -> Result<Generation, RuntimeError> {
        self.runtime.remap(edit)
    }

    /// [`InputRuntime::apply_bindings`].
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn apply_bindings(&mut self, saved: GraphSpec) -> Result<Generation, RuntimeError> {
        self.runtime.apply_bindings(saved)
    }

    /// [`InputRuntime::restore_defaults`].
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn restore_defaults(&mut self) -> Result<Generation, RuntimeError> {
        self.runtime.restore_defaults()
    }

    /// [`InputRuntime::set_active`].
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn set_active(&mut self, set: &str, active: bool) -> Result<(), RuntimeError> {
        self.runtime.set_active(set, active)
    }

    /// [`InputRuntime::push_layer`].
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn push_layer(&mut self, layer: &str) -> Result<(), RuntimeError> {
        self.runtime.push_layer(layer)
    }

    /// [`InputRuntime::pop_layer`].
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn pop_layer(&mut self, layer: &str) -> Result<bool, RuntimeError> {
        self.runtime.pop_layer(layer)
    }

    /// The name of the action `action` in the installed graph.
    #[must_use]
    pub fn action_name(&self, action: crate::input::ActionId) -> Option<&str> {
        self.runtime.graph()?.action(action).map(crate::input::Action::name)
    }

    // Ticking.

    /// Runs one tick at `now`: the synthetic clock catches up to it (so events injected after
    /// the tick are stamped no earlier than it), and the runtime drains, applies, and evaluates.
    pub fn tick(&mut self, now: Timestamp) -> Tick<'_> {
        if now > self.synthetic.now() {
            self.synthetic.advance(now - self.synthetic.now());
        }
        self.last_tick = now;
        let generation = self.runtime.generation();
        let events = self.runtime.tick(now);
        Tick { now, generation, events }
    }

    /// Forgets what a script configured: the graph. Device state and the queue stay, since
    /// devices did not restart. For a host at a script restart boundary.
    pub fn reset(&mut self) {
        self.uninstall();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{ActionEventKind, Control, Key, NodeSpec, Owner, ValueType};

    fn accept() -> GraphSpec {
        let mut spec = GraphSpec::new();
        spec.action("ui.accept", ValueType::Digital).set("ui", "@jess/test");
        spec.add_binding(
            &Owner::Set("ui".to_owned()),
            "ui.accept",
            NodeSpec::button(Control::Key(Key::Enter)),
        )
        .unwrap();
        spec
    }

    #[test]
    fn a_tick_returns_its_action_events_and_a_reset_keeps_the_devices() {
        let mut service = InputService::new();
        let generation = service.install(accept()).unwrap();
        service.set_active("ui", true).unwrap();
        let keyboard = service.synthetic().connect_keyboard();
        service.synthetic().press(keyboard, Control::Key(Key::Enter));
        let tick = service.tick(Timestamp::from_millis(16));
        assert_eq!((tick.generation, tick.events.len()), (generation, 1));
        let event = tick.events[0];
        assert_eq!(event.kind, ActionEventKind::Pressed);
        assert_eq!(service.action_name(event.action), Some("ui.accept"));
        assert_eq!(service.now(), Timestamp::from_millis(16), "the synthetic clock caught up");
        assert_eq!(service.device(keyboard.index()), Some(keyboard));

        // After a reinstall, a key pressed again is an edge again.
        service.synthetic().release(keyboard, Control::Key(Key::Enter));
        service.install(accept()).unwrap();
        service.synthetic().press(keyboard, Control::Key(Key::Enter));
        let tick = service.tick(Timestamp::from_millis(32));
        assert_eq!(tick.events.len(), 1);

        service.reset();
        assert!(service.runtime().graph().is_none());
        assert!(
            service.runtime().state().device(keyboard).is_some(),
            "device truth outlives the script's configuration"
        );
    }
}
