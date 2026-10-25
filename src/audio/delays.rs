//! Latency compensation: the delays that line up paths of different latency where they meet.
//!
//! A bus's processors can delay what passes through them (a limiter's lookahead), so two paths
//! from the same voice to the same bus can arrive apart: one through the latent bus, one
//! around it. Where paths meet, every one but the latest is delayed to match it. The control
//! side works the delays out from the same longest-path walk that reports each bus's latency
//! ([`Graph::compensation`](crate::audio::graph::Graph::compensation)), allocates the lines, and
//! sends them to the engine whole whenever they change; the engine moves each line's history
//! into its replacement and sends the old set back to be dropped.
//!
//! A line sits on one of two places: a bus's own input, delaying what voices add into it
//! directly (they arrive with no latency at all), or the edge from one bus to another, its
//! parent or a send. A sidechain key is heard as it left its bus, uncompensated.

use crate::audio::QUANTUM;

/// The `from` of a line on a bus's own input.
pub const INPUT: u32 = u32::MAX;

/// One delay: what goes in comes out `frames` frames later.
#[derive(Debug)]
pub struct Line {
    /// The bus slot the signal leaves, or [`INPUT`].
    pub from: u32,
    /// The bus slot it reaches.
    pub to: u32,
    frames: usize,
    channels: usize,
    /// The last `frames` frames in, interleaved; `next` is the oldest.
    ring: Box<[f32]>,
    next: usize,
}

impl Line {
    fn new(from: u32, to: u32, frames: usize, channels: usize) -> Line {
        Line {
            from,
            to,
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

/// Every line a context mixes with, sorted by where they sit.
#[derive(Debug, Default)]
pub struct Delays {
    lines: Box<[Line]>,
}

impl Delays {
    /// Lines for `wanted`, each `(from, to, frames)`, in a context of `channels`.
    pub fn new(wanted: &[(u32, u32, u64)], channels: usize) -> Delays {
        let mut lines: Vec<Line> = wanted
            .iter()
            .map(|&(from, to, frames)| Line::new(from, to, usize::try_from(frames).unwrap_or(usize::MAX), channels))
            .collect();
        lines.sort_by_key(|line| (line.from, line.to));
        Delays {
            lines: lines.into_boxed_slice(),
        }
    }

    /// The line from `from` to `to`, if there is one.
    pub fn line(&mut self, from: u32, to: u32) -> Option<&mut Line> {
        let index = self
            .lines
            .binary_search_by_key(&(from, to), |line| (line.from, line.to))
            .ok()?;
        Some(&mut self.lines[index])
    }

    /// The lines on buses' own inputs.
    pub fn inputs(&mut self) -> impl Iterator<Item = &mut Line> {
        self.lines.iter_mut().filter(|line| line.from == INPUT)
    }

    /// Takes on the history of every line `old` has in the same place.
    pub fn inherit(&mut self, old: &Delays) {
        for line in &mut self.lines {
            if let Ok(index) = old
                .lines
                .binary_search_by_key(&(line.from, line.to), |known| (known.from, known.to))
            {
                line.inherit(&old.lines[index]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(samples: &[f32]) -> Vec<f32> {
        let mut block = vec![0.0; QUANTUM];
        block[..samples.len()].copy_from_slice(samples);
        block
    }

    #[test]
    fn a_line_delays_by_its_length_across_blocks() {
        let mut line = Line::new(0, 1, 3, 1);
        let mut first = block(&[1.0, 2.0]);
        line.process(&mut first, 2);
        assert_eq!(first[..2], [0.0, 0.0]);
        let mut second = block(&[3.0, 4.0, 5.0]);
        line.process(&mut second, 3);
        assert_eq!(second[..3], [0.0, 1.0, 2.0]);
    }

    #[test]
    fn a_replacement_line_plays_on_from_the_history_it_inherits() {
        let mut old = Line::new(0, 1, 3, 1);
        let mut input = block(&[1.0, 2.0, 3.0, 4.0]);
        old.process(&mut input, 4);
        // Same length: no seam.
        let mut same = Line::new(0, 1, 3, 1);
        same.inherit(&old);
        let mut out = block(&[0.0; 3]);
        same.process(&mut out, 3);
        assert_eq!(out[..3], [2.0, 3.0, 4.0]);
        // Longer: silence first, then the history.
        let mut longer = Line::new(0, 1, 5, 1);
        longer.inherit(&old);
        let mut out = block(&[0.0; 5]);
        longer.process(&mut out, 5);
        assert_eq!(out[..5], [0.0, 0.0, 2.0, 3.0, 4.0]);
        // Shorter: the oldest frames go.
        let mut shorter = Line::new(0, 1, 2, 1);
        shorter.inherit(&old);
        let mut out = block(&[0.0; 2]);
        shorter.process(&mut out, 2);
        assert_eq!(out[..2], [3.0, 4.0]);
    }
}
