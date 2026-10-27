//! Crossfading two signals: a node with two inputs.

use crate::audio::QUANTUM;
use crate::audio::processor::{Io, ParamInfo, Port, PortKind, Ports, Processor};

/// Blends its two inputs, `a` and `b`, by its `mix`.
///
/// All of `a` at a mix of 0, all of `b` at 1, and the straight line between, sample by sample
/// while the mix moves. A dry path into `a` and a wet one into `b` is a wet and dry control after
/// the fact.
#[derive(Debug)]
pub struct Crossfade {
    mix: f32,
}

impl Crossfade {
    const PARAMS: &'static [ParamInfo] = &[ParamInfo::audio("mix", 0.0, 1.0, 0.5, "")];

    const PORTS: Ports = Ports {
        inputs: &[
            Port {
                name: "a",
                kind: PortKind::Audio,
            },
            Port {
                name: "b",
                kind: PortKind::Audio,
            },
        ],
        outputs: &["out"],
        in_place: true,
    };

    /// An even blend.
    #[must_use]
    pub const fn new() -> Crossfade {
        Crossfade { mix: 0.5 }
    }
}

impl Default for Crossfade {
    fn default() -> Self {
        Crossfade::new()
    }
}

impl Processor for Crossfade {
    fn kind(&self) -> &'static str {
        "crossfade"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Crossfade::PARAMS
    }

    fn ports(&self) -> &'static Ports {
        &Crossfade::PORTS
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, _index: usize, value: f64) {
        self.mix = value as f32;
    }

    fn value(&self, _index: usize) -> f64 {
        f64::from(self.mix)
    }

    fn process(&mut self, io: &mut Io<'_>) {
        let (channels, frames) = (io.channels(), io.frames());
        let mut mixes = [self.mix; QUANTUM];
        if let Some(values) = io.param(0) {
            mixes[..frames].copy_from_slice(values);
        }
        let (mut block, Some(other)) = io.main_and_input(1) else {
            return;
        };
        for channel in 0..channels {
            let b = &other[channel * QUANTUM..channel * QUANTUM + frames];
            for ((a, b), mix) in block.channel(channel).iter_mut().zip(b).zip(&mixes) {
                *a += (b - *a) * mix;
            }
        }
    }
}
