//! Biquad filters: the second-order sections of Robert Bristow-Johnson's audio EQ cookbook.

use std::f64::consts::PI;
use std::str::FromStr;

use crate::audio::processor::{Io, ParamInfo, Processor};
use crate::audio::{AudioError, MAX_CHANNELS};

/// What a filter does to the spectrum.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// Passes below the frequency.
    Lowpass,
    /// Passes above it.
    Highpass,
    /// Passes a band around it, `q` wide.
    Bandpass,
    /// Cuts a band around it.
    Notch,
    /// Boosts or cuts a band around it by `gain` decibels: one band of a parametric EQ.
    Peak,
    /// Boosts or cuts below it by `gain` decibels.
    Lowshelf,
    /// Boosts or cuts above it by `gain` decibels.
    Highshelf,
    /// Passes everything, turning the phase around the frequency.
    Allpass,
}

impl Shape {
    /// The shapes, by the names scripts use.
    pub const NAMES: &'static str = "lowpass, highpass, bandpass, notch, peak, lowshelf, highshelf, allpass";
}

impl FromStr for Shape {
    type Err = AudioError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Ok(match name {
            "lowpass" => Shape::Lowpass,
            "highpass" => Shape::Highpass,
            "bandpass" => Shape::Bandpass,
            "notch" => Shape::Notch,
            "peak" => Shape::Peak,
            "lowshelf" => Shape::Lowshelf,
            "highshelf" => Shape::Highshelf,
            "allpass" => Shape::Allpass,
            _ => {
                return Err(AudioError::Setting {
                    kind: "filter",
                    name: "shape".to_owned(),
                    expected: "one of lowpass, highpass, bandpass, notch, peak, lowshelf, highshelf, allpass",
                });
            }
        })
    }
}

/// A biquad's coefficients, normalized so `a0` is one.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Coefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coefficients {
    /// The cookbook's section for `shape` at `frequency` (clamped below the Nyquist frequency),
    /// `q`, and, for the peak and shelves, `gain` decibels, at `rate` frames per second.
    #[must_use]
    pub fn new(shape: Shape, rate: u32, frequency: f64, q: f64, gain: f64) -> Coefficients {
        let rate = f64::from(rate);
        let frequency = frequency.clamp(1.0, rate * 0.49);
        let omega = 2.0 * PI * frequency / rate;
        let (sin, cos) = omega.sin_cos();
        let alpha = sin / (2.0 * q.max(1e-3));
        let amplitude = 10f64.powf(gain / 40.0);
        let shelf = 2.0 * amplitude.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::Lowpass => (
                f64::midpoint(1.0, -cos),
                1.0 - cos,
                f64::midpoint(1.0, -cos),
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            Shape::Highpass => (
                f64::midpoint(1.0, cos),
                -(1.0 + cos),
                f64::midpoint(1.0, cos),
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            Shape::Bandpass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            Shape::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            Shape::Allpass => (1.0 - alpha, -2.0 * cos, 1.0 + alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            Shape::Peak => (
                1.0 + alpha * amplitude,
                -2.0 * cos,
                1.0 - alpha * amplitude,
                1.0 + alpha / amplitude,
                -2.0 * cos,
                1.0 - alpha / amplitude,
            ),
            Shape::Lowshelf => {
                let (up, down) = (amplitude + 1.0, amplitude - 1.0);
                (
                    amplitude * (up - down * cos + shelf),
                    2.0 * amplitude * (down - up * cos),
                    amplitude * (up - down * cos - shelf),
                    up + down * cos + shelf,
                    -2.0 * (down + up * cos),
                    up + down * cos - shelf,
                )
            }
            Shape::Highshelf => {
                let (up, down) = (amplitude + 1.0, amplitude - 1.0);
                (
                    amplitude * (up + down * cos + shelf),
                    -2.0 * amplitude * (down + up * cos),
                    amplitude * (up + down * cos - shelf),
                    up - down * cos + shelf,
                    2.0 * (down - up * cos),
                    up - down * cos - shelf,
                )
            }
        };
        Coefficients {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

/// One channel's memory of a biquad, transposed direct form II.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Section {
    first: f64,
    second: f64,
}

impl Section {
    /// Filters one sample.
    pub fn next(&mut self, coefficients: &Coefficients, input: f64) -> f64 {
        let output = coefficients.b0 * input + self.first;
        self.first = coefficients.b1 * input - coefficients.a1 * output + self.second;
        self.second = coefficients.b2 * input - coefficients.a2 * output;
        // Below the smallest normal float the state is silence; keeping it would make every
        // sample after it slow on many CPUs.
        if self.first.abs() < 1e-30 {
            self.first = 0.0;
        }
        if self.second.abs() < 1e-30 {
            self.second = 0.0;
        }
        output
    }
}

/// A biquad filter over every channel of a bus.
#[derive(Debug)]
pub struct Filter {
    shape: Shape,
    rate: u32,
    frequency: f64,
    q: f64,
    gain: f64,
    coefficients: Coefficients,
    sections: [Section; MAX_CHANNELS],
}

impl Filter {
    const PARAMS: &'static [ParamInfo] = &[
        ParamInfo::control("frequency", 1.0, 192_000.0, 1000.0, "Hz"),
        ParamInfo::control("q", 0.025, 40.0, std::f64::consts::FRAC_1_SQRT_2, ""),
        ParamInfo::control("gain", -48.0, 48.0, 0.0, "dB"),
    ];

    /// A filter of `shape` at the default frequency, Q, and gain, for `rate` frames per second.
    #[must_use]
    pub fn new(shape: Shape, rate: u32) -> Filter {
        let mut filter = Filter {
            shape,
            rate,
            frequency: Filter::PARAMS[0].default,
            q: Filter::PARAMS[1].default,
            gain: Filter::PARAMS[2].default,
            coefficients: Coefficients::default(),
            sections: [Section::default(); MAX_CHANNELS],
        };
        filter.update();
        filter
    }

    fn update(&mut self) {
        self.coefficients = Coefficients::new(self.shape, self.rate, self.frequency, self.q, self.gain);
    }
}

impl Processor for Filter {
    fn kind(&self) -> &'static str {
        "filter"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Filter::PARAMS
    }

    fn set(&mut self, index: usize, value: f64) {
        match index {
            0 => self.frequency = value,
            1 => self.q = value,
            _ => self.gain = value,
        }
        self.update();
    }

    fn value(&self, index: usize) -> f64 {
        match index {
            0 => self.frequency,
            1 => self.q,
            _ => self.gain,
        }
    }

    #[allow(clippy::cast_possible_truncation)] // Back to the engine's f32.
    fn process(&mut self, io: &mut Io<'_>) {
        let (mut block, _) = io.effect();
        let block = &mut block;
        let coefficients = self.coefficients;
        for channel in 0..block.channels {
            let section = &mut self.sections[channel];
            for sample in block.channel(channel) {
                *sample = section.next(&coefficients, f64::from(*sample)) as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::QUANTUM;

    /// The steady amplitude a filter leaves a sine of `frequency` at.
    fn response(filter: &mut Filter, frequency: f64) -> f64 {
        let rate = 48_000.0;
        let mut peak = 0.0f64;
        let mut samples = vec![0.0f32; QUANTUM];
        for block in 0..400 {
            for (index, sample) in samples.iter_mut().enumerate() {
                let frame = f64::from(u32::try_from(block * QUANTUM + index).unwrap());
                #[allow(clippy::cast_possible_truncation)]
                let value = (2.0 * PI * frequency * frame / rate).sin() as f32;
                *sample = value;
            }
            let mut view = Io::new(&mut samples, 1, QUANTUM);
            filter.process(&mut view);
            if block > 300 {
                peak = samples.iter().fold(peak, |peak, sample| peak.max(f64::from(sample.abs())));
            }
        }
        peak
    }

    #[test]
    fn a_lowpass_passes_below_its_frequency_and_cuts_above() {
        let mut filter = Filter::new(Shape::Lowpass, 48_000);
        filter.set(0, 1000.0);
        assert!((response(&mut filter, 100.0) - 1.0).abs() < 0.01);
        let mut filter = Filter::new(Shape::Lowpass, 48_000);
        filter.set(0, 1000.0);
        // Twelve decibels an octave: about -40 dB a decade and a third up.
        assert!(response(&mut filter, 10_000.0) < 0.02);
    }

    #[test]
    fn a_peak_boosts_its_band_by_its_gain() {
        let mut filter = Filter::new(Shape::Peak, 48_000);
        filter.set(0, 2000.0);
        filter.set(2, 6.0);
        let boost = response(&mut filter, 2000.0);
        assert!((boost - 10f64.powf(6.0 / 20.0)).abs() < 0.02, "{boost}");
    }

    #[test]
    fn shapes_are_named() {
        assert_eq!("highshelf".parse::<Shape>(), Ok(Shape::Highshelf));
        assert!(matches!("bell".parse::<Shape>(), Err(AudioError::Setting { .. })));
    }
}
