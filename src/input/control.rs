//! The Jess control vocabulary: what a device can report, named without reference to any backend.
//!
//! A [`Control`] is one physical thing on a device: a key at a position on the keyboard, a pointer
//! button or axis, a gamepad button or axis, or a touch contact. The names are Jess's, stable, and
//! what bindings serialize; a backend translates its own codes into them and nothing above the
//! backend ever sees an evdev code, a gilrs button, or a virtual key.
//!
//! Keys are named by physical position, after the W3C UI Events `KeyboardEvent.code` list
//! (`KeyA`, `ControlLeft`, `Numpad7`), so a binding to `KeyW` is the same key on an AZERTY board
//! and the label a user sees comes from the layout, later. Gamepad buttons are named by position
//! too (`South`, not `A` or `Cross`); a [`crate::input::PresentationFamily`] supplies the letter.
//!
//! Axis values are normalized: sticks are [`AxisRange::Centered`], `-1.0..=1.0`, with `Y` positive
//! upward; triggers are [`AxisRange::Unipolar`], `0.0..=1.0`; pointer position, motion, and wheel
//! axes are [`AxisRange::Unbounded`], in surface pixels and wheel steps.

use std::fmt;
use std::str::FromStr;

use crate::input::DeviceKind;

macro_rules! named_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
        pub enum $name {
            $(
                #[doc = concat!("The `", stringify!($variant), "` control.")]
                $variant,
            )+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The stable name bindings serialize.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant),)+
                }
            }

            /// The value named `name`, if there is one.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $(stringify!($variant) => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.name())
            }
        }
    };
}

named_enum! {
    /// A key, by its position on the keyboard.
    ///
    /// The names are the W3C `KeyboardEvent.code` names, less the media and browser keys.
    Key {
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM,
        KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
        Minus, Equal, BracketLeft, BracketRight, Backslash, Semicolon, Quote, Backquote,
        Comma, Period, Slash, IntlBackslash, IntlRo, IntlYen,
        Space, Enter, Tab, Backspace, Escape, CapsLock,
        ShiftLeft, ShiftRight, ControlLeft, ControlRight, AltLeft, AltRight, MetaLeft, MetaRight,
        ContextMenu,
        ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
        Insert, Delete, Home, End, PageUp, PageDown,
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
        F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24,
        PrintScreen, ScrollLock, Pause, NumLock,
        Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
        NumpadAdd, NumpadSubtract, NumpadMultiply, NumpadDivide, NumpadDecimal, NumpadEnter,
        NumpadEqual, NumpadComma,
    }
}

named_enum! {
    /// A pointer (mouse, trackpad, pen) button, by role rather than by hand.
    PointerButton {
        Primary, Secondary, Middle, Back, Forward,
    }
}

named_enum! {
    /// A pointer axis.
    ///
    /// `X` and `Y` are the absolute position in surface pixels, when the surface reports one;
    /// `DeltaX` and `DeltaY` are the motion since the last event, in pixels, `Y` positive downward
    /// as surfaces count; `WheelX` and `WheelY` are wheel steps, `WheelY` positive away from the
    /// user.
    PointerAxis {
        X, Y, DeltaX, DeltaY, WheelX, WheelY,
    }
}

named_enum! {
    /// A gamepad button, by position.
    ///
    /// `South` is the bottom face button (`A` on an Xbox pad, `Cross` on Sony's, `B` on
    /// Nintendo's); `East`, `West`, and `North` go round from it. `LeftTrigger` and
    /// `RightTrigger` are the triggers as buttons; their pressure is a [`GamepadAxis`].
    GamepadButton {
        South, East, West, North,
        LeftBumper, RightBumper, LeftTrigger, RightTrigger,
        Select, Start, Guide,
        LeftStick, RightStick,
        DPadUp, DPadDown, DPadLeft, DPadRight,
    }
}

named_enum! {
    /// A gamepad axis.
    ///
    /// Sticks are centered, `-1.0..=1.0`, `X` positive right and `Y` positive up; triggers are
    /// `0.0..=1.0`.
    GamepadAxis {
        LeftStickX, LeftStickY, RightStickX, RightStickY, LeftTrigger, RightTrigger,
    }
}

named_enum! {
    /// A touch contact's axis: position in surface pixels, or pressure in `0.0..=1.0`.
    TouchAxis {
        X, Y, Pressure,
    }
}

/// The range an axis control reports in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AxisRange {
    /// `-1.0..=1.0`, resting at zero: a stick axis.
    Centered,
    /// `0.0..=1.0`, resting at zero: a trigger, or touch pressure.
    Unipolar,
    /// Any value, in the control's own units: pixels for pointer position and motion, steps for
    /// a wheel.
    Unbounded,
}

/// Whether a control is a button or an axis.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ControlKind {
    /// Pressed or not.
    Digital,
    /// A value in the given range.
    Axis(AxisRange),
}

/// One physical control on a device, in Jess's vocabulary.
///
/// Its text form is `<class>:<name>`: `key:KeyA`, `pointerButton:Primary`, `pointerAxis:DeltaX`,
/// `gamepadButton:South`, `gamepadAxis:LeftStickX`, `touch:0` (a contact, as a button), and
/// `touchAxis:0:X`. That form is what bindings serialize and what Luau scripts write.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Control {
    /// A keyboard key.
    Key(Key),
    /// A pointer button.
    PointerButton(PointerButton),
    /// A pointer axis.
    PointerAxis(PointerAxis),
    /// A gamepad button.
    GamepadButton(GamepadButton),
    /// A gamepad axis.
    GamepadAxis(GamepadAxis),
    /// A touch contact, pressed while the finger is down. Contacts are numbered from zero in the
    /// order they land.
    TouchContact(u8),
    /// One axis of a touch contact.
    TouchAxis {
        /// The contact, numbered as [`Control::TouchContact`] numbers it.
        contact: u8,
        /// Which axis.
        axis: TouchAxis,
    },
}

impl Control {
    /// Whether the control is a button or an axis, and the axis's range.
    #[must_use]
    pub const fn kind(self) -> ControlKind {
        match self {
            Self::Key(_) | Self::PointerButton(_) | Self::GamepadButton(_) | Self::TouchContact(_) => ControlKind::Digital,
            Self::PointerAxis(_)
            | Self::TouchAxis {
                axis: TouchAxis::X | TouchAxis::Y,
                ..
            } => ControlKind::Axis(AxisRange::Unbounded),
            Self::GamepadAxis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
            | Self::TouchAxis {
                axis: TouchAxis::Pressure,
                ..
            } => ControlKind::Axis(AxisRange::Unipolar),
            Self::GamepadAxis(_) => ControlKind::Axis(AxisRange::Centered),
        }
    }

    /// Whether the control is a button.
    #[must_use]
    pub const fn is_digital(self) -> bool {
        matches!(self.kind(), ControlKind::Digital)
    }

    /// Whether the control reports motion rather than a position: pointer deltas and wheels.
    ///
    /// A relative axis has no resting value a device returns to. [`crate::input::InputState`] adds its
    /// events up over a tick and [`crate::input::InputState::end_tick`] zeroes the sum, so an action
    /// bound to `pointerAxis:DeltaX` reads how far the pointer moved this tick.
    #[must_use]
    pub const fn is_relative(self) -> bool {
        matches!(
            self,
            Self::PointerAxis(PointerAxis::DeltaX | PointerAxis::DeltaY | PointerAxis::WheelX | PointerAxis::WheelY)
        )
    }

    /// The kind of device this control belongs to.
    #[must_use]
    pub const fn device_kind(self) -> DeviceKind {
        match self {
            Self::Key(_) => DeviceKind::Keyboard,
            Self::PointerButton(_) | Self::PointerAxis(_) => DeviceKind::Pointer,
            Self::GamepadButton(_) | Self::GamepadAxis(_) => DeviceKind::Gamepad,
            Self::TouchContact(_) | Self::TouchAxis { .. } => DeviceKind::Touch,
        }
    }

    /// The class part of the text form (`key`, `gamepadButton`, ...).
    #[must_use]
    pub const fn class(self) -> &'static str {
        match self {
            Self::Key(_) => "key",
            Self::PointerButton(_) => "pointerButton",
            Self::PointerAxis(_) => "pointerAxis",
            Self::GamepadButton(_) => "gamepadButton",
            Self::GamepadAxis(_) => "gamepadAxis",
            Self::TouchContact(_) => "touch",
            Self::TouchAxis { .. } => "touchAxis",
        }
    }
}

impl fmt::Display for Control {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let class = self.class();
        match self {
            Self::Key(key) => write!(formatter, "{class}:{key}"),
            Self::PointerButton(button) => write!(formatter, "{class}:{button}"),
            Self::PointerAxis(axis) => write!(formatter, "{class}:{axis}"),
            Self::GamepadButton(button) => write!(formatter, "{class}:{button}"),
            Self::GamepadAxis(axis) => write!(formatter, "{class}:{axis}"),
            Self::TouchContact(contact) => write!(formatter, "{class}:{contact}"),
            Self::TouchAxis { contact, axis } => write!(formatter, "{class}:{contact}:{axis}"),
        }
    }
}

/// Why a control's text form didn't parse.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ControlParseError {
    text: String,
    problem: &'static str,
}

impl fmt::Display for ControlParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unknown control {:?}: {} (a control is <class>:<name>, such as key:KeyA, gamepadButton:South, or gamepadAxis:LeftStickX)",
            self.text, self.problem
        )
    }
}

impl std::error::Error for ControlParseError {}

impl FromStr for Control {
    type Err = ControlParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let fail = |problem: &'static str| ControlParseError {
            text: text.to_owned(),
            problem,
        };
        let Some((class, name)) = text.split_once(':') else {
            return Err(fail("no class before ':'"));
        };
        let contact = |name: &str| {
            name.parse::<u8>()
                .map_err(|_| fail("a touch contact is a number from 0 to 255"))
        };
        match class {
            "key" => Key::from_name(name).map(Self::Key).ok_or_else(|| fail("no key of that name")),
            "pointerButton" => PointerButton::from_name(name)
                .map(Self::PointerButton)
                .ok_or_else(|| fail("no pointer button of that name")),
            "pointerAxis" => PointerAxis::from_name(name)
                .map(Self::PointerAxis)
                .ok_or_else(|| fail("no pointer axis of that name")),
            "gamepadButton" => GamepadButton::from_name(name)
                .map(Self::GamepadButton)
                .ok_or_else(|| fail("no gamepad button of that name")),
            "gamepadAxis" => GamepadAxis::from_name(name)
                .map(Self::GamepadAxis)
                .ok_or_else(|| fail("no gamepad axis of that name")),
            "touch" => contact(name).map(Self::TouchContact),
            "touchAxis" => {
                let Some((contact_text, axis)) = name.split_once(':') else {
                    return Err(fail("a touch axis is touchAxis:<contact>:<X|Y|Pressure>"));
                };
                let contact = contact(contact_text)?;
                let axis = TouchAxis::from_name(axis).ok_or_else(|| fail("no touch axis of that name"))?;
                Ok(Self::TouchAxis { contact, axis })
            }
            _ => Err(fail("no control class of that name")),
        }
    }
}

impl serde::Serialize for Control {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Control {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_round_trips_through_its_text_form() {
        let mut controls: Vec<Control> = Vec::new();
        controls.extend(Key::ALL.iter().map(|&key| Control::Key(key)));
        controls.extend(PointerButton::ALL.iter().map(|&button| Control::PointerButton(button)));
        controls.extend(PointerAxis::ALL.iter().map(|&axis| Control::PointerAxis(axis)));
        controls.extend(GamepadButton::ALL.iter().map(|&button| Control::GamepadButton(button)));
        controls.extend(GamepadAxis::ALL.iter().map(|&axis| Control::GamepadAxis(axis)));
        controls.push(Control::TouchContact(3));
        controls.push(Control::TouchAxis {
            contact: 1,
            axis: TouchAxis::Pressure,
        });
        for control in controls {
            let text = control.to_string();
            assert_eq!(text.parse::<Control>().unwrap(), control, "{text}");
            let json = serde_json::to_string(&control).unwrap();
            assert_eq!(json, format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<Control>(&json).unwrap(), control);
        }
    }

    #[test]
    fn unknown_controls_say_what_a_control_looks_like() {
        let error = "key:KeyÄ".parse::<Control>().unwrap_err().to_string();
        assert!(error.contains("no key of that name"), "{error}");
        assert!(error.contains("key:KeyA"), "{error}");
        let error = "South".parse::<Control>().unwrap_err().to_string();
        assert!(error.contains("no class before ':'"), "{error}");
        let error = "touchAxis:1".parse::<Control>().unwrap_err().to_string();
        assert!(error.contains("touchAxis:<contact>:<X|Y|Pressure>"), "{error}");
    }

    #[test]
    fn kinds_and_ranges_follow_the_control() {
        assert_eq!(Control::Key(Key::KeyA).kind(), ControlKind::Digital);
        assert_eq!(
            Control::GamepadAxis(GamepadAxis::LeftStickY).kind(),
            ControlKind::Axis(AxisRange::Centered)
        );
        assert_eq!(
            Control::GamepadAxis(GamepadAxis::RightTrigger).kind(),
            ControlKind::Axis(AxisRange::Unipolar)
        );
        assert_eq!(
            Control::PointerAxis(PointerAxis::WheelY).kind(),
            ControlKind::Axis(AxisRange::Unbounded)
        );
        assert_eq!(Control::TouchContact(0).device_kind(), DeviceKind::Touch);
        assert_eq!(Control::PointerButton(PointerButton::Back).device_kind(), DeviceKind::Pointer);
    }
}
