//! A delay line with feedback: echoes.

use crate::audio::processor::{Block, ParamInfo, Processor, Tail};

/// Echoes a bus.
///
/// Each sample comes back `time` seconds later, each repeat `feedback` times the last, mixed with
/// the dry signal at `mix` (0 dry, 1 only the echoes). The longest time is fixed when it's made,
/// and its memory with it.
#[derive(Debug)]
pub struct Delay {
    rate: u32,
    /// Frames each channel's line holds.
    length: usize,
    lines: Box<[f32]>,
    write: usize,
    time: f64,
    feedback: f32,
    mix: f32,
}

impl Delay {
    const PARAMS: &'static [ParamInfo] = &[
        ParamInfo {
            name: "time",
            min: 0.0,
            max: 10.0,
            default: 0.25,
        },
        ParamInfo {
            name: "feedback",
            min: -0.99,
            max: 0.99,
            default: 0.3,
        },
        ParamInfo {
            name: "mix",
            min: 0.0,
            max: 1.0,
            default: 0.5,
        },
    ];

    /// The longest delay one can be made with, in seconds.
    pub const LONGEST: f64 = 10.0;

    /// A delay of up to `longest` seconds over `channels` channels at `rate`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Seconds checked in range.
    pub fn new(rate: u32, channels: usize, longest: f64) -> Delay {
        let length = (longest.clamp(0.0, Delay::LONGEST) * f64::from(rate)).ceil() as usize + 1;
        Delay {
            rate,
            length,
            lines: vec![0.0; length * channels].into_boxed_slice(),
            write: 0,
            time: Delay::PARAMS[0].default,
            feedback: 0.3,
            mix: 0.5,
        }
    }

    /// The delay in frames, inside the line.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Clamped to the line.
    fn frames(&self) -> usize {
        ((self.time * f64::from(self.rate)).round() as usize).clamp(1, self.length - 1)
    }
}

impl Processor for Delay {
    fn kind(&self) -> &'static str {
        "delay"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Delay::PARAMS
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, index: usize, value: f64) {
        match index {
            0 => self.time = value,
            1 => self.feedback = value as f32,
            _ => self.mix = value as f32,
        }
    }

    fn value(&self, index: usize) -> f64 {
        match index {
            0 => self.time,
            1 => f64::from(self.feedback),
            _ => f64::from(self.mix),
        }
    }

    /// Until the echoes have died to -60 dB.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    fn tail(&self) -> Tail {
        let delay = self.frames() as f64;
        let repeats = if self.feedback.abs() < 1e-3 {
            1.0
        } else {
            1.0 + (1e-3f64).ln() / f64::from(self.feedback.abs()).ln()
        };
        Tail::Frames((delay * repeats).ceil() as u64)
    }

    fn process(&mut self, block: &mut Block<'_>, _key: Option<&[f32]>) {
        let (delay, length, start) = (self.frames(), self.length, self.write);
        let (feedback, mix) = (self.feedback, self.mix);
        for channel in 0..block.channels {
            let line = &mut self.lines[channel * length..(channel + 1) * length];
            let mut write = start;
            for sample in block.channel(channel) {
                let read = (write + length - delay) % length;
                let echo = line[read];
                line[write] = *sample + echo * feedback;
                *sample = *sample * (1.0 - mix) + echo * mix;
                write = (write + 1) % length;
            }
        }
        self.write = (start + block.frames) % length;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::QUANTUM;

    #[test]
    fn an_echo_comes_back_after_its_time_quieter_each_repeat() {
        let mut delay = Delay::new(1000, 1, 1.0);
        delay.set(0, 0.01);
        delay.set(1, 0.5);
        delay.set(2, 1.0);
        let mut samples = vec![0.0f32; QUANTUM];
        samples[0] = 1.0;
        let mut block = Block {
            samples: &mut samples,
            channels: 1,
            frames: 40,
        };
        delay.process(&mut block, None);
        assert_eq!(samples[10], 1.0);
        assert_eq!(samples[20], 0.5);
        assert_eq!(samples[30], 0.25);
        assert_eq!(samples[0], 0.0, "only the echoes at a mix of one");
        // Ten frames a repeat, and about ten repeats to -60 dB.
        assert!(matches!(delay.tail(), Tail::Frames(frames) if (100..=110).contains(&frames)));
    }
}
