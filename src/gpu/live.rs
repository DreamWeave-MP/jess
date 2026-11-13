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

/// How many bytes a device's live buffers and textures hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveBytes {
    /// The sizes of its buffers.
    pub buffers: u64,
    /// The texels of its textures, every level, layer, and sample of them. A window's images are
    /// the window system's, and count nothing.
    pub textures: u64,
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

/// A device's counts: of each kind alive, of each kind ever made, and of the bytes its live
/// buffers and textures hold.
#[derive(Debug, Default)]
pub struct Live([AtomicUsize; 5], [AtomicUsize; 5], [AtomicU64; 2]);

impl Live {
    /// A token counting one object of `kind` while it lives.
    pub fn token(self: &Arc<Live>, kind: Kind) -> Token {
        self.sized(kind, 0)
    }

    /// A token counting one buffer or texture of `bytes` bytes while it lives.
    pub fn sized(self: &Arc<Live>, kind: Kind, bytes: u64) -> Token {
        self.0[kind as usize].fetch_add(1, Ordering::Relaxed);
        self.1[kind as usize].fetch_add(1, Ordering::Relaxed);
        if let Some(held) = self.held(kind) {
            held.fetch_add(bytes, Ordering::Relaxed);
        }
        Token {
            live: Arc::clone(self),
            kind,
            bytes,
            id: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Where the bytes of `kind` are counted, if it is a kind that holds any.
    fn held(&self, kind: Kind) -> Option<&AtomicU64> {
        match kind {
            Kind::Buffer => Some(&self.2[0]),
            Kind::Texture => Some(&self.2[1]),
            Kind::Sampler | Kind::BindGroup | Kind::Pipeline => None,
        }
    }

    /// How many bytes its live buffers and textures hold.
    pub fn bytes(&self) -> LiveBytes {
        LiveBytes {
            buffers: self.2[0].load(Ordering::Relaxed),
            textures: self.2[1].load(Ordering::Relaxed),
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
    bytes: u64,
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
        if let Some(held) = self.live.held(self.kind) {
            held.fetch_sub(self.bytes, Ordering::Relaxed);
        }
    }
}
