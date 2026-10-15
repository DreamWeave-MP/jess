//! The input side of a host: a backend polled into the domain's [`InputService`], one tick per
//! frame at the host's clock, and the tick's action events as the input phase's payload.
//!
//! Turning them into intents is the script's (`@jess/input`'s router).
//!
//! The service is the domain's ([`crate::host::SharedInput`]): `jess.input.runtime()` under the domain
//! hands scripts the same one, so what a script installs is what the host ticks, and what the
//! host ticks is what a script reads after the phase. Nothing here runs a callback, and nothing
//! enters Luau but the scheduler: a tick that produced no action event produces no phase.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::input::{
    ActionEventKind, Backend, BackendError, InputService, MonotonicClock, StubBackend, Tick, Timestamp, Value,
};
use crate::runtime::Message;

use crate::host::SharedInput;

/// Where a host's device events come from.
pub enum InputSource {
    /// A backend the host polls every frame, at the monotonic clock: real devices.
    Backend(Box<dyn Backend>),
    /// No backend: only the service's synthetic side, whose clock the host's frames tick at.
    /// Tests, replays, and headless runs.
    ///
    /// The frames read that clock and never move it: whoever injects the events moves it with
    /// `InputService::advance`. A replay keeps its recorded timing whatever frame cadence plays
    /// it back, and a tap or a hold is decided by input time, not by how many frames ran.
    Synthetic,
}

impl fmt::Debug for InputSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputSource::Backend(backend) => f.debug_tuple("Backend").field(&backend.name()).finish(),
            InputSource::Synthetic => f.write_str("Synthetic"),
        }
    }
}

impl InputSource {
    /// The platform's backend for this build: evdev on Linux with `linux-evdev`, gilrs on macOS
    /// and Windows with `gilrs`, and the stub (no devices, no events) otherwise.
    ///
    /// # Errors
    ///
    /// The platform's device layer could not be opened.
    pub fn platform() -> Result<InputSource, BackendError> {
        crate::input::platform_backend().map(InputSource::Backend)
    }

    /// The stub backend: a real poll loop over no devices, for a host that wants the backend
    /// path exercised without hardware.
    #[must_use]
    pub fn stub() -> InputSource {
        InputSource::Backend(Box::new(StubBackend))
    }
}

/// The host's clock for input: real time, or the synthetic side's.
enum Clock {
    Monotonic(MonotonicClock),
    Synthetic,
}

/// The input service and what feeds it.
pub(crate) struct Driver {
    service: Arc<SharedInput>,
    backend: Option<Box<dyn Backend>>,
    clock: Clock,
}

/// A poisoned lock still holds a whole service; what panicked was elsewhere.
fn lock(service: &SharedInput) -> MutexGuard<'_, InputService> {
    service.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Driver {
    pub(crate) fn new(source: InputSource) -> Driver {
        let (backend, clock) = match source {
            InputSource::Backend(backend) => (Some(backend), Clock::Monotonic(MonotonicClock::new())),
            InputSource::Synthetic => (None, Clock::Synthetic),
        };
        Driver {
            service: Arc::new(Mutex::new(InputService::new())),
            backend,
            clock,
        }
    }

    pub(crate) fn service(&self) -> &Arc<SharedInput> {
        &self.service
    }

    pub(crate) fn source_name(&self) -> &'static str {
        self.backend.as_ref().map_or("synthetic", |backend| backend.name())
    }

    /// Polls the backend, ticks the service, and returns the phase payload if the tick produced
    /// action events.
    pub(crate) fn tick(&mut self) -> Result<Option<Message>, BackendError> {
        let mut service = lock(&self.service);
        let now = match self.clock {
            Clock::Monotonic(clock) => clock.now(),
            Clock::Synthetic => service.now(),
        };
        if let Some(backend) = &mut self.backend {
            backend.poll(&service.sender(), now)?;
        }
        let (generation, events) = {
            let tick = service.tick(now);
            (tick.generation, tick.events.to_vec())
        };
        if events.is_empty() {
            return Ok(None);
        }
        let tick = Tick {
            now,
            generation,
            events: &events,
        };
        let payload = payload(&service, &tick);
        drop(service);
        Ok(Some(payload))
    }

    /// Forgets what a script generation configured.
    pub(crate) fn reset(&self) {
        lock(&self.service).reset();
    }
}

fn seconds(timestamp: Timestamp) -> f64 {
    std::time::Duration::from_nanos(timestamp.nanos()).as_secs_f64()
}

const fn kind_name(kind: ActionEventKind) -> &'static str {
    match kind {
        ActionEventKind::Pressed => "pressed",
        ActionEventKind::Released => "released",
        ActionEventKind::Changed => "changed",
    }
}

fn value_message(value: Value) -> Message {
    match value {
        Value::Digital(on) => Message::Boolean(on),
        Value::Scalar(number) => Message::Number(f64::from(number)),
        Value::Vector2([x, y]) => Message::Vector([x, y, 0.0]),
    }
}

/// One event row: `{ action, kind, value, time, device? }`, the row `InputRuntime:tick()` returns
/// to a script.
fn row(
    service: &InputService,
    action: crate::input::ActionId,
    kind: ActionEventKind,
    value: Value,
    time: Timestamp,
    device: Option<crate::input::DeviceId>,
) -> Message {
    let mut fields: Vec<(&str, Message)> = Vec::with_capacity(5);
    fields.push(("action", Message::text(service.action_name(action).unwrap_or_default())));
    fields.push(("kind", Message::text(kind_name(kind))));
    fields.push(("value", value_message(value)));
    fields.push(("time", Message::Number(seconds(time))));
    if let Some(device) = device {
        fields.push(("device", Message::Number(f64::from(device.index()))));
    }
    Message::map(fields)
}

/// The input phase's payload for one tick:
///
/// ```text
/// {
///   time = <seconds>,             -- the tick's time on the host's clock
///   generation = <number>,        -- the graph generation the events came from
///   events = { { action, kind, value, time, device? }, ... },
/// }
/// ```
///
/// The rows are the ones `InputRuntime:tick()` returns, so a script reads a phase payload and a
/// headless tick the same way.
#[must_use]
pub fn payload(service: &InputService, tick: &Tick<'_>) -> Message {
    let events = tick
        .events
        .iter()
        .map(|event| row(service, event.action, event.kind, event.value, event.timestamp, event.device))
        .collect();
    #[allow(clippy::cast_precision_loss)] // Generations are counted in small numbers.
    let generation = tick.generation.value() as f64;
    Message::map([
        ("time", Message::Number(seconds(tick.now))),
        ("generation", Message::Number(generation)),
        ("events", Message::List(events)),
    ])
}
