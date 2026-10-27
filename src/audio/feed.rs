//! Feeds: a decoder kept ahead of the stream it plays into, on a thread of its own.
//!
//! A script can keep a stream full itself, a block each tick, but then the sound stops when the
//! script does: a long frame, a level loading, a breakpoint. A feed moves that loop off the
//! script. Its thread waits for room in the stream, decodes a block into it, and goes round
//! again, so the stream plays on whatever the script is doing. It knows no formats: it reads
//! whatever implements [`Decode`].
//!
//! A feed ends when its decoder runs out (unless it loops), when it is stopped, when the decoder
//! fails, or when nothing but the feed holds the stream any more: no voice plays it and nothing
//! else has it. Its thread checks for the last two at least every [`POLL`], so dropping a
//! [`Feed`], which stops it and waits, never waits longer than that and one block's decoding.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::audio::{AudioError, Stream};

/// The longest a feed's thread waits for room before it looks again at whether it should end.
pub const POLL: Duration = Duration::from_millis(50);

/// Something a feed reads interleaved frames from.
pub trait Decode: Send {
    /// Fills `out` with as many whole frames as it holds and the source has left, and returns
    /// how many: `0` at the end.
    ///
    /// # Errors
    ///
    /// Why the source can't be read here, in words for a script.
    fn read(&mut self, out: &mut [f32]) -> Result<usize, String>;

    /// Moves to `frame`, for a feed that loops.
    ///
    /// # Errors
    ///
    /// Why the source can't be read from there.
    fn seek(&mut self, frame: u64) -> Result<(), String>;
}

/// How a feed reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FeedSpec {
    /// Frames it decodes at a time.
    pub block: usize,
    /// Where it starts again when the decoder runs out, or `None` to finish the stream there.
    pub looping: Option<u64>,
}

impl Default for FeedSpec {
    fn default() -> Self {
        FeedSpec {
            block: 4096,
            looping: None,
        }
    }
}

/// What a feed has done, as of now.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FeedStatus {
    /// Its thread has ended.
    pub done: bool,
    /// Times it went back to its loop's start.
    pub loops: u64,
    /// Why the decoder failed, if it did; the stream was finished there.
    pub error: Option<String>,
}

#[derive(Debug, Default)]
struct Shared {
    stop: AtomicBool,
    done: AtomicBool,
    loops: AtomicU64,
    error: Mutex<Option<String>>,
}

/// What a feed has done, readable from anywhere while its thread runs, and a way to stop it
/// that doesn't wait.
#[derive(Clone, Debug)]
pub struct FeedWatch(Arc<Shared>);

impl FeedWatch {
    /// What the feed has done.
    #[must_use]
    pub fn status(&self) -> FeedStatus {
        FeedStatus {
            done: self.0.done.load(Ordering::Acquire),
            loops: self.0.loops.load(Ordering::Relaxed),
            error: self.0.error.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        }
    }

    /// Asks the feed to stop, without waiting: its thread ends within [`POLL`] and a block's
    /// decoding, leaving what it wrote in the stream.
    pub fn stop(&self) {
        self.0.stop.store(true, Ordering::Release);
    }
}

/// A decoder feeding a stream on its own thread. See the module docs.
#[derive(Debug)]
pub struct Feed {
    watch: FeedWatch,
    thread: Option<JoinHandle<()>>,
}

impl Feed {
    /// Starts a thread reading `decoder` into `stream`, whose layout the decoder's frames have.
    ///
    /// # Errors
    ///
    /// [`AudioError::Parameter`] for a block of no frames, and [`AudioError::Device`] when the
    /// system won't start a thread.
    pub fn start(decoder: Box<dyn Decode>, stream: Arc<Stream>, spec: FeedSpec) -> Result<Feed, AudioError> {
        if spec.block == 0 {
            return Err(AudioError::Parameter {
                name: "a feed's block",
                value: 0.0,
                expected: "at least one frame",
            });
        }
        let shared = Arc::new(Shared::default());
        let watch = FeedWatch(Arc::clone(&shared));
        let thread = std::thread::Builder::new()
            .name("jess-audio-feed".to_owned())
            .spawn(move || {
                // A decoder that panics fails like one that errs: the stream finishes, and the
                // feed says why.
                let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| run(decoder, &stream, spec, &shared)));
                let failure = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error),
                    Err(_) => Some("the decoder panicked".to_owned()),
                };
                if let Some(error) = failure {
                    *shared.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                    stream.finish();
                }
                shared.done.store(true, Ordering::Release);
            })
            .map_err(|error| AudioError::Device(format!("a feed's thread wouldn't start: {error}")))?;
        Ok(Feed {
            watch,
            thread: Some(thread),
        })
    }

    /// What the feed has done.
    #[must_use]
    pub fn status(&self) -> FeedStatus {
        self.watch.status()
    }

    /// A view of what the feed has done, for whoever doesn't own it.
    #[must_use]
    pub fn watch(&self) -> FeedWatch {
        self.watch.clone()
    }

    /// Stops the feed and waits for its thread. What it already wrote stays in the stream, which
    /// isn't finished: whatever else writes it can go on.
    pub fn stop(&mut self) {
        self.watch.stop();
        if let Some(thread) = self.thread.take() {
            // The thread catches its own panics, so joining has nothing to pass on.
            drop(thread.join());
        }
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.stop();
    }
}

/// How often a feed waiting for room or for a seek looks for one asked for.
const SEEKING: Duration = Duration::from_millis(2);

fn run(decoder: Box<dyn Decode>, stream: &Arc<Stream>, spec: FeedSpec, shared: &Shared) -> Result<(), String> {
    stream.set_seekable(true);
    let mut handled = 0;
    let ran = feed(decoder, stream, spec, shared, &mut handled);
    // A seek asked for that this feed won't carry out still lets its voice go on: it hears what
    // the stream has, and its end.
    stream.set_seekable(false);
    if let Some((epoch, _)) = stream.seek_wanted(handled) {
        stream.mark(epoch);
        stream.finish();
    }
    ran
}

/// True once the feed should end: stopped, or nothing but it holds the stream.
fn over(stream: &Arc<Stream>, shared: &Shared) -> bool {
    shared.stop.load(Ordering::Acquire) || Arc::strong_count(stream) == 1
}

/// Waits up to [`POLL`] for room for `frames`, or for a seek after `handled`; true for the room.
fn room(stream: &Stream, frames: u64, handled: u64) -> bool {
    let deadline = std::time::Instant::now() + POLL;
    loop {
        if stream.seek_wanted(handled).is_some() {
            return false;
        }
        if stream.wait(frames, SEEKING) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
    }
}

fn feed(
    mut decoder: Box<dyn Decode>,
    stream: &Arc<Stream>,
    spec: FeedSpec,
    shared: &Shared,
    handled: &mut u64,
) -> Result<(), String> {
    let channels = stream.channels();
    let mut block = vec![0.0; spec.block * channels];
    // True right after a seek back to the loop's start: a loop that reads nothing from there
    // would go round for ever.
    let mut looped = false;
    'decoding: loop {
        let frames = loop {
            if over(stream, shared) {
                return Ok(());
            }
            if let Some((epoch, frame)) = stream.seek_wanted(*handled) {
                decoder.seek(frame)?;
                stream.mark(epoch);
                *handled = epoch;
                looped = false;
            }
            if room(stream, spec.block as u64, *handled) {
                break decoder.read(&mut block)?;
            }
        };
        if frames == 0 {
            let Some(start) = spec.looping.filter(|_| !looped) else {
                stream.finish();
                // A finished stream still seeks, until its voice has played it to the end.
                while !over(stream, shared) && !stream.drained() {
                    if stream.seek_wanted(*handled).is_some() {
                        continue 'decoding;
                    }
                    std::thread::sleep(SEEKING);
                }
                return Ok(());
            };
            decoder.seek(start)?;
            shared.loops.fetch_add(1, Ordering::Relaxed);
            looped = true;
            continue;
        }
        looped = false;
        let mut written = 0;
        while written < frames {
            if stream.seek_wanted(*handled).is_some() {
                // What is left of the block is from before the seek: no one will hear it.
                continue 'decoding;
            }
            let accepted = stream
                .write(&block[written * channels..frames * channels])
                .map_err(|error| error.to_string())?;
            written += accepted;
            if accepted > 0 {
                continue;
            }
            if stream.stats().finished {
                // Something else finished the stream: there is nowhere left to write.
                return Ok(());
            }
            if over(stream, shared) {
                return Ok(());
            }
            room(stream, (frames - written) as u64, *handled);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::audio::{AudioConfig, OfflineContext, PlaySpec};

    /// A ramp of `frames` mono frames, 0, 1, 2, ...
    struct Ramp {
        frames: u64,
        position: u64,
    }

    impl Decode for Ramp {
        #[allow(clippy::cast_precision_loss)] // Small whole numbers.
        fn read(&mut self, out: &mut [f32]) -> Result<usize, String> {
            let count = (out.len() as u64).min(self.frames - self.position);
            for (index, sample) in out.iter_mut().take(usize::try_from(count).unwrap()).enumerate() {
                *sample = (self.position + index as u64) as f32;
            }
            self.position += count;
            Ok(usize::try_from(count).unwrap())
        }

        fn seek(&mut self, frame: u64) -> Result<(), String> {
            self.position = frame.min(self.frames);
            Ok(())
        }
    }

    /// Renders `frames` frames of `context`, a block at a time once the feed has written well
    /// ahead of it: an offline render runs ahead of any writer that it doesn't wait for, and a
    /// voice reads a few frames past the block it plays.
    fn play(context: &mut OfflineContext, stream: &Stream, frames: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let started = Instant::now();
        while out.len() < frames {
            let ready = stream.stats();
            let waiting = ready.written - ready.read;
            if waiting < 512 && !ready.finished {
                assert!(started.elapsed() < Duration::from_secs(20), "the feed stopped writing");
                std::thread::yield_now();
                continue;
            }
            out.extend(context.render(256.min(frames - out.len())));
        }
        out
    }

    fn mono() -> OfflineContext {
        OfflineContext::new(&AudioConfig {
            channels: 1,
            ..AudioConfig::default()
        })
        .unwrap()
    }

    #[test]
    #[allow(clippy::cast_precision_loss)] // Small whole numbers.
    fn a_feed_decodes_ahead_of_the_voice_and_finishes_the_stream_at_the_end() {
        let mut context = mono();
        let stream = Stream::new(1, 48_000, 0.05).unwrap();
        let voice = context.control().play_stream(&stream, &PlaySpec::default()).unwrap();
        let feed = Feed::start(
            Box::new(Ramp {
                frames: 10_000,
                position: 0,
            }),
            Arc::clone(&stream),
            FeedSpec {
                block: 300,
                looping: None,
            },
        )
        .unwrap();
        let out = play(&mut context, &stream, 10_000);
        let expected: Vec<f32> = (0..10_000).map(|frame| frame as f32).collect();
        assert_eq!(out, expected);
        context.render(256);
        assert_eq!(context.control().state(voice), crate::audio::VoiceState::Ended);
        let started = Instant::now();
        while !feed.status().done {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::yield_now();
        }
        assert_eq!(
            feed.status(),
            FeedStatus {
                done: true,
                loops: 0,
                error: None
            }
        );
    }

    #[test]
    #[allow(clippy::cast_precision_loss)] // Whole numbers below 2^24.
    fn a_feed_seeks_and_its_voice_follows_on_the_seek_s_frame() {
        let mut context = mono();
        let stream = Stream::new(1, 48_000, 0.05).unwrap();
        let voice = context.control().play_stream(&stream, &PlaySpec::default()).unwrap();
        let _feed = Feed::start(
            Box::new(Ramp {
                frames: 100_000,
                position: 0,
            }),
            Arc::clone(&stream),
            FeedSpec {
                block: 300,
                looping: None,
            },
        )
        .unwrap();
        let mut out = play(&mut context, &stream, 1000);
        context.control().seek(voice, 50_000, Some(1500)).unwrap();
        // The feed seeks at once, and marks where its new frames start; the voice plays what it
        // had up to the seek's frame, and never past the mark before then.
        out.extend(context.render(500));
        let started = Instant::now();
        loop {
            let (epoch, at) = stream.marked();
            if epoch == 1 && stream.stats().written >= at + 600 {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "the feed never sought");
            std::thread::yield_now();
        }
        out.extend(context.render(500));
        let before: Vec<f32> = (0..1500).map(|frame| frame as f32).collect();
        let after: Vec<f32> = (50_000..50_500).map(|frame| frame as f32).collect();
        assert_eq!(out[..1500], before);
        assert_eq!(out[1500..], after);
        assert_eq!(context.control().position(voice), Some(50_500));
        assert_eq!(stream.stats().missed, 0, "nothing played as silence");
    }

    #[test]
    #[allow(clippy::cast_precision_loss)] // Small whole numbers.
    fn a_looping_feed_goes_round_without_a_seam() {
        let mut context = mono();
        let stream = Stream::new(1, 48_000, 0.05).unwrap();
        context.control().play_stream(&stream, &PlaySpec::default()).unwrap();
        let feed = Feed::start(
            Box::new(Ramp {
                frames: 1000,
                position: 0,
            }),
            Arc::clone(&stream),
            FeedSpec {
                block: 256,
                looping: Some(200),
            },
        )
        .unwrap();
        let out = play(&mut context, &stream, 3000);
        // 0 to 999, then 200 to 999 again and again.
        let expected: Vec<f32> = (0..3000u64)
            .map(|frame| if frame < 1000 { frame } else { 200 + (frame - 1000) % 800 } as f32)
            .collect();
        assert_eq!(out, expected);
        assert!(feed.status().loops >= 2);
        assert_eq!(stream.stats().missed, 0);
    }

    #[test]
    fn a_feed_ends_when_nothing_else_holds_its_stream_and_stops_when_dropped() {
        let endless = || {
            Box::new(Ramp {
                frames: u64::MAX,
                position: 0,
            })
        };
        let stream = Stream::new(1, 48_000, 0.01).unwrap();
        let feed = Feed::start(endless(), Arc::clone(&stream), FeedSpec::default()).unwrap();
        drop(stream);
        let started = Instant::now();
        while !feed.status().done {
            assert!(started.elapsed() < Duration::from_secs(5), "the feed outlived its stream");
            std::thread::yield_now();
        }

        // Nothing renders this one, so its thread waits for room it never gets, until dropped.
        let stream = Stream::new(1, 48_000, 0.01).unwrap();
        let feed = Feed::start(endless(), Arc::clone(&stream), FeedSpec::default()).unwrap();
        let watch = feed.watch();
        let started = Instant::now();
        drop(feed);
        assert!(watch.status().done);
        assert!(started.elapsed() < Duration::from_secs(1));

        // Asked through a watch, it stops without being waited for.
        let feed = Feed::start(endless(), Arc::clone(&stream), FeedSpec::default()).unwrap();
        feed.watch().stop();
        let started = Instant::now();
        while !feed.status().done {
            assert!(started.elapsed() < Duration::from_secs(5), "the feed didn't stop");
            std::thread::yield_now();
        }
        assert!(
            !stream.stats().finished,
            "a stopped feed leaves the stream to whoever else writes it"
        );
    }

    /// Waits, as a device's clock would, until the feed is as far ahead as it would be playing
    /// in real time: it has carried out the latest seek asked of it, and filled the stream.
    fn keep_up(stream: &Stream, block: u64) {
        let started = Instant::now();
        while stream.marked().0 != stream.wanted() || (stream.room() >= block && !stream.stats().finished) {
            assert!(started.elapsed() < Duration::from_secs(20), "the feed stopped");
            std::thread::yield_now();
        }
    }

    #[test]
    #[allow(clippy::cast_precision_loss)] // Whole numbers below 2^24.
    fn prepared_seeks_however_far_and_often_land_on_their_frames_without_a_gap() {
        let mut context = mono();
        let stream = Stream::new(1, 48_000, 0.25).unwrap();
        let voice = context.control().play_stream(&stream, &PlaySpec::default()).unwrap();
        let _feed = Feed::start(
            Box::new(Ramp {
                frames: 16_000_000,
                position: 0,
            }),
            Arc::clone(&stream),
            FeedSpec {
                block: 1024,
                looping: None,
            },
        )
        .unwrap();
        keep_up(&stream, 1024);
        let lead = context.control().seek_lead(voice).unwrap();
        assert_eq!(lead, 6000, "half the stream");

        // Each seek is stamped further ahead than the stream holds, and the next is sent once it has
        // landed: the feed plays on until a lead before each, and is asked then.
        let destinations = [9_000_000, 120, 15_500_000, 4_000_000, 4_000_100, 11_111_111, 0, 7_777_777];
        let (mut out, mut expected) = (Vec::new(), Vec::new());
        let (mut from, mut since) = (0u64, 0u64);
        for destination in destinations.into_iter().map(Some).chain([None]) {
            let now = out.len() as u64;
            let due = now + 3 * lead + 700;
            if let Some(destination) = destination {
                assert!(context.control().seek(voice, destination, Some(due)).unwrap());
            }
            while (out.len() as u64) < due + 1500 {
                keep_up(&stream, 1024);
                out.extend(context.render(1000));
            }
            let end = out.len() as u64;
            let (switch, next) = destination.map_or((end, from + end - since), |destination| (due, destination));
            expected.extend((since..switch).map(|frame| (from + frame - since) as f32));
            expected.extend((switch..end).map(|frame| (next + frame - switch) as f32));
            (from, since) = (next + end - switch, end);
        }
        assert!(out == expected, "every frame where the timeline says");
        assert_eq!(stream.stats().missed, 0, "nothing played as silence");
        assert_eq!(stream.stats().late, 0);
        assert_eq!(context.control().position(voice), Some(from));
    }

    /// A ramp whose seeks wait until `open`.
    struct Held {
        ramp: Ramp,
        open: Arc<AtomicBool>,
    }

    impl Decode for Held {
        fn read(&mut self, out: &mut [f32]) -> Result<usize, String> {
            self.ramp.read(out)
        }

        fn seek(&mut self, frame: u64) -> Result<(), String> {
            while !self.open.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.ramp.seek(frame)
        }
    }

    #[test]
    #[allow(clippy::cast_precision_loss)] // Whole numbers below 2^24.
    fn a_seek_its_feed_is_late_for_keeps_time_and_joins_where_the_timeline_has_got_to() {
        let mut context = mono();
        let stream = Stream::new(1, 48_000, 0.25).unwrap();
        let voice = context.control().play_stream(&stream, &PlaySpec::default()).unwrap();
        let open = Arc::new(AtomicBool::new(false));
        let _feed = Feed::start(
            Box::new(Held {
                ramp: Ramp {
                    frames: 1_000_000,
                    position: 0,
                },
                open: Arc::clone(&open),
            }),
            Arc::clone(&stream),
            FeedSpec {
                block: 1024,
                looping: None,
            },
        )
        .unwrap();
        keep_up(&stream, 1024);
        let mut out = context.render(1000);

        // Stamped far closer than the lead: the feed is asked at once, and is held up.
        assert!(context.control().seek(voice, 50_000, Some(1100)).unwrap());
        out.extend(context.render(600));
        assert_eq!(out[..1100], (0..1100).map(|frame| frame as f32).collect::<Vec<_>>());
        assert_eq!(out[1100..], [0.0; 500], "silence while the feed is late");
        assert_eq!(stream.stats().late, 500);
        assert_eq!(context.control().position(voice), Some(50_500), "time kept");

        open.store(true, Ordering::Release);
        let started = Instant::now();
        loop {
            let (epoch, at) = stream.marked();
            if epoch == stream.wanted() && stream.stats().written >= at + 4000 {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "the feed never sought");
            std::thread::yield_now();
        }
        let joined = context.render(500);
        assert_eq!(joined, (50_500..51_000).map(|frame| frame as f32).collect::<Vec<_>>());
        assert_eq!(stream.stats().late, 500, "nothing late once the frames came");
    }

    /// Fails on its first read, by erring or by panicking.
    struct Broken {
        panics: bool,
    }

    impl Decode for Broken {
        fn read(&mut self, _out: &mut [f32]) -> Result<usize, String> {
            assert!(!self.panics, "a decoder bug");
            Err("the file is damaged here".to_owned())
        }

        fn seek(&mut self, _frame: u64) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_decoder_that_fails_finishes_the_stream_and_says_why() {
        for (panics, why) in [(false, "the file is damaged here"), (true, "the decoder panicked")] {
            let stream = Stream::new(1, 48_000, 0.01).unwrap();
            let feed = Feed::start(Box::new(Broken { panics }), Arc::clone(&stream), FeedSpec::default()).unwrap();
            let started = Instant::now();
            while !feed.status().done {
                assert!(started.elapsed() < Duration::from_secs(5));
                std::thread::yield_now();
            }
            assert_eq!(feed.status().error.as_deref(), Some(why));
            assert!(stream.stats().finished);
        }
    }
}
