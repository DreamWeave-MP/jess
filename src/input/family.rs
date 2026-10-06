//! Presentation families: how a device's controls are labeled, apart from which device it is.
//!
//! A family is a set of glyph identifiers, not a device and not artwork. Two different pads are
//! two devices and one family; a reconnect is a new connection and the same family. A theme maps
//! a family's labels to whatever it draws, and this crate never sees the result.

use crate::input::{Control, DeviceKind, GamepadButton};

/// How a device's controls are labeled: a family of glyphs, not a device.
///
/// Two different controllers are two devices and can be one family; a reconnect gets a new
/// connection and the same family.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresentationFamily {
    /// Keys and pointer buttons.
    KeyboardMouse,
    /// Xbox-style labels: `A`, `B`, `X`, `Y`, `LB`, `RT`.
    Xbox,
    /// PlayStation-style labels: `Cross`, `Circle`, `L1`, `R2`.
    Playstation,
    /// Nintendo-style labels: `B` at the bottom, `A` on the right, `ZL`, `ZR`.
    Nintendo,
    /// A gamepad of no known family: positional labels, `South`, `LeftBumper`.
    GenericGamepad,
    /// Touch contacts.
    Touch,
}

impl PresentationFamily {
    /// Every family.
    pub const ALL: [Self; 6] = [
        Self::KeyboardMouse,
        Self::Xbox,
        Self::Playstation,
        Self::Nintendo,
        Self::GenericGamepad,
        Self::Touch,
    ];

    /// The family's identifier, as it serializes and as a theme would key artwork by.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::KeyboardMouse => "keyboardMouse",
            Self::Xbox => "xbox",
            Self::Playstation => "playstation",
            Self::Nintendo => "nintendo",
            Self::GenericGamepad => "genericGamepad",
            Self::Touch => "touch",
        }
    }

    /// The family named `name`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|family| family.name() == name)
    }

    /// The family a gamepad's USB vendor id suggests; the generic family when there is no id or
    /// it isn't one of the three console makers'.
    #[must_use]
    pub const fn for_gamepad(vendor_id: Option<u16>) -> Self {
        match vendor_id {
            Some(0x045e) => Self::Xbox,
            Some(0x054c) => Self::Playstation,
            Some(0x057e) => Self::Nintendo,
            _ => Self::GenericGamepad,
        }
    }

    /// Whether controls of a device kind are labeled by this family.
    #[must_use]
    pub const fn covers(self, kind: DeviceKind) -> bool {
        match self {
            Self::KeyboardMouse => matches!(kind, DeviceKind::Keyboard | DeviceKind::Pointer),
            Self::Xbox | Self::Playstation | Self::Nintendo | Self::GenericGamepad => {
                matches!(kind, DeviceKind::Gamepad)
            }
            Self::Touch => matches!(kind, DeviceKind::Touch),
        }
    }

    /// The semantic label this family gives a control: an identifier for a glyph, not text for
    /// a person. Keys keep their positional names (`KeyW`); what is printed on the keycap
    /// depends on the layout, which the platform knows and this crate doesn't.
    #[must_use]
    pub fn label(self, control: Control) -> String {
        let Control::GamepadButton(button) = control else {
            return match control {
                Control::Key(key) => key.name().to_owned(),
                Control::PointerButton(button) => button.name().to_owned(),
                Control::PointerAxis(axis) => axis.name().to_owned(),
                Control::GamepadAxis(axis) => axis.name().to_owned(),
                Control::TouchContact(contact) => format!("Contact{contact}"),
                Control::TouchAxis { contact, axis } => format!("Contact{contact}{axis}"),
                Control::GamepadButton(button) => button.name().to_owned(),
            };
        };
        let label = match (self, button) {
            (Self::Xbox, GamepadButton::South) | (Self::Nintendo, GamepadButton::East) => "A",
            (Self::Xbox, GamepadButton::East) | (Self::Nintendo, GamepadButton::South) => "B",
            (Self::Xbox, GamepadButton::West) | (Self::Nintendo, GamepadButton::North) => "X",
            (Self::Xbox, GamepadButton::North) | (Self::Nintendo, GamepadButton::West) => "Y",
            (Self::Xbox, GamepadButton::LeftBumper) => "LB",
            (Self::Xbox, GamepadButton::RightBumper) => "RB",
            (Self::Xbox, GamepadButton::LeftTrigger) => "LT",
            (Self::Xbox, GamepadButton::RightTrigger) => "RT",
            (Self::Xbox, GamepadButton::Select) => "View",
            (Self::Xbox, GamepadButton::Start) => "Menu",
            (Self::Xbox, GamepadButton::Guide) => "Xbox",
            (Self::Xbox | Self::Nintendo, GamepadButton::LeftStick) => "LS",
            (Self::Xbox | Self::Nintendo, GamepadButton::RightStick) => "RS",
            (Self::Playstation, GamepadButton::South) => "Cross",
            (Self::Playstation, GamepadButton::East) => "Circle",
            (Self::Playstation, GamepadButton::West) => "Square",
            (Self::Playstation, GamepadButton::North) => "Triangle",
            (Self::Playstation, GamepadButton::LeftBumper) => "L1",
            (Self::Playstation, GamepadButton::RightBumper) => "R1",
            (Self::Playstation, GamepadButton::LeftTrigger) => "L2",
            (Self::Playstation, GamepadButton::RightTrigger) => "R2",
            (Self::Playstation, GamepadButton::Select) => "Create",
            (Self::Playstation, GamepadButton::Start) => "Options",
            (Self::Playstation, GamepadButton::Guide) => "PS",
            (Self::Playstation, GamepadButton::LeftStick) => "L3",
            (Self::Playstation, GamepadButton::RightStick) => "R3",
            (Self::Nintendo, GamepadButton::LeftBumper) => "L",
            (Self::Nintendo, GamepadButton::RightBumper) => "R",
            (Self::Nintendo, GamepadButton::LeftTrigger) => "ZL",
            (Self::Nintendo, GamepadButton::RightTrigger) => "ZR",
            (Self::Nintendo, GamepadButton::Select) => "Minus",
            (Self::Nintendo, GamepadButton::Start) => "Plus",
            (Self::Nintendo, GamepadButton::Guide) => "Home",
            (_, button) => button.name(),
        };
        label.to_owned()
    }
}
