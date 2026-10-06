//! Synthetic input: devices and events that exist because a test, a replay, or a script says so.

use std::time::Duration;

use crate::input::{Control, DeviceDescriptor, DeviceId, DeviceIdentity, EventSender, Timestamp};

/// An input source with no hardware behind it and a clock that moves only when told to.
///
/// A synthetic source pushes through an ordinary [`EventSender`], so nothing downstream can tell
/// its events from a backend's. Its time starts at zero and advances only through
/// [`SyntheticSource::advance`], which is what makes tap and hold timing deterministic: the same
/// calls always produce the same events at the same timestamps.
///
/// ```
/// use std::time::Duration;
/// use jess::input::{Control, InputQueue, InputState, Key};
///
/// let queue = InputQueue::new();
/// let mut source = queue.synthetic();
/// let keyboard = source.connect_keyboard();
/// source.press(keyboard, Control::Key(Key::ControlLeft));
/// source.advance(Duration::from_millis(40));
/// source.press(keyboard, Control::Key(Key::KeyS));
///
/// let (mut events, mut state) = (Vec::new(), InputState::new());
/// queue.drain(&mut events);
/// for event in &events {
///     state.apply(event);
/// }
/// assert!(state.device(keyboard).unwrap().is_pressed(Control::Key(Key::KeyS)));
/// assert_eq!(events[2].timestamp.nanos(), 40_000_000);
/// ```
#[derive(Clone, Debug)]
pub struct SyntheticSource {
    sender: EventSender,
    now: Timestamp,
    connected: u32,
}

impl SyntheticSource {
    /// A source pushing through `sender`, its clock at zero.
    #[must_use]
    pub const fn new(sender: EventSender) -> Self {
        Self {
            sender,
            now: Timestamp::ZERO,
            connected: 0,
        }
    }

    /// The source's current time.
    #[must_use]
    pub const fn now(&self) -> Timestamp {
        self.now
    }

    /// Moves the source's clock forward.
    pub fn advance(&mut self, duration: Duration) {
        self.now = self.now + duration;
    }

    /// Connects a device under `identity`. The same identity gets the same id back.
    pub fn connect(&mut self, identity: &DeviceIdentity, descriptor: DeviceDescriptor) -> DeviceId {
        self.sender.connect(identity, descriptor, self.now)
    }

    fn connect_numbered(&mut self, kind: &str, descriptor: DeviceDescriptor) -> DeviceId {
        self.connected += 1;
        let identity = DeviceIdentity::new(format!("synthetic:{kind}:{}", self.connected));
        self.connect(&identity, descriptor)
    }

    /// Connects a new keyboard.
    pub fn connect_keyboard(&mut self) -> DeviceId {
        self.connect_numbered("keyboard", DeviceDescriptor::keyboard("Synthetic Keyboard"))
    }

    /// Connects a new pointer.
    pub fn connect_pointer(&mut self) -> DeviceId {
        self.connect_numbered("pointer", DeviceDescriptor::pointer("Synthetic Pointer"))
    }

    /// Connects a new gamepad with no vendor, so its presentation family is the generic one.
    pub fn connect_gamepad(&mut self) -> DeviceId {
        self.connect_numbered("gamepad", DeviceDescriptor::gamepad("Synthetic Gamepad", None, None))
    }

    /// Connects a new touch surface.
    pub fn connect_touch(&mut self) -> DeviceId {
        self.connect_numbered("touch", DeviceDescriptor::touch("Synthetic Touch"))
    }

    /// Disconnects a device.
    pub fn disconnect(&mut self, device: DeviceId) {
        self.sender.disconnect(device, self.now);
    }

    /// Presses a button.
    pub fn press(&mut self, device: DeviceId, control: Control) {
        self.sender.button(device, control, true, self.now);
    }

    /// Releases a button.
    pub fn release(&mut self, device: DeviceId, control: Control) {
        self.sender.button(device, control, false, self.now);
    }

    /// Moves an axis to `value`, or, for a relative axis, by `value`.
    pub fn axis(&mut self, device: DeviceId, control: Control, value: f32) {
        self.sender.axis(device, control, value, self.now);
    }

    /// Gives or takes input focus.
    pub fn focus(&mut self, focused: bool) {
        self.sender.focus(focused, self.now);
    }
}
