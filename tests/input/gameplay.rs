//! The first acceptance fixture (handoff section 39): one gameplay set driven by synthetic events
//! on a clock that only moves when told to.

use std::path::PathBuf;
use std::time::Duration;

use jess::input::{ActionEventKind, Control, GamepadButton, GraphSpec, InputRuntime, Key, SyntheticSource, Value, ValueType};

use crate::common;

use common::{HOLD_MS, TAP_MS, gameplay};

const SOUTH: Control = Control::GamepadButton(GamepadButton::South);

struct Rig {
    runtime: InputRuntime,
    source: SyntheticSource,
    keyboard: jess::input::DeviceId,
    pad: jess::input::DeviceId,
}

impl Rig {
    fn new() -> Self {
        let mut runtime = InputRuntime::new();
        runtime.install(gameplay()).unwrap();
        runtime.set_active("gameplay", true).unwrap();
        let mut source = runtime.synthetic();
        let keyboard = source.connect_keyboard();
        let pad = source.connect_gamepad();
        let mut rig = Self {
            runtime,
            source,
            keyboard,
            pad,
        };
        rig.tick();
        rig
    }

    fn tick(&mut self) -> Vec<(String, ActionEventKind)> {
        let now = self.source.now();
        let graph = self.runtime.graph().unwrap();
        let names: Vec<String> = graph.actions().iter().map(|action| action.name().to_owned()).collect();
        self.runtime
            .tick(now)
            .iter()
            .map(|event| (names[event.action.index() as usize].clone(), event.kind))
            .collect()
    }

    fn advance_and_tick(&mut self, millis: u64) -> Vec<(String, ActionEventKind)> {
        self.source.advance(Duration::from_millis(millis));
        self.tick()
    }

    fn key(&mut self, key: Key, pressed: bool) {
        if pressed {
            self.source.press(self.keyboard, Control::Key(key));
        } else {
            self.source.release(self.keyboard, Control::Key(key));
        }
    }

    fn value(&self, action: &str) -> Value {
        self.runtime.action(action).unwrap().value
    }

    fn active(&self, action: &str) -> bool {
        self.runtime.action(action).unwrap().active
    }

    fn pressed(&self, action: &str) -> bool {
        self.runtime.action(action).unwrap().pressed_this_tick
    }

    fn released(&self, action: &str) -> bool {
        self.runtime.action(action).unwrap().released_this_tick
    }
}

fn edges(events: &[(String, ActionEventKind)], action: &str, kind: ActionEventKind) -> usize {
    events.iter().filter(|(name, edge)| name == action && *edge == kind).count()
}

#[test]
fn wasd_produces_the_movement_vector() {
    let mut rig = Rig::new();
    rig.key(Key::KeyW, true);
    rig.advance_and_tick(16);
    assert_eq!(rig.value("move"), Value::Vector2([0.0, 1.0]));
    assert!(rig.pressed("move"));

    rig.key(Key::KeyD, true);
    rig.advance_and_tick(16);
    assert_eq!(rig.value("move"), Value::Vector2([1.0, 1.0]));
    assert!(!rig.pressed("move"), "a vector that keeps moving doesn't press again");

    rig.key(Key::KeyW, false);
    rig.key(Key::KeyS, true);
    rig.advance_and_tick(16);
    assert_eq!(rig.value("move"), Value::Vector2([1.0, -1.0]));

    rig.key(Key::KeyA, true);
    rig.advance_and_tick(16);
    assert_eq!(rig.value("move"), Value::Vector2([0.0, -1.0]), "opposite keys cancel");

    rig.key(Key::KeyA, false);
    rig.key(Key::KeyD, false);
    rig.key(Key::KeyS, false);
    let events = rig.advance_and_tick(16);
    assert_eq!(rig.value("move"), Value::Vector2([0.0, 0.0]));
    assert!(rig.released("move"));
    assert_eq!(edges(&events, "move", ActionEventKind::Released), 1);
    assert_eq!(edges(&events, "move", ActionEventKind::Changed), 1);
}

#[test]
fn the_chord_fires_only_when_complete_and_exactly_once() {
    let mut rig = Rig::new();
    rig.key(Key::ControlLeft, true);
    let events = rig.advance_and_tick(16);
    assert!(!rig.active("quickSave"), "Ctrl alone is not Ctrl+S");
    assert_eq!(edges(&events, "quickSave", ActionEventKind::Pressed), 0);

    rig.key(Key::KeyS, true);
    let events = rig.advance_and_tick(16);
    assert!(rig.pressed("quickSave"));
    assert_eq!(edges(&events, "quickSave", ActionEventKind::Pressed), 1);
    let state = rig.runtime.action("quickSave").unwrap();
    assert_eq!(state.device, Some(rig.keyboard), "the value traces to the keyboard");
    let binding = state.binding.expect("the chord is the binding that fired");
    let graph = rig.runtime.graph().unwrap();
    assert_eq!(graph.binding(binding).unwrap().index(), 1, "the set's second binding");

    let events = rig.advance_and_tick(16);
    assert!(
        rig.active("quickSave") && !rig.pressed("quickSave"),
        "held, not pressed again"
    );
    assert!(events.is_empty(), "nothing changed: {events:?}");

    rig.key(Key::ControlLeft, false);
    let events = rig.advance_and_tick(16);
    assert!(rig.released("quickSave"), "any member releasing breaks the chord");
    assert_eq!(edges(&events, "quickSave", ActionEventKind::Released), 1);

    // S is still held; pressing Ctrl again completes the chord again, whatever the order.
    rig.key(Key::ControlLeft, true);
    rig.advance_and_tick(16);
    assert!(rig.pressed("quickSave"), "member order doesn't matter");
}

#[test]
fn tap_and_hold_on_one_button_are_told_apart() {
    let mut rig = Rig::new();

    // A short press is a tap and never a hold.
    rig.source.press(rig.pad, SOUTH);
    rig.advance_and_tick(16);
    assert!(
        !rig.active("interact") && !rig.active("contextMenu"),
        "nothing fires on the press"
    );
    rig.source.advance(Duration::from_millis(100));
    rig.source.release(rig.pad, SOUTH);
    let events = rig.tick();
    assert!(rig.pressed("interact"), "released inside {TAP_MS} ms: a tap");
    assert_eq!(edges(&events, "interact", ActionEventKind::Pressed), 1);
    assert!(!rig.active("contextMenu"));
    let events = rig.advance_and_tick(16);
    assert!(rig.released("interact"), "a tap lasts one tick");
    assert_eq!(edges(&events, "interact", ActionEventKind::Released), 1);

    // A long press is a hold, fires once when the threshold is crossed, and is never a tap.
    rig.source.press(rig.pad, SOUTH);
    rig.advance_and_tick(16);
    rig.advance_and_tick(300);
    assert!(!rig.active("contextMenu"), "316 ms is not {HOLD_MS} ms yet");
    let events = rig.advance_and_tick(50);
    assert!(rig.pressed("contextMenu"), "366 ms is past {HOLD_MS} ms");
    assert_eq!(edges(&events, "contextMenu", ActionEventKind::Pressed), 1);
    let events = rig.advance_and_tick(500);
    assert!(rig.active("contextMenu") && !rig.pressed("contextMenu"));
    assert!(events.is_empty(), "holding on produces no edges: {events:?}");
    rig.source.release(rig.pad, SOUTH);
    let events = rig.tick();
    assert!(rig.released("contextMenu"));
    assert!(!rig.active("interact"), "a release after the tap window is not a tap");
    assert_eq!(edges(&events, "interact", ActionEventKind::Pressed), 0);
}

#[test]
fn timing_follows_the_test_clock_not_the_wall_clock() {
    let mut rig = Rig::new();
    rig.source.press(rig.pad, SOUTH);
    for _ in 0..100 {
        rig.tick();
    }
    assert!(
        !rig.active("contextMenu"),
        "a hundred ticks at the same instant is no time at all"
    );
    rig.advance_and_tick(u64::from(HOLD_MS));
    assert!(rig.pressed("contextMenu"), "one tick at the threshold is enough");
}

#[test]
fn a_hold_measures_from_the_press_timestamp_not_the_tick_that_saw_it() {
    let mut rig = Rig::new();
    rig.source.press(rig.pad, SOUTH);
    // The first tick to see the press is already 340 ms after it.
    rig.advance_and_tick(340);
    assert!(!rig.active("contextMenu"));
    rig.advance_and_tick(10);
    assert!(rig.pressed("contextMenu"), "350 ms since the device said so");
}

#[test]
fn the_gate_enables_sprint_only_while_move_is_active() {
    let mut rig = Rig::new();
    rig.key(Key::ShiftLeft, true);
    rig.advance_and_tick(16);
    assert!(!rig.active("sprint"), "Shift while standing still is nothing");

    rig.key(Key::KeyW, true);
    let events = rig.advance_and_tick(16);
    assert!(
        rig.active("move") && rig.pressed("sprint"),
        "move is evaluated before what reads it"
    );
    assert_eq!(edges(&events, "sprint", ActionEventKind::Pressed), 1);

    rig.key(Key::KeyW, false);
    rig.advance_and_tick(16);
    assert!(rig.released("sprint"), "stop moving and the gate closes, Shift still held");

    rig.key(Key::KeyW, true);
    rig.advance_and_tick(16);
    assert!(rig.pressed("sprint"), "and opens again");
}

#[test]
fn deactivating_the_set_suppresses_every_output() {
    let mut rig = Rig::new();
    rig.key(Key::KeyW, true);
    rig.key(Key::ShiftLeft, true);
    rig.advance_and_tick(16);
    assert!(rig.active("move") && rig.active("sprint"));

    rig.runtime.set_active("gameplay", false).unwrap();
    let events = rig.advance_and_tick(16);
    assert!(!rig.active("move") && !rig.active("sprint"));
    assert_eq!(edges(&events, "move", ActionEventKind::Released), 1);
    assert_eq!(edges(&events, "sprint", ActionEventKind::Released), 1);

    rig.key(Key::ControlLeft, true);
    rig.key(Key::KeyS, true);
    let events = rig.advance_and_tick(16);
    assert!(events.is_empty(), "an inactive set hears nothing: {events:?}");

    // Reactivating: W and Shift were held before the set changed, so they stay invisible until
    // released; Ctrl+S too.
    rig.runtime.set_active("gameplay", true).unwrap();
    rig.advance_and_tick(16);
    assert!(
        !rig.active("move") && !rig.active("quickSave"),
        "held across the switch is not a press"
    );
    rig.key(Key::KeyW, false);
    rig.key(Key::KeyW, true);
    rig.advance_and_tick(16);
    assert!(rig.active("move"), "released and pressed again counts");
}

#[test]
fn a_layer_overrides_one_binding_and_masks_another() {
    let mut rig = Rig::new();
    rig.runtime.push_layer("inventory").unwrap();
    rig.advance_and_tick(16);

    // The layer binds interact to E, replacing the tap of A.
    rig.source.press(rig.pad, SOUTH);
    rig.advance_and_tick(50);
    rig.source.release(rig.pad, SOUTH);
    rig.advance_and_tick(16);
    assert!(!rig.active("interact"), "the set's tap is replaced while the layer is up");
    rig.key(Key::KeyE, true);
    let events = rig.advance_and_tick(16);
    assert!(rig.pressed("interact"));
    let binding = rig.runtime.action("interact").unwrap().binding.unwrap();
    let (owner, _) = rig.runtime.graph().unwrap().binding_spec(binding).unwrap();
    assert_eq!(owner, jess::input::Owner::Layer("inventory".to_owned()));
    assert_eq!(edges(&events, "interact", ActionEventKind::Pressed), 1);
    rig.key(Key::KeyE, false);

    // The layer masks sprint; the actions it doesn't name pass through.
    rig.key(Key::KeyW, true);
    rig.key(Key::ShiftLeft, true);
    rig.advance_and_tick(16);
    assert!(rig.active("move"), "move passes through the layer");
    assert!(!rig.active("sprint"), "sprint is masked");
    assert!(!rig.active("contextMenu"), "the hold of A stays in the set, and A is up");

    // Popping the layer puts the set's bindings back; Shift and W were held across the change,
    // so sprint waits for a fresh press.
    rig.runtime.pop_layer("inventory").unwrap();
    rig.advance_and_tick(16);
    assert!(!rig.active("sprint"));
    rig.key(Key::ShiftLeft, false);
    rig.key(Key::ShiftLeft, true);
    rig.advance_and_tick(16);
    assert!(rig.active("sprint"), "the set's gate is back");
    rig.key(Key::KeyE, true);
    rig.advance_and_tick(16);
    assert!(!rig.active("interact"), "E means nothing without the layer");
}

#[test]
fn a_disconnected_device_releases_what_it_held() {
    let mut rig = Rig::new();
    rig.key(Key::KeyW, true);
    rig.advance_and_tick(16);
    assert!(rig.active("move"));
    rig.source.disconnect(rig.keyboard);
    let events = rig.advance_and_tick(16);
    assert!(!rig.active("move"));
    assert_eq!(edges(&events, "move", ActionEventKind::Released), 1);

    // The same identity reconnects with the same id and a clean slate.
    let again = rig.source.connect_keyboard();
    assert_ne!(again, rig.keyboard, "a new synthetic keyboard is a new identity");
    rig.advance_and_tick(16);
    assert!(
        rig.runtime
            .state()
            .device(rig.keyboard)
            .is_some_and(|device| !device.is_connected())
    );
}

#[test]
fn the_evaluator_allocates_nothing_after_the_first_tick_and_reports_its_cost() {
    let mut rig = Rig::new();
    rig.key(Key::KeyW, true);
    rig.advance_and_tick(16);
    let stats = rig.runtime.stats();
    assert_eq!(stats.device_events, 1);
    assert_eq!(stats.evaluation.bindings_evaluated, 5);
    assert_eq!(stats.evaluation.nodes_evaluated, 7 + 3 + 2 + 2 + 3);
    assert_eq!(stats.evaluation.actions_changed, 1);
    assert_eq!(stats.evaluation.action_events, 2, "move pressed and changed");
    assert_eq!(stats.evaluation.active_sets, 1);
    assert_eq!(stats.evaluation.active_layers, 0);
    assert!(stats.evaluation.total_evaluation_time >= stats.evaluation.evaluation_time);
}

#[test]
fn the_document_round_trips_deterministically() {
    let spec = gameplay();
    let json = spec.to_json();
    let parsed = GraphSpec::from_json(&json).unwrap();
    assert_eq!(parsed, spec);
    assert_eq!(parsed.to_json(), json, "the same document is the same text");

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/input/golden/gameplay.json");
    if std::env::var_os("JESS_BLESS").is_some() {
        std::fs::write(&path, &json).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .replace("\r\n", "\n");
    assert!(json == expected, "{} differs from the document:\n{json}", path.display());

    // What the golden holds is what a runtime compiles.
    let mut runtime = InputRuntime::new();
    runtime.install(GraphSpec::from_json(&expected).unwrap()).unwrap();
    let graph = runtime.graph().unwrap();
    assert_eq!(graph.actions().len(), 5);
    let move_action = graph.action(graph.action_id("move").unwrap()).unwrap();
    assert_eq!(move_action.value_type(), ValueType::Vector2);
}
