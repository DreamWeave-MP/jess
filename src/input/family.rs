//! Presentation families: how a device's controls are labeled, apart from which device it is.
//!
//! A family names a set of glyphs, not a device and not artwork. Two different pads are two
//! devices and one family; a reconnect is a new connection and the same family. The labels a
//! family gives controls are `@jess/input`'s, in Luau; a theme maps them to whatever it draws.

use crate::input::DeviceKind;

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
}
