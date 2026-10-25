//! What a device has alive: every buffer, texture, sampler, bind group, and pipeline made on it
//! holds a token that counts it until it drops, so whoever tears a generation down can see that
//! nothing it made is left.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many of each kind of object a device has alive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveCounts {
    /// Buffers.
    pub buffers: usize,
    /// Textures, surface images included while they're drawn into.
    pub textures: usize,
    /// Samplers.
    pub samplers: usize,
    /// Bind groups.
    pub bind_groups: usize,
    /// Render and compute pipelines.
    pub pipelines: usize,
}

/// A kind of object counted.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Buffer,
    Texture,
    Sampler,
    BindGroup,
    Pipeline,
}

/// A device's counts.
#[derive(Debug, Default)]
pub struct Live([AtomicUsize; 5]);

impl Live {
    /// A token counting one object of `kind` while it lives.
    pub fn token(self: &Arc<Live>, kind: Kind) -> Token {
        self.0[kind as usize].fetch_add(1, Ordering::Relaxed);
        Token {
            live: Arc::clone(self),
            kind,
        }
    }

    pub fn counts(&self) -> LiveCounts {
        let count = |kind: Kind| self.0[kind as usize].load(Ordering::Relaxed);
        LiveCounts {
            buffers: count(Kind::Buffer),
            textures: count(Kind::Texture),
            samplers: count(Kind::Sampler),
            bind_groups: count(Kind::BindGroup),
            pipelines: count(Kind::Pipeline),
        }
    }
}

/// One counted object's place in its device's counts, given back when it drops.
#[derive(Debug)]
pub struct Token {
    live: Arc<Live>,
    kind: Kind,
}

impl Drop for Token {
    fn drop(&mut self) {
        self.live.0[self.kind as usize].fetch_sub(1, Ordering::Relaxed);
    }
}
