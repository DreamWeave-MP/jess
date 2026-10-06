//! Binding presentation and conflict detection: data for a GUI, from the fixture's document.

use jess::input::{
    BindingLocation, Conflict, ConflictKind, Control, GamepadButton, Key, NodeSpec, Owner, Presentation, PresentationFamily,
    conflicts, presentations,
};

use crate::common;

use common::{gameplay, key, south};

fn set() -> Owner {
    Owner::Set("gameplay".to_owned())
}

#[test]
fn presentations_are_shapes_with_family_labels() {
    let spec = gameplay();
    let keyboard = presentations(&spec, "quickSave", PresentationFamily::KeyboardMouse);
    assert_eq!(keyboard.len(), 1);
    let Presentation::Chord(parts) = &keyboard[0].shape else {
        panic!("{:?}", keyboard[0].shape);
    };
    let labels: Vec<&str> = parts
        .iter()
        .map(|part| match part {
            Presentation::Control(control) => control.label.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(labels, ["ControlLeft", "KeyS"]);
    assert!(
        presentations(&spec, "quickSave", PresentationFamily::Xbox).is_empty(),
        "a keyboard chord isn't a gamepad binding"
    );

    let xbox = presentations(&spec, "contextMenu", PresentationFamily::Xbox);
    let Presentation::Hold { input, duration_ms } = &xbox[0].shape else {
        panic!("{:?}", xbox[0].shape);
    };
    assert_eq!(*duration_ms, 350);
    let Presentation::Control(control) = &**input else {
        panic!("{input:?}");
    };
    assert_eq!(control.label, "A");
    assert_eq!(control.control, Control::GamepadButton(GamepadButton::South));
    let playstation = presentations(&spec, "contextMenu", PresentationFamily::Playstation);
    let Presentation::Hold { input, .. } = &playstation[0].shape else {
        panic!("{:?}", playstation[0].shape);
    };
    let Presentation::Control(control) = &**input else {
        panic!("{input:?}");
    };
    assert_eq!(control.label, "Cross");

    let interact: Vec<_> = presentations(&spec, "interact", PresentationFamily::KeyboardMouse);
    assert_eq!(interact.len(), 1, "the layer's binding, labeled for the keyboard");
    assert_eq!(interact[0].location.owner, Owner::Layer("inventory".to_owned()));

    let sprint = presentations(&spec, "sprint", PresentationFamily::KeyboardMouse);
    let Presentation::While { input, condition } = &sprint[0].shape else {
        panic!("{:?}", sprint[0].shape);
    };
    assert!(matches!(&**input, Presentation::Control(control) if control.label == "ShiftLeft"));
    assert_eq!(**condition, Presentation::Action("move".to_owned()));

    let movement = presentations(&spec, "move", PresentationFamily::KeyboardMouse);
    let Presentation::Vector2 { x, .. } = &movement[0].shape else {
        panic!("{:?}", movement[0].shape);
    };
    assert!(matches!(&**x, Presentation::Axis { .. }));
}

#[test]
fn plain_conflicts_are_found_and_contextual_ones_are_not() {
    let mut spec = gameplay();
    assert!(conflicts(&spec).is_empty(), "tap A and hold A are not a conflict");
    spec.add_binding(&set(), "sprint", NodeSpec::chord([key(Key::KeyS), key(Key::ControlLeft)]))
        .unwrap();
    spec.add_binding(&set(), "quickSave", key(Key::KeyS)).unwrap();
    spec.add_binding(&set(), "contextMenu", south()).unwrap();
    let found = conflicts(&spec);
    assert_eq!(
        found,
        [Conflict {
            owner: set(),
            kind: ConflictKind::Press,
            controls: vec![Control::Key(Key::KeyS), Control::Key(Key::ControlLeft)],
            actions: ["quickSave".to_owned(), "sprint".to_owned()],
            bindings: [
                BindingLocation { owner: set(), index: 1 },
                BindingLocation { owner: set(), index: 5 }
            ],
        }],
        "the same chord in any order is a conflict; S beside Ctrl+S and a press beside a tap are not"
    );
}
