//! Panning a bus: the balance of a stereo bus.

use crate::audio::processor::{Io, ParamInfo, Processor};

/// Balances a stereo bus: at `-1` the right side is silent, at `1` the left, and at `0` both pass
/// as they are. On a bus of any other width it does nothing.
#[derive(Debug, Default)]
pub struct Pan {
    pan: f32,
}

impl Pan {
    const PARAMS: &'static [ParamInfo] = &[ParamInfo::control("pan", -1.0, 1.0, 0.0, "")];
}

impl Processor for Pan {
    fn kind(&self) -> &'static str {
        "pan"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Pan::PARAMS
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, _index: usize, value: f64) {
        self.pan = value as f32;
    }

    fn value(&self, _index: usize) -> f64 {
        f64::from(self.pan)
    }

    fn process(&mut self, io: &mut Io<'_>) {
        let (mut block, _) = io.effect();
        let block = &mut block;
        if block.channels != 2 {
            return;
        }
        let (left, right) = (1.0 - self.pan.max(0.0), 1.0 + self.pan.min(0.0));
        for sample in block.channel(0) {
            *sample *= left;
        }
        for sample in block.channel(1) {
            *sample *= right;
        }
    }
}
