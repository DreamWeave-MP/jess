//! The acceptance fixture the input tests share: one gameplay set, one inventory layer.

#![allow(dead_code)] // Each test binary uses the part it needs.

use jess::input::{Control, GamepadButton, GraphSpec, Key, LayerSpec, NodeSpec, Origin, Owner, ValueType};

pub const PACKAGE: &str = "@jess/acceptance";
pub const TAP_MS: u32 = 200;
pub const HOLD_MS: u32 = 350;

pub fn key(key: Key) -> NodeSpec {
    NodeSpec::button(Control::Key(key))
}

pub fn south() -> NodeSpec {
    NodeSpec::button(Control::GamepadButton(GamepadButton::South))
}

/// Section 39's fixture: `move` (WASD), `quickSave` (Ctrl+S), `interact` (tap A), `contextMenu`
/// (hold A 350 ms), and `sprint` (Shift while `move` is active), in the `gameplay` set, plus an
/// `inventory` layer that binds `interact` to `E` and masks `sprint`.
pub fn gameplay() -> GraphSpec {
    let mut spec = GraphSpec::new();
    spec.action("move", ValueType::Vector2)
        .action("quickSave", ValueType::Digital)
        .action("interact", ValueType::Digital)
        .action("contextMenu", ValueType::Digital)
        .action("sprint", ValueType::Digital)
        .set("gameplay", PACKAGE);
    spec.sets[0].origin = Some(Origin {
        file: "acceptance.luau".to_owned(),
        line: 12,
        column: 1,
    });
    let gameplay = Owner::Set("gameplay".to_owned());
    spec.add_binding(
        &gameplay,
        "move",
        NodeSpec::vector2(
            NodeSpec::axis_from_buttons(key(Key::KeyA), key(Key::KeyD)),
            NodeSpec::axis_from_buttons(key(Key::KeyS), key(Key::KeyW)),
        ),
    )
    .unwrap();
    spec.add_binding(
        &gameplay,
        "quickSave",
        NodeSpec::chord([key(Key::ControlLeft), key(Key::KeyS)]),
    )
    .unwrap();
    spec.add_binding(&gameplay, "interact", NodeSpec::tap(south(), TAP_MS))
        .unwrap();
    spec.add_binding(&gameplay, "contextMenu", NodeSpec::hold(south(), HOLD_MS))
        .unwrap();
    spec.add_binding(
        &gameplay,
        "sprint",
        NodeSpec::gate(key(Key::ShiftLeft), NodeSpec::action_active("move")),
    )
    .unwrap();
    spec.layers.push(LayerSpec {
        name: "inventory".to_owned(),
        owner: PACKAGE.to_owned(),
        bindings: Vec::new(),
        masks: vec!["sprint".to_owned()],
        origin: None,
    });
    spec.add_binding(&Owner::Layer("inventory".to_owned()), "interact", key(Key::KeyE))
        .unwrap();
    spec
}
