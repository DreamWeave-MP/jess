//! The intent acceptance fixture (handoff section 40): `ui.accept` becomes `ui.activate`, with
//! no target in sight.

use jess::input::{
    ActionEventKind, Control, GamepadButton, GraphSpec, InputRuntime, IntentRegistry, IntentRoute, IntentRouter,
    IntentScope, Key, NodeSpec, Owner, Value, ValueType,
};

const SOUTH: Control = Control::GamepadButton(GamepadButton::South);

fn ui() -> GraphSpec {
    let mut spec = GraphSpec::new();
    spec.action("ui.accept", ValueType::Digital)
        .action("ui.cancel", ValueType::Digital)
        .set("ui", "@jess/acceptance");
    let set = Owner::Set("ui".to_owned());
    spec.add_binding(&set, "ui.accept", NodeSpec::button(SOUTH)).unwrap();
    spec.add_binding(&set, "ui.accept", NodeSpec::button(Control::Key(Key::Enter)))
        .unwrap();
    spec.add_binding(&set, "ui.cancel", NodeSpec::button(Control::Key(Key::Escape)))
        .unwrap();
    spec
}

#[test]
fn an_action_edge_becomes_an_intent_with_its_provenance() {
    let mut runtime = InputRuntime::new();
    let generation = runtime.install(ui()).unwrap();
    runtime.set_active("ui", true).unwrap();
    let graph = runtime.graph().unwrap();
    let accept = graph.action_id("ui.accept").unwrap();
    let cancel = graph.action_id("ui.cancel").unwrap();

    let mut intents = IntentRegistry::new();
    let activate = intents.intent("ui.activate");
    let dismiss = intents.intent("ui.dismiss");
    assert_eq!(intents.intent("ui.activate"), activate, "names intern");
    let mut router = IntentRouter::new();
    router.add(IntentRoute {
        action: accept,
        kind: ActionEventKind::Pressed,
        intent: activate,
    });
    router.add(IntentRoute {
        action: cancel,
        kind: ActionEventKind::Pressed,
        intent: dismiss,
    });

    let mut source = runtime.synthetic();
    let pad = source.connect_gamepad();
    source.press(pad, SOUTH);
    let events = runtime.tick(source.now());
    let scope = IntentScope {
        domain: Some(7),
        target: Some(42),
        ..IntentScope::default()
    };
    let dispatches = router.route(events, generation, scope);
    assert_eq!(dispatches.len(), 1);
    let dispatch = dispatches[0];
    assert_eq!(dispatch.intent, activate);
    assert_eq!(intents.name(dispatch.intent), Some("ui.activate"));
    assert_eq!(dispatch.action, accept, "the action is kept");
    assert_eq!(dispatch.kind, ActionEventKind::Pressed);
    assert_eq!(dispatch.value, Value::Digital(true));
    assert_eq!(dispatch.device, Some(pad), "and so is the device behind it");
    let binding = dispatch.binding.expect("the binding that fired");
    assert_eq!(
        runtime.graph().unwrap().binding(binding).unwrap().index(),
        0,
        "the pad's binding, not Enter's"
    );
    assert_eq!(dispatch.scope, scope, "the scope is the consumer's, passed through untouched");
    assert_eq!(dispatch.generation, generation);
    assert_eq!(dispatch.timestamp, source.now());

    // Releasing is not routed; nothing is dispatched.
    source.release(pad, SOUTH);
    let events = runtime.tick(source.now());
    assert_eq!(events.len(), 1);
    assert_eq!(router.route(events, generation, scope), []);
    assert_eq!(router.dispatched(), 1);
}

#[test]
fn dispatch_order_is_the_event_order_then_the_route_order() {
    let mut runtime = InputRuntime::new();
    let generation = runtime.install(ui()).unwrap();
    runtime.set_active("ui", true).unwrap();
    let graph = runtime.graph().unwrap();
    let accept = graph.action_id("ui.accept").unwrap();
    let cancel = graph.action_id("ui.cancel").unwrap();
    let mut intents = IntentRegistry::new();
    let (activate, log, dismiss) = (
        intents.intent("ui.activate"),
        intents.intent("debug.log"),
        intents.intent("ui.dismiss"),
    );
    let mut router = IntentRouter::new();
    for (action, intent) in [(cancel, dismiss), (accept, activate), (accept, log)] {
        router.add(IntentRoute {
            action,
            kind: ActionEventKind::Pressed,
            intent,
        });
    }

    let mut source = runtime.synthetic();
    let keyboard = source.connect_keyboard();
    source.press(keyboard, Control::Key(Key::Escape));
    source.press(keyboard, Control::Key(Key::Enter));
    let events = runtime.tick(source.now());
    let order: Vec<_> = router
        .route(events, generation, IntentScope::default())
        .iter()
        .map(|dispatch| dispatch.intent)
        .collect();
    // Actions are evaluated in declaration order (accept before cancel); each action's routes
    // dispatch in the order they were added.
    assert_eq!(order, [activate, log, dismiss]);
    for _ in 0..3 {
        let repeat: Vec<_> = router
            .route(events, generation, IntentScope::default())
            .iter()
            .map(|dispatch| dispatch.intent)
            .collect();
        assert_eq!(repeat, order, "the same input routes the same way every time");
    }
}

#[test]
fn events_from_an_older_generation_never_become_intents() {
    let mut runtime = InputRuntime::new();
    let first = runtime.install(ui()).unwrap();
    runtime.set_active("ui", true).unwrap();
    let accept = runtime.graph().unwrap().action_id("ui.accept").unwrap();
    let mut intents = IntentRegistry::new();
    let mut router = IntentRouter::new();
    router.add(IntentRoute {
        action: accept,
        kind: ActionEventKind::Pressed,
        intent: intents.intent("ui.activate"),
    });

    let mut source = runtime.synthetic();
    let pad = source.connect_gamepad();
    source.press(pad, SOUTH);
    let stale: Vec<_> = runtime.tick(source.now()).to_vec();
    assert_eq!(stale.len(), 1);

    // The graph is replaced under the router; what the old one produced is dropped.
    let second = runtime.install(ui()).unwrap();
    assert!(second > first);
    assert_eq!(router.route(&stale, second, IntentScope::default()), []);
    assert_eq!(router.dropped_stale(), 1);
}
