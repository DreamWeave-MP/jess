//! Devices: what is connected, what it can report, and how it is told apart from the others.
//!
//! A [`DeviceDescriptor`] is what a backend knows about a device when it connects. A
//! [`DeviceIdentity`] is the backend's best stable key for it; the registry behind an
//! [`crate::input::InputQueue`] maps that key to a [`crate::input::DeviceId`], and hands the same id back when
//! the same key connects again. How stable the key is depends on the platform: a Bluetooth
//! controller's address survives a reconnect, a USB pad with no serial is told apart only by the
//! port it's in.
//!
//! The connection identity is not the binding profile. Two different pads are two devices and
//! one [`PresentationFamily`]; a binding names a [`crate::input::Control`] and works on both.

use std::fmt;

use crate::input::family::PresentationFamily;

/// What kind of device a device is. Backends and vendors are metadata, never kinds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    /// Keys.
    Keyboard,
    /// A mouse, trackpad, or pen: buttons, motion, position, and wheels.
    Pointer,
    /// A controller: buttons, sticks, and triggers.
    Gamepad,
    /// A touch surface: contacts with position and pressure.
    Touch,
    /// A device the backend can't classify. It still connects and disconnects, and reports
    /// whatever controls the backend could translate.
    Unknown,
}

impl DeviceKind {
    /// The kind's name as it serializes.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::Pointer => "pointer",
            Self::Gamepad => "gamepad",
            Self::Touch => "touch",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for DeviceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// One thing a device can report.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum DeviceCapability {
    /// [`crate::input::Control::Key`] presses.
    Keys,
    /// [`crate::input::Control::PointerButton`] presses.
    PointerButtons,
    /// Relative pointer motion (`DeltaX`, `DeltaY`).
    PointerMotion,
    /// Absolute pointer position (`X`, `Y`).
    PointerPosition,
    /// Wheel steps (`WheelX`, `WheelY`).
    Wheel,
    /// [`crate::input::Control::GamepadButton`] presses.
    GamepadButtons,
    /// [`crate::input::Control::GamepadAxis`] values.
    GamepadAxes,
    /// Touch contacts and their axes.
    TouchContacts,
}

/// What an output channel drives. Reserved for haptics: nothing in this crate writes to one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[non_exhaustive]
pub enum OutputChannelKind {
    /// A rumble motor.
    Rumble,
    /// A trigger's own actuator.
    TriggerEffect,
}

/// An output channel a device has, by the identity a future haptics service will address.
///
/// Reserved: a backend may list the channels it finds, and nothing here drives them. The shape
/// exists so haptics later attaches to the device model instead of growing a second one.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct OutputChannel {
    /// What the channel drives.
    pub kind: OutputChannelKind,
    /// The channel's name on its device, such as `strong` or `leftTrigger`.
    pub name: String,
}

/// What a backend knows about a device when it connects.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceDescriptor {
    /// What kind of device it is.
    pub kind: DeviceKind,
    /// The name the device reports, for people.
    pub name: String,
    /// The USB or Bluetooth vendor id, where the platform has one.
    pub vendor_id: Option<u16>,
    /// The product id, where the platform has one.
    pub product_id: Option<u16>,
    /// Which backend found the device and what it calls it (`evdev:/dev/input/event7`), for
    /// diagnostics only. Never part of a binding.
    pub backend: Option<String>,
    /// What the device reports.
    pub capabilities: Vec<DeviceCapability>,
    /// The output channels the device has, reserved for haptics.
    pub outputs: Vec<OutputChannel>,
    /// How bindings to this device should be labeled.
    pub family: PresentationFamily,
}

impl DeviceDescriptor {
    /// A keyboard named `name`.
    #[must_use]
    pub fn keyboard(name: impl Into<String>) -> Self {
        Self {
            kind: DeviceKind::Keyboard,
            name: name.into(),
            vendor_id: None,
            product_id: None,
            backend: None,
            capabilities: vec![DeviceCapability::Keys],
            outputs: Vec::new(),
            family: PresentationFamily::KeyboardMouse,
        }
    }

    /// A relative pointer with buttons and a wheel, named `name`.
    #[must_use]
    pub fn pointer(name: impl Into<String>) -> Self {
        Self {
            kind: DeviceKind::Pointer,
            name: name.into(),
            vendor_id: None,
            product_id: None,
            backend: None,
            capabilities: vec![
                DeviceCapability::PointerButtons,
                DeviceCapability::PointerMotion,
                DeviceCapability::Wheel,
            ],
            outputs: Vec::new(),
            family: PresentationFamily::KeyboardMouse,
        }
    }

    /// A gamepad named `name`, its presentation family guessed from the vendor id.
    #[must_use]
    pub fn gamepad(name: impl Into<String>, vendor_id: Option<u16>, product_id: Option<u16>) -> Self {
        Self {
            kind: DeviceKind::Gamepad,
            name: name.into(),
            vendor_id,
            product_id,
            backend: None,
            capabilities: vec![DeviceCapability::GamepadButtons, DeviceCapability::GamepadAxes],
            outputs: Vec::new(),
            family: PresentationFamily::for_gamepad(vendor_id),
        }
    }

    /// A touch surface named `name`.
    #[must_use]
    pub fn touch(name: impl Into<String>) -> Self {
        Self {
            kind: DeviceKind::Touch,
            name: name.into(),
            vendor_id: None,
            product_id: None,
            backend: None,
            capabilities: vec![DeviceCapability::TouchContacts],
            outputs: Vec::new(),
            family: PresentationFamily::Touch,
        }
    }

    /// Whether the device reports `capability`.
    #[must_use]
    pub fn has(&self, capability: DeviceCapability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// A backend's stable key for a device: what it uses to say "this is the one that was here
/// before".
///
/// The registry compares keys as text and knows nothing of their contents. A backend builds the
/// key from the most stable facts the platform gives it (a serial or Bluetooth address, else the
/// physical port, else vendor, product, and name), and prefixes its own name so two backends
/// never collide.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct DeviceIdentity(String);

impl DeviceIdentity {
    /// An identity with the key `key`.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
