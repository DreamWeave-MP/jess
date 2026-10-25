//! Compressed audio through `@jess/audio`: MP3, Ogg Vorbis and Ogg Opus load whole into PCM at
//! their own rate and length, and a feed decodes one into a stream on a thread of its own,
//! finishing it at the end or looping without a gap.
//!
//! The fixtures are one second of 440 Hz left and 660 Hz right at half scale, encoded by ffmpeg
//! with libmp3lame (44.1 kHz), libvorbis (44.1 kHz) and libopus (48 kHz).

use jess::lua::l3i::Runtime;
use jess::lua::l3i::convert::NewBuffer;

use super::common;

fn runtime() -> Runtime {
    let runtime = common::runtime();
    for (name, bytes) in [
        ("mp3", &include_bytes!("../fixtures/audio/tone.mp3")[..]),
        ("vorbis", &include_bytes!("../fixtures/audio/tone.ogg")[..]),
        ("opus", &include_bytes!("../fixtures/audio/tone.opus")[..]),
    ] {
        common::set_global(&runtime, name, &NewBuffer(bytes.to_vec()));
    }
    runtime
}

/// `render(context, stream, frames)`: renders `frames` of an offline context once the feed is
/// well ahead of it. An offline render runs as fast as it is asked, so without waiting it would
/// outrun the feed's thread and play silence.
const RENDER: &str = r"
    local function render(context: AudioContext, stream: AudioStream, frames: number): string
        local deadline = os.clock() + 20
        while true do
            local stats = stream:stats()
            if stats.finished or stats.written - stats.read >= frames + 64 then break end
            assert(os.clock() < deadline, 'the feed fell behind')
        end
        return buffer.tostring(context:render(frames))
    end
";

#[test]
fn compressed_files_load_whole_at_their_own_rate_and_length() {
    common::run(
        &runtime(),
        r"
        local audio = require '@jess/audio'
        for _, case in { { mp3, 44100 }, { vorbis, 44100 }, { opus, 48000 } } do
            local pcm = audio.load(case[1])
            assert(pcm:rate() == case[2] and pcm:channels() == 2, pcm:rate())
            assert(pcm:frames() == case[2], pcm:frames())
            local context = audio.offline { rate = case[2] }
            context:play(pcm)
            local out = context:render(case[2] // 2)
            local peak = 0
            for index = 0, case[2] - 1 do
                peak = math.max(peak, math.abs(buffer.readf32(out, index * 4)))
            end
            assert(peak > 0.45 and peak < 0.6, peak)
        end
        -- WAV still loads through the same call.
        local wav = audio.writeWav(buffer.create(8 * 10), { channels = 2, rate = 8000 })
        assert(audio.load(wav):frames() == 10 and audio.load(buffer.tostring(wav)):rate() == 8000)
        ",
    );
}

#[test]
fn a_feed_streams_a_file_frame_for_frame_and_finishes_it() {
    common::run(
        &runtime(),
        &format!(
            "{RENDER}{}",
            r"
            local audio = require '@jess/audio'
            local feed = audio.feed(vorbis, { seconds = 0.1, block = 1000 })
            local stream = feed:stream()
            assert(stream:channels() == 2 and stream:rate() == 44100)
            local streamed = audio.offline { rate = 44100 }
            local voice = streamed:play(stream)
            local chunks = {}
            while voice:state() ~= 'ended' do
                table.insert(chunks, render(streamed, stream, 512))
            end
            assert(stream:stats().finished and stream:stats().missed == 0, 'the feed kept ahead')
            assert(stream:stats().written == 44100, stream:stats().written)
            local deadline = os.clock() + 20
            while not feed:status().done do
                assert(os.clock() < deadline, 'the feed never ended')
            end
            assert(feed:status().loops == 0 and feed:status().error == nil)

            -- What the stream played is the file, frame for frame, as a voice plays it whole.
            local whole = audio.offline { rate = 44100 }
            whole:play(audio.load(vorbis))
            local expected = buffer.tostring(whole:render(44100))
            assert(string.sub(table.concat(chunks), 1, #expected) == expected, 'the streamed frames are the file')

            -- From a frame of its own, it is the file from there.
            local late = audio.feed(vorbis, { start = 22050 })
            local from = audio.offline { rate = 44100 }
            from:play(late:stream())
            local half = render(from, late:stream(), 22050)
            assert(half == string.sub(expected, 22050 * 8 + 1), 'a feed starts where it is asked')
            ",
        ),
    );
}

#[test]
fn a_looping_feed_comes_round_without_a_gap() {
    common::run(
        &runtime(),
        &format!(
            "{RENDER}{}",
            r"
            local audio = require '@jess/audio'
            local feed = audio.feed(mp3, { loop = true, seconds = 0.5 })
            local stream = feed:stream()
            local context = audio.offline { rate = 44100 }
            context:play(stream)
            local chunks = {}
            for _ = 1, 50 do
                table.insert(chunks, render(context, stream, 2205))
            end
            assert(not feed:status().done and feed:status().loops >= 2, feed:status().loops)
            assert(stream:stats().missed == 0)
            local played = buffer.fromstring(table.concat(chunks))
            -- Two and a half seconds of a one-second file: each time round is the first, frame
            -- for frame, because the seek back to the start is exact.
            for frame = 0, 44099, 7 do
                assert(buffer.readf32(played, (frame + 44100) * 8) == buffer.readf32(played, frame * 8), frame)
                if frame < 22050 then
                    assert(buffer.readf32(played, (frame + 88200) * 8 + 4) == buffer.readf32(played, frame * 8 + 4), frame)
                end
            end

            feed:stop()
            local deadline = os.clock() + 20
            while not feed:status().done do
                assert(os.clock() < deadline, 'the feed never stopped')
            end
            assert(not stream:stats().finished, 'a stopped feed leaves the stream to play out what it has')
            ",
        ),
    );
}

#[test]
fn a_feed_says_what_it_cannot_read() {
    let runtime = runtime();
    let error = runtime.exec("jess.audio.feed(buffer.create(100))").unwrap_err().to_string();
    assert!(error.contains("not MP3, Ogg Vorbis or Ogg Opus"), "{error}");
    let error = runtime
        .exec("jess.audio.feed(vorbis, { format = 'flac' })")
        .unwrap_err()
        .to_string();
    assert!(error.contains("format is mp3, vorbis, or opus"), "{error}");
}

#[test]
fn a_runtimes_end_stops_its_feeds() {
    let runtime = runtime();
    let id: f64 = runtime
        .eval(
            r"
            local feed = jess.audio.feed(opus, { loop = true })
            return feed:stream():handle().id
            ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // An id from `handle().id`.
    let id = id as u64;
    // Held here, the stream keeps the feed running: only the runtime's end can stop it.
    let stream = jess::audio::Stream::find(id).expect("the feed's stream");
    drop(runtime);
    assert_eq!(
        std::sync::Arc::strong_count(&stream),
        1,
        "the feed's thread outlived its runtime"
    );
}
