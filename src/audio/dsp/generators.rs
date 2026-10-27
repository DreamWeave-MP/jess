//! Generators: nodes that make signal rather than change it, to hear or to modulate with.

use std::f64::consts::TAU;
use std::str::FromStr;

use crate::audio::AudioError;
use crate::audio::processor::{Io, ParamInfo, Ports, Processor};

/// An oscillator's waveform.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wave {
    /// A sine.
    Sine,
    /// A triangle.
    Triangle,
    /// A rising sawtooth, band-limited.
    Saw,
    /// A square, band-limited.
    Square,
}

impl FromStr for Wave {
    type Err = AudioError;

    fn from_str(name: &str) -> Result<Wave, AudioError> {
        match name {
            "sine" => Ok(Wave::Sine),
            "triangle" => Ok(Wave::Triangle),
            "saw" => Ok(Wave::Saw),
            "square" => Ok(Wave::Square),
            _ => Err(AudioError::Setting {
                kind: "oscillator",
                name: "wave".to_owned(),
                expected: "one of sine, triangle, saw, and square",
            }),
        }
    }
}

/// The correction a band-limited step needs at `phase` (0 to 1) for a step every cycle that moves
/// `step` of a cycle a sample: Välimäki's two-sample polynomial, which takes most of the alias out
/// of a saw or a square.
fn blep(phase: f64, step: f64) -> f64 {
    if phase < step {
        let t = phase / step;
        return t + t - t * t - 1.0;
    }
    if phase > 1.0 - step {
        let t = (phase - 1.0) / step;
        return t * t + t + t + 1.0;
    }
    0.0
}

/// A periodic wave at `frequency` hertz and `gain`.
///
/// Both are audio rate, so another node can modulate them sample by sample: frequency modulation,
/// or an amplifier an envelope drives. It writes the same wave on every channel of its node.
#[derive(Debug)]
pub struct Oscillator {
    wave: Wave,
    rate: f64,
    frequency: f32,
    gain: f32,
    /// Where it is in its cycle, from 0 up to 1.
    phase: f64,
}

impl Oscillator {
    const PARAMS: &'static [ParamInfo] = &[
        ParamInfo::audio("frequency", 0.0, 96_000.0, 440.0, "Hz"),
        ParamInfo::audio("gain", -64.0, 64.0, 1.0, ""),
    ];

    /// An oscillator of `wave` at 440 Hz and full scale, for `rate` frames per second.
    #[must_use]
    pub fn new(wave: Wave, rate: u32) -> Oscillator {
        Oscillator {
            wave,
            rate: f64::from(rate),
            frequency: 440.0,
            gain: 1.0,
            phase: 0.0,
        }
    }

    /// The wave's value at the phase it is at, moving `step` of a cycle a sample.
    fn sample(&self, step: f64) -> f64 {
        let phase = self.phase;
        match self.wave {
            Wave::Sine => (TAU * phase).sin(),
            Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            Wave::Saw => 2.0 * phase - 1.0 - blep(phase, step),
            Wave::Square => {
                let level = if phase < 0.5 { 1.0 } else { -1.0 };
                level + blep(phase, step) - blep((phase + 0.5) % 1.0, step)
            }
        }
    }
}

impl Processor for Oscillator {
    fn kind(&self) -> &'static str {
        "oscillator"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Oscillator::PARAMS
    }

    fn ports(&self) -> &'static Ports {
        &Ports::SOURCE
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, index: usize, value: f64) {
        match index {
            0 => self.frequency = value as f32,
            _ => self.gain = value as f32,
        }
    }

    fn value(&self, index: usize) -> f64 {
        f64::from(match index {
            0 => self.frequency,
            _ => self.gain,
        })
    }

    #[allow(clippy::cast_possible_truncation)] // Back to the engine's f32.
    fn process(&mut self, io: &mut Io<'_>) {
        let (channels, frames) = (io.channels(), io.frames());
        let mut wave = [0.0f32; crate::audio::QUANTUM];
        for (frame, out) in wave[..frames].iter_mut().enumerate() {
            let frequency = io.param(0).map_or(self.frequency, |values| values[frame]);
            let gain = io.param(1).map_or(self.gain, |values| values[frame]);
            let step = (f64::from(frequency) / self.rate).min(0.5);
            *out = (self.sample(step) * f64::from(gain)) as f32;
            self.phase += step;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
        let mut block = io.main();
        for channel in 0..channels {
            block.channel(channel).copy_from_slice(&wave[..frames]);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// An attack, decay, sustain, release envelope, from 0 to 1, opened and closed by its `gate`.
///
/// It rises to 1 over `attack` seconds from wherever it is when the gate opens, falls to `sustain`
/// over `decay`, holds there, and falls to 0 over `release` once the gate closes. Every segment is
/// a straight line. A gate set on a frame opens or closes on that frame. It writes the same level
/// on every channel of its node: what a gain or a filter's cutoff is modulated by.
#[derive(Debug)]
pub struct Envelope {
    rate: f64,
    attack: f64,
    decay: f64,
    sustain: f64,
    release: f64,
    gate: bool,
    stage: Stage,
    level: f64,
    /// What the release takes off a sample, worked out from the level it starts at.
    falling: f64,
}

impl Envelope {
    const PARAMS: &'static [ParamInfo] = &[
        ParamInfo::control("attack", 0.0, 60.0, 0.01, "s"),
        ParamInfo::control("decay", 0.0, 60.0, 0.1, "s"),
        ParamInfo::control("sustain", 0.0, 1.0, 0.7, ""),
        ParamInfo::control("release", 0.0, 60.0, 0.3, "s"),
        ParamInfo::control("gate", 0.0, 1.0, 0.0, ""),
    ];

    /// A closed envelope at the default times, for `rate` frames per second.
    #[must_use]
    pub fn new(rate: u32) -> Envelope {
        Envelope {
            rate: f64::from(rate),
            attack: 0.01,
            decay: 0.1,
            sustain: 0.7,
            release: 0.3,
            gate: false,
            stage: Stage::Idle,
            level: 0.0,
            falling: 0.0,
        }
    }

    /// How much a segment of `seconds` moves its level a sample to cover `span`, or all of it at
    /// once for no time at all.
    fn per_sample(&self, seconds: f64, span: f64) -> f64 {
        if seconds <= 0.0 {
            return f64::INFINITY;
        }
        span / (seconds * self.rate)
    }

    fn next(&mut self) -> f64 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += self.per_sample(self.attack, 1.0);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level -= self.per_sample(self.decay, 1.0 - self.sustain);
                if self.level <= self.sustain {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level = self.sustain,
            Stage::Release => {
                self.level -= self.falling;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

impl Processor for Envelope {
    fn kind(&self) -> &'static str {
        "envelope"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Envelope::PARAMS
    }

    fn ports(&self) -> &'static Ports {
        &Ports::SOURCE
    }

    fn set(&mut self, index: usize, value: f64) {
        match index {
            0 => self.attack = value,
            1 => self.decay = value,
            2 => self.sustain = value,
            3 => self.release = value,
            _ => {
                let open = value >= 0.5;
                if open && !self.gate {
                    self.stage = Stage::Attack;
                } else if !open && self.gate {
                    self.stage = Stage::Release;
                    self.falling = self.per_sample(self.release, self.level);
                }
                self.gate = open;
            }
        }
    }

    fn value(&self, index: usize) -> f64 {
        match index {
            0 => self.attack,
            1 => self.decay,
            2 => self.sustain,
            3 => self.release,
            _ => f64::from(u8::from(self.gate)),
        }
    }

    #[allow(clippy::cast_possible_truncation)] // Back to the engine's f32.
    fn process(&mut self, io: &mut Io<'_>) {
        let (channels, frames) = (io.channels(), io.frames());
        let mut levels = [0.0f32; crate::audio::QUANTUM];
        for level in &mut levels[..frames] {
            *level = self.next() as f32;
        }
        let mut block = io.main();
        for channel in 0..channels {
            block.channel(channel).copy_from_slice(&levels[..frames]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::QUANTUM;

    fn run(processor: &mut dyn Processor, frames: usize) -> Vec<f32> {
        let mut samples = vec![0.0f32; QUANTUM];
        let mut out = Vec::new();
        let mut left = frames;
        while left > 0 {
            let block = left.min(QUANTUM);
            processor.process(&mut Io::new(&mut samples, 1, block));
            out.extend_from_slice(&samples[..block]);
            left -= block;
        }
        out
    }

    #[test]
    fn a_sine_runs_at_its_frequency_from_phase_zero() {
        let mut sine = Oscillator::new(Wave::Sine, 48_000);
        sine.set(0, 12_000.0);
        let out = run(&mut sine, 8);
        let expected = [0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0];
        for (sample, expected) in out.iter().zip(expected) {
            assert!((sample - expected).abs() < 1e-6, "{out:?}");
        }
    }

    #[test]
    fn a_band_limited_saw_spends_less_on_the_folded_harmonics_than_a_naive_one() {
        // Near a quarter of the rate, a naive saw's harmonics fold straight back; the corrected
        // one's jump is spread over two samples, so its largest step is smaller.
        let mut saw = Oscillator::new(Wave::Saw, 48_000);
        saw.set(0, 4_100.0);
        let out = run(&mut saw, 2_000);
        let largest = out.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0f32, f32::max);
        assert!(largest < 1.9, "{largest}");
        assert!(out.iter().all(|sample| sample.abs() <= 1.2));
    }

    #[test]
    fn an_envelope_rises_falls_holds_and_lets_go_in_straight_lines() {
        let mut envelope = Envelope::new(1000);
        envelope.set(0, 0.004);
        envelope.set(1, 0.004);
        envelope.set(2, 0.5);
        envelope.set(3, 0.002);
        envelope.set(4, 1.0);
        let opened = run(&mut envelope, 10);
        assert_eq!(opened, [0.25, 0.5, 0.75, 1.0, 0.875, 0.75, 0.625, 0.5, 0.5, 0.5]);
        envelope.set(4, 0.0);
        assert_eq!(run(&mut envelope, 4), [0.25, 0.0, 0.0, 0.0]);
    }
}
