//! Current device state: every event so far, applied in order.

use std::collections::HashMap;

use crate::input::{Control, DeviceDescriptor, DeviceId, DeviceKind, InputEvent, InputEventKind, Timestamp};

/// A button's current state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ButtonState {
    /// Whether the button is held.
    pub pressed: bool,
    /// When it last changed.
    pub since: Timestamp,
}

/// An axis's current state.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AxisState {
    /// The current value, normalized to the control's range.
    pub value: f32,
    /// When it last changed.
    pub since: Timestamp,
}

/// One device's current state.
#[derive(Clone, Debug)]
pub struct DeviceState {
    id: DeviceId,
    descriptor: DeviceDescriptor,
    connected: bool,
    buttons: HashMap<Control, ButtonState>,
    axes: HashMap<Control, AxisState>,
}

impl DeviceState {
    /// The device's id.
    #[must_use]
    pub const fn id(&self) -> DeviceId {
        self.id
    }

    /// What the backend said about the device when it last connected.
    #[must_use]
    pub const fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    /// Whether the device is connected now. A disconnected device keeps its descriptor, so a
    /// diagnostic can still name it, and holds nothing.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    /// A button's state, if the device has ever reported it.
    #[must_use]
    pub fn button(&self, control: Control) -> Option<ButtonState> {
        self.buttons.get(&control).copied()
    }

    /// Whether a button is held.
    #[must_use]
    pub fn is_pressed(&self, control: Control) -> bool {
        self.buttons.get(&control).is_some_and(|button| button.pressed)
    }

    /// An axis's state, if the device has ever reported it.
    #[must_use]
    pub fn axis(&self, control: Control) -> Option<AxisState> {
        self.axes.get(&control).copied()
    }

    /// An axis's value, zero when the device hasn't reported it.
    #[must_use]
    pub fn axis_value(&self, control: Control) -> f32 {
        self.axes.get(&control).map_or(0.0, |axis| axis.value)
    }

    /// The buttons held now, in no particular order.
    pub fn pressed(&self) -> impl Iterator<Item = Control> + '_ {
        self.buttons
            .iter()
            .filter(|(_, button)| button.pressed)
            .map(|(control, _)| *control)
    }
}

/// The current state of every device a runtime has seen.
///
/// State is what the events so far add up to: [`InputState::apply`] is the only way it changes,
/// and applying the same events in the same order always gives the same state. Reading it takes
/// the `input.observe` capability once a script context enforces one; action state doesn't.
#[derive(Clone, Debug)]
pub struct InputState {
    /// Indexed by `DeviceId`; ids a queue allocated but this state never saw connect are `None`.
    devices: Vec<Option<DeviceState>>,
    focused: bool,
    latest: Timestamp,
}

impl InputState {
    /// A state with no devices, focused.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            devices: Vec::new(),
            focused: true,
            latest: Timestamp::ZERO,
        }
    }

    /// Applies one event.
    ///
    /// A button or axis event for a device that isn't connected is ignored: a backend raced its
    /// own disconnect, and a gone device holds nothing. An axis event for a digital control, the
    /// reverse, or a value that isn't finite is a backend bug and is ignored the same way.
    /// Relative axes ([`Control::is_relative`]) add up until [`InputState::end_tick`].
    pub fn apply(&mut self, event: &InputEvent) {
        self.latest = self.latest.max(event.timestamp);
        match &event.kind {
            InputEventKind::Connected { device, descriptor } => {
                let position = device.position();
                if self.devices.len() <= position {
                    self.devices.resize_with(position + 1, || None);
                }
                match &mut self.devices[position] {
                    Some(state) => {
                        state.descriptor = (**descriptor).clone();
                        state.connected = true;
                    }
                    slot @ None => {
                        *slot = Some(DeviceState {
                            id: *device,
                            descriptor: (**descriptor).clone(),
                            connected: true,
                            buttons: HashMap::new(),
                            axes: HashMap::new(),
                        });
                    }
                }
            }
            InputEventKind::Disconnected { device } => {
                if let Some(state) = self.device_mut(*device) {
                    state.connected = false;
                    for button in state.buttons.values_mut() {
                        if button.pressed {
                            *button = ButtonState {
                                pressed: false,
                                since: event.timestamp,
                            };
                        }
                    }
                    for axis in state.axes.values_mut() {
                        if axis.value.to_bits() != 0.0f32.to_bits() {
                            *axis = AxisState {
                                value: 0.0,
                                since: event.timestamp,
                            };
                        }
                    }
                }
            }
            InputEventKind::Button {
                device,
                control,
                pressed,
            } => {
                if !control.is_digital() {
                    return;
                }
                let Some(state) = self.device_mut(*device).filter(|state| state.connected) else {
                    return;
                };
                let button = state.buttons.entry(*control).or_insert(ButtonState {
                    pressed: false,
                    since: event.timestamp,
                });
                if button.pressed != *pressed {
                    *button = ButtonState {
                        pressed: *pressed,
                        since: event.timestamp,
                    };
                }
            }
            InputEventKind::Axis { device, control, value } => {
                if control.is_digital() || !value.is_finite() {
                    return;
                }
                let Some(state) = self.device_mut(*device).filter(|state| state.connected) else {
                    return;
                };
                let axis = state.axes.entry(*control).or_insert(AxisState {
                    value: 0.0,
                    since: event.timestamp,
                });
                *axis = AxisState {
                    value: if control.is_relative() { axis.value + *value } else { *value },
                    since: event.timestamp,
                };
            }
            InputEventKind::Focus { focused } => self.focused = *focused,
        }
    }

    /// Ends a tick: zeroes every relative axis ([`Control::is_relative`]), whose events add up
    /// between calls. The runtime calls this after it evaluates; a host driving state by hand
    /// calls it once per tick, after reading.
    pub fn end_tick(&mut self) {
        for device in self.devices.iter_mut().flatten() {
            for (control, axis) in &mut device.axes {
                if control.is_relative() {
                    axis.value = 0.0;
                }
            }
        }
    }

    fn device_mut(&mut self, device: DeviceId) -> Option<&mut DeviceState> {
        self.devices.get_mut(device.position()).and_then(Option::as_mut)
    }

    /// A device's state, connected or not, if this state ever saw it connect.
    #[must_use]
    pub fn device(&self, device: DeviceId) -> Option<&DeviceState> {
        self.devices.get(device.position()).and_then(Option::as_ref)
    }

    /// The connected devices, in id order.
    pub fn devices(&self) -> impl Iterator<Item = &DeviceState> + '_ {
        self.devices.iter().flatten().filter(|device| device.connected)
    }

    /// The connected devices of one kind, in id order.
    pub fn devices_of(&self, kind: DeviceKind) -> impl Iterator<Item = &DeviceState> + '_ {
        self.devices().filter(move |device| device.descriptor.kind == kind)
    }

    /// Whether the application has input focus.
    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    /// The latest timestamp any applied event carried.
    #[must_use]
    pub const fn latest(&self) -> Timestamp {
        self.latest
    }

    /// A button on any connected device: the device that pressed it most recently, else the one
    /// that released it most recently, else nothing.
    #[must_use]
    pub fn button_on_any(&self, control: Control) -> Option<(DeviceId, ButtonState)> {
        let mut found: Option<(DeviceId, ButtonState)> = None;
        for device in self.devices() {
            let Some(button) = device.button(control) else {
                continue;
            };
            let better = found.is_none_or(|(_, best)| (button.pressed, button.since) > (best.pressed, best.since));
            if better {
                found = Some((device.id, button));
            }
        }
        found
    }

    /// An axis on any connected device: the value furthest from zero, and the device holding it.
    #[must_use]
    pub fn axis_on_any(&self, control: Control) -> Option<(DeviceId, AxisState)> {
        let mut found: Option<(DeviceId, AxisState)> = None;
        for device in self.devices() {
            let Some(axis) = device.axis(control) else {
                continue;
            };
            if found.is_none_or(|(_, best)| axis.value.abs() > best.value.abs()) {
                found = Some((device.id, axis));
            }
        }
        found
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}
