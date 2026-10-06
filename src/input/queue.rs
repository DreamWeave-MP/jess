//! The event queue: the only way device events reach a runtime.
//!
//! A backend, on whatever thread it polls, pushes normalized events through an [`EventSender`].
//! The runtime drains the [`InputQueue`] at its own scheduler point and nothing else happens in
//! between: no callback runs on the backend's thread, and no Luau is ever entered from one. A
//! future script context drains the same queue the same way, in its input phase.
//!
//! The queue also owns the device registry, because the two must agree: a backend gets a
//! [`DeviceId`] when it announces a device ([`EventSender::connect`]), and every event it pushes
//! afterwards names that id.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::input::{Control, DeviceDescriptor, DeviceId, DeviceIdentity, InputEvent, InputEventKind, SyntheticSource, Timestamp};

#[derive(Debug, Default)]
struct Shared {
    events: Mutex<VecDeque<InputEvent>>,
    /// Indexed by `DeviceId`: every identity ever announced, in the order it first connected.
    identities: Mutex<Vec<DeviceIdentity>>,
}

/// A poisoned lock's data is still a queue of events or a list of identities; a backend thread
/// that panicked mid-push left them whole.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The receiving end: drained by the runtime, on the runtime's thread.
///
/// Cloning a queue gives another handle to the same queue and registry.
#[derive(Clone, Debug, Default)]
pub struct InputQueue {
    shared: Arc<Shared>,
}

impl InputQueue {
    /// An empty queue with an empty device registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A sender for a backend. Senders are `Send` and cheap to clone.
    #[must_use]
    pub fn sender(&self) -> EventSender {
        EventSender {
            shared: Arc::clone(&self.shared),
        }
    }

    /// A synthetic source pushing into this queue, its clock at zero.
    #[must_use]
    pub fn synthetic(&self) -> SyntheticSource {
        SyntheticSource::new(self.sender())
    }

    /// Moves every queued event onto the end of `into`, in the order they were pushed.
    ///
    /// `into` keeps its capacity between ticks, so a steady stream of events allocates nothing
    /// here once the buffer has grown to fit a tick's worth.
    pub fn drain(&self, into: &mut Vec<InputEvent>) {
        let mut events = lock(&self.shared.events);
        into.extend(events.drain(..));
        drop(events);
    }

    /// How many events are queued.
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.shared.events).len()
    }

    /// Whether no events are queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The id the registry gave `identity`, if it has ever connected.
    #[must_use]
    pub fn device_id(&self, identity: &DeviceIdentity) -> Option<DeviceId> {
        lock(&self.shared.identities)
            .iter()
            .position(|known| known == identity)
            .map(DeviceId::from_position)
    }

    /// The identity a device connected with.
    #[must_use]
    pub fn identity(&self, device: DeviceId) -> Option<DeviceIdentity> {
        lock(&self.shared.identities).get(device.position()).cloned()
    }

    /// The id numbered `index`, if the registry has allocated it: the way a script's device
    /// number becomes an id, from the moment the device was announced, before any tick applied
    /// its connect event.
    #[must_use]
    pub fn device(&self, index: u32) -> Option<DeviceId> {
        let position = index as usize;
        (position < lock(&self.shared.identities).len()).then(|| DeviceId::from_position(position))
    }
}

/// The sending end: what a backend pushes events through, from any thread.
#[derive(Clone, Debug)]
pub struct EventSender {
    shared: Arc<Shared>,
}

impl EventSender {
    /// Announces a device and returns its id: the id `identity` had before if it has connected
    /// to this queue before, a new one otherwise. Queues a [`InputEventKind::Connected`] event.
    #[must_use = "the id is how every later event names the device"]
    pub fn connect(&self, identity: &DeviceIdentity, descriptor: DeviceDescriptor, at: Timestamp) -> DeviceId {
        let mut identities = lock(&self.shared.identities);
        let position = identities.iter().position(|known| known == identity).unwrap_or_else(|| {
            identities.push(identity.clone());
            identities.len() - 1
        });
        drop(identities);
        let device = DeviceId::from_position(position);
        self.push(InputEvent {
            timestamp: at,
            kind: InputEventKind::Connected {
                device,
                descriptor: Box::new(descriptor),
            },
        });
        device
    }

    /// Queues a device's disconnect.
    pub fn disconnect(&self, device: DeviceId, at: Timestamp) {
        self.push(InputEvent {
            timestamp: at,
            kind: InputEventKind::Disconnected { device },
        });
    }

    /// Queues a button press or release.
    pub fn button(&self, device: DeviceId, control: Control, pressed: bool, at: Timestamp) {
        self.push(InputEvent {
            timestamp: at,
            kind: InputEventKind::Button {
                device,
                control,
                pressed,
            },
        });
    }

    /// Queues an axis value, already normalized to the control's range.
    pub fn axis(&self, device: DeviceId, control: Control, value: f32, at: Timestamp) {
        self.push(InputEvent {
            timestamp: at,
            kind: InputEventKind::Axis { device, control, value },
        });
    }

    /// Queues a focus change.
    pub fn focus(&self, focused: bool, at: Timestamp) {
        self.push(InputEvent {
            timestamp: at,
            kind: InputEventKind::Focus { focused },
        });
    }

    /// Queues an event as it is.
    pub fn push(&self, event: InputEvent) {
        lock(&self.shared.events).push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reconnecting_identity_keeps_its_id() {
        let queue = InputQueue::new();
        let sender = queue.sender();
        let pad = DeviceIdentity::new("test:pad-a");
        let other = DeviceIdentity::new("test:pad-b");
        let first = sender.connect(&pad, DeviceDescriptor::gamepad("Pad A", None, None), Timestamp::ZERO);
        let second = sender.connect(&other, DeviceDescriptor::gamepad("Pad B", None, None), Timestamp::ZERO);
        sender.disconnect(first, Timestamp::from_millis(10));
        let again = sender.connect(
            &pad,
            DeviceDescriptor::gamepad("Pad A", None, None),
            Timestamp::from_millis(20),
        );
        assert_ne!(first, second);
        assert_eq!(first, again);
        assert_eq!(queue.device_id(&pad), Some(first));
        assert_eq!(queue.identity(second), Some(other));
        assert_eq!(queue.len(), 4);
    }

    #[test]
    fn events_cross_threads_and_drain_in_push_order() {
        let queue = InputQueue::new();
        let sender = queue.sender();
        let worker = std::thread::spawn(move || {
            let keyboard = sender.connect(
                &DeviceIdentity::new("test:keyboard"),
                DeviceDescriptor::keyboard("Keyboard"),
                Timestamp::ZERO,
            );
            for step in 0..4u64 {
                sender.button(
                    keyboard,
                    Control::Key(crate::input::Key::KeyA),
                    step % 2 == 0,
                    Timestamp::from_millis(step),
                );
            }
        });
        worker.join().unwrap();
        let mut events = Vec::new();
        queue.drain(&mut events);
        assert!(queue.is_empty());
        let stamps: Vec<u64> = events.iter().skip(1).map(|event| event.timestamp.nanos()).collect();
        assert_eq!(stamps, [0, 1_000_000, 2_000_000, 3_000_000]);
    }
}
