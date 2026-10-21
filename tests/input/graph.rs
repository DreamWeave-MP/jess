//! Compiling binding documents: what is rejected, how it is named, and what remapping does.

use std::time::Duration;

use jess::input::{
    ActionGraph, BindingLocation, Control, GamepadAxis, GraphErrorKind, GraphSpec, InputRuntime, Key, NodeSpec, Owner,
    RuntimeError, SpecError, Value, ValueType,
};

use super::common;

use common::{PACKAGE, gameplay, key, south};

fn set() -> Owner {
    Owner::Set("gameplay".to_owned())
}

/// Compiles `spec`, expecting failure, and returns every error's message.
fn errors(spec: GraphSpec) -> Vec<String> {
    ActionGraph::compile(spec)
        .expect_err("the document has errors")
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn a_chord_member_of_the_wrong_type_is_named_by_place_and_path() {
    let mut spec = gameplay();
    spec.replace_binding(
        &BindingLocation { owner: set(), index: 1 },
        NodeSpec::chord([
            key(Key::ControlLeft),
            NodeSpec::axis(Control::GamepadAxis(GamepadAxis::LeftStickX)),
        ]),
    )
    .unwrap();
    let messages = errors(spec);
    assert_eq!(messages.len(), 1, "{messages:#?}");
    assert_eq!(
        messages[0],
        "acceptance.jsl:12:1, set \"gameplay\" (from @jess/acceptance), action \"quickSave\", binding 1, \
         at chord: chord's members[1] must be digital, and it is scalar"
    );
}

#[test]
fn a_binding_of_the_wrong_type_names_both_types() {
    let mut spec = gameplay();
    spec.add_binding(&set(), "move", key(Key::Space)).unwrap();
    let messages = errors(spec);
    assert_eq!(
        messages,
        [
            "acceptance.jsl:12:1, set \"gameplay\" (from @jess/acceptance), action \"move\", binding 5: \
             the action is vector2, and this binding produces digital"
        ]
    );
}

#[test]
fn a_cycle_between_actions_is_an_error_naming_the_cycle() {
    let mut spec = GraphSpec::new();
    spec.action("a", ValueType::Digital)
        .action("b", ValueType::Digital)
        .action("c", ValueType::Digital)
        .set("s", PACKAGE);
    let owner = Owner::Set("s".to_owned());
    spec.add_binding(&owner, "a", NodeSpec::action_active("b")).unwrap();
    spec.add_binding(&owner, "b", NodeSpec::action_active("c")).unwrap();
    spec.add_binding(&owner, "c", NodeSpec::gate(key(Key::KeyX), NodeSpec::action_active("a")))
        .unwrap();
    let messages = errors(spec);
    assert_eq!(
        messages,
        ["action \"a\": actions read each other in a cycle: a -> b -> c -> a"]
    );
}

#[test]
fn every_kind_of_mistake_is_reported_at_once() {
    let mut spec = GraphSpec::new();
    spec.action("go", ValueType::Scalar)
        .action("go", ValueType::Digital)
        .action("", ValueType::Digital)
        .set("s", PACKAGE)
        .set("s", PACKAGE)
        .layer("l", PACKAGE);
    let owner = Owner::Set("s".to_owned());
    spec.add_binding(&owner, "go", NodeSpec::button(Control::GamepadAxis(GamepadAxis::LeftTrigger)))
        .unwrap();
    spec.add_binding(&owner, "go", NodeSpec::axis(Control::Key(Key::KeyA)))
        .unwrap();
    spec.add_binding(&owner, "go", NodeSpec::chord([key(Key::KeyA)])).unwrap();
    spec.add_binding(&owner, "go", NodeSpec::tap(key(Key::KeyA), 0)).unwrap();
    spec.add_binding(
        &owner,
        "go",
        NodeSpec::deadzone(NodeSpec::axis(Control::GamepadAxis(GamepadAxis::LeftStickX)), 0.9, 0.2),
    )
    .unwrap();
    spec.add_binding(
        &owner,
        "go",
        NodeSpec::threshold(NodeSpec::axis(Control::GamepadAxis(GamepadAxis::LeftStickX)), 0.0),
    )
    .unwrap();
    spec.add_binding(&owner, "go", NodeSpec::scale(south(), f32::NAN)).unwrap();
    spec.add_binding(&owner, "go", NodeSpec::action_active("nowhere")).unwrap();
    spec.layers[0].masks.push("nowhere".to_owned());
    spec.sets[0].bindings.push(jess::input::BindingSpec {
        action: "missing".to_owned(),
        input: key(Key::KeyB),
    });
    let messages = errors(spec);
    let kinds = [
        "action \"go\" is declared twice",
        "action has an empty name",
        "set \"s\" is declared twice",
        "sourceButton reads a button, and gamepadAxis:LeftTrigger is an axis; read it with sourceAxis, and make a button of it with threshold",
        "sourceAxis reads an axis, and key:KeyA is a button; read it with sourceButton",
        "chord needs at least 2 members, and has 1",
        "tap's withinMs is 0 ms; a threshold must be at least 1 ms",
        "deadzone edges must be 0 <= inner < outer <= 1, and they are inner 0.9, outer 0.2",
        "threshold's level must be above 0, and it is 0",
        "scale's factor isn't a finite number",
        "scale's input must be scalar or vector2, and it is digital",
        "no action \"nowhere\" is declared",
        "no action \"missing\" is declared",
        "the action is scalar, and this binding produces digital",
    ];
    for kind in kinds {
        assert!(
            messages.iter().any(|message| message.ends_with(kind)),
            "no error ends with {kind:?}:\n{}",
            messages.join("\n")
        );
    }
    let layer_mask = messages
        .iter()
        .find(|message| message.starts_with("layer \"l\""))
        .expect("the layer's mask is reported at the layer");
    assert_eq!(
        layer_mask,
        "layer \"l\" (from @jess/acceptance): no action \"nowhere\" is declared"
    );
}

#[test]
fn the_errors_are_data_too() {
    let mut spec = gameplay();
    spec.add_binding(&set(), "sprint", NodeSpec::hold(key(Key::KeyQ), 0)).unwrap();
    let error = ActionGraph::compile(spec).unwrap_err().remove(0);
    assert_eq!(
        error.kind,
        GraphErrorKind::ZeroDuration {
            node: "hold",
            field: "durationMs"
        }
    );
    assert_eq!(error.owner, Some(set()));
    assert_eq!(error.package.as_deref(), Some(PACKAGE));
    assert_eq!(error.action.as_deref(), Some("sprint"));
    assert_eq!(error.binding, Some(5));
    assert_eq!(error.path.as_deref(), Some("hold"));
    assert_eq!(error.origin.as_ref().map(|origin| origin.line), Some(12));
}

#[test]
fn unknown_nodes_and_fields_are_refused_loudly() {
    let text = gameplay().to_json().replace("\"type\": \"chord\"", "\"type\": \"doubleTap\"");
    let error = GraphSpec::from_json(&text).unwrap_err();
    let SpecError::Malformed(message) = &error else {
        panic!("{error}");
    };
    assert!(message.contains("unknown variant `doubleTap`"), "{message}");
    assert!(message.contains("line "), "{message}");

    let text = gameplay().to_json().replace("\"withinMs\"", "\"within\"");
    let error = GraphSpec::from_json(&text).unwrap_err().to_string();
    assert!(error.contains("unknown field `within`"), "{error}");

    let text = gameplay().to_json().replacen("\"version\": 1", "\"version\": 2", 1);
    assert_eq!(
        GraphSpec::from_json(&text).unwrap_err().to_string(),
        "the binding document is format version 2, and this jess::input reads version 1"
    );
}

#[test]
fn remapping_edits_the_document_and_the_runtime_recompiles_it() {
    let mut runtime = InputRuntime::new();
    let first = runtime.install(gameplay()).unwrap();
    runtime.set_active("gameplay", true).unwrap();
    let mut source = runtime.synthetic();
    let keyboard = source.connect_keyboard();

    // Inspect: quickSave has one binding, in the set.
    let locations = runtime.graph().unwrap().spec().bindings_for("quickSave");
    assert_eq!(locations, [BindingLocation { owner: set(), index: 1 }]);

    // Add an alternate, replace the original, and the set stays active across the swap.
    let second = runtime
        .remap(|spec| {
            spec.add_binding(&set(), "quickSave", key(Key::F5))?;
            spec.replace_binding(&locations[0], NodeSpec::chord([key(Key::ControlRight), key(Key::KeyS)]))?;
            Ok(())
        })
        .unwrap();
    assert!(second > first);
    assert_eq!(runtime.graph().unwrap().spec().bindings_for("quickSave").len(), 2);
    source.press(keyboard, Control::Key(Key::F5));
    source.advance(Duration::from_millis(16));
    runtime.tick(source.now());
    assert!(runtime.action("quickSave").unwrap().pressed_this_tick, "the alternate works");
    source.release(keyboard, Control::Key(Key::F5));
    source.press(keyboard, Control::Key(Key::ControlLeft));
    source.press(keyboard, Control::Key(Key::KeyS));
    source.advance(Duration::from_millis(16));
    runtime.tick(source.now());
    assert!(!runtime.action("quickSave").unwrap().active, "the replaced chord is gone");

    // A remap that doesn't compile changes nothing.
    let failed = runtime.remap(|spec| {
        spec.add_binding(
            &set(),
            "quickSave",
            NodeSpec::axis(Control::GamepadAxis(GamepadAxis::LeftStickX)),
        )?;
        Ok(())
    });
    assert!(matches!(failed, Err(RuntimeError::Graph(_))), "{failed:?}");
    assert_eq!(runtime.generation(), second);
    assert_eq!(runtime.graph().unwrap().spec().bindings_for("quickSave").len(), 2);

    // Remove, then restore the defaults.
    runtime
        .remap(|spec| {
            spec.remove_binding(&BindingLocation { owner: set(), index: 5 })?;
            Ok(())
        })
        .unwrap();
    assert_eq!(runtime.graph().unwrap().spec().bindings_for("quickSave").len(), 1);
    runtime.restore_defaults().unwrap();
    assert_eq!(runtime.graph().unwrap().spec(), &gameplay());
    assert!(runtime.evaluator().unwrap().active_sets().count() == 1, "still active");

    // Saved bindings must declare the installed actions.
    let mut saved = gameplay();
    saved.actions.pop();
    let error = runtime.apply_bindings(saved).unwrap_err().to_string();
    assert_eq!(
        error,
        "the saved bindings don't fit the installed actions: \"sprint\" is missing"
    );
    let mut saved = gameplay();
    saved.actions[0].value_type = ValueType::Scalar;
    saved.actions.push(jess::input::ActionSpec {
        name: "extra".to_owned(),
        value_type: ValueType::Digital,
        origin: None,
    });
    let error = runtime.apply_bindings(saved).unwrap_err().to_string();
    assert!(
        error.contains("\"move\" is scalar and the installed action is vector2"),
        "{error}"
    );
    assert!(error.contains("\"extra\" is not an installed action"), "{error}");

    let error = runtime.remap(|spec| spec.add_binding(&set(), "nope", key(Key::KeyA)).map(drop));
    assert_eq!(
        error.unwrap_err().to_string(),
        "the binding document declares no action \"nope\""
    );
    let error = runtime.remap(|spec| {
        spec.add_binding(&Owner::Layer("menu".to_owned()), "move", key(Key::KeyA))
            .map(drop)
    });
    assert_eq!(error.unwrap_err().to_string(), "the binding document has no layer \"menu\"");
}

#[test]
fn a_button_held_across_a_graph_swap_is_invisible_until_released() {
    let mut runtime = InputRuntime::new();
    runtime.install(gameplay()).unwrap();
    runtime.set_active("gameplay", true).unwrap();
    let mut source = runtime.synthetic();
    let keyboard = source.connect_keyboard();
    source.advance(Duration::from_millis(16));
    source.press(keyboard, Control::Key(Key::KeyW));
    source.advance(Duration::from_millis(16));
    runtime.tick(source.now());
    assert!(
        runtime.action("move").unwrap().active,
        "held before the swap: the old graph sees it"
    );

    // The same document again: the set stays active, and W, still down, is nobody's press.
    runtime.install(gameplay()).unwrap();
    source.advance(Duration::from_millis(16));
    runtime.tick(source.now());
    assert!(!runtime.action("move").unwrap().active, "held across the swap is not a press");
    source.release(keyboard, Control::Key(Key::KeyW));
    source.advance(Duration::from_millis(16));
    source.press(keyboard, Control::Key(Key::KeyW));
    source.advance(Duration::from_millis(16));
    runtime.tick(source.now());
    assert!(
        runtime.action("move").unwrap().pressed_this_tick,
        "pressed again after the swap"
    );
}

#[test]
fn an_uninstalled_runtime_still_tracks_devices() {
    let mut runtime = InputRuntime::new();
    let mut source = runtime.synthetic();
    let keyboard = source.connect_keyboard();
    source.press(keyboard, Control::Key(Key::KeyA));
    assert_eq!(runtime.tick(source.now()), []);
    assert!(runtime.state().device(keyboard).unwrap().is_pressed(Control::Key(Key::KeyA)));
    assert_eq!(runtime.set_active("gameplay", true), Err(RuntimeError::NoGraph));
    assert_eq!(runtime.action("move"), None);
    let generation = runtime.install(gameplay()).unwrap();
    assert_eq!(runtime.uninstall(), generation.next());
    assert!(runtime.graph().is_none());
}

#[test]
fn analog_transforms_shape_a_stick() {
    let mut spec = GraphSpec::new();
    spec.action("look", ValueType::Vector2)
        .action("throttle", ValueType::Scalar)
        .action("boost", ValueType::Digital)
        .set("s", PACKAGE);
    let owner = Owner::Set("s".to_owned());
    let stick = |axis| NodeSpec::axis(Control::GamepadAxis(axis));
    spec.add_binding(
        &owner,
        "look",
        NodeSpec::scale(
            NodeSpec::deadzone(
                NodeSpec::vector2(
                    stick(GamepadAxis::RightStickX),
                    NodeSpec::invert(stick(GamepadAxis::RightStickY)),
                ),
                0.2,
                0.9,
            ),
            2.0,
        ),
    )
    .unwrap();
    spec.add_binding(
        &owner,
        "throttle",
        NodeSpec::deadzone(stick(GamepadAxis::RightTrigger), 0.1, 1.0),
    )
    .unwrap();
    spec.add_binding(&owner, "boost", NodeSpec::threshold(stick(GamepadAxis::RightTrigger), 0.75))
        .unwrap();
    let mut runtime = InputRuntime::new();
    runtime.install(spec).unwrap();
    runtime.set_active("s", true).unwrap();
    let mut source = runtime.synthetic();
    let pad = source.connect_gamepad();

    source.axis(pad, Control::GamepadAxis(GamepadAxis::RightStickX), 0.1);
    source.axis(pad, Control::GamepadAxis(GamepadAxis::RightStickY), 0.1);
    runtime.tick(source.now());
    assert_eq!(
        runtime.action("look").unwrap().value,
        Value::Vector2([0.0, 0.0]),
        "inside the deadzone"
    );
    assert!(!runtime.action("look").unwrap().active);

    source.axis(pad, Control::GamepadAxis(GamepadAxis::RightStickX), 0.9);
    source.axis(pad, Control::GamepadAxis(GamepadAxis::RightStickY), 0.0);
    runtime.tick(source.now());
    assert_eq!(
        runtime.action("look").unwrap().value,
        Value::Vector2([2.0, 0.0]),
        "full deflection, scaled"
    );

    source.axis(pad, Control::GamepadAxis(GamepadAxis::RightStickX), 0.0);
    source.axis(pad, Control::GamepadAxis(GamepadAxis::RightStickY), 0.55);
    runtime.tick(source.now());
    let Value::Vector2([x, y]) = runtime.action("look").unwrap().value else {
        panic!("look is a vector");
    };
    assert!(
        x.abs() < 1e-6 && (y + 1.0).abs() < 1e-6,
        "halfway through the band, inverted: {x}, {y}"
    );

    source.axis(pad, Control::GamepadAxis(GamepadAxis::RightTrigger), 0.55);
    runtime.tick(source.now());
    let throttle = runtime.action("throttle").unwrap().value.as_scalar().unwrap();
    assert!((throttle - 0.5).abs() < 1e-6, "{throttle}");
    assert!(!runtime.action("boost").unwrap().active);
    source.axis(pad, Control::GamepadAxis(GamepadAxis::RightTrigger), 0.8);
    runtime.tick(source.now());
    assert!(runtime.action("boost").unwrap().pressed_this_tick);
}

#[test]
fn relative_axes_add_up_over_a_tick_and_reset() {
    let mut spec = GraphSpec::new();
    spec.action("look", ValueType::Vector2).set("s", PACKAGE);
    spec.add_binding(
        &Owner::Set("s".to_owned()),
        "look",
        NodeSpec::vector2(
            NodeSpec::axis(Control::PointerAxis(jess::input::PointerAxis::DeltaX)),
            NodeSpec::axis(Control::PointerAxis(jess::input::PointerAxis::DeltaY)),
        ),
    )
    .unwrap();
    let mut runtime = InputRuntime::new();
    runtime.install(spec).unwrap();
    runtime.set_active("s", true).unwrap();
    let mut source = runtime.synthetic();
    let mouse = source.connect_pointer();
    source.axis(mouse, Control::PointerAxis(jess::input::PointerAxis::DeltaX), 3.0);
    source.axis(mouse, Control::PointerAxis(jess::input::PointerAxis::DeltaX), 4.0);
    source.axis(mouse, Control::PointerAxis(jess::input::PointerAxis::DeltaY), -2.0);
    runtime.tick(source.now());
    assert_eq!(runtime.action("look").unwrap().value, Value::Vector2([7.0, -2.0]));
    runtime.tick(source.now());
    assert_eq!(
        runtime.action("look").unwrap().value,
        Value::Vector2([0.0, 0.0]),
        "no motion this tick"
    );
    assert!(runtime.action("look").unwrap().released_this_tick);
}
