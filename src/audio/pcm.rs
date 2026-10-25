//! PCM: decoded audio in memory, shared by every voice that plays it.

use std::fmt;
use std::str::FromStr;

use crate::audio::{AudioError, MAX_CHANNELS, MAX_RATE};

/// How one sample is stored in interleaved bytes. Every format is little-endian; `U8` is
/// unsigned with its midpoint at 128, the rest are signed or IEEE floats.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SampleFormat {
    /// Unsigned 8-bit.
    U8,
    /// Signed 16-bit.
    S16,
    /// Signed 24-bit, packed in three bytes.
    S24,
    /// Signed 32-bit.
    S32,
    /// 32-bit float.
    F32,
    /// 64-bit float.
    F64,
}

impl SampleFormat {
    /// Bytes per sample.
    #[must_use]
    pub const fn size(self) -> usize {
        match self {
            SampleFormat::U8 => 1,
            SampleFormat::S16 => 2,
            SampleFormat::S24 => 3,
            SampleFormat::S32 | SampleFormat::F32 => 4,
            SampleFormat::F64 => 8,
        }
    }

    /// The name scripts use: `u8`, `s16`, `s24`, `s32`, `f32`, `f64`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            SampleFormat::U8 => "u8",
            SampleFormat::S16 => "s16",
            SampleFormat::S24 => "s24",
            SampleFormat::S32 => "s32",
            SampleFormat::F32 => "f32",
            SampleFormat::F64 => "f64",
        }
    }

    /// The sample at the start of `bytes`, which holds at least [`size`](Self::size) of them,
    /// scaled so integer full scale is `-1.0..1.0`.
    #[allow(clippy::cast_possible_truncation)] // An f64 sample narrows to the engine's f32.
    fn read(self, bytes: &[u8]) -> f32 {
        match self {
            SampleFormat::U8 => (f32::from(bytes[0]) - 128.0) / 128.0,
            SampleFormat::S16 => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32768.0,
            SampleFormat::S24 => {
                // Shift the three bytes into the top of an i32 so the sign extends.
                let value = i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]) >> 8;
                to_f32(value) / 8_388_608.0
            }
            SampleFormat::S32 => {
                let value = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                (f64::from(value) / 2_147_483_648.0) as f32
            }
            SampleFormat::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            SampleFormat::F64 => {
                f64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]) as f32
            }
        }
    }
}

/// A 24-bit integer as an f32, exactly: every value fits the mantissa.
#[allow(clippy::cast_precision_loss)]
const fn to_f32(value: i32) -> f32 {
    value as f32
}

impl fmt::Display for SampleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for SampleFormat {
    type Err = AudioError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Ok(match name {
            "u8" => SampleFormat::U8,
            "s16" => SampleFormat::S16,
            "s24" => SampleFormat::S24,
            "s32" => SampleFormat::S32,
            "f32" => SampleFormat::F32,
            "f64" => SampleFormat::F64,
            other => return Err(AudioError::UnknownFormat(other.to_owned())),
        })
    }
}

/// Decoded audio: planar 32-bit float channels at one sample rate. Immutable once made; voices
/// share one behind an `Arc`, so playing it a hundred times copies nothing.
#[derive(Clone, PartialEq)]
pub struct Pcm {
    rate: u32,
    channels: usize,
    frames: usize,
    /// `channels × frames` samples, one channel after another.
    samples: Box<[f32]>,
}

impl fmt::Debug for Pcm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pcm")
            .field("rate", &self.rate)
            .field("channels", &self.channels())
            .field("frames", &self.frames)
            .finish_non_exhaustive()
    }
}

impl Pcm {
    /// PCM from interleaved bytes: `channels` samples per frame, each stored as `format`.
    ///
    /// # Errors
    ///
    /// [`AudioError::Rate`] or [`AudioError::Channels`] out of range, and
    /// [`AudioError::PartialFrame`] when the bytes don't divide into frames.
    pub fn from_interleaved(bytes: &[u8], format: SampleFormat, channels: usize, rate: u32) -> Result<Pcm, AudioError> {
        check_layout(channels, rate)?;
        let frame = format.size() * channels;
        if !bytes.len().is_multiple_of(frame) {
            return Err(AudioError::PartialFrame {
                bytes: bytes.len(),
                frame,
                format,
            });
        }
        let frames = bytes.len() / frame;
        let mut samples = vec![0.0; frames * channels].into_boxed_slice();
        for (index, bytes) in bytes.chunks_exact(format.size()).enumerate() {
            let (frame, channel) = (index / channels, index % channels);
            samples[channel * frames + frame] = format.read(bytes);
        }
        Ok(Pcm {
            rate,
            channels,
            frames,
            samples,
        })
    }

    /// PCM from one slice of samples per channel, all the same length.
    ///
    /// # Errors
    ///
    /// As [`from_interleaved`](Self::from_interleaved); channels of different lengths are a
    /// [`AudioError::PartialFrame`].
    pub fn from_planar(channels: &[&[f32]], rate: u32) -> Result<Pcm, AudioError> {
        check_layout(channels.len(), rate)?;
        let frames = channels[0].len();
        if let Some(uneven) = channels.iter().find(|channel| channel.len() != frames) {
            return Err(AudioError::PartialFrame {
                bytes: uneven.len() * 4,
                frame: frames * 4,
                format: SampleFormat::F32,
            });
        }
        Ok(Pcm {
            rate,
            channels: channels.len(),
            frames,
            samples: channels.concat().into_boxed_slice(),
        })
    }

    /// Frames per second.
    #[must_use]
    pub const fn rate(&self) -> u32 {
        self.rate
    }

    /// Samples per frame.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// The length in frames.
    #[must_use]
    pub const fn frames(&self) -> usize {
        self.frames
    }

    /// One channel's samples.
    ///
    /// # Panics
    ///
    /// If `channel` is not below [`channels`](Self::channels).
    #[must_use]
    pub fn channel(&self, channel: usize) -> &[f32] {
        &self.samples[channel * self.frames..(channel + 1) * self.frames]
    }
}

pub fn check_layout(channels: usize, rate: u32) -> Result<(), AudioError> {
    if rate == 0 || rate > MAX_RATE {
        return Err(AudioError::Rate(rate));
    }
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(AudioError::Channels(channels));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_formats_scale_to_full_range() {
        let pcm = Pcm::from_interleaved(&[0, 128, 255], SampleFormat::U8, 1, 8000).unwrap();
        assert_eq!(pcm.channel(0), [-1.0, 0.0, 127.0 / 128.0]);

        let bytes: Vec<u8> = [i16::MIN, 0, 16384].iter().flat_map(|value| value.to_le_bytes()).collect();
        let pcm = Pcm::from_interleaved(&bytes, SampleFormat::S16, 1, 8000).unwrap();
        assert_eq!(pcm.channel(0), [-1.0, 0.0, 0.5]);

        // -2^23 and 2^22, three bytes each.
        let pcm = Pcm::from_interleaved(&[0, 0, 0x80, 0, 0, 0x40], SampleFormat::S24, 1, 8000).unwrap();
        assert_eq!(pcm.channel(0), [-1.0, 0.5]);
    }

    #[test]
    fn interleaved_frames_become_planar_channels() {
        let bytes: Vec<u8> = [0.25f32, -0.25, 0.5, -0.5]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let pcm = Pcm::from_interleaved(&bytes, SampleFormat::F32, 2, 48000).unwrap();
        assert_eq!((pcm.channels(), pcm.frames()), (2, 2));
        assert_eq!(pcm.channel(0), [0.25, 0.5]);
        assert_eq!(pcm.channel(1), [-0.25, -0.5]);
    }

    #[test]
    fn partial_frames_and_bad_layouts_are_refused() {
        assert!(matches!(
            Pcm::from_interleaved(&[0; 3], SampleFormat::S16, 1, 8000),
            Err(AudioError::PartialFrame { .. })
        ));
        assert_eq!(
            Pcm::from_interleaved(&[], SampleFormat::S16, 0, 8000),
            Err(AudioError::Channels(0))
        );
        assert_eq!(Pcm::from_interleaved(&[], SampleFormat::S16, 1, 0), Err(AudioError::Rate(0)));
        assert!(matches!("s12".parse::<SampleFormat>(), Err(AudioError::UnknownFormat(_))));
    }
}
