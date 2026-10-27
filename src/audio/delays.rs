//! Delay lines a plan carries: the compensation that lines up paths of different latency where
//! they meet, and the rings that delay feedback.
//!
//! A node's processors can delay what passes through them (a limiter's lookahead), so two paths
//! from the same voice to the same node can arrive apart: one through the latent node, one around
//! it. Where paths meet, every one but the latest is delayed to match it. The compiler works the
//! delays out from the same longest-path walk that reports each node's latency
//! ([`compile`](crate::audio::graph::Definition::compile)), and allocates the lines into the plan.
//! The engine moves each line's history into its replacement when it swaps plans.
//!
//! A line sits on one of two places: a node input's own entry, delaying what voices add into it
//! directly (they arrive with no latency at all), or a connection from one node to another.
//!
//! A feedback ring is a different animal: it delays a connection that closes a cycle by a fixed
//! number of frames, indexed by the absolute output frame, so what a loop does is the same however
//! a render is cut into blocks.

use crate::audio::QUANTUM;

/// The `from` of a line on a node input's own entry, which voices add into.
pub const INPUT: u32 = u32::MAX;

/// Where a line or ring sits: a node's output and the node input it reaches, by slot.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Edge {
    /// The slot of the node the signal leaves, or [`INPUT`].
    pub from: u32,
    /// Which of its outputs.
    pub output: u8,
    /// The slot of the node it reaches.
    pub to: u32,
    /// Which of its inputs, sidechains counted.
    pub input: u8,
}

/// One delay: what goes in comes out `frames` frames later.
#[derive(Debug)]
pub struct Line {
    /// Where it sits.
    pub edge: Edge,
    frames: usize,
    channels: usize,
    /// The last `frames` frames in, interleaved; `next` is the oldest.
    ring: Box<[f32]>,
    next: usize,
}

impl Line {
    pub fn new(edge: Edge, frames: usize, channels: usize) -> Line {
        Line {
            edge,
            frames,
            channels,
            ring: vec![0.0; frames * channels].into_boxed_slice(),
            next: 0,
        }
    }

    /// Delays the first `frames` of each channel of `block` (planar, a [`QUANTUM`] a channel)
    /// in place.
    pub fn process(&mut self, block: &mut [f32], frames: usize) {
        for frame in 0..frames {
            let base = self.next * self.channels;
            for channel in 0..self.channels {
                std::mem::swap(&mut block[channel * QUANTUM + frame], &mut self.ring[base + channel]);
            }
            self.next += 1;
            if self.next == self.frames {
                self.next = 0;
            }
        }
    }

    /// Takes on the most recent of what `old` holds, so a line that keeps its length plays on
    /// without a seam. One that grows plays silence where it has no history, and one that
    /// shrinks skips its oldest frames.
    fn inherit(&mut self, old: &Line) {
        if old.channels != self.channels {
            return;
        }
        let kept = self.frames.min(old.frames);
        for index in 0..kept {
            let source = (old.next + old.frames - kept + index) % old.frames;
            let target = self.frames - kept + index;
            self.ring[target * self.channels..(target + 1) * self.channels]
                .copy_from_slice(&old.ring[source * self.channels..(source + 1) * self.channels]);
        }
        self.next = 0;
    }
}

/// The lines of a plan, sorted by where they sit.
#[derive(Debug, Default)]
pub struct Delays {
    lines: Box<[Line]>,
}

impl Delays {
    /// Lines for `wanted`, each an edge and its frames, in a context of `channels`, sorted.
    pub fn new(mut wanted: Vec<(Edge, u64, usize)>) -> Delays {
        wanted.sort_unstable_by_key(|(edge, ..)| *edge);
        let lines = wanted
            .into_iter()
            .map(|(edge, frames, channels)| Line::new(edge, usize::try_from(frames).unwrap_or(usize::MAX), channels))
            .collect();
        Delays { lines }
    }

    /// The index of the line at `edge`, if there is one.
    pub fn find(&self, edge: Edge) -> Option<u32> {
        let index = self.lines.binary_search_by_key(&edge, |line| line.edge).ok()?;
        u32::try_from(index).ok()
    }

    /// Line `index`.
    pub fn line(&mut self, index: u32) -> &mut Line {
        &mut self.lines[index as usize]
    }

    /// Takes on the history of every line `old` has in the same place.
    pub fn inherit(&mut self, old: &Delays) {
        for line in &mut self.lines {
            if let Ok(index) = old.lines.binary_search_by_key(&line.edge, |known| known.edge) {
                line.inherit(&old.lines[index]);
            }
        }
    }
}

/// A feedback connection's delay: frames written at an absolute output frame come out `delay`
/// frames later. It holds `delay` frames and a quantum more, so a block can read what is due
/// before writing what it renders, whatever its length up to a quantum.
#[derive(Debug)]
pub struct Ring {
    /// Where it sits.
    pub edge: Edge,
    delay: u64,
    channels: usize,
    /// Planar: each channel `length` frames.
    samples: Box<[f32]>,
    length: usize,
}

impl Ring {
    pub fn new(edge: Edge, delay: u64, channels: usize) -> Ring {
        let length = usize::try_from(delay).unwrap_or(usize::MAX) + QUANTUM;
        Ring {
            edge,
            delay,
            channels,
            samples: vec![0.0; length * channels].into_boxed_slice(),
            length,
        }
    }

    pub const fn channels(&self) -> usize {
        self.channels
    }

    fn index(&self, frame: u64) -> usize {
        usize::try_from(frame % self.length as u64).unwrap_or(0)
    }

    /// Adds what was written `delay` frames before output frame `now`, for `frames` frames, into
    /// `block`, a channel a [`QUANTUM`].
    pub fn read(&self, now: u64, block: &mut [f32], frames: usize) {
        let start = self.index(now + self.length as u64 - self.delay);
        for channel in 0..self.channels {
            let line = &self.samples[channel * self.length..(channel + 1) * self.length];
            let out = &mut block[channel * QUANTUM..channel * QUANTUM + frames];
            for (offset, sample) in out.iter_mut().enumerate() {
                *sample += line[(start + offset) % self.length];
            }
        }
    }

    /// Writes `frames` frames of `block` (a channel a [`QUANTUM`]) as output frames from `now`.
    pub fn write(&mut self, now: u64, block: &[f32], frames: usize) {
        let start = self.index(now);
        for channel in 0..self.channels {
            let line = &mut self.samples[channel * self.length..(channel + 1) * self.length];
            for (offset, sample) in block[channel * QUANTUM..channel * QUANTUM + frames].iter().enumerate() {
                line[(start + offset) % self.length] = *sample;
            }
        }
    }

    /// Takes on `old`'s history when it has the same delay and layout: absolute frames index both
    /// the same way, so the loop plays on without a seam. Otherwise it starts silent.
    fn inherit(&mut self, old: &Ring) {
        if old.delay == self.delay && old.channels == self.channels {
            self.samples.copy_from_slice(&old.samples);
        }
    }
}

/// The feedback rings of a plan, sorted by where they sit.
#[derive(Debug, Default)]
pub struct Rings {
    rings: Box<[Ring]>,
}

impl Rings {
    /// Rings for `wanted`, each an edge, its delay, and its channels, sorted.
    pub fn new(mut wanted: Vec<(Edge, u64, usize)>) -> Rings {
        wanted.sort_unstable_by_key(|(edge, ..)| *edge);
        let rings = wanted
            .into_iter()
            .map(|(edge, delay, channels)| Ring::new(edge, delay, channels))
            .collect();
        Rings { rings }
    }

    /// The index of the ring at `edge`, if there is one.
    pub fn find(&self, edge: Edge) -> Option<u32> {
        let index = self.rings.binary_search_by_key(&edge, |ring| ring.edge).ok()?;
        u32::try_from(index).ok()
    }

    pub fn ring(&mut self, index: u32) -> &mut Ring {
        &mut self.rings[index as usize]
    }

    /// Takes on the history of every ring `old` has in the same place.
    pub fn inherit(&mut self, old: &Rings) {
        for ring in &mut self.rings {
            if let Ok(index) = old.rings.binary_search_by_key(&ring.edge, |known| known.edge) {
                ring.inherit(&old.rings[index]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGE: Edge = Edge {
        from: 0,
        output: 0,
        to: 1,
        input: 0,
    };

    fn block(samples: &[f32]) -> Vec<f32> {
        let mut block = vec![0.0; QUANTUM];
        block[..samples.len()].copy_from_slice(samples);
        block
    }

    #[test]
    fn a_line_delays_by_its_length_across_blocks() {
        let mut line = Line::new(EDGE, 3, 1);
        let mut first = block(&[1.0, 2.0]);
        line.process(&mut first, 2);
        assert_eq!(first[..2], [0.0, 0.0]);
        let mut second = block(&[3.0, 4.0, 5.0]);
        line.process(&mut second, 3);
        assert_eq!(second[..3], [0.0, 1.0, 2.0]);
    }

    #[test]
    fn a_replacement_line_plays_on_from_the_history_it_inherits() {
        let mut old = Line::new(EDGE, 3, 1);
        let mut input = block(&[1.0, 2.0, 3.0, 4.0]);
        old.process(&mut input, 4);
        // Same length: no seam.
        let mut same = Line::new(EDGE, 3, 1);
        same.inherit(&old);
        let mut out = block(&[0.0; 3]);
        same.process(&mut out, 3);
        assert_eq!(out[..3], [2.0, 3.0, 4.0]);
        // Longer: silence first, then the history.
        let mut longer = Line::new(EDGE, 5, 1);
        longer.inherit(&old);
        let mut out = block(&[0.0; 5]);
        longer.process(&mut out, 5);
        assert_eq!(out[..5], [0.0, 0.0, 2.0, 3.0, 4.0]);
        // Shorter: the oldest frames go.
        let mut shorter = Line::new(EDGE, 2, 1);
        shorter.inherit(&old);
        let mut out = block(&[0.0; 2]);
        shorter.process(&mut out, 2);
        assert_eq!(out[..2], [3.0, 4.0]);
    }

    #[test]
    fn a_ring_gives_back_each_frame_its_delay_later_however_the_blocks_fall() {
        let input: Vec<f32> = (1..=400u16).map(f32::from).collect();
        let mut outputs = Vec::new();
        for size in [1, 7, 128] {
            let mut ring = Ring::new(EDGE, 130, 1);
            let mut out = Vec::new();
            let mut now = 0;
            while now < input.len() {
                let frames = size.min(input.len() - now);
                let mut read = vec![0.0; QUANTUM];
                ring.read(now as u64, &mut read, frames);
                out.extend_from_slice(&read[..frames]);
                ring.write(now as u64, &block(&input[now..now + frames]), frames);
                now += frames;
            }
            outputs.push(out);
        }
        assert_eq!(outputs[0][..130], [0.0; 130]);
        assert_eq!(outputs[0][130..], input[..270]);
        assert_eq!(outputs[0], outputs[1]);
        assert_eq!(outputs[0], outputs[2]);
    }
}
