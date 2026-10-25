//! The device through miniaudio's device layer: `device.c` is the C side, and this the only Rust
//! that touches it.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::audio::backend::{DeviceEvents, DeviceInfo, DeviceRequest, LiveContext};
use crate::audio::{AudioConfig, AudioError, Engine, MAX_CHANNELS};

/// The C side's device: opaque here.
#[repr(C)]
struct Raw {
    _private: [u8; 0],
}

type Render = extern "C" fn(*mut c_void, *mut f32, u32);
type Notify = extern "C" fn(*mut c_void, c_int);

// SAFETY: these are the functions `device.c` defines, with its signatures; build.rs links them.
unsafe extern "C" {
    fn jess_audio_device_open(
        rate: u32,
        channels: u32,
        period: u32,
        null_only: c_int,
        error: *mut c_char,
        error_length: usize,
    ) -> *mut Raw;
    fn jess_audio_device_attach(device: *mut Raw, render: Render, notify: Notify, user: *mut c_void);
    fn jess_audio_device_start(device: *mut Raw) -> c_int;
    fn jess_audio_device_close(device: *mut Raw);
    fn jess_audio_device_rate(device: *const Raw) -> u32;
    fn jess_audio_device_channels(device: *const Raw) -> u32;
    fn jess_audio_device_period(device: *const Raw) -> u32;
    fn jess_audio_device_periods(device: *const Raw) -> u32;
    fn jess_audio_device_backend(device: *const Raw) -> *const c_char;
}

/// miniaudio's notification types, as `device.c` passes them.
const STOPPED: c_int = 1;
const REROUTED: c_int = 2;
const INTERRUPTED: c_int = 3;

// The engine runs on the device's thread.
const _: fn() = || {
    fn sent<T: Send>() {}
    sent::<Engine>();
};

/// What the device's thread holds: the engine, and where it counts what happens.
struct Callback {
    engine: Engine,
    channels: usize,
    events: Arc<DeviceEvents>,
}

extern "C" fn render(user: *mut c_void, out: *mut f32, frames: u32) {
    // SAFETY: `user` is the `Callback` attached to this device, which lives until `Device::drop`
    // has closed the device, and closing waits for this thread; nothing else touches it while
    // the device runs. `out` is miniaudio's buffer of `frames` frames of `channels` floats, ours
    // for this call.
    let callback = unsafe { &mut *user.cast::<Callback>() };
    let samples = unsafe { std::slice::from_raw_parts_mut(out, frames as usize * callback.channels) };
    if callback.events.failed.load(Ordering::Relaxed) {
        samples.fill(0.0);
        return;
    }
    // A panic must not unwind into C. It is a jess bug; the device plays silence from then on.
    if std::panic::catch_unwind(AssertUnwindSafe(|| callback.engine.render(samples))).is_err() {
        callback.events.failed.store(true, Ordering::Relaxed);
        samples.fill(0.0);
    }
}

extern "C" fn notify(user: *mut c_void, kind: c_int) {
    // SAFETY: as in `render`; a notification only reads the shared counters.
    let callback = unsafe { &*user.cast::<Callback>() };
    let counter = match kind {
        STOPPED => &callback.events.stopped,
        REROUTED => &callback.events.rerouted,
        INTERRUPTED => &callback.events.interrupted,
        _ => return,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

/// An open device and the callback its thread runs.
pub struct Device {
    raw: NonNull<Raw>,
    callback: NonNull<Callback>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device").finish_non_exhaustive()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: `raw` came from `jess_audio_device_open` and is closed once, here; closing stops
        // the device and joins its thread, after which nothing reads `callback`, which came from
        // `Box::into_raw` and is reclaimed once, here.
        unsafe {
            jess_audio_device_close(self.raw.as_ptr());
            drop(Box::from_raw(self.callback.as_ptr()));
        }
    }
}

/// An opened device not yet given an engine.
struct Opened(NonNull<Raw>);

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: opened and never attached or started; closed once.
        unsafe { jess_audio_device_close(self.0.as_ptr()) };
    }
}

pub fn open(request: &DeviceRequest, config: &AudioConfig) -> Result<LiveContext, AudioError> {
    let channels = u32::try_from(config.channels.min(MAX_CHANNELS)).unwrap_or(2);
    let mut error: [c_char; 256] = [0; 256];
    // SAFETY: `error` is writable for its length, which is passed with it.
    let raw = unsafe {
        jess_audio_device_open(
            request.rate.unwrap_or(0),
            channels,
            request.period.unwrap_or(0),
            c_int::from(request.null),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    let Some(raw) = NonNull::new(raw) else {
        // SAFETY: the C side wrote a NUL-terminated message into `error`, or left it zeroed.
        let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy().into_owned();
        return Err(AudioError::Device(message));
    };
    let opened = Opened(raw);
    // SAFETY: `raw` is an open device; these only read it.
    let info = unsafe {
        let backend = CStr::from_ptr(jess_audio_device_backend(raw.as_ptr()))
            .to_string_lossy()
            .into_owned();
        DeviceInfo {
            backend,
            rate: jess_audio_device_rate(raw.as_ptr()),
            channels: jess_audio_device_channels(raw.as_ptr()) as usize,
            period: jess_audio_device_period(raw.as_ptr()),
            periods: jess_audio_device_periods(raw.as_ptr()),
        }
    };
    let config = AudioConfig {
        rate: info.rate,
        channels: info.channels,
        ..*config
    };
    let (control, engine) = crate::audio::context(&config)?;
    let events = Arc::new(DeviceEvents::default());
    let callback = Box::new(Callback {
        engine,
        channels: info.channels,
        events: Arc::clone(&events),
    });
    let callback = NonNull::from(Box::leak(callback));
    std::mem::forget(opened);
    let device = Device { raw, callback };
    // SAFETY: the device is open and not started, so attaching races nothing; `callback` lives
    // as long as `device`, whose drop closes the device first.
    let started = unsafe {
        jess_audio_device_attach(raw.as_ptr(), render, notify, device.callback.as_ptr().cast());
        jess_audio_device_start(raw.as_ptr())
    };
    if started == 0 {
        return Err(AudioError::Device("the audio device would not start".to_owned()));
    }
    Ok(LiveContext {
        control,
        info,
        events,
        _device: device,
    })
}
