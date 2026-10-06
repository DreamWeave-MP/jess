//! One application's input as one shareable unit: the runtime, the intents routed from its
//! action events, the routes by name that survive a graph replacement, and a synthetic side.
//!
//! An [`InputRuntime`] is the evaluation engine; an [`InputService`] is what a host owns and a
//! script configures. The host ticks it at its input phase and delivers what the tick produced;
//! a script installs the graph, activates sets, and names intents; a test presses keys on the
//! synthetic side. Behind a `Mutex`, one service is the application's input, wherever it is
//! reached from, and the crate that ticks it and the crate that binds it to a script need to
//! agree on nothing but this type.

use std::time::Duration;

use crate::input::graph::compile::GraphError;
use crate::input::graph::eval::{ActionEvent, ActionEventKind};
use crate::input::graph::spec::{GraphSpec, SpecError};
use crate::input::runtime::RuntimeError;
use crate::input::{
    DeviceId, EventSender, Generation, InputRuntime, IntentDispatch, IntentRegistry, IntentRoute, IntentRouter, IntentScope,
    SyntheticSource, Timestamp,
};

/// A route by name: an edge of the action `action` dispatches the intent `intent`. Names
/// outlive graph generations; the router's ids are rebuilt from these on every replacement.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NamedRoute {
    /// The action's name in the installed graph.
    pub action: String,
    /// Which edge of the action.
    pub kind: ActionEventKind,
    /// The intent's name.
    pub intent: String,
}

/// What one tick produced: the action events and the intents routed from them, both for the
/// tick just run.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tick<'a> {
    /// The tick's time.
    pub now: Timestamp,
    /// The graph generation the events came from.
    pub generation: Generation,
    /// The action edges, in evaluation order.
    pub events: &'a [ActionEvent],
    /// The intents routed from them, in route order.
    pub intents: &'a [IntentDispatch],
}

/// One application's input: an [`InputRuntime`], its intent routes by name, and a synthetic
/// source into its queue. See the module docs.
#[derive(Debug)]
pub struct InputService {
    runtime: InputRuntime,
    synthetic: SyntheticSource,
    intents: IntentRegistry,
    router: IntentRouter,
    routes: Vec<NamedRoute>,
    last_tick: Timestamp,
}

impl Default for InputService {
    fn default() -> Self {
        Self::new()
    }
}

impl InputService {
    /// A service over a new runtime: no devices, no graph, no routes, its clock at zero.
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
            intents: IntentRegistry::new(),
            router: IntentRouter::new(),
            routes: Vec::new(),
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

    // The graph, through the runtime, rebuilding the routes on every replacement.

    fn rebuild_routes(&mut self) {
        self.router.clear();
        let Some(graph) = self.runtime.graph() else {
            return;
        };
        for route in &self.routes {
            if let Some(action) = graph.action_id(&route.action) {
                self.router.add(IntentRoute {
                    action,
                    kind: route.kind,
                    intent: self.intents.intent(&route.intent),
                });
            }
        }
    }

    /// [`InputRuntime::install`], with the routes rebuilt for the new graph.
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn install(&mut self, spec: GraphSpec) -> Result<Generation, Vec<GraphError>> {
        let generation = self.runtime.install(spec)?;
        self.rebuild_routes();
        Ok(generation)
    }

    /// [`InputRuntime::uninstall`]; the routes stay by name and dispatch nothing until a graph
    /// is installed again.
    pub fn uninstall(&mut self) -> Generation {
        let generation = self.runtime.uninstall();
        self.router.clear();
        generation
    }

    /// [`InputRuntime::remap`], with the routes rebuilt.
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn remap(&mut self, edit: impl FnOnce(&mut GraphSpec) -> Result<(), SpecError>) -> Result<Generation, RuntimeError> {
        let generation = self.runtime.remap(edit)?;
        self.rebuild_routes();
        Ok(generation)
    }

    /// [`InputRuntime::apply_bindings`], with the routes rebuilt.
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn apply_bindings(&mut self, saved: GraphSpec) -> Result<Generation, RuntimeError> {
        let generation = self.runtime.apply_bindings(saved)?;
        self.rebuild_routes();
        Ok(generation)
    }

    /// [`InputRuntime::restore_defaults`], with the routes rebuilt.
    ///
    /// # Errors
    ///
    /// As the runtime's.
    pub fn restore_defaults(&mut self) -> Result<Generation, RuntimeError> {
        let generation = self.runtime.restore_defaults()?;
        self.rebuild_routes();
        Ok(generation)
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

    // Intents.

    /// Routes an edge of the action named `action` to the intent named `intent`, from the next
    /// tick, in this graph and every one installed after it. A route to an action no graph
    /// declares waits for one that does.
    pub fn route(&mut self, action: &str, kind: ActionEventKind, intent: &str) {
        self.routes.push(NamedRoute {
            action: action.to_owned(),
            kind,
            intent: intent.to_owned(),
        });
        self.rebuild_routes();
    }

    /// The routes by name, in the order they were added.
    #[must_use]
    pub fn routes(&self) -> &[NamedRoute] {
        &self.routes
    }

    /// Removes every route.
    pub fn clear_routes(&mut self) {
        self.routes.clear();
        self.router.clear();
    }

    /// The intents named so far.
    #[must_use]
    pub const fn intents(&self) -> &IntentRegistry {
        &self.intents
    }

    /// The last tick's dispatches.
    #[must_use]
    pub fn dispatches(&self) -> &[IntentDispatch] {
        self.router.dispatches()
    }

    /// The name of the action `action` in the installed graph.
    #[must_use]
    pub fn action_name(&self, action: crate::input::ActionId) -> Option<&str> {
        self.runtime.graph()?.action(action).map(crate::input::Action::name)
    }

    // Ticking.

    /// Runs one tick at `now`: the synthetic clock catches up to it (so events injected after
    /// the tick are stamped no earlier than it), the runtime drains, applies, and evaluates, and
    /// the router turns the action events into intents under `scope`.
    pub fn tick(&mut self, now: Timestamp, scope: IntentScope) -> Tick<'_> {
        if now > self.synthetic.now() {
            self.synthetic.advance(now - self.synthetic.now());
        }
        self.last_tick = now;
        let generation = self.runtime.generation();
        let events = self.runtime.tick(now);
        let intents = self.router.route(events, generation, scope);
        Tick {
            now,
            generation,
            events,
            intents,
        }
    }

    /// Forgets what a script configured: the graph and the routes. Device state and the queue
    /// stay, since devices did not restart. For a host at a script restart boundary.
    pub fn reset(&mut self) {
        self.uninstall();
        self.clear_routes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Control, Key, NodeSpec, Owner, ValueType};

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
    fn routes_by_name_survive_a_graph_replacement_and_a_tick_returns_both_layers() {
        let mut service = InputService::new();
        service.route("ui.accept", ActionEventKind::Pressed, "ui.activate");
        assert_eq!(service.routes().len(), 1);
        assert_eq!(service.dispatches(), []);

        let generation = service.install(accept()).unwrap();
        service.set_active("ui", true).unwrap();
        let keyboard = service.synthetic().connect_keyboard();
        service.synthetic().press(keyboard, Control::Key(Key::Enter));
        let tick = service.tick(Timestamp::from_millis(16), IntentScope::default());
        assert_eq!((tick.generation, tick.events.len(), tick.intents.len()), (generation, 1, 1));
        let (event, intent) = (tick.events[0], tick.intents[0]);
        assert_eq!(service.intents().name(intent.intent), Some("ui.activate"));
        assert_eq!(service.action_name(event.action), Some("ui.accept"));
        assert_eq!(service.now(), Timestamp::from_millis(16), "the synthetic clock caught up");
        assert_eq!(service.device(keyboard.index()), Some(keyboard));

        // A reinstall keeps the route, by name, and a held key stays invisible until released.
        service.synthetic().release(keyboard, Control::Key(Key::Enter));
        service.install(accept()).unwrap();
        service.synthetic().press(keyboard, Control::Key(Key::Enter));
        let tick = service.tick(Timestamp::from_millis(32), IntentScope::default());
        assert_eq!(tick.intents.len(), 1);

        service.reset();
        assert!(service.runtime().graph().is_none() && service.routes().is_empty());
        assert!(
            service.runtime().state().device(keyboard).is_some(),
            "device truth outlives the script's configuration"
        );
    }
}
