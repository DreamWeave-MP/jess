//! Reading a source between its frames: a windowed sinc, widened when the source is read faster
//! than the output plays it.
//!
//! A voice whose source frames don't land on output frames (another rate, a playback rate, a
//! Doppler shift) reads each output sample as a weighted sum of the source frames around it.
//! The weights are a sinc with zero crossings on whole source frames, under a Kaiser window
//! [`HALF`] crossings wide on each side, read from a table at [`PER_UNIT`] points a crossing and
//! interpolated between them. Its zero crossings fall on source frames, so a source read slower
//! than the output (upsampling) passes through its own samples exactly where an output frame
//! lands on one.
//!
//! Read faster than the output (downsampling, or a rate above one), the source holds frequencies
//! the output can't, and the plain kernel would fold them back down as aliases. So the kernel is
//! stretched by the step, which lowers its cutoff to the output's Nyquist, and reads that many
//! times the frames. The stretch stops at [`MAX_STRETCH`]: a voice read faster than that
//! aliases again, past where any ordinary playback goes, rather than reading ever more frames.
//! Each sum is divided by the sum of its weights, so a stretched kernel between table points
//! still passes a constant through unchanged.
//!
//! Against the cutoff, the kernel is flat to 0.75, 1.6 dB down at 0.9, 6 dB down at the cutoff,
//! 50 dB down at 1.25, and about 80 dB down from 1.5 on: what aliases louder than -50 dB lands
//! in the top quarter of the output's band.

// The weights are sums of products. `mul_add` is a libm call on a CPU built for without FMA,
// many times slower than the plain expression.
#![allow(clippy::suboptimal_flops)]

use std::sync::LazyLock;

/// Zero crossings of the sinc on each side of the point read.
pub const HALF: usize = 8;

/// Table points between one zero crossing and the next.
pub const PER_UNIT: usize = 64;

/// The most the kernel widens: the step past which a faster read aliases.
pub const MAX_STRETCH: f32 = 4.0;

/// The most source frames a read reaches on either side of the frame it's on.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // A small whole number.
pub const REACH: usize = HALF * MAX_STRETCH as usize;

/// The Kaiser window's shape: about 70 dB down on its side lobes.
const BETA: f64 = 7.0;

/// One side of the kernel, from its centre out to [`HALF`], and one point past it (zero) so
/// interpolating the last stretch reads no further.
static TABLE: LazyLock<Box<[f32]>> = LazyLock::new(|| {
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // Small table indices; f32 weights.
    (0..=HALF * PER_UNIT + 1)
        .map(|point| {
            let distance = point as f64 / PER_UNIT as f64;
            if distance >= HALF as f64 {
                return 0.0;
            }
            (sinc(distance) * kaiser(distance / HALF as f64)) as f32
        })
        .collect()
});

fn sinc(distance: f64) -> f64 {
    if distance == 0.0 {
        return 1.0;
    }
    let angle = std::f64::consts::PI * distance;
    angle.sin() / angle
}

/// The Kaiser window at `position` across its half width, from 0 at the centre to 1 at the edge.
fn kaiser(position: f64) -> f64 {
    bessel(BETA * (1.0 - position * position).max(0.0).sqrt()) / bessel(BETA)
}

/// The zeroth-order modified Bessel function of the first kind, by its power series.
fn bessel(value: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let quarter = value * value / 4.0;
    for order in 1..40 {
        term *= quarter / f64::from(order * order);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// The kernel's table. The first call builds it, so a context builds it on the control side
/// before any engine reads it, and rendering never does.
pub fn table() -> &'static [f32] {
    &TABLE
}

/// The kernel's weight `distance` zero crossings from its centre.
#[inline]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
// A non-negative table position below the table's length.
pub fn weight(table: &[f32], distance: f32) -> f32 {
    let point = distance.abs() * PER_UNIT as f32;
    let index = point as usize;
    if index >= HALF * PER_UNIT {
        return 0.0;
    }
    let between = point - index as f32;
    table[index] + between * (table[index + 1] - table[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernel_is_one_at_its_centre_and_zero_on_every_other_crossing() {
        let table = table();
        assert_eq!(weight(table, 0.0), 1.0);
        for crossing in 1..=HALF {
            #[allow(clippy::cast_precision_loss)] // Small whole numbers.
            let distance = crossing as f32;
            assert!(weight(table, distance).abs() < 1e-6, "{crossing}");
            assert!(weight(table, -distance).abs() < 1e-6, "{crossing}");
        }
        assert_eq!(weight(table, 8.5), 0.0);
    }
}
