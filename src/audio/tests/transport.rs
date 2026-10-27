//! Transport: voices paused, played on, and sought on their frames, and asked where they are.

use super::{allocations, at, mono, pcm};
use crate::audio::{AudioConfig, AudioError, LoopRegion, OfflineContext, PlaySpec, Stream, VoiceState};

/// 0, 1, 2, ... for `frames` frames: every sample says where in the source it is.
fn ramp(frames: u16) -> std::sync::Arc<crate::audio::Pcm> {
    pcm(&(0..frames).map(f32::from).collect::<Vec<_>>(), 48_000)
}

/// Renders `frames` of `context` in blocks of `sizes`, round and round, none of them allocating.
fn render_in(context: &mut OfflineContext, frames: usize, sizes: &[usize]) -> Vec<f32> {
    let mut out = Vec::new();
    for size in sizes.iter().cycle() {
        if out.len() >= frames {
            break;
        }
        let (_, engine) = context.parts();
        let mut block = vec![0.0; (*size).min(frames - out.len())];
        assert_eq!(allocations(|| engine.render(&mut block)), 0);
        out.extend(block);
        context.control().collect();
    }
    out
}

#[test]
fn a_voice_pauses_and_plays_on_from_where_it_was_on_their_frames() {
    let render = |sizes: &[usize]| {
        let mut context = mono(AudioConfig::default());
        let voice = context.control().play(&ramp(64), &at(0)).unwrap();
        context.control().pause(voice, Some(4)).unwrap();
        context.control().resume(voice, Some(8)).unwrap();
        context.control().seek(voice, 40, Some(10)).unwrap();
        render_in(&mut context, 14, sizes)
    };
    let out = render(&[14]);
    assert_eq!(
        out,
        [0.0, 1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 4.0, 5.0, 40.0, 41.0, 42.0, 43.0]
    );
    for sizes in [&[1][..], &[3], &[5, 2]] {
        assert_eq!(render(sizes), out, "blocks of {sizes:?}");
    }
}

#[test]
fn a_seek_lands_on_its_source_frame_whatever_the_rate() {
    let mut context = mono(AudioConfig::default());
    let spec = PlaySpec { rate: 0.5, ..at(0) };
    let voice = context.control().play(&ramp(256), &spec).unwrap();
    context.control().seek(voice, 100, Some(5)).unwrap();
    let out = context.render(8);
    // Half speed reads between frames; the seek starts again on a whole one, its own sample.
    assert_eq!(out[5], 100.0);
    assert!((out[6] - 100.5).abs() < 0.01, "{}", out[6]);
    assert_eq!(out[7], 101.0);
    assert_eq!(context.control().position(voice), Some(101));
}

#[test]
fn a_voice_made_paused_waits_for_its_play_and_counts_against_nothing() {
    let mut context = mono(AudioConfig {
        voices: 4,
        physical: Some(1),
        ..AudioConfig::default()
    });
    let waiting = context
        .control()
        .play(&ramp(32), &PlaySpec { paused: true, ..at(0) })
        .unwrap();
    let other = context.control().play(&ramp(32), &at(0)).unwrap();
    let out = context.render(4);
    assert_eq!(out, [0.0, 1.0, 2.0, 3.0], "the one playing is the only one mixed");
    assert_eq!(context.control().stats().culled, 0, "the paused one isn't culled");
    assert_eq!(context.control().state(waiting), VoiceState::Paused);
    assert_eq!(context.control().position(waiting), Some(0));
    context.control().stop(other, None, 0).unwrap();
    context.control().resume(waiting, None).unwrap();
    assert_eq!(context.render(3), [0.0, 1.0, 2.0]);
    assert_eq!(context.control().state(waiting), VoiceState::Playing);
}

#[test]
fn a_seek_past_a_loop_s_end_goes_where_the_loop_would_have_taken_it() {
    let mut context = mono(AudioConfig::default());
    let spec = PlaySpec {
        looping: Some(LoopRegion { start: 4, end: 8 }),
        ..at(0)
    };
    let voice = context.control().play(&ramp(16), &spec).unwrap();
    context.control().seek(voice, 13, None).unwrap();
    // 13 is one round and a frame past the loop's start, the way playback through it gets there.
    assert_eq!(context.render(4), [5.0, 6.0, 7.0, 4.0]);
    assert!(matches!(
        context.control().seek(voice, 17, None),
        Err(AudioError::Parameter { name: "seek", .. })
    ));
}

#[test]
fn a_voice_sought_before_it_starts_starts_there() {
    let mut context = mono(AudioConfig::default());
    let voice = context.control().play(&ramp(32), &at(2)).unwrap();
    context.control().seek(voice, 20, None).unwrap();
    assert_eq!(
        context.control().position(voice),
        Some(0),
        "where it was told to start, until it does"
    );
    assert_eq!(context.render(4), [0.0, 0.0, 20.0, 21.0]);
    assert_eq!(context.control().position(voice), Some(22));
}

#[test]
fn a_stream_a_script_writes_says_it_can_t_seek() {
    let mut context = mono(AudioConfig::default());
    let stream = Stream::new(1, 48_000, 0.1).unwrap();
    let voice = context.control().play_stream(&stream, &PlaySpec::default()).unwrap();
    assert!(matches!(
        context.control().seek(voice, 10, None),
        Err(AudioError::Unsupported(_))
    ));
    assert!(context.control().pause(voice, None).unwrap(), "pausing it is fine");
}
