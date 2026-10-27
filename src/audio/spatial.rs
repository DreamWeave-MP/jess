//! Space: where a voice is relative to the listener, and what that does to it.
//!
//! The engine takes positions, velocities, and directions in whatever units and axes the world
//! uses (only the handedness of its axes matters, and it is the context's to say), and works out
//! a voice's distance attenuation, its cone, its pan, and its Doppler shift once a block, from
//! the listener's frame. What lies between a voice and the listener is not the engine's to know:
//! occlusion, obstruction, and the like arrive as an extra gain and a low-pass cutoff that the
//! application worked out.

// Vector arithmetic is sums of products; `mul_add` is a libm call on a CPU built for without FMA.
#![allow(clippy::suboptimal_flops)]

use crate::audio::AudioError;

/// A point or direction.
pub type Vector = [f64; 3];

fn dot(first: Vector, second: Vector) -> f64 {
    first[0] * second[0] + first[1] * second[1] + first[2] * second[2]
}

fn cross(first: Vector, second: Vector) -> Vector {
    [
        first[1] * second[2] - first[2] * second[1],
        first[2] * second[0] - first[0] * second[2],
        first[0] * second[1] - first[1] * second[0],
    ]
}

fn minus(first: Vector, second: Vector) -> Vector {
    [first[0] - second[0], first[1] - second[1], first[2] - second[2]]
}

fn length(vector: Vector) -> f64 {
    dot(vector, vector).sqrt()
}

fn unit(vector: Vector) -> Option<Vector> {
    let length = length(vector);
    (length > 1e-12).then(|| [vector[0] / length, vector[1] / length, vector[2] / length])
}

/// How the world's axes turn: whether forward crossed with up points right (right-handed, as
/// with `x` right, `y` up, and `z` toward the viewer) or left.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Handedness {
    /// Forward × up is right.
    #[default]
    Right,
    /// Up × forward is right.
    Left,
}

/// How a voice gets quieter with distance, between its reference distance (full level) and its
/// maximum (no quieter beyond), the way OpenAL's clamped models do.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Distance {
    /// Not at all.
    None,
    /// `reference / (reference + rolloff × (distance - reference))`.
    #[default]
    Inverse,
    /// `1 - rolloff × (distance - reference) / (maximum - reference)`.
    Linear,
    /// `(distance / reference) ^ -rolloff`.
    Exponential,
}

impl Distance {
    /// The name scripts use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Distance::None => "none",
            Distance::Inverse => "inverse",
            Distance::Linear => "linear",
            Distance::Exponential => "exponential",
        }
    }

    /// The model called `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Distance> {
        [Distance::None, Distance::Inverse, Distance::Linear, Distance::Exponential]
            .into_iter()
            .find(|model| model.name() == name)
    }
}

/// Where the listener is and which way it faces.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Listener {
    /// Its position.
    pub position: Vector,
    /// The way it faces.
    pub forward: Vector,
    /// Its up.
    pub up: Vector,
    /// How fast it moves, in units a second, for Doppler.
    pub velocity: Vector,
}

impl Default for Listener {
    fn default() -> Self {
        Listener {
            position: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            velocity: [0.0; 3],
        }
    }
}

/// A voice in space.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Emitter {
    /// Its position.
    pub position: Vector,
    /// How fast it moves, in units a second, for Doppler.
    pub velocity: Vector,
    /// The way it faces, if it's directional; at full level inside half of `inner` radians of it,
    /// at `outside` beyond half of `outer`, and in between in between.
    pub direction: Option<Vector>,
    /// The inner cone's whole angle, in radians.
    pub inner: f64,
    /// The outer cone's whole angle, in radians.
    pub outer: f64,
    /// The gain outside the outer cone.
    pub outside: f64,
    /// How distance turns it down.
    pub distance: Distance,
    /// The distance it is at full level within.
    pub reference: f64,
    /// The distance beyond which it gets no quieter.
    pub maximum: f64,
    /// How quickly it gets quieter.
    pub rolloff: f64,
    /// How much its and the listener's motion shift its pitch: 0 not at all, 1 as sound does.
    pub doppler: f64,
    /// A further gain the application works out: occlusion, obstruction, a wall between.
    pub gain: f64,
    /// A low-pass cutoff the application works out, in hertz: what a wall does to the highs. At
    /// or above half the output's rate, none.
    pub lowpass: f64,
}

impl Default for Emitter {
    fn default() -> Self {
        Emitter {
            position: [0.0; 3],
            velocity: [0.0; 3],
            direction: None,
            inner: std::f64::consts::TAU,
            outer: std::f64::consts::TAU,
            outside: 0.0,
            distance: Distance::Inverse,
            reference: 1.0,
            maximum: f64::INFINITY,
            rolloff: 1.0,
            doppler: 1.0,
            gain: 1.0,
            lowpass: f64::INFINITY,
        }
    }
}

impl Emitter {
    /// Checks every field is one the engine can hear.
    pub(crate) fn check(&self) -> Result<(), AudioError> {
        let finite = |vector: Vector| vector.iter().all(|value| value.is_finite());
        let problem =
            |name: &'static str, value: f64, expected: &'static str| Err(AudioError::Parameter { name, value, expected });
        if !finite(self.position) || !finite(self.velocity) || !self.direction.is_none_or(finite) {
            return problem("position", f64::NAN, "finite vectors for position, velocity, and direction");
        }
        if !(self.reference > 0.0 && self.reference.is_finite()) {
            return problem("reference", self.reference, "a finite distance above 0");
        }
        if self.maximum < self.reference || self.maximum.is_nan() {
            return problem("maximum", self.maximum, "at least the reference distance");
        }
        if !(self.rolloff >= 0.0 && self.rolloff.is_finite()) {
            return problem("rolloff", self.rolloff, "a finite number from 0");
        }
        if !(0.0..=std::f64::consts::TAU).contains(&self.inner)
            || !(self.inner..=std::f64::consts::TAU).contains(&self.outer)
        {
            return problem(
                "cone",
                self.outer,
                "inner and outer angles from 0 to two pi, the outer at least the inner",
            );
        }
        if !(0.0..=1.0).contains(&self.outside) {
            return problem("outside", self.outside, "from 0 to 1");
        }
        if !(self.doppler >= 0.0 && self.doppler.is_finite()) {
            return problem("doppler", self.doppler, "a finite number from 0");
        }
        if !(self.gain >= 0.0 && self.gain.is_finite()) {
            return problem("gain", self.gain, "a finite number from 0");
        }
        if self.lowpass <= 0.0 || self.lowpass.is_nan() {
            return problem("lowpass", self.lowpass, "a frequency above 0");
        }
        Ok(())
    }
}

/// The space a context's voices are in: its listener, the speed of sound in the world's units,
/// and its handedness.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Space {
    /// The listener.
    pub listener: Listener,
    /// Units a second.
    pub speed: f64,
    /// Which way the axes turn.
    pub handedness: Handedness,
}

impl Default for Space {
    fn default() -> Self {
        Space {
            listener: Listener::default(),
            speed: 343.0,
            handedness: Handedness::Right,
        }
    }
}

/// What a voice's place does to it: a gain, a pan, and a playback-rate factor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Heard {
    /// Distance, cone, and the application's own gain together.
    pub gain: f64,
    /// `-1` left to `1` right, from the listener's frame.
    pub pan: f64,
    /// What Doppler multiplies its playback rate by.
    pub doppler: f64,
}

impl Space {
    /// How `emitter` sounds from where the listener is.
    #[must_use]
    pub fn hear(&self, emitter: &Emitter) -> Heard {
        let listener = &self.listener;
        let offset = minus(emitter.position, listener.position);
        let distance = length(offset);
        let toward = unit(offset);
        let right = match self.handedness {
            Handedness::Right => cross(listener.forward, listener.up),
            Handedness::Left => cross(listener.up, listener.forward),
        };
        let pan = match (toward, unit(right)) {
            (Some(toward), Some(right)) => dot(toward, right).clamp(-1.0, 1.0),
            _ => 0.0,
        };
        let gain = Space::attenuation(emitter, distance) * Space::cone(emitter, toward) * emitter.gain;
        Heard {
            gain,
            pan,
            doppler: self.doppler(emitter, toward),
        }
    }

    fn attenuation(emitter: &Emitter, distance: f64) -> f64 {
        let near = distance.clamp(emitter.reference, emitter.maximum);
        let gain = match emitter.distance {
            Distance::Inverse => emitter.reference / (emitter.reference + emitter.rolloff * (near - emitter.reference)),
            Distance::Linear if emitter.maximum.is_finite() => {
                1.0 - emitter.rolloff * (near - emitter.reference) / (emitter.maximum - emitter.reference).max(1e-12)
            }
            // A linear model with no maximum has nowhere to fall to.
            Distance::None | Distance::Linear => 1.0,
            Distance::Exponential => (near / emitter.reference).powf(-emitter.rolloff),
        };
        gain.clamp(0.0, 1.0)
    }

    /// The cone's gain for a listener `toward` the emitter (from the listener).
    fn cone(emitter: &Emitter, toward: Option<Vector>) -> f64 {
        let (Some(direction), Some(toward)) = (emitter.direction.and_then(unit), toward) else {
            return 1.0;
        };
        // From the emitter, the listener is the other way.
        let angle = (-dot(direction, toward)).clamp(-1.0, 1.0).acos();
        let (inner, outer) = (emitter.inner / 2.0, emitter.outer / 2.0);
        if angle <= inner {
            return 1.0;
        }
        if angle >= outer {
            return emitter.outside;
        }
        1.0 + (emitter.outside - 1.0) * (angle - inner) / (outer - inner)
    }

    /// OpenAL 1.1's Doppler: each velocity along the line from the listener to the emitter, held
    /// under the speed of sound.
    fn doppler(&self, emitter: &Emitter, toward: Option<Vector>) -> f64 {
        let Some(toward) = toward.filter(|_| emitter.doppler > 0.0 && self.speed > 0.0) else {
            return 1.0;
        };
        let limit = self.speed / emitter.doppler * 0.99;
        let listener = dot(self.listener.velocity, toward).clamp(-limit, limit);
        let source = dot(emitter.velocity, toward).clamp(-limit, limit);
        // OpenAL measures both along the line from the emitter to the listener, the other way
        // from `toward`: closing on each other raises the pitch.
        let factor = (self.speed - emitter.doppler * -listener) / (self.speed - emitter.doppler * -source);
        factor.clamp(0.1, 10.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(position: Vector) -> Emitter {
        Emitter {
            position,
            ..Emitter::default()
        }
    }

    #[test]
    fn a_voice_to_the_right_pans_right_and_gets_quieter_with_distance() {
        let space = Space::default();
        let right = space.hear(&at([2.0, 0.0, 0.0]));
        assert!((right.pan - 1.0).abs() < 1e-9);
        assert!((right.gain - 0.5).abs() < 1e-9, "inverse distance at twice the reference");
        let ahead = space.hear(&at([0.0, 0.0, -1.0]));
        assert!(ahead.pan.abs() < 1e-9 && (ahead.gain - 1.0).abs() < 1e-9);
        let left_handed = Space {
            handedness: Handedness::Left,
            listener: Listener {
                forward: [0.0, 0.0, 1.0],
                ..Listener::default()
            },
            ..Space::default()
        };
        assert!((left_handed.hear(&at([2.0, 0.0, 0.0])).pan - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_distance_models_clamp_between_reference_and_maximum() {
        let space = Space::default();
        let linear = Emitter {
            distance: Distance::Linear,
            maximum: 11.0,
            ..at([6.0, 0.0, 0.0])
        };
        assert!((space.hear(&linear).gain - 0.5).abs() < 1e-9);
        let far = Emitter {
            position: [100.0, 0.0, 0.0],
            ..linear
        };
        assert!(space.hear(&far).gain.abs() < 1e-9);
        let exponential = Emitter {
            distance: Distance::Exponential,
            rolloff: 2.0,
            ..at([4.0, 0.0, 0.0])
        };
        assert!((space.hear(&exponential).gain - 1.0 / 16.0).abs() < 1e-9);
        let close = at([0.1, 0.0, 0.0]);
        assert!(
            (space.hear(&close).gain - 1.0).abs() < 1e-9,
            "never louder than at the reference"
        );
    }

    #[test]
    fn a_cone_turns_down_a_voice_facing_away() {
        let space = Space::default();
        let facing = Emitter {
            direction: Some([0.0, 0.0, 1.0]),
            inner: std::f64::consts::FRAC_PI_2,
            outer: std::f64::consts::PI,
            outside: 0.25,
            distance: Distance::None,
            ..at([0.0, 0.0, -5.0])
        };
        assert!((space.hear(&facing).gain - 1.0).abs() < 1e-9, "pointed at the listener");
        let away = Emitter {
            direction: Some([0.0, 0.0, -1.0]),
            ..facing
        };
        assert!((space.hear(&away).gain - 0.25).abs() < 1e-9);
    }

    #[test]
    fn an_approaching_voice_is_higher_and_a_receding_one_lower() {
        let space = Space::default();
        let approaching = Emitter {
            velocity: [0.0, 0.0, 34.3],
            ..at([0.0, 0.0, -100.0])
        };
        let receding = Emitter {
            velocity: [0.0, 0.0, -34.3],
            ..approaching
        };
        assert!((space.hear(&approaching).doppler - 1.0 / 0.9).abs() < 1e-9);
        assert!((space.hear(&receding).doppler - 1.0 / 1.1).abs() < 1e-9);
        let still = Emitter {
            doppler: 0.0,
            ..approaching
        };
        assert!((space.hear(&still).doppler - 1.0).abs() < 1e-9);
    }
}
