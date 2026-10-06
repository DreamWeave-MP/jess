//! Timestamped device events: the changes, as opposed to [`crate::input::InputState`]'s current values.

use crate::input::{Control, DeviceDescriptor, DeviceId, Timestamp};

/// One change a backend observed, and when.
///
/// Events are ordered by the queue they were pushed to, and carry the monotonic time the backend
/// saw them at. Consumers that need edges read events; consumers that need "is it held" read
/// [`crate::input::InputState`], which is every event so far applied in order.
#[derive(Clone, PartialEq, Debug)]
pub struct InputEvent {
    /// When the backend observed the change, on the runtime's monotonic clock.
    pub timestamp: Timestamp,
    /// What changed.
    pub kind: InputEventKind,
}

/// What changed.
#[derive(Clone, PartialEq, Debug)]
pub enum InputEventKind {
    /// A device connected, or reconnected with the id it had before.
    Connected {
        /// The device.
        device: DeviceId,
        /// What the backend knows about it.
        descriptor: Box<DeviceDescriptor>,
    },
    /// A device disconnected. Everything it held is released: state drops its buttons and
    /// zeroes its axes, so nothing stays pressed on a device that is gone.
    Disconnected {
        /// The device.
        device: DeviceId,
    },
    /// A button was pressed or released.
    Button {
        /// The device.
        device: DeviceId,
        /// The button. Its [`Control::kind`] is digital.
        control: Control,
        /// Whether it is now pressed.
        pressed: bool,
    },
    /// An axis moved.
    Axis {
        /// The device.
        device: DeviceId,
        /// The axis. Its [`Control::kind`] is an axis.
        control: Control,
        /// The new value, normalized to the control's [`crate::input::AxisRange`].
        value: f32,
    },
    /// The application gained or lost input focus. While unfocused, the host decides whether it
    /// still feeds events; the runtime only records the fact.
    Focus {
        /// Whether the application now has focus.
        focused: bool,
    },
}

impl InputEventKind {
    /// The device the event is about, if it is about one.
    #[must_use]
    pub const fn device(&self) -> Option<DeviceId> {
        match self {
            Self::Connected { device, .. }
            | Self::Disconnected { device }
            | Self::Button { device, .. }
            | Self::Axis { device, .. } => Some(*device),
            Self::Focus { .. } => None,
        }
    }
}
