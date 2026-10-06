//! The macOS and Windows gamepad backend, through gilrs.
//!
//! gilrs already names buttons by position and normalizes axes, so this is a translation and
//! little else. Three things it says differently from Jess:
//!
//! - its `LeftTrigger` and `RightTrigger` are the bumpers, and `LeftTrigger2` and `RightTrigger2`
//!   the triggers;
//! - a trigger's pressure is a button's value (`ButtonChanged`), where Jess has a trigger axis;
//! - a D-pad that is a hat arrives as `DPadX`/`DPadY` axes, which become D-pad button events
//!   here, as they do in the evdev backend.
//!
//! Keyboards and pointers aren't gilrs's: on these platforms the window system owns them.

use std::collections::HashMap;

use ::gilrs::{Axis, Button, EventType, GamepadId, Gilrs};

use super::{Backend, BackendError};
use crate::input::{Control, DeviceDescriptor, DeviceId, DeviceIdentity, EventSender, GamepadAxis, GamepadButton, Timestamp};

const fn button(button: Button) -> Option<GamepadButton> {
    Some(match button {
        Button::South => GamepadButton::South,
        Button::East => GamepadButton::East,
        Button::North => GamepadButton::North,
        Button::West => GamepadButton::West,
        Button::LeftTrigger => GamepadButton::LeftBumper,
        Button::RightTrigger => GamepadButton::RightBumper,
        Button::LeftTrigger2 => GamepadButton::LeftTrigger,
        Button::RightTrigger2 => GamepadButton::RightTrigger,
        Button::Select => GamepadButton::Select,
        Button::Start => GamepadButton::Start,
        Button::Mode => GamepadButton::Guide,
        Button::LeftThumb => GamepadButton::LeftStick,
        Button::RightThumb => GamepadButton::RightStick,
        Button::DPadUp => GamepadButton::DPadUp,
        Button::DPadDown => GamepadButton::DPadDown,
        Button::DPadLeft => GamepadButton::DPadLeft,
        Button::DPadRight => GamepadButton::DPadRight,
        Button::C | Button::Z | Button::Unknown => return None,
    })
}

const fn axis(axis: Axis) -> Option<GamepadAxis> {
    Some(match axis {
        Axis::LeftStickX => GamepadAxis::LeftStickX,
        Axis::LeftStickY => GamepadAxis::LeftStickY,
        Axis::RightStickX => GamepadAxis::RightStickX,
        Axis::RightStickY => GamepadAxis::RightStickY,
        Axis::LeftZ => GamepadAxis::LeftTrigger,
        Axis::RightZ => GamepadAxis::RightTrigger,
        Axis::DPadX | Axis::DPadY | Axis::Unknown => return None,
    })
}

/// The D-pad buttons a hat axis holds at `value`: `(negative, positive)` pressed.
fn hat(value: f32) -> (bool, bool) {
    (value < -0.5, value > 0.5)
}

#[derive(Debug)]
struct Pad {
    id: DeviceId,
    /// The hat's last `[x, y]` as `(negative, positive)` pressed.
    hat: [(bool, bool); 2],
}

/// The gilrs backend: every gamepad the platform's controller API reports.
pub struct GilrsBackend {
    gilrs: Gilrs,
    pads: HashMap<GamepadId, Pad>,
    announced: bool,
}

impl std::fmt::Debug for GilrsBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GilrsBackend")
            .field("pads", &self.pads)
            .finish_non_exhaustive()
    }
}

impl GilrsBackend {
    pub fn new() -> Result<Self, BackendError> {
        let gilrs = Gilrs::new().map_err(|error| BackendError::new("gilrs", error.to_string()))?;
        Ok(Self {
            gilrs,
            pads: HashMap::new(),
            announced: false,
        })
    }

    /// Announces a gamepad. Its identity is gilrs's UUID for it, with its slot so two pads of one
    /// model are two devices; gilrs gives a reconnecting pad its old slot back when it can.
    fn connect(&mut self, gamepad: GamepadId, sender: &EventSender, now: Timestamp) {
        let pad = self.gilrs.gamepad(gamepad);
        let uuid = pad.uuid().iter().fold(String::with_capacity(32), |mut text, byte| {
            use std::fmt::Write as _;
            let _ = write!(text, "{byte:02x}");
            text
        });
        let identity = DeviceIdentity::new(format!("gilrs:{uuid}/{}", usize::from(gamepad)));
        let mut descriptor = DeviceDescriptor::gamepad(pad.name(), pad.vendor_id(), pad.product_id());
        descriptor.backend = Some(format!("gilrs:{}", usize::from(gamepad)));
        let id = sender.connect(&identity, descriptor, now);
        self.pads.insert(
            gamepad,
            Pad {
                id,
                hat: [(false, false); 2],
            },
        );
    }
}

impl Backend for GilrsBackend {
    fn name(&self) -> &'static str {
        "gilrs"
    }

    fn poll(&mut self, sender: &EventSender, now: Timestamp) -> Result<(), BackendError> {
        // Pads that were plugged in before gilrs started never get a `Connected` event.
        if !self.announced {
            self.announced = true;
            let present: Vec<GamepadId> = self.gilrs.gamepads().map(|(id, _)| id).collect();
            for gamepad in present {
                self.connect(gamepad, sender, now);
            }
        }
        while let Some(event) = self.gilrs.next_event() {
            match event.event {
                EventType::Connected => self.connect(event.id, sender, now),
                EventType::Disconnected => {
                    if let Some(pad) = self.pads.remove(&event.id) {
                        sender.disconnect(pad.id, now);
                    }
                }
                EventType::ButtonPressed(pressed, _) | EventType::ButtonReleased(pressed, _) => {
                    let (Some(pad), Some(control)) = (self.pads.get(&event.id), button(pressed)) else {
                        continue;
                    };
                    let down = matches!(event.event, EventType::ButtonPressed(..));
                    sender.button(pad.id, Control::GamepadButton(control), down, now);
                }
                EventType::ButtonChanged(changed, value, _) => {
                    let trigger = match changed {
                        Button::LeftTrigger2 => GamepadAxis::LeftTrigger,
                        Button::RightTrigger2 => GamepadAxis::RightTrigger,
                        _ => continue,
                    };
                    if let Some(pad) = self.pads.get(&event.id) {
                        sender.axis(pad.id, Control::GamepadAxis(trigger), value.clamp(0.0, 1.0), now);
                    }
                }
                EventType::AxisChanged(changed, value, _) => {
                    let Some(pad) = self.pads.get_mut(&event.id) else {
                        continue;
                    };
                    if let Some(control) = axis(changed) {
                        sender.axis(pad.id, Control::GamepadAxis(control), value.clamp(-1.0, 1.0), now);
                        continue;
                    }
                    // gilrs's `DPadY` is positive upward.
                    let (slot, negative, positive) = match changed {
                        Axis::DPadX => (0, GamepadButton::DPadLeft, GamepadButton::DPadRight),
                        Axis::DPadY => (1, GamepadButton::DPadDown, GamepadButton::DPadUp),
                        _ => continue,
                    };
                    let held = hat(value);
                    let previous = std::mem::replace(&mut pad.hat[slot], held);
                    for (control, was, is) in [(negative, previous.0, held.0), (positive, previous.1, held.1)] {
                        if was != is {
                            sender.button(pad.id, Control::GamepadButton(control), is, now);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gilrs_names_translate_to_the_jess_vocabulary() {
        assert_eq!(button(Button::LeftTrigger), Some(GamepadButton::LeftBumper));
        assert_eq!(button(Button::LeftTrigger2), Some(GamepadButton::LeftTrigger));
        assert_eq!(button(Button::Mode), Some(GamepadButton::Guide));
        assert_eq!(button(Button::Unknown), None);
        assert_eq!(axis(Axis::LeftZ), Some(GamepadAxis::LeftTrigger));
        assert_eq!(axis(Axis::DPadX), None);
        assert_eq!(hat(-1.0), (true, false));
        assert_eq!(hat(0.0), (false, false));
        assert_eq!(hat(1.0), (false, true));
    }
}
