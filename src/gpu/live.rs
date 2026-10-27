//! What a device has alive: every buffer, texture, sampler, bind group, and pipeline made on it
//! holds a token that counts it until it drops, so whoever tears a generation down can see that
//! nothing it made is left.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// The next object's identity: process-wide, so no two objects alive or dead ever share one.
static NEXT: AtomicU64 = AtomicU64::new(1);

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

/// A device's counts: of each kind alive, and of each kind ever made.
#[derive(Debug, Default)]
pub struct Live([AtomicUsize; 5], [AtomicUsize; 5]);

impl Live {
    /// A token counting one object of `kind` while it lives.
    pub fn token(self: &Arc<Live>, kind: Kind) -> Token {
        self.0[kind as usize].fetch_add(1, Ordering::Relaxed);
        self.1[kind as usize].fetch_add(1, Ordering::Relaxed);
        Token {
            live: Arc::clone(self),
            kind,
            id: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub fn counts(&self) -> LiveCounts {
        Self::read(&self.0)
    }

    /// How many of each kind were ever made.
    pub fn made(&self) -> LiveCounts {
        Self::read(&self.1)
    }

    fn read(counts: &[AtomicUsize; 5]) -> LiveCounts {
        let count = |kind: Kind| counts[kind as usize].load(Ordering::Relaxed);
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
    id: u64,
}

impl Token {
    /// The object's identity, which no other object ever has: what a cache of things made from
    /// it keys on, where an address could be reused.
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        self.live.0[self.kind as usize].fetch_sub(1, Ordering::Relaxed);
    }
}
