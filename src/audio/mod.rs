//! Audio for jess applications: PCM, voices, an engine that mixes them on a sample clock, and
//! the commands that drive it.
//!
//! The module knows frames, channels, samples, voices, and when. It doesn't know what music or
//! a footstep is: what to play, when in musical terms, and how loud relative to what are Luau's
//! (`@jess/audio` and the packages above it), which turn those decisions into commands stamped
//! with output frames.
//!
//! ```text
//! application side    Control          play, stop, set, each stamped with a frame
//!     v                                  (bounded queue; a full one is an error)
//! render side         Engine::render   carries commands out on their exact frame, mixes
//!     ^                                  (no allocation, no lock, no Luau)
//! application side    Control::collect ended voices, their PCM dropped here
//! ```
//!
//! An engine is rendered by a device or by an [`OfflineContext`], and it is the same engine
//! either way, so an offline render is the test of a live one. The default build has no device:
//! everything here runs headless.
//!
//! # Example
//!
//! Two voices stamped for the same frame start on the same sample:
//!
//! ```
//! use std::sync::Arc;
//! use jess::audio::{AudioConfig, OfflineContext, Pcm, PlaySpec};
//!
//! let click = Arc::new(Pcm::from_planar(&[&[1.0, 0.5]], 48_000)?);
//! let mut context = OfflineContext::new(&AudioConfig { channels: 1, ..AudioConfig::default() })?;
//! let at = PlaySpec { at: Some(300), ..PlaySpec::default() };
//! context.control().play(&click, &at)?;
//! context.control().play(&click, &at)?;
//!
//! let out = context.render(512);
//! assert_eq!(out[299..303], [0.0, 2.0, 1.0, 0.0]);
//! # Ok::<(), jess::audio::AudioError>(())
//! ```
//!
//! # Time
//!
//! The audio clock is output frames rendered, a `u64` that only the engine moves. It is not the
//! application's time, which can stall, nor a device's presentation time, which lags it by the
//! device's latency, nor musical time, which is Luau's to map onto frames.

#![warn(missing_docs)]

mod bus;
mod context;
mod engine;
mod error;
mod graph;
mod pcm;
#[cfg(test)]
mod tests;
mod voice;

pub use bus::{BusKey, BusParam, BusSpec};
pub use context::{AudioConfig, AudioStats, Control, OfflineContext, VoiceState, context};
pub use engine::Engine;
pub use error::AudioError;
pub use pcm::{Pcm, SampleFormat};
pub use voice::{LoopRegion, PlaySpec, VoiceKey, VoiceParam};

/// The most frames the engine mixes at once. A command's frame can split a block anywhere.
pub const QUANTUM: usize = 128;

/// The most channels a source or an output has.
pub const MAX_CHANNELS: usize = 8;

/// The highest sample rate, in frames per second.
pub const MAX_RATE: u32 = 384_000;

/// The most voices one context holds.
pub const MAX_VOICES: usize = 65_536;

/// The most buses one context holds, the master included.
pub const MAX_BUSES: usize = 4096;

/// The fastest a voice plays its source.
pub const MAX_PLAYBACK_RATE: f64 = 64.0;
