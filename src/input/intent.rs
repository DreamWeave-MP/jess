//! Intents: semantic interaction, routed toward whoever should handle it.
//!
//! An action is interpreted input: `ui.accept` became active. An intent is what that means to a
//! target: activate whatever has focus. A command is an application operation, which an intent
//! may invoke and a menu, a palette, or a script may invoke just as well. This module is the
//! middle step and nothing more: a table of routes from action edges to intents, and a dispatch
//! list a consumer drains. It has no targets of its own, no focus, no bubbling, and no command
//! table; the scope a dispatch carries is whatever the consumer supplied.

use crate::input::graph::eval::{ActionEvent, ActionEventKind, Value};
use crate::input::id::BindingId;
use crate::input::{ActionId, DeviceId, Generation, IntentId, Timestamp};

/// Interns intent names. Intent ids are this registry's; names are the stable identity.
#[derive(Clone, Debug, Default)]
pub struct IntentRegistry {
    names: Vec<String>,
}

impl IntentRegistry {
    /// An empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self { names: Vec::new() }
    }

    /// The id of the intent named `name`, registering it on first use.
    pub fn intent(&mut self, name: &str) -> IntentId {
        let position = self.names.iter().position(|known| known == name).unwrap_or_else(|| {
            self.names.push(name.to_owned());
            self.names.len() - 1
        });
        IntentId::from_position(position)
    }

    /// The id of the intent named `name`, if it is registered.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<IntentId> {
        self.names.iter().position(|known| known == name).map(IntentId::from_position)
    }

    /// An intent's name.
    #[must_use]
    pub fn name(&self, intent: IntentId) -> Option<&str> {
        self.names.get(intent.position()).map(String::as_str)
    }
}

/// One route: this edge of this action is this intent.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IntentRoute {
    /// The action.
    pub action: ActionId,
    /// Which of the action's edges dispatches the intent.
    pub kind: ActionEventKind,
    /// The intent dispatched.
    pub intent: IntentId,
}

/// Where a dispatch is headed, as far as the router was told.
///
/// Every field is an opaque token the consumer chose and the consumer reads back; jess::input
/// compares nothing and resolves nothing. The script runtime will put its domain here, a
/// workspace its scope, a GUI its focused target. A field left `None` means nobody said.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct IntentScope {
    /// The script domain the intent belongs to.
    pub domain: Option<u64>,
    /// The workspace scope.
    pub workspace: Option<u64>,
    /// The session scope.
    pub session: Option<u64>,
    /// The target, such as a focused component, when a consumer has one.
    pub target: Option<u64>,
}

/// An intent on its way to a handler, with where it came from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct IntentDispatch {
    /// The intent.
    pub intent: IntentId,
    /// The scope the router was given for this batch.
    pub scope: IntentScope,
    /// The action whose edge produced the intent.
    pub action: ActionId,
    /// Which edge.
    pub kind: ActionEventKind,
    /// The action's value at that edge: the intent's payload.
    pub value: Value,
    /// The binding that produced the action's value.
    pub binding: Option<BindingId>,
    /// The device behind it.
    pub device: Option<DeviceId>,
    /// The tick the edge happened in.
    pub timestamp: Timestamp,
    /// The graph generation the action event came from.
    pub generation: Generation,
}

/// Routes action events to intents.
///
/// Routing is a table lookup in route order: for each action event, in the order the evaluator
/// produced them, every matching route dispatches once, in the order the routes were added. The
/// same events and the same routes always give the same dispatches in the same order.
///
/// The router holds action ids, which belong to one compiled graph. When the graph is replaced,
/// clear the routes and add them again by name; [`IntentRouter::route`] drops events from any
/// generation but the one it is told is current, so a stale event never becomes an intent.
#[derive(Clone, Debug, Default)]
pub struct IntentRouter {
    routes: Vec<IntentRoute>,
    dispatches: Vec<IntentDispatch>,
    dropped_stale: u64,
    dispatched: u64,
}

impl IntentRouter {
    /// A router with no routes.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            routes: Vec::new(),
            dispatches: Vec::new(),
            dropped_stale: 0,
            dispatched: 0,
        }
    }

    /// Adds a route. Adding the same route twice dispatches twice.
    pub fn add(&mut self, route: IntentRoute) {
        self.routes.push(route);
    }

    /// Removes every route, for a graph replacement.
    pub fn clear(&mut self) {
        self.routes.clear();
    }

    /// The routes, in the order they dispatch.
    #[must_use]
    pub fn routes(&self) -> &[IntentRoute] {
        &self.routes
    }

    /// Turns a tick's action events into dispatches under `scope`, replacing the last batch.
    /// Events not from generation `current` are dropped and counted.
    pub fn route(&mut self, events: &[ActionEvent], current: Generation, scope: IntentScope) -> &[IntentDispatch] {
        self.dispatches.clear();
        for event in events {
            if event.generation != current {
                self.dropped_stale += 1;
                continue;
            }
            for route in &self.routes {
                if route.action == event.action && route.kind == event.kind {
                    self.dispatches.push(IntentDispatch {
                        intent: route.intent,
                        scope,
                        action: event.action,
                        kind: event.kind,
                        value: event.value,
                        binding: event.binding,
                        device: event.device,
                        timestamp: event.timestamp,
                        generation: event.generation,
                    });
                }
            }
        }
        self.dispatched += u64::try_from(self.dispatches.len()).unwrap_or(u64::MAX);
        &self.dispatches
    }

    /// The last batch of dispatches.
    #[must_use]
    pub fn dispatches(&self) -> &[IntentDispatch] {
        &self.dispatches
    }

    /// How many intents this router has dispatched.
    #[must_use]
    pub const fn dispatched(&self) -> u64 {
        self.dispatched
    }

    /// How many action events this router dropped for being from another generation.
    #[must_use]
    pub const fn dropped_stale(&self) -> u64 {
        self.dropped_stale
    }
}
