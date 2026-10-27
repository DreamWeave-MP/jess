//! Output devices: an [`Engine`](crate::audio::Engine) rendered by the platform's audio thread.
//!
//! The engine is the same one an [`OfflineContext`](crate::audio::OfflineContext) renders; a
//! device only calls [`Engine::render`](crate::audio::Engine::render) when its hardware wants
//! frames. Opening one needs the `audio-device` feature, which builds miniaudio's device layer
//! (`vendor/miniaudio`) and is on by default; without it, [`DeviceRequest::open`] says so.

#[cfg(feature = "audio-device")]
mod miniaudio;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::audio::{AudioConfig, AudioError, Control};

/// What to open: the platform's default output, or the null device, which plays to nowhere in
/// real time (for tests, and machines with no sound).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DeviceRequest {
    /// Frames per second, or the device's own.
    pub rate: Option<u32>,
    /// Frames the device asks for at a time, as a hint; the device decides.
    pub period: Option<u32>,
    /// The null device rather than the default output.
    pub null: bool,
}

/// What a device is, once open.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceInfo {
    /// The platform API it plays through (`PulseAudio`, `WASAPI`, `Core Audio`, `Null`).
    pub backend: String,
    /// Frames per second.
    pub rate: u32,
    /// Output channels.
    pub channels: usize,
    /// Frames it asks for at a time.
    pub period: u32,
    /// Periods it buffers.
    pub periods: u32,
}

impl DeviceInfo {
    /// How far the device's output lags what the engine has rendered, about: the frames it buffers.
    #[must_use]
    pub fn latency(&self) -> u64 {
        u64::from(self.period) * u64::from(self.periods.max(1))
    }
}

/// What has happened to a device, counted as it happens on the platform's threads.
#[derive(Debug, Default)]
pub struct DeviceEvents {
    pub(crate) stopped: AtomicU64,
    pub(crate) rerouted: AtomicU64,
    pub(crate) interrupted: AtomicU64,
    /// The engine panicked on the audio thread; the device has played silence since.
    pub(crate) failed: AtomicBool,
}

/// What a device has been through.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DeviceStats {
    /// Times it stopped: by request, or because the device went away.
    pub stopped: u64,
    /// Times it moved to another device, the default having changed.
    pub rerouted: u64,
    /// Times the system interrupted it (a call, another app taking the device).
    pub interrupted: u64,
    /// The engine failed on the audio thread and the device plays silence.
    pub failed: bool,
}

impl DeviceEvents {
    fn stats(&self) -> DeviceStats {
        DeviceStats {
            stopped: self.stopped.load(Ordering::Relaxed),
            rerouted: self.rerouted.load(Ordering::Relaxed),
            interrupted: self.interrupted.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
        }
    }
}

/// A context a device renders: a [`Control`] here, its engine on the device's thread. Closing
/// or dropping it stops the device and waits for that thread before the engine goes.
#[derive(Debug)]
pub struct LiveContext {
    control: Control,
    info: DeviceInfo,
    events: Arc<DeviceEvents>,
    /// `None` once closed.
    #[cfg(feature = "audio-device")]
    device: Option<miniaudio::Device>,
}

impl LiveContext {
    /// Stops the device and drops the engine with every voice it held. Commands sent after it
    /// fail with [`AudioError::Disconnected`]; what the control side knows (the clock, the
    /// buses) stays readable, frozen where the device left it.
    pub fn close(&mut self) {
        #[cfg(feature = "audio-device")]
        self.device.take();
    }

    /// True once [`LiveContext::close`] has run.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        #[cfg(feature = "audio-device")]
        return self.device.is_none();
        #[cfg(not(feature = "audio-device"))]
        true
    }

    /// The control side, to play and schedule on.
    pub const fn control(&mut self) -> &mut Control {
        &mut self.control
    }

    /// What the device is.
    #[must_use]
    pub const fn info(&self) -> &DeviceInfo {
        &self.info
    }

    /// What the device has been through.
    #[must_use]
    pub fn stats(&self) -> DeviceStats {
        self.events.stats()
    }
}

impl DeviceRequest {
    /// Opens the device and starts it rendering a context made from `config`, at the device's
    /// rate and channels (`config`'s rate is ignored unless asked for).
    ///
    /// # Errors
    ///
    /// [`AudioError::Device`] when there is no device to open, or this jess was built without
    /// the `audio-device` feature; and anything [`context`](crate::audio::context) refuses.
    pub fn open(&self, config: &AudioConfig) -> Result<LiveContext, AudioError> {
        #[cfg(feature = "audio-device")]
        return miniaudio::open(self, config);
        #[cfg(not(feature = "audio-device"))]
        {
            let _ = (self, config);
            Err(AudioError::Device(
                "this jess was built without an audio device: build it with the audio-device feature".to_owned(),
            ))
        }
    }
}

#[cfg(all(test, feature = "audio-device"))]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::audio::{Pcm, PlaySpec, VoiceState};

    #[test]
    fn the_null_device_renders_the_engine_in_real_time_and_closes_cleanly() {
        let request = DeviceRequest {
            null: true,
            ..DeviceRequest::default()
        };
        let mut live = request.open(&AudioConfig::default()).unwrap();
        assert_eq!(live.info().backend, "Null");
        assert!(live.info().rate > 0 && live.info().channels == 2);
        let rate = live.info().rate;
        let click = Arc::new(Pcm::from_planar(&[&[1.0; 64]], 48_000).unwrap());
        let voice = live.control().play(&click, &PlaySpec::default()).unwrap();
        let started = Instant::now();
        // The device's thread renders as time passes; nothing here drives it.
        while live.control().now() < u64::from(rate) / 10 {
            assert!(started.elapsed() < Duration::from_secs(10), "the device never rendered");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(live.control().state(voice), VoiceState::Ended);
        assert!(!live.stats().failed);
        assert!(!live.is_closed());
        live.close();
        assert!(live.is_closed());
        let now = live.control().now();
        assert_eq!(
            live.control().play(&click, &PlaySpec::default()),
            Err(AudioError::Disconnected)
        );
        assert_eq!(live.control().now(), now, "a closed output's clock stands still");
        drop(live);
    }
}
