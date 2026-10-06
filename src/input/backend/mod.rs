//! Device backends: where real events come from.
//!
//! A [`Backend`] translates one platform's devices into Jess's vocabulary and pushes them through
//! an [`EventSender`]. That is all it does: it names no actions, decides no meanings, and never
//! runs a callback. Nothing a backend is made of appears in its public face, so the choice of
//! crate behind one is free to change.
//!
//! Which backend exists depends on the target and on features, all off by default:
//!
//! | Target | Feature | Backend | Devices |
//! | --- | --- | --- | --- |
//! | Linux | `linux-evdev` | `/dev/input` through evdev | keyboards, pointers, gamepads |
//! | macOS, Windows | `gilrs` | gilrs | gamepads |
//! | anything else, or no feature | | [`StubBackend`] | none |
//!
//! Linux reads evdev directly rather than through gilrs, which would pull libudev and pkg-config
//! into every build. On macOS and Windows the window system owns keyboards and pointers; whoever
//! owns the window pushes those through the same [`EventSender`], and the two streams meet in
//! one queue.
//!
//! A backend is polled: the host calls [`Backend::poll`] from wherever it likes, as often as it
//! likes, and the backend pushes whatever arrived since the last call. A host that wants a
//! dedicated input thread runs the poll loop on one; senders are `Send`.

use std::fmt;

use crate::input::{EventSender, Timestamp};

#[cfg(all(feature = "linux-evdev", target_os = "linux"))]
mod evdev;
#[cfg(all(feature = "gilrs", any(target_os = "macos", target_os = "windows")))]
mod gilrs;

/// Why a backend couldn't start or couldn't poll.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BackendError {
    backend: &'static str,
    message: String,
}

impl BackendError {
    #[allow(dead_code)] // Only the compiled-in backends make one.
    pub(crate) fn new(backend: &'static str, message: impl Into<String>) -> Self {
        Self {
            backend,
            message: message.into(),
        }
    }

    /// The backend that failed.
    #[must_use]
    pub const fn backend(&self) -> &'static str {
        self.backend
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the {} input backend failed: {}", self.backend, self.message)
    }
}

impl std::error::Error for BackendError {}

/// A source of real device events.
pub trait Backend: fmt::Debug {
    /// The backend's name, for diagnostics: `evdev`, `gilrs`, `stub`.
    fn name(&self) -> &'static str;

    /// Pushes everything that happened since the last poll through `sender`, stamped from `now`:
    /// connects and disconnects it noticed, and every button and axis change, in the order the
    /// platform reported them. Never blocks.
    ///
    /// # Errors
    ///
    /// The platform's device layer became unusable as a whole. One device failing is a
    /// disconnect, not an error.
    fn poll(&mut self, sender: &EventSender, now: Timestamp) -> Result<(), BackendError>;
}

/// The backend for targets and builds that have no other: no devices, no events, no failure.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubBackend;

impl Backend for StubBackend {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn poll(&mut self, _sender: &EventSender, _now: Timestamp) -> Result<(), BackendError> {
        Ok(())
    }
}

/// The backend this build has for this target: evdev on Linux with `linux-evdev`, gilrs on macOS
/// and Windows with `gilrs`, and [`StubBackend`] otherwise.
///
/// # Errors
///
/// The platform's device layer couldn't be opened.
pub fn platform_backend() -> Result<Box<dyn Backend>, BackendError> {
    #[cfg(all(feature = "linux-evdev", target_os = "linux"))]
    {
        Ok(Box::new(evdev::EvdevBackend::new()))
    }
    #[cfg(all(feature = "gilrs", any(target_os = "macos", target_os = "windows")))]
    {
        Ok(Box::new(gilrs::GilrsBackend::new()?))
    }
    #[cfg(not(any(
        all(feature = "linux-evdev", target_os = "linux"),
        all(feature = "gilrs", any(target_os = "macos", target_os = "windows"))
    )))]
    {
        Ok(Box::new(StubBackend))
    }
}
