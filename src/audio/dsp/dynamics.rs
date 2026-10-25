//! Dynamics: a compressor that can listen to another bus, and a lookahead limiter.

use crate::audio::QUANTUM;
use crate::audio::processor::{Block, ParamInfo, Processor};

/// The coefficient that makes a one-pole follower settle about two-thirds of the way in `seconds`.
fn smoothing(seconds: f64, rate: u32) -> f32 {
    #[allow(clippy::cast_possible_truncation)] // A coefficient below one.
    let coefficient = (-1.0 / (seconds * f64::from(rate)).max(1.0)).exp() as f32;
    coefficient
}

fn decibels(level: f32) -> f32 {
    20.0 * level.max(1e-9).log10()
}

fn linear(decibels: f32) -> f32 {
    10f32.powf(decibels / 20.0)
}

/// Turns down what goes over a threshold.
///
/// It follows the level of its own bus, or of the bus it listens to (its sidechain key), across
/// every channel at once, rising over `attack` seconds and falling over `release`. Above
/// `threshold` decibels each decibel more comes out as `1 / ratio` of one, the bend softened over
/// `knee` decibels; `makeup` decibels are added after. A compressor on the music listening to the
/// dialogue ducks the music while anyone speaks.
#[derive(Debug)]
pub struct Compressor {
    rate: u32,
    threshold: f32,
    ratio: f32,
    attack: f64,
    release: f64,
    knee: f32,
    makeup: f32,
    rise: f32,
    fall: f32,
    envelope: f32,
}

impl Compressor {
    const PARAMS: &'static [ParamInfo] = &[
        ParamInfo {
            name: "threshold",
            min: -80.0,
            max: 0.0,
            default: -20.0,
        },
        ParamInfo {
            name: "ratio",
            min: 1.0,
            max: 50.0,
            default: 4.0,
        },
        ParamInfo {
            name: "attack",
            min: 0.0001,
            max: 1.0,
            default: 0.005,
        },
        ParamInfo {
            name: "release",
            min: 0.001,
            max: 5.0,
            default: 0.1,
        },
        ParamInfo {
            name: "knee",
            min: 0.0,
            max: 24.0,
            default: 6.0,
        },
        ParamInfo {
            name: "makeup",
            min: -24.0,
            max: 24.0,
            default: 0.0,
        },
    ];

    /// A compressor at the default settings for `rate` frames per second.
    #[must_use]
    pub fn new(rate: u32) -> Compressor {
        let mut compressor = Compressor {
            rate,
            threshold: -20.0,
            ratio: 4.0,
            attack: 0.005,
            release: 0.1,
            knee: 6.0,
            makeup: 0.0,
            rise: 0.0,
            fall: 0.0,
            envelope: 0.0,
        };
        compressor.update();
        compressor
    }

    fn update(&mut self) {
        self.rise = smoothing(self.attack, self.rate);
        self.fall = smoothing(self.release, self.rate);
    }

    /// The decibels to add to a signal whose level is `level` decibels, before makeup.
    fn reduction(&self, level: f32) -> f32 {
        let over = level - self.threshold;
        let slope = 1.0 / self.ratio - 1.0;
        if 2.0 * over <= -self.knee {
            return 0.0;
        }
        if 2.0 * over.abs() < self.knee {
            let into = over + self.knee / 2.0;
            return slope * into * into / (2.0 * self.knee);
        }
        slope * over
    }
}

impl Processor for Compressor {
    fn kind(&self) -> &'static str {
        "compressor"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Compressor::PARAMS
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, index: usize, value: f64) {
        match index {
            0 => self.threshold = value as f32,
            1 => self.ratio = value as f32,
            2 => self.attack = value,
            3 => self.release = value,
            4 => self.knee = value as f32,
            _ => self.makeup = value as f32,
        }
        self.update();
    }

    fn value(&self, index: usize) -> f64 {
        match index {
            0 => f64::from(self.threshold),
            1 => f64::from(self.ratio),
            2 => self.attack,
            3 => self.release,
            4 => f64::from(self.knee),
            _ => f64::from(self.makeup),
        }
    }

    fn process(&mut self, block: &mut Block<'_>, key: Option<&[f32]>) {
        let makeup = linear(self.makeup);
        // Below the knee there is nothing to work out per sample.
        let quiet = linear(self.threshold - self.knee / 2.0);
        for frame in 0..block.frames {
            let heard = key.unwrap_or(&*block.samples);
            let level = (0..block.channels).fold(0.0f32, |level, channel| level.max(heard[channel * QUANTUM + frame].abs()));
            let coefficient = if level > self.envelope { self.rise } else { self.fall };
            self.envelope = level + coefficient * (self.envelope - level);
            let gain = if self.envelope <= quiet {
                makeup
            } else {
                linear(self.reduction(decibels(self.envelope)) + self.makeup)
            };
            for channel in 0..block.channels {
                block.samples[channel * QUANTUM + frame] *= gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(compressor: &mut Compressor, level: f32, key: Option<f32>) -> f32 {
        let mut samples = vec![level; QUANTUM];
        let keys = key.map(|key| vec![key; QUANTUM]);
        for _ in 0..200 {
            samples.fill(level);
            let mut block = Block {
                samples: &mut samples,
                channels: 1,
                frames: QUANTUM,
            };
            compressor.process(&mut block, keys.as_deref());
        }
        samples[QUANTUM - 1]
    }

    #[test]
    fn a_hard_knee_turns_each_decibel_over_into_a_quarter_of_one() {
        let mut compressor = Compressor::new(48_000);
        compressor.set(4, 0.0);
        // 0 dB in, 20 over a -20 dB threshold at 4:1: -15 dB out.
        let out = run(&mut compressor, 1.0, None);
        assert!((decibels(out) + 15.0).abs() < 0.05, "{}", decibels(out));
        let mut compressor = Compressor::new(48_000);
        assert!(
            (run(&mut compressor, 0.01, None) - 0.01).abs() < 1e-6,
            "below the threshold passes"
        );
    }

    #[test]
    fn a_sidechain_key_turns_down_a_bus_that_is_itself_quiet() {
        let mut compressor = Compressor::new(48_000);
        compressor.set(4, 0.0);
        compressor.set(1, 50.0);
        // The bus is at -40 dB, under the threshold, but the key is at 0 dB.
        let ducked = run(&mut compressor, 0.01, Some(1.0));
        assert!(decibels(ducked) < -55.0, "{}", decibels(ducked));
    }
}
