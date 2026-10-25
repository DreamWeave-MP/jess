//! Compressed audio through `@jess/audio`: MP3, Ogg Vorbis and Ogg Opus load whole into PCM at
//! their own rate and length.
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
