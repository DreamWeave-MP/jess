//! Streams: audio written while it plays, by whatever thread has it, a block ahead or a minute.
//!
//! A stream is a bounded ring of interleaved frames between one writer and the engine. The writer
//! (a script generating sound, a worker decoding a file) writes what fits and learns how much did;
//! the voice playing the stream reads what is there, and when the ring runs dry it plays silence
//! and counts the frames it missed. Nothing blocks either side: the ring is atomics, and a writer
//! that wants to wait for room sleeps for as long as the engine takes to make it.
//!
//! A stream a feed writes can seek: asked by the engine, ahead of the seek's frame when it can be,
//! the feed repositions its decoder and marks where in the ring what it writes from then on starts
//! (an epoch and a position), and the voice playing it lets go of everything before that mark when
//! its seek's frame comes. If the mark isn't there yet, the voice keeps time in silence until it
//! is. A stream a script writes has no decoder to reposition, and refuses to seek.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::audio::pcm::check_layout;
use crate::audio::{AudioError, MAX_RATE};

/// The longest a stream may be, in seconds of its own rate.
pub const LONGEST: f64 = 60.0;

/// A ring of interleaved frames. See the module docs.
#[derive(Debug)]
pub struct Stream {
    id: u64,
    channels: usize,
    rate: u32,
    /// Frames it holds.
    capacity: u64,
    /// Each sample's bits, so writer and engine share it without a lock.
    samples: Box<[AtomicU32]>,
    /// Frames ever written, and ever read: both only grow.
    written: AtomicU64,
    read: AtomicU64,
    /// Silent frames played for want of data, all told.
    missed: AtomicU64,
    /// Of those, the ones played since a seek's frame because the writer hadn't got there yet.
    late: AtomicU64,
    /// The writer is done: once read to the end, the voice playing it ends.
    finished: AtomicBool,
    /// A voice is playing it: a stream plays once.
    claimed: AtomicBool,
    /// A writer is writing: a stream has one at a time.
    writing: AtomicBool,
    /// A feed writes it, which can seek.
    seekable: AtomicBool,
    /// The latest seek asked for: its epoch, counting from 1, and the source frame it goes to.
    wanted: AtomicU64,
    wanted_frame: AtomicU64,
    /// The latest seek the writer carried out: its epoch, and the ring frame what it wrote from
    /// there starts at. Each pair is written number first and read epoch, number, epoch, so a
    /// reader never pairs an epoch with another's number.
    marked: AtomicU64,
    marked_at: AtomicU64,
}

/// What a stream has done.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StreamStats {
    /// Frames written, all told.
    pub written: u64,
    /// Frames played, all told.
    pub read: u64,
    /// Frames there is room for now.
    pub room: u64,
    /// Silent frames played for want of data.
    pub missed: u64,
    /// Of `missed`, the frames played since a seek's frame before its feed had decoded from the
    /// new frame: none for a seek prepared in time.
    pub late: u64,
    /// Whether the writer has finished.
    pub finished: bool,
}

/// Every live stream by id, so a handle sent to a worker finds the same one there.
static STREAMS: Mutex<Option<HashMap<u64, Weak<Stream>>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

impl Stream {
    /// A stream of `channels` channels at `rate` frames per second holding `seconds` of audio.
    ///
    /// # Errors
    ///
    /// [`AudioError::Rate`] or [`AudioError::Channels`] out of range, or
    /// [`AudioError::Parameter`] for a length that isn't above 0 and at most [`LONGEST`].
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Seconds checked in range.
    pub fn new(channels: usize, rate: u32, seconds: f64) -> Result<Arc<Stream>, AudioError> {
        check_layout(channels, rate)?;
        if !(seconds > 0.0 && seconds <= LONGEST) {
            return Err(AudioError::Parameter {
                name: "stream length",
                value: seconds,
                expected: "seconds above 0 and at most 60",
            });
        }
        let capacity = ((seconds * f64::from(rate.min(MAX_RATE))).ceil() as u64).max(1);
        let stream = Arc::new(Stream {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            channels,
            rate,
            capacity,
            samples: std::iter::repeat_with(|| AtomicU32::new(0))
                .take(usize::try_from(capacity).unwrap_or(usize::MAX) * channels)
                .collect(),
            written: AtomicU64::new(0),
            read: AtomicU64::new(0),
            missed: AtomicU64::new(0),
            late: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            claimed: AtomicBool::new(false),
            writing: AtomicBool::new(false),
            seekable: AtomicBool::new(false),
            wanted: AtomicU64::new(0),
            wanted_frame: AtomicU64::new(0),
            marked: AtomicU64::new(0),
            marked_at: AtomicU64::new(0),
        });
        let mut registry = STREAMS.lock().unwrap_or_else(PoisonError::into_inner);
        let streams = registry.get_or_insert_with(HashMap::new);
        streams.retain(|_, stream| stream.strong_count() > 0);
        streams.insert(stream.id, Arc::downgrade(&stream));
        drop(registry);
        Ok(stream)
    }

    /// The live stream with `id`, from any thread.
    #[must_use]
    pub fn find(id: u64) -> Option<Arc<Stream>> {
        let streams = STREAMS.lock().unwrap_or_else(PoisonError::into_inner);
        streams.as_ref()?.get(&id)?.upgrade()
    }

    /// Its id, for a handle another thread can find it by.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Samples per frame.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// Frames per second.
    #[must_use]
    pub const fn rate(&self) -> u32 {
        self.rate
    }

    /// Frames there is room for now.
    #[must_use]
    pub fn room(&self) -> u64 {
        let written = self.written.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        self.capacity - (written - read)
    }

    /// Writes as many whole frames of interleaved `samples` as there is room for, and returns
    /// how many. A finished stream takes nothing more.
    ///
    /// # Errors
    ///
    /// [`AudioError::PartialFrame`] for samples that aren't whole frames, and
    /// [`AudioError::StreamBusy`] while another thread is writing.
    pub fn write(&self, samples: &[f32]) -> Result<usize, AudioError> {
        if !samples.len().is_multiple_of(self.channels) {
            return Err(AudioError::PartialFrame {
                bytes: samples.len() * 4,
                frame: self.channels * 4,
                format: crate::audio::SampleFormat::F32,
            });
        }
        if self.writing.swap(true, Ordering::Acquire) {
            return Err(AudioError::StreamBusy);
        }
        let accepted = if self.finished.load(Ordering::Relaxed) {
            0
        } else {
            let frames = (samples.len() / self.channels).min(usize::try_from(self.room()).unwrap_or(usize::MAX));
            let start = self.written.load(Ordering::Relaxed);
            for (index, frame) in samples.chunks_exact(self.channels).take(frames).enumerate() {
                let at = self.slot(start + index as u64);
                for (channel, sample) in frame.iter().enumerate() {
                    self.samples[at + channel].store(sample.to_bits(), Ordering::Relaxed);
                }
            }
            self.written.store(start + frames as u64, Ordering::Release);
            frames
        };
        self.writing.store(false, Ordering::Release);
        Ok(accepted)
    }

    /// Says the writer is done: once played to its end, the voice playing it ends.
    pub fn finish(&self) {
        self.finished.store(true, Ordering::Release);
    }

    /// Waits until there is room for `frames` frames, or `timeout` passes; true if there is. It
    /// sleeps as long as the engine, playing at the stream's rate, takes to make the room, so a
    /// stream that nothing plays (or an offline context nobody renders) waits the whole timeout.
    pub fn wait(&self, frames: u64, timeout: Duration) -> bool {
        let frames = frames.min(self.capacity);
        let deadline = Instant::now() + timeout;
        loop {
            let room = self.room();
            if room >= frames {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            #[allow(clippy::cast_precision_loss)] // A wait in seconds.
            let needed = Duration::from_secs_f64((frames - room) as f64 / f64::from(self.rate));
            std::thread::sleep(needed.max(Duration::from_micros(500)).min(deadline - now));
        }
    }

    /// What it has done.
    #[must_use]
    pub fn stats(&self) -> StreamStats {
        StreamStats {
            written: self.written.load(Ordering::Relaxed),
            read: self.read.load(Ordering::Relaxed),
            room: self.room(),
            missed: self.missed.load(Ordering::Relaxed),
            late: self.late.load(Ordering::Relaxed),
            finished: self.finished.load(Ordering::Relaxed),
        }
    }

    /// Takes the stream for a voice to play; false if one already has.
    pub(crate) fn claim(&self) -> bool {
        !self.claimed.swap(true, Ordering::AcqRel)
    }

    /// The first sample of frame `frame` in the ring.
    fn slot(&self, frame: u64) -> usize {
        usize::try_from(frame % self.capacity).unwrap_or(0) * self.channels
    }

    /// For the engine: copies up to `frames` frames into `into`, one block of `stride` samples
    /// per channel starting at `offset` in each, and returns how many it had. Only the voice
    /// playing the stream reads it.
    pub(crate) fn take(&self, into: &mut [f32], stride: usize, offset: usize, frames: usize, epoch: u64) -> usize {
        let frames = frames.min(usize::try_from(self.readable(epoch)).unwrap_or(usize::MAX));
        let read = self.read.load(Ordering::Relaxed);
        for index in 0..frames {
            let at = self.slot(read + index as u64);
            for channel in 0..self.channels {
                into[channel * stride + offset + index] = f32::from_bits(self.samples[at + channel].load(Ordering::Relaxed));
            }
        }
        self.read.store(read + frames as u64, Ordering::Release);
        frames
    }

    /// For the engine: lets up to `frames` frames go unread, as a voice that keeps time without
    /// being mixed does, and returns how many it had.
    pub(crate) fn skip(&self, frames: u64, epoch: u64) -> u64 {
        let frames = frames.min(self.readable(epoch));
        let read = self.read.load(Ordering::Relaxed);
        self.read.store(read + frames, Ordering::Release);
        frames
    }

    /// For the engine: frames written and not yet read that a reader playing seek `epoch` may
    /// take.
    pub(crate) fn unread(&self, epoch: u64) -> u64 {
        self.readable(epoch)
    }

    /// For the engine: counts frames played as silence for want of data.
    pub(crate) fn miss(&self, frames: u64) {
        self.missed.fetch_add(frames, Ordering::Relaxed);
    }

    /// For the engine: counts frames played as silence since a seek's frame, waiting for its
    /// writer, as missed and as late.
    pub(crate) fn miss_late(&self, frames: u64) {
        self.miss(frames);
        self.late.fetch_add(frames, Ordering::Relaxed);
    }

    /// Frames ever read.
    pub(crate) fn read_frames(&self) -> u64 {
        self.read.load(Ordering::Relaxed)
    }

    /// Frames it holds.
    #[must_use]
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }

    /// True once the writer has said it is done.
    pub(crate) fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Gives the stream back for another voice, when the voice meant to play it never started.
    pub(crate) fn release(&self) {
        self.claimed.store(false, Ordering::Release);
    }

    /// True while a feed writes it, so it can seek.
    #[must_use]
    pub fn seekable(&self) -> bool {
        self.seekable.load(Ordering::Acquire)
    }

    /// For a feed: says whether it writes the stream, and so whether it can seek.
    pub(crate) fn set_seekable(&self, seekable: bool) {
        self.seekable.store(seekable, Ordering::Release);
    }

    /// The latest seek asked for, from 1; 0 for none.
    pub(crate) fn wanted(&self) -> u64 {
        self.wanted.load(Ordering::Acquire)
    }

    /// For the engine: asks the writer to go to source frame `frame`, and returns the seek's
    /// epoch. Only the engine asks, so the epochs count up in the order it asks.
    pub(crate) fn request_seek(&self, frame: u64) -> u64 {
        let epoch = self.wanted.load(Ordering::Relaxed) + 1;
        self.wanted_frame.store(frame, Ordering::Release);
        self.wanted.store(epoch, Ordering::Release);
        epoch
    }

    /// For the writer: the latest seek asked for after epoch `handled`, and its frame.
    pub(crate) fn seek_wanted(&self, handled: u64) -> Option<(u64, u64)> {
        loop {
            let epoch = self.wanted.load(Ordering::Acquire);
            if epoch <= handled {
                return None;
            }
            let frame = self.wanted_frame.load(Ordering::Acquire);
            if self.wanted.load(Ordering::Acquire) == epoch {
                return Some((epoch, frame));
            }
        }
    }

    /// For the writer: what it writes from now on is from seek `epoch`. A finished stream it
    /// seeks in is unfinished again first.
    pub(crate) fn mark(&self, epoch: u64) {
        self.finished.store(false, Ordering::Release);
        self.marked_at.store(self.written.load(Ordering::Relaxed), Ordering::Release);
        self.marked.store(epoch, Ordering::Release);
    }

    /// The latest seek the writer carried out: its epoch and the ring frame it starts at.
    pub(crate) fn marked(&self) -> (u64, u64) {
        loop {
            let epoch = self.marked.load(Ordering::Acquire);
            let at = self.marked_at.load(Ordering::Acquire);
            if self.marked.load(Ordering::Acquire) == epoch {
                return (epoch, at);
            }
        }
    }

    /// The frames written and not yet read that a reader playing seek `epoch` may take: up to a
    /// later seek's mark, which it doesn't hear until that seek's frame.
    fn readable(&self, epoch: u64) -> u64 {
        let written = self.written.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Relaxed);
        let (marked, at) = self.marked();
        let end = if marked > epoch { at.min(written) } else { written };
        end.saturating_sub(read)
    }

    /// True once it is finished and everything written has been read: its voice has played it to
    /// the end, and nothing is left to seek in.
    pub(crate) fn drained(&self) -> bool {
        self.finished() && self.read.load(Ordering::Acquire) == self.written.load(Ordering::Acquire)
    }

    /// For the engine: lets go of everything before ring frame `at`, where a seek's frames start.
    pub(crate) fn discard_to(&self, at: u64) {
        let read = self.read.load(Ordering::Relaxed);
        self.read.store(read.max(at), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_takes_what_fits_and_gives_back_what_was_written() {
        let stream = Stream::new(2, 4, 1.0).unwrap();
        assert_eq!(stream.write(&[1.0, -1.0, 2.0, -2.0, 3.0, -3.0]).unwrap(), 3);
        assert_eq!(
            stream.write(&[4.0, -4.0, 5.0, -5.0]).unwrap(),
            1,
            "four frames is all it holds"
        );
        assert_eq!(stream.room(), 0);
        let mut into = [0.0f32; 8];
        assert_eq!(stream.take(&mut into, 4, 0, 4, 0), 4);
        assert_eq!(into, [1.0, 2.0, 3.0, 4.0, -1.0, -2.0, -3.0, -4.0]);
        assert_eq!(stream.take(&mut into, 4, 0, 4, 0), 0, "dry");
        assert!(matches!(stream.write(&[1.0]), Err(AudioError::PartialFrame { .. })));
    }

    #[test]
    fn a_handle_finds_its_stream_while_it_lives() {
        let stream = Stream::new(1, 8000, 0.1).unwrap();
        let id = stream.id();
        assert!(Arc::ptr_eq(&Stream::find(id).unwrap(), &stream));
        drop(stream);
        assert!(Stream::find(id).is_none());
    }

    #[test]
    fn waiting_for_room_that_never_comes_gives_up_at_the_timeout() {
        let stream = Stream::new(1, 1000, 0.01).unwrap();
        stream.write(&[0.0; 10]).unwrap();
        let started = Instant::now();
        assert!(!stream.wait(5, Duration::from_millis(20)));
        assert!(started.elapsed() >= Duration::from_millis(20));
        assert!(stream.wait(0, Duration::ZERO));
    }
}
