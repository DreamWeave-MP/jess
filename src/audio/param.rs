//! Parameters that move: a value the engine steps or ramps, sample by sample.
//!
//! A step lands on its frame. A ramp starts on its frame and reaches its target exactly on the
//! last frame of its length, linear or exponential, worked out from where it started rather than
//! accumulated, so a long ramp ends where it was told to. Nothing here waits for the application:
//! once a ramp is sent, the engine carries it to the end.

use crate::audio::AudioError;

/// The shape of a ramp.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Curve {
    /// A straight line from where it is to the target.
    #[default]
    Linear,
    /// The same ratio every frame: even steps in decibels, for gains and frequencies. Both ends
    /// are above zero; a ramp from zero or below starts at 1e-5, -100 dB.
    Exponential,
}

impl Curve {
    /// The name scripts use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Curve::Linear => "linear",
            Curve::Exponential => "exponential",
        }
    }

    /// The curve called `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Curve> {
        [Curve::Linear, Curve::Exponential]
            .into_iter()
            .find(|curve| curve.name() == name)
    }
}

/// A ramp to a target over a number of frames.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Glide {
    /// How many frames it takes; the target is reached on the last.
    pub frames: u64,
    /// Its shape.
    pub curve: Curve,
}

impl Glide {
    /// Checks a ramp to `target` takes this shape.
    pub(crate) fn check(self, name: &'static str, target: f64) -> Result<(), AudioError> {
        if self.curve == Curve::Exponential && target <= 0.0 {
            return Err(AudioError::Parameter {
                name,
                value: target,
                expected: "above 0 for an exponential ramp",
            });
        }
        Ok(())
    }
}

/// A value the engine holds, stepping or ramping.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    value: f64,
    start: f64,
    target: f64,
    /// What a frame adds (linear) or multiplies by (exponential).
    step: f64,
    /// Frames into the ramp, and its length; equal when it isn't ramping.
    elapsed: u64,
    length: u64,
    curve: Curve,
}

/// How often a ramp stepping frame by frame goes back to its formula, so rounding never builds.
const ANCHOR: u64 = 128;

impl Param {
    /// Where an exponential ramp from zero or below starts: -100 dB.
    pub const FLOOR: f64 = 1e-5;

    /// A steady value.
    pub const fn new(value: f64) -> Param {
        Param {
            value,
            start: value,
            target: value,
            step: 0.0,
            elapsed: 0,
            length: 0,
            curve: Curve::Linear,
        }
    }

    /// The value now.
    pub const fn value(&self) -> f64 {
        self.value
    }

    /// True when it isn't ramping.
    pub const fn steady(&self) -> bool {
        self.elapsed == self.length
    }

    /// Jumps to `value`, ending any ramp.
    pub const fn set(&mut self, value: f64) {
        *self = Param::new(value);
    }

    /// Ramps from where it is to `target` over `glide`, or jumps there for a ramp of no frames.
    pub fn ramp(&mut self, target: f64, glide: Glide) {
        if glide.frames == 0 {
            self.set(target);
            return;
        }
        let start = match glide.curve {
            Curve::Exponential => self.value.max(Param::FLOOR),
            Curve::Linear => self.value,
        };
        #[allow(clippy::cast_precision_loss)] // A ramp's length as a divisor.
        let frames = glide.frames as f64;
        let step = match glide.curve {
            Curve::Linear => (target - start) / frames,
            Curve::Exponential => (target / start).powf(frames.recip()),
        };
        *self = Param {
            value: start,
            start,
            target,
            step,
            elapsed: 0,
            length: glide.frames,
            curve: glide.curve,
        };
    }

    /// Moves `frames` along, and returns the value it reached.
    pub fn advance(&mut self, frames: u64) -> f64 {
        if self.steady() {
            return self.value;
        }
        self.elapsed = (self.elapsed + frames).min(self.length);
        if self.steady() {
            self.set(self.target);
        } else {
            self.value = self.at(self.elapsed);
        }
        self.value
    }

    /// Writes the value of each of the next `out.len()` frames into `out`, moving along as it
    /// goes: the first is the value on the next frame.
    #[allow(clippy::cast_possible_truncation)] // Engine values are f32.
    pub fn fill(&mut self, out: &mut [f32]) {
        for sample in out {
            *sample = self.value as f32;
            self.next();
        }
    }

    /// Moves one frame along: a step of the ramp, and every [`ANCHOR`] frames its formula.
    fn next(&mut self) {
        if self.steady() {
            return;
        }
        self.elapsed += 1;
        if self.steady() {
            self.set(self.target);
            return;
        }
        if self.elapsed.is_multiple_of(ANCHOR) {
            self.value = self.at(self.elapsed);
            return;
        }
        match self.curve {
            Curve::Linear => self.value += self.step,
            Curve::Exponential => self.value *= self.step,
        }
    }

    /// The ramp's value `elapsed` frames in, from its formula.
    #[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)] // A fraction of its length; no libm fma.
    fn at(&self, elapsed: u64) -> f64 {
        let through = elapsed as f64 / self.length as f64;
        match self.curve {
            Curve::Linear => self.start + (self.target - self.start) * through,
            Curve::Exponential => self.start * (self.target / self.start).powf(through),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linear_ramp_reaches_its_target_on_its_last_frame() {
        let mut param = Param::new(0.0);
        param.ramp(
            1.0,
            Glide {
                frames: 4,
                curve: Curve::Linear,
            },
        );
        let mut out = [0.0; 6];
        param.fill(&mut out);
        assert_eq!(out, [0.0, 0.25, 0.5, 0.75, 1.0, 1.0]);
        assert!(param.steady());
    }

    #[test]
    fn an_exponential_ramp_keeps_one_ratio_and_starts_above_zero() {
        let mut param = Param::new(1.0);
        param.ramp(
            8.0,
            Glide {
                frames: 3,
                curve: Curve::Exponential,
            },
        );
        let mut out = [0.0; 4];
        param.fill(&mut out);
        for (sample, expected) in out.iter().zip([1.0, 2.0, 4.0, 8.0]) {
            assert!((sample - expected).abs() < 1e-5, "{out:?}");
        }
        let mut silent = Param::new(0.0);
        silent.ramp(
            1.0,
            Glide {
                frames: 10,
                curve: Curve::Exponential,
            },
        );
        assert_eq!(silent.value(), Param::FLOOR);
    }

    #[test]
    fn a_long_ramp_ends_exactly_where_it_was_told() {
        let mut param = Param::new(0.3);
        param.ramp(
            0.7,
            Glide {
                frames: 1_000_003,
                curve: Curve::Linear,
            },
        );
        for _ in 0..1_000_003 {
            param.advance(1);
        }
        assert_eq!(param.value(), 0.7);
        assert!(param.steady());
    }
}
