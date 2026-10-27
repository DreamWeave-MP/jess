//! A room: Jezar's Freeverb structure (public domain), eight damped combs into four allpasses a
//! side, the right side's delays a little longer than the left's for width.

use crate::audio::processor::{Io, ParamInfo, Processor, Tail};

/// The comb delays at 44.1 kHz, in frames; scaled to the context's rate.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
/// The allpass delays at 44.1 kHz.
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
/// How much longer the right side's delays are, at 44.1 kHz.
const SPREAD: usize = 23;
/// What the input is scaled by before the combs, which sum eight times over.
const INPUT: f32 = 0.015;
const WET: f32 = 3.0;
const DAMP: f32 = 0.4;
const ROOM: f32 = 0.28;
const ROOM_OFFSET: f32 = 0.7;
const ALLPASS_FEEDBACK: f32 = 0.5;

/// A feedback comb with a one-pole lowpass in the loop: the room's decay, darker as it goes.
#[derive(Debug)]
struct Comb {
    line: Box<[f32]>,
    index: usize,
    store: f32,
}

impl Comb {
    fn new(length: usize) -> Comb {
        Comb {
            line: vec![0.0; length.max(1)].into_boxed_slice(),
            index: 0,
            store: 0.0,
        }
    }

    fn next(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let output = self.line[self.index];
        self.store = output * (1.0 - damp) + self.store * damp;
        if self.store.abs() < 1e-25 {
            self.store = 0.0;
        }
        self.line[self.index] = input + self.store * feedback;
        self.index = (self.index + 1) % self.line.len();
        output
    }
}

/// A Schroeder allpass: diffusion without colour.
#[derive(Debug)]
struct Allpass {
    line: Box<[f32]>,
    index: usize,
}

impl Allpass {
    fn new(length: usize) -> Allpass {
        Allpass {
            line: vec![0.0; length.max(1)].into_boxed_slice(),
            index: 0,
        }
    }

    fn next(&mut self, input: f32) -> f32 {
        let delayed = self.line[self.index];
        let mut stored = input + delayed * ALLPASS_FEEDBACK;
        if stored.abs() < 1e-25 {
            stored = 0.0;
        }
        self.line[self.index] = stored;
        self.index = (self.index + 1) % self.line.len();
        delayed - input
    }
}

/// One side of the room.
#[derive(Debug)]
struct Side {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

impl Side {
    fn new(rate: u32, spread: usize) -> Side {
        let scale = |length: usize| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
            let scaled = ((length + spread) as f64 * f64::from(rate) / 44_100.0).round() as usize;
            scaled
        };
        Side {
            combs: COMBS.iter().map(|length| Comb::new(scale(*length))).collect(),
            allpasses: ALLPASSES.iter().map(|length| Allpass::new(scale(*length))).collect(),
        }
    }

    fn next(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let mut out = 0.0;
        for comb in &mut self.combs {
            out += comb.next(input, feedback, damp);
        }
        for allpass in &mut self.allpasses {
            out = allpass.next(out);
        }
        out
    }
}

/// A reverb over a mono or stereo bus; on a wider one, the first two channels.
///
/// `room` sets how long it rings, `damping` how fast the highs die, `wet` and `dry` the mix, and
/// `width` how far apart the two sides are. On a bus that voices send to, leave `dry` at 0: the
/// bus is only the room.
#[derive(Debug)]
pub struct Reverb {
    rate: u32,
    sides: [Side; 2],
    room: f32,
    damping: f32,
    wet: f32,
    dry: f32,
    width: f32,
}

impl Reverb {
    const PARAMS: &'static [ParamInfo] = &[
        ParamInfo::control("room", 0.0, 1.0, 0.5, ""),
        ParamInfo::control("damping", 0.0, 1.0, 0.5, ""),
        ParamInfo::control("wet", 0.0, 1.0, 0.3, ""),
        ParamInfo::control("dry", 0.0, 1.0, 0.0, ""),
        ParamInfo::control("width", 0.0, 1.0, 1.0, ""),
    ];

    /// A room at the default settings for `rate` frames per second.
    #[must_use]
    pub fn new(rate: u32) -> Reverb {
        Reverb {
            rate,
            sides: [Side::new(rate, 0), Side::new(rate, SPREAD)],
            room: 0.5,
            damping: 0.5,
            wet: 0.3,
            dry: 0.0,
            width: 1.0,
        }
    }

    fn feedback(&self) -> f32 {
        self.room * ROOM + ROOM_OFFSET
    }
}

impl Processor for Reverb {
    fn kind(&self) -> &'static str {
        "reverb"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Reverb::PARAMS
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, index: usize, value: f64) {
        let value = value as f32;
        match index {
            0 => self.room = value,
            1 => self.damping = value,
            2 => self.wet = value,
            3 => self.dry = value,
            _ => self.width = value,
        }
    }

    fn value(&self, index: usize) -> f64 {
        f64::from(match index {
            0 => self.room,
            1 => self.damping,
            2 => self.wet,
            3 => self.dry,
            _ => self.width,
        })
    }

    /// Until the longest comb has died to -60 dB, with the allpasses' smear on top.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    fn tail(&self) -> Tail {
        let longest = (COMBS[7] + SPREAD) as f64 * f64::from(self.rate) / 44_100.0;
        let repeats = 3.0 / -f64::from(self.feedback()).log10();
        let smear = ALLPASSES.iter().sum::<usize>() as f64 * f64::from(self.rate) / 44_100.0;
        Tail::Frames((longest * repeats + smear * 8.0).ceil() as u64)
    }

    fn process(&mut self, io: &mut Io<'_>) {
        let (mut block, _) = io.effect();
        let block = &mut block;
        let (feedback, damp) = (self.feedback(), self.damping * DAMP);
        let wet = self.wet * WET;
        let (near, far) = (wet * (self.width / 2.0 + 0.5), wet * ((1.0 - self.width) / 2.0));
        let stereo = block.channels >= 2;
        for frame in 0..block.frames {
            let left = block.samples[frame];
            let right = if stereo {
                block.samples[crate::audio::QUANTUM + frame]
            } else {
                left
            };
            let input = (left + right) * INPUT;
            let out_left = self.sides[0].next(input, feedback, damp);
            if !stereo {
                block.samples[frame] = out_left * wet + left * self.dry;
                continue;
            }
            let out_right = self.sides[1].next(input, feedback, damp);
            block.samples[frame] = out_left * near + out_right * far + left * self.dry;
            block.samples[crate::audio::QUANTUM + frame] = out_right * near + out_left * far + right * self.dry;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::QUANTUM;

    #[test]
    fn an_impulse_rings_on_long_after_it_and_dies_away() {
        let mut reverb = Reverb::new(48_000);
        let mut samples = vec![0.0f32; 2 * QUANTUM];
        let mut energy = Vec::new();
        for block in 0..2000 {
            samples.fill(0.0);
            if block == 0 {
                samples[0] = 1.0;
                samples[QUANTUM] = 1.0;
            }
            let mut view = Io::new(&mut samples, 2, QUANTUM);
            reverb.process(&mut view);
            energy.push(samples.iter().map(|sample| sample * sample).sum::<f32>());
        }
        let early: f32 = energy[10..100].iter().sum();
        let late: f32 = energy[1800..].iter().sum();
        assert!(early > 1e-4, "{early}");
        assert!(late < early * 1e-3, "{late} after {early}");
        assert!(matches!(reverb.tail(), Tail::Frames(frames) if frames > 48_000 && frames < 48_000 * 10));
    }
}
