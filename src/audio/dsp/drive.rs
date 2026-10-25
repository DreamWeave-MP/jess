//! Saturation: a bus driven into a soft clip.

use crate::audio::processor::{Block, ParamInfo, Processor};

/// Drives a bus into a hyperbolic tangent: `drive` decibels in, a curve that rounds off instead
/// of clipping, `level` decibels out, blended with the clean signal at `mix`.
#[derive(Debug)]
pub struct Drive {
    amount: f32,
    level: f32,
    mix: f32,
    gain_in: f32,
    gain_out: f32,
}

impl Drive {
    const PARAMS: &'static [ParamInfo] = &[
        ParamInfo {
            name: "drive",
            min: 0.0,
            max: 48.0,
            default: 12.0,
        },
        ParamInfo {
            name: "level",
            min: -48.0,
            max: 12.0,
            default: -6.0,
        },
        ParamInfo {
            name: "mix",
            min: 0.0,
            max: 1.0,
            default: 1.0,
        },
    ];

    /// A drive at the default settings.
    #[must_use]
    pub fn new() -> Drive {
        let mut drive = Drive {
            amount: 12.0,
            level: -6.0,
            mix: 1.0,
            gain_in: 0.0,
            gain_out: 0.0,
        };
        drive.update();
        drive
    }

    fn update(&mut self) {
        self.gain_in = 10f32.powf(self.amount / 20.0);
        self.gain_out = 10f32.powf(self.level / 20.0);
    }
}

impl Default for Drive {
    fn default() -> Self {
        Drive::new()
    }
}

impl Processor for Drive {
    fn kind(&self) -> &'static str {
        "drive"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Drive::PARAMS
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, index: usize, value: f64) {
        let value = value as f32;
        match index {
            0 => self.amount = value,
            1 => self.level = value,
            _ => self.mix = value,
        }
        self.update();
    }

    fn value(&self, index: usize) -> f64 {
        f64::from(match index {
            0 => self.amount,
            1 => self.level,
            _ => self.mix,
        })
    }

    fn process(&mut self, block: &mut Block<'_>, _key: Option<&[f32]>) {
        let (into, out, mix) = (self.gain_in, self.gain_out, self.mix);
        for channel in 0..block.channels {
            for sample in block.channel(channel) {
                let driven = (*sample * into).tanh() * out;
                *sample = *sample + (driven - *sample) * mix;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::QUANTUM;

    #[test]
    fn a_driven_bus_rounds_off_at_its_level_instead_of_clipping() {
        let mut drive = Drive::new();
        drive.set(1, 0.0);
        let mut samples = vec![0.0f32; QUANTUM];
        samples[0] = 10.0;
        samples[1] = 0.001;
        let mut block = Block {
            samples: &mut samples,
            channels: 1,
            frames: 2,
        };
        drive.process(&mut block, None);
        assert!(samples[0] <= 1.0 && samples[0] > 0.99, "{}", samples[0]);
        assert!(
            (samples[1] - 0.001 * 10f32.powf(12.0 / 20.0)).abs() < 1e-5,
            "small signals just gain"
        );
    }
}
