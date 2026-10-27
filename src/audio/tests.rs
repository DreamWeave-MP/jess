//! The engine against renders worked out by hand.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use crate::audio::processor::{Gain, Io, ParamInfo, Processor, Tail};
use crate::audio::{
    AudioConfig, AudioError, BusSpec, Culled, Curve, Distance, Emitter, Glide, Listener, LoopRegion, Meter, NodeKey,
    OfflineContext, Pcm, PlaySpec, QUANTUM, Stream, Tap, VoiceParam, VoiceSend, VoiceState,
};

/// Counts allocations and frees on a thread while it is armed, so a test can prove the render
/// path makes none. Every other allocation passes straight through to the system allocator.
struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static COUNTED: Cell<usize> = const { Cell::new(0) };
}

fn note() {
    // `try_with` because the allocator also runs while thread-locals are being torn down.
    let _ = ARMED.try_with(|armed| {
        if armed.get() {
            COUNTED.with(|counted| counted.set(counted.get() + 1));
        }
    });
}

// SAFETY: every call forwards to `System` with the caller's arguments; counting touches only
// const-initialized thread-locals, which never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: the caller's contract, passed on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        // SAFETY: the caller's contract, passed on.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// How many allocations and frees `work` made on this thread.
fn allocations(work: impl FnOnce()) -> usize {
    COUNTED.with(|counted| counted.set(0));
    ARMED.with(|armed| armed.set(true));
    work();
    ARMED.with(|armed| armed.set(false));
    COUNTED.with(Cell::get)
}

fn mono(config: AudioConfig) -> OfflineContext {
    OfflineContext::new(&AudioConfig { channels: 1, ..config }).unwrap()
}

fn pcm(samples: &[f32], rate: u32) -> Arc<Pcm> {
    Arc::new(Pcm::from_planar(&[samples], rate).unwrap())
}

fn at(frame: u64) -> PlaySpec {
    PlaySpec {
        at: Some(frame),
        ..PlaySpec::default()
    }
}

#[test]
fn stamped_starts_land_on_their_frame_inside_any_block() {
    let click = pcm(&[1.0], 48_000);
    let frames = [0, 5, 77, 127, 128, 129, 300, 1000];
    let render = |sizes: &[usize]| {
        let mut context = mono(AudioConfig::default());
        for frame in frames {
            context.control().play(&click, &at(frame)).unwrap();
        }
        let mut out = Vec::new();
        for size in sizes.iter().cycle() {
            if out.len() >= 1100 {
                break;
            }
            out.extend(context.render(*size));
        }
        out.truncate(1100);
        out
    };
    let whole = render(&[1100]);
    for (frame, sample) in whole.iter().enumerate() {
        let expected = if frames.contains(&(frame as u64)) { 1.0 } else { 0.0 };
        assert_eq!(*sample, expected, "frame {frame}");
    }
    // A device asks for whatever block it likes; the frames don't move.
    assert_eq!(render(&[1, 37, 128, 500, 3]), whole);
}

#[test]
fn stops_and_sets_carry_out_on_their_frame() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = mono(AudioConfig::default());
    let voice = context.control().play(&tone, &at(10)).unwrap();
    assert!(context.control().set(voice, VoiceParam::Gain, 0.5, Some(20), None).unwrap());
    assert!(context.control().stop(voice, Some(30), 0).unwrap());
    let out = context.render(40);
    assert_eq!(out[..10], [0.0; 10]);
    assert_eq!(out[10..20], [1.0; 10]);
    assert_eq!(out[20..30], [0.5; 10]);
    assert_eq!(out[30..], [0.0; 10]);
    assert_eq!(context.control().state(voice), VoiceState::Ended);
}

#[test]
fn a_voice_stopped_or_changed_before_it_starts_does_so_from_its_first_frame() {
    let tone = pcm(&[1.0; 8], 48_000);
    let mut context = mono(AudioConfig::default());
    let quiet = context.control().play(&tone, &at(100)).unwrap();
    let cancelled = context.control().play(&tone, &at(100)).unwrap();
    context.control().set(quiet, VoiceParam::Gain, 0.25, None, None).unwrap();
    context.control().stop(cancelled, None, 0).unwrap();
    assert_eq!(context.control().state(quiet), VoiceState::Scheduled);

    let out = context.render(110);
    assert_eq!(out[100..108], [0.25; 8]);
    assert_eq!(context.control().state(cancelled), VoiceState::Ended);
    assert_eq!(context.control().stats().allocated, 0, "both slots are free again");
}

#[test]
fn a_source_at_the_output_rate_copies_exactly_and_ends_on_its_last_frame() {
    let ramp: Vec<f32> = (0..300u16).map(|frame| f32::from(frame) / 300.0).collect();
    let mut context = mono(AudioConfig::default());
    let voice = context.control().play(&pcm(&ramp, 48_000), &at(0)).unwrap();
    let out = context.render(400);
    assert_eq!(out[..300], ramp[..]);
    assert_eq!(out[300..], [0.0; 100]);
    assert_eq!(context.control().state(voice), VoiceState::Ended);
}

#[test]
fn a_slower_source_is_interpolated_through_its_own_samples() {
    let samples = [0.0, 1.0, 0.0, -1.0, 0.0, 1.0];
    let mut context = mono(AudioConfig::default());
    context.control().play(&pcm(&samples, 24_000), &at(0)).unwrap();
    let out = context.render(12);
    for (frame, sample) in samples.iter().enumerate() {
        assert_eq!(out[frame * 2], *sample, "every second frame is a source frame");
    }
    assert!(
        out[1] > 0.5 && out[1] < 0.6,
        "between 0 and 1, bowed by the neighbours: {}",
        out[1]
    );
}

/// A second of a sine at `frequency` in a source at `rate`, played into a 48 kHz context: the
/// RMS of the output past the first and last tenths, over the source's own RMS.
fn passed(frequency: f64, rate: u32, playback: f64) -> f64 {
    #[allow(clippy::cast_possible_truncation)] // Test signals.
    let tone: Vec<f32> = (0..rate)
        .map(|frame| (std::f64::consts::TAU * frequency * f64::from(frame) / f64::from(rate)).sin() as f32)
        .collect();
    let mut context = mono(AudioConfig::default());
    let spec = PlaySpec { rate: playback, ..at(0) };
    context.control().play(&pcm(&tone, rate), &spec).unwrap();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Whole frames.
    let frames = (48_000.0 / playback) as usize;
    let out = context.render(frames);
    let middle = &out[frames / 10..frames - frames / 10];
    let energy: f64 = middle.iter().map(|sample| f64::from(*sample).powi(2)).sum();
    #[allow(clippy::cast_precision_loss)] // A count of samples.
    let rms = (energy / middle.len() as f64).sqrt();
    rms / std::f64::consts::FRAC_1_SQRT_2
}

#[test]
fn a_faster_source_is_filtered_to_what_the_output_can_hold() {
    // 96 kHz into 48 kHz: what is under 24 kHz passes, and what is over it doesn't come back as
    // an alias below. Read straight through every other frame, 36 kHz would be 12 kHz at full level.
    assert!(
        (passed(6000.0, 96_000, 1.0) - 1.0).abs() < 0.01,
        "{}",
        passed(6000.0, 96_000, 1.0)
    );
    assert!(passed(36_000.0, 96_000, 1.0) < 1e-3, "{}", passed(36_000.0, 96_000, 1.0));
    // A rate above one is the same: a 48 kHz source played at 1.5, where 23 kHz is 34.5.
    assert!(passed(23_000.0, 48_000, 1.5) < 1e-3, "{}", passed(23_000.0, 48_000, 1.5));
    assert!(
        (passed(4000.0, 48_000, 1.5) - 1.0).abs() < 0.01,
        "{}",
        passed(4000.0, 48_000, 1.5)
    );
    // And upsampling keeps what is there: 44.1 kHz into 48 kHz.
    assert!(
        (passed(10_000.0, 44_100, 1.0) - 1.0).abs() < 0.01,
        "{}",
        passed(10_000.0, 44_100, 1.0)
    );
}

#[test]
fn a_loop_wraps_without_a_gap() {
    // Intro 0, 1; loop 2..5.
    let samples = [10.0, 11.0, 2.0, 3.0, 4.0];
    let mut context = mono(AudioConfig::default());
    let spec = PlaySpec {
        looping: Some(LoopRegion { start: 2, end: 5 }),
        ..at(0)
    };
    let voice = context.control().play(&pcm(&samples, 48_000), &spec).unwrap();
    let out = context.render(QUANTUM * 2 + 3);
    let expected: Vec<f32> = [10.0, 11.0]
        .into_iter()
        .chain([2.0, 3.0, 4.0].into_iter().cycle())
        .take(out.len())
        .collect();
    assert_eq!(out, expected);
    assert_eq!(context.control().state(voice), VoiceState::Playing);
}

#[test]
fn a_loop_shorter_than_the_resampler_reaches_reads_only_itself_once_it_has_wrapped() {
    for length in [2, 4, 8] {
        for rate in [0.75, 1.5] {
            // An intro of nines the kernel must never reach back into once the loop has wrapped.
            let mut samples = vec![9.0; 8];
            samples.resize(8 + length, 1.0);
            let mut context = mono(AudioConfig::default());
            let spec = PlaySpec {
                rate,
                offset: 8,
                looping: Some(LoopRegion {
                    start: 8,
                    end: 8 + length as u64,
                }),
                ..at(0)
            };
            context.control().play(&pcm(&samples, 48_000), &spec).unwrap();
            let out = context.render(QUANTUM * 2);
            for (frame, sample) in out.iter().enumerate().skip(4 * length + 2) {
                assert!((sample - 1.0).abs() < 1e-5, "{length} frames at {rate}: {sample} at {frame}");
            }
        }
    }
}

#[test]
fn a_centred_mono_voice_pans_with_equal_power_and_a_stereo_one_balances() {
    let mut context = OfflineContext::new(&AudioConfig::default()).unwrap();
    context.control().play(&pcm(&[1.0], 48_000), &at(0)).unwrap();
    let left = PlaySpec { pan: -1.0, ..at(1) };
    context.control().play(&pcm(&[1.0], 48_000), &left).unwrap();
    let stereo = Arc::new(Pcm::from_planar(&[&[1.0], &[1.0]], 48_000).unwrap());
    let right = PlaySpec { pan: 0.5, ..at(2) };
    context.control().play(&stereo, &right).unwrap();
    let out = context.render(3);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    assert!((out[0] - half).abs() < 1e-6 && (out[1] - half).abs() < 1e-6);
    assert!((out[2] - 1.0).abs() < 1e-6 && out[3].abs() < 1e-6);
    assert_eq!(out[4..6], [0.5, 1.0]);
}

#[test]
fn ended_voices_free_their_slots_and_stale_handles_name_nothing() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig {
        voices: 2,
        ..AudioConfig::default()
    });
    let first = context.control().play(&click, &at(0)).unwrap();
    context.control().play(&click, &at(0)).unwrap();
    assert_eq!(context.control().play(&click, &at(0)), Err(AudioError::VoicesExhausted(2)));
    context.render(4);
    assert!(!context.control().stop(first, None, 0).unwrap(), "already ended");
    let reused = context.control().play(&click, &at(10)).unwrap();
    assert_ne!(reused, first, "a reused slot is a new voice");
    assert_eq!(context.control().state(first), VoiceState::Ended);
    assert!(!context.control().set(first, VoiceParam::Gain, 0.0, None, None).unwrap());
    assert_eq!(context.render(12)[6], 1.0, "the stale set didn't reach the new voice");
}

#[test]
fn a_full_queue_and_bad_parameters_are_errors() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig {
        commands: 2,
        ..AudioConfig::default()
    });
    context.control().play(&click, &PlaySpec::default()).unwrap();
    context.control().play(&click, &PlaySpec::default()).unwrap();
    assert_eq!(
        context.control().play(&click, &PlaySpec::default()),
        Err(AudioError::QueueFull(2))
    );
    assert_eq!(context.control().stats().allocated, 2, "the refused voice gave its slot back");
    context.render(1);
    context.control().play(&click, &at(10)).unwrap();

    let bad = PlaySpec { rate: 0.0, ..at(0) };
    assert!(matches!(
        context.control().play(&click, &bad),
        Err(AudioError::Parameter { name: "rate", .. })
    ));
    let outside = PlaySpec {
        looping: Some(LoopRegion { start: 0, end: 2 }),
        ..at(0)
    };
    assert!(matches!(
        context.control().play(&click, &outside),
        Err(AudioError::LoopRegion { .. })
    ));
    let surround = Arc::new(Pcm::from_planar(&[&[0.0f32][..]; 6], 48_000).unwrap());
    assert_eq!(
        context.control().play(&surround, &at(0)),
        Err(AudioError::Layout { source: 6, output: 1 })
    );
}

#[test]
fn commands_for_later_never_hold_up_a_command_for_now() {
    let tone = pcm(&[1.0; 8], 48_000);
    let mut context = mono(AudioConfig {
        commands: 4,
        ..AudioConfig::default()
    });
    let looping = PlaySpec {
        looping: Some(LoopRegion { start: 0, end: 8 }),
        ..PlaySpec::default()
    };
    let first = context.control().play(&tone, &looping).unwrap();
    let second = context.control().play(&tone, &looping).unwrap();
    assert_eq!(context.render(1), [2.0]);
    let minute = Some(48_000 * 60);
    for _ in 0..4 {
        context.control().set(first, VoiceParam::Gain, 0.5, minute, None).unwrap();
    }
    assert_eq!(
        context.control().set(first, VoiceParam::Gain, 0.5, minute, None),
        Err(AudioError::ScheduleFull(4)),
        "a fifth command for later is refused, not queued behind the four"
    );
    context.render(1);
    assert_eq!(context.control().stats().pending, 4);

    context.control().stop(first, None, 0).unwrap();
    assert_eq!(context.render(1), [1.0], "the stop for now went straight past the full list");
    context.control().begin_group().unwrap();
    context.control().stop(second, None, 0).unwrap();
    context.control().end_group().unwrap();
    assert_eq!(context.render(1), [0.0], "and so did a group with nothing for later");
}

#[test]
fn a_command_stamped_in_the_past_runs_at_once_and_is_counted_late() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    context.render(100);
    context.control().play(&click, &at(50)).unwrap();
    let out = context.render(2);
    assert_eq!(out, [1.0, 0.0]);
    assert_eq!(context.control().stats().late, 1);
}

#[test]
fn samples_that_arent_finite_are_written_as_silence() {
    let mut context = mono(AudioConfig::default());
    context.control().play(&pcm(&[f32::NAN, 0.5], 48_000), &at(0)).unwrap();
    assert_eq!(context.render(2), [0.0, 0.5]);
    assert_eq!(context.control().stats().nonfinite, 1);
}

#[test]
fn rendering_never_allocates_or_frees() {
    let short = pcm(&[0.5; 40], 44_100);
    let long = Arc::new(Pcm::from_planar(&[&[0.25; 3000], &[-0.25; 3000]], 48_000).unwrap());
    let mut context = OfflineContext::new(&AudioConfig::default()).unwrap();
    let mut voices = Vec::new();
    for index in 0..64u32 {
        let spec = PlaySpec {
            rate: 0.5 + f64::from(index) / 40.0,
            pan: -0.5,
            looping: (index % 3 == 0).then_some(LoopRegion { start: 10, end: 30 }),
            ..at(u64::from(index) * 37)
        };
        let source = if index % 2 == 0 { &short } else { &long };
        voices.push(context.control().play(source, &spec).unwrap());
    }
    for (index, voice) in voices.iter().enumerate() {
        let frame = 100 + 53 * u64::try_from(index).unwrap();
        context
            .control()
            .set(*voice, VoiceParam::Gain, 0.5, Some(frame), None)
            .unwrap();
        context.control().stop(*voice, Some(frame + 400), 0).unwrap();
    }
    // Voices start, change, loop, run out, and are stopped during these renders, and every ended
    // one goes back with its PCM.
    let mut out = vec![0.0; 2 * 333];
    let (control, engine) = context.parts();
    assert!(
        allocations(|| drop(std::hint::black_box(vec![0u8; 16]))) >= 1,
        "the counter sees allocations"
    );
    let counted = allocations(|| {
        for _ in 0..30 {
            engine.render(&mut out);
        }
    });
    assert_eq!(counted, 0);
    control.collect();
    assert_eq!(control.stats().allocated, 0, "every voice ended and came back");
}

fn bus(context: &mut OfflineContext, parent: NodeKey, gain: f32) -> NodeKey {
    let spec = BusSpec {
        parent,
        gain,
        ..BusSpec::default()
    };
    context.control().add_bus(&spec).unwrap()
}

fn into(bus: NodeKey, frame: u64) -> PlaySpec {
    PlaySpec { bus, ..at(frame) }
}

#[test]
fn voices_sum_through_nested_buses_at_each_bus_s_gain() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let music = bus(&mut context, NodeKey::MASTER, 0.5);
    let strings = bus(&mut context, music, 0.5);
    let effects = bus(&mut context, NodeKey::MASTER, 2.0);
    context.control().play(&click, &into(strings, 0)).unwrap();
    context.control().play(&click, &into(music, 1)).unwrap();
    context.control().play(&click, &into(effects, 1)).unwrap();
    context.control().play(&click, &at(2)).unwrap();
    assert_eq!(context.render(3), [0.25, 2.5, 1.0]);
}

#[test]
fn bus_gain_and_mute_change_on_their_frame() {
    let tone = pcm(&[1.0; 8], 48_000);
    let mut context = mono(AudioConfig::default());
    let group = bus(&mut context, NodeKey::MASTER, 1.0);
    context.control().play(&tone, &into(group, 0)).unwrap();
    context.control().set_param(group, "gain", 0.5, Some(2), None).unwrap();
    context.control().set_param(group, "mute", 1.0, Some(4), None).unwrap();
    context.control().set_param(group, "mute", 0.0, Some(6), None).unwrap();
    context
        .control()
        .set_param(NodeKey::MASTER, "gain", 4.0, Some(7), None)
        .unwrap();
    assert_eq!(context.render(8), [1.0, 1.0, 0.5, 0.5, 0.0, 0.0, 0.5, 2.0]);
}

#[test]
fn rerouting_moves_a_bus_on_its_frame_and_a_loop_is_refused() {
    let tone = pcm(&[1.0; 4], 48_000);
    let mut context = mono(AudioConfig::default());
    let quiet = bus(&mut context, NodeKey::MASTER, 0.25);
    let group = bus(&mut context, NodeKey::MASTER, 1.0);
    context.control().play(&tone, &into(group, 0)).unwrap();
    context.control().route(group, quiet, Some(2)).unwrap();
    assert_eq!(context.render(4), [1.0, 1.0, 0.25, 0.25]);

    assert_eq!(context.control().route(quiet, group, None), Err(AudioError::Cycle));
    assert_eq!(context.control().route(group, group, None), Err(AudioError::Cycle));
    assert_eq!(context.control().route(NodeKey::MASTER, group, None), Err(AudioError::Master));
}

#[test]
fn a_bus_is_removed_only_once_nothing_routes_into_it() {
    let tone = pcm(&[1.0; 4], 48_000);
    let mut context = mono(AudioConfig {
        nodes: 3,
        ..AudioConfig::default()
    });
    let outer = bus(&mut context, NodeKey::MASTER, 1.0);
    let inner = bus(&mut context, outer, 1.0);
    assert_eq!(
        context.control().add_bus(&BusSpec::default()),
        Err(AudioError::NodesExhausted(3))
    );
    let voice = context.control().play(&tone, &into(inner, 0)).unwrap();
    assert_eq!(
        context.control().remove(inner, None),
        Err(AudioError::NodeInUse { voices: 1, nodes: 0 })
    );
    assert_eq!(
        context.control().remove(outer, None),
        Err(AudioError::NodeInUse { voices: 0, nodes: 1 })
    );
    context.control().stop(voice, None, 0).unwrap();
    context.render(1);
    assert!(context.control().remove(inner, None).unwrap());
    assert!(!context.control().node_live(inner));
    assert_eq!(context.control().play(&tone, &into(inner, 0)), Err(AudioError::StaleNode));
    context.render(1);
    assert!(context.control().remove(outer, None).unwrap());
    context.render(1);
    let reused = bus(&mut context, NodeKey::MASTER, 1.0);
    assert_ne!(reused, outer, "a reused slot is a new bus");
    assert!(!context.control().set_param(outer, "gain", 0.0, None, None).unwrap());
    assert_eq!(context.control().remove(NodeKey::MASTER, None), Err(AudioError::Master));
}

#[test]
fn voices_on_different_buses_share_one_send_bus_mixed_once() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let music = bus(&mut context, NodeKey::MASTER, 1.0);
    let effects = bus(&mut context, NodeKey::MASTER, 1.0);
    let aux = bus(&mut context, NodeKey::MASTER, 0.5);
    let sends = |level| vec![VoiceSend { bus: aux, level }];
    let spec = |bus, level| PlaySpec {
        sends: sends(level),
        ..into(bus, 0)
    };
    let first = context.control().play(&click, &spec(music, 0.5)).unwrap();
    context.control().play(&click, &spec(effects, 1.0)).unwrap();
    // The aux bus applies its gain once to what both sent: 0.5 * (0.5 + 1).
    assert_eq!(context.render(1), [2.75]);
    assert!(
        context.control().set_send(first, music, 0.0, None).unwrap(),
        "where a voice plays is an output with a level like its sends"
    );
}

#[test]
fn a_voice_send_level_changes_on_its_frame() {
    let tone = pcm(&[1.0; 4], 48_000);
    let mut context = mono(AudioConfig::default());
    let aux = bus(&mut context, NodeKey::MASTER, 1.0);
    let spec = PlaySpec {
        gain: 0.5,
        sends: vec![VoiceSend { bus: aux, level: 1.0 }],
        ..at(0)
    };
    let voice = context.control().play(&tone, &spec).unwrap();
    assert!(context.control().set_send(voice, aux, 0.0, Some(2)).unwrap());
    // The send's level is relative to the voice's gain.
    assert_eq!(context.render(4), [1.0, 1.0, 0.5, 0.5]);
}

#[test]
fn a_bus_send_taps_after_the_fader_unless_asked_to_tap_before_it() {
    let tone = pcm(&[1.0; 4], 48_000);
    let render = |tap| {
        let mut context = mono(AudioConfig::default());
        let group = bus(&mut context, NodeKey::MASTER, 0.5);
        let aux = bus(&mut context, NodeKey::MASTER, 1.0);
        context.control().send_to(group, aux, 1.0, tap, None).unwrap();
        context.control().play(&tone, &into(group, 0)).unwrap();
        context.control().set_param(group, "mute", 1.0, Some(2), None).unwrap();
        context.control().unsend(group, aux, Some(3)).unwrap();
        context.render(4)
    };
    assert_eq!(render(Tap::PostFader), [1.0, 1.0, 0.0, 0.0]);
    assert_eq!(render(Tap::PreFader), [1.5, 1.5, 1.0, 0.0]);
}

#[test]
fn a_routing_change_the_queue_has_no_room_for_leaves_the_routing_as_it_was() {
    let mut context = mono(AudioConfig {
        commands: 4,
        ..AudioConfig::default()
    });
    let first = bus(&mut context, NodeKey::MASTER, 1.0);
    let second = bus(&mut context, NodeKey::MASTER, 1.0);
    let aux = bus(&mut context, NodeKey::MASTER, 1.0);
    context.control().send_to(first, aux, 1.0, Tap::PostFader, None).unwrap();
    context.render(1);
    while context.control().set_param(first, "gain", 1.0, None, None).is_ok() {}

    assert_eq!(context.control().route(first, second, None), Err(AudioError::QueueFull(4)));
    assert_eq!(context.control().unsend(first, aux, None), Err(AudioError::QueueFull(4)));
    context.render(1);
    assert_eq!(
        context.control().route(second, first, None),
        Ok(()),
        "the first bus never went under the second, so this is no loop"
    );
    assert_eq!(
        context.control().remove(aux, None),
        Err(AudioError::NodeInUse { voices: 0, nodes: 1 }),
        "the send the engine still mixes is still there"
    );
}

#[test]
fn a_stale_bus_handle_names_no_send_to_the_bus_in_its_slot_now() {
    let mut context = mono(AudioConfig::default());
    let source = bus(&mut context, NodeKey::MASTER, 1.0);
    let old = bus(&mut context, NodeKey::MASTER, 1.0);
    context.control().send_to(source, old, 1.0, Tap::PostFader, None).unwrap();
    assert!(context.control().unsend(source, old, None).unwrap());
    assert!(context.control().remove(old, None).unwrap());
    context.render(1);
    let new = bus(&mut context, NodeKey::MASTER, 1.0);
    assert_eq!(new.slot, old.slot);
    context.control().send_to(source, new, 1.0, Tap::PostFader, None).unwrap();

    assert!(!context.control().unsend(source, old, None).unwrap());
    assert_eq!(
        context.control().remove(new, None),
        Err(AudioError::NodeInUse { voices: 0, nodes: 1 }),
        "the send to the new bus is still there"
    );
}

#[test]
fn sends_that_would_loop_or_feed_a_removed_bus_are_refused() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let group = bus(&mut context, NodeKey::MASTER, 1.0);
    let aux = bus(&mut context, NodeKey::MASTER, 1.0);
    context.control().send_to(group, aux, 1.0, Tap::PostFader, None).unwrap();
    assert_eq!(
        context.control().send_to(aux, group, 1.0, Tap::PostFader, None),
        Err(AudioError::Cycle)
    );
    assert_eq!(context.control().route(aux, group, None), Err(AudioError::Cycle));
    assert_eq!(
        context.control().send_to(group, group, 1.0, Tap::PostFader, None),
        Err(AudioError::Cycle)
    );
    assert_eq!(
        context.control().remove(aux, None),
        Err(AudioError::NodeInUse { voices: 0, nodes: 1 })
    );
    let spec = PlaySpec {
        sends: vec![VoiceSend { bus: group, level: 1.0 }],
        ..at(5)
    };
    context.control().play(&click, &spec).unwrap();
    assert_eq!(
        context.control().remove(group, None),
        Err(AudioError::NodeInUse { voices: 1, nodes: 0 })
    );
}

/// Adds a constant: a chain's order shows in what comes out.
struct Offset(f32);

impl Processor for Offset {
    fn kind(&self) -> &'static str {
        "offset"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }

    fn set(&mut self, _index: usize, _value: f64) {}

    fn value(&self, _index: usize) -> f64 {
        0.0
    }

    fn process(&mut self, io: &mut Io<'_>) {
        let mut block = io.main();
        let block = &mut block;
        for sample in block.channel(0) {
            *sample += self.0;
        }
    }
}

/// Holds the last sample it heard for `tail` frames after its input stops, and says its output
/// lags by `latency` (which it doesn't, to keep renders readable).
struct Ring {
    tail: u64,
    latency: u64,
    held: f32,
    left: u64,
}

fn ring(tail: u64, latency: u64) -> Box<dyn Processor> {
    Box::new(Ring {
        tail,
        latency,
        held: 0.0,
        left: 0,
    })
}

impl Processor for Ring {
    fn kind(&self) -> &'static str {
        "ring"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }

    fn set(&mut self, _index: usize, _value: f64) {}

    fn value(&self, _index: usize) -> f64 {
        0.0
    }

    fn latency(&self) -> u64 {
        self.latency
    }

    fn tail(&self) -> Tail {
        Tail::Frames(self.tail)
    }

    fn process(&mut self, io: &mut Io<'_>) {
        let mut block = io.main();
        let block = &mut block;
        for sample in block.channel(0) {
            if *sample != 0.0 {
                (self.held, self.left) = (*sample, self.tail);
            } else if self.left > 0 {
                *sample = self.held;
                self.left -= 1;
            }
        }
    }
}

#[test]
fn a_bus_runs_its_chain_in_order_before_its_gain() {
    let tone = pcm(&[1.0; 8], 48_000);
    let mut context = mono(AudioConfig::default());
    let group = bus(&mut context, NodeKey::MASTER, 0.5);
    let gain = context.control().insert(group, Box::new(Gain::new()), None).unwrap();
    context.control().insert(group, Box::new(Offset(1.0)), None).unwrap();
    context.control().set_param(gain, "gain", 2.0, None, None).unwrap();
    context.control().play(&tone, &into(group, 0)).unwrap();
    // (1 * 2 + 1) * 0.5.
    assert_eq!(context.render(1), [1.5]);

    context.control().insert(group, Box::new(Offset(1.0)), Some(0)).unwrap();
    // ((1 + 1) * 2 + 1) * 0.5.
    assert_eq!(context.render(1), [2.5]);
    context.control().set_param(gain, "gain", 0.0, Some(3), None).unwrap();
    assert!(context.control().remove(gain, Some(4)).unwrap());
    // Then ((1 + 1) * 0 + 1) * 0.5, and without the gain (1 + 1 + 1) * 0.5.
    assert_eq!(context.render(3), [2.5, 0.5, 1.5]);
    assert!(!context.control().node_live(gain));
    assert!(!context.control().set_param(gain, "gain", 1.0, None, None).unwrap());
}

#[test]
fn processor_parameters_are_checked_by_name_and_range() {
    let mut context = mono(AudioConfig::default());
    let gain = context
        .control()
        .insert(NodeKey::MASTER, Box::new(Gain::new()), None)
        .unwrap();
    assert!(matches!(
        context.control().set_param(gain, "volume", 1.0, None, None),
        Err(AudioError::UnknownParameter { kind: "gain", .. })
    ));
    assert!(matches!(
        context.control().set_param(gain, "gain", 100.0, None, None),
        Err(AudioError::Parameter { name: "gain", .. })
    ));
    // The master is a node too.
    let mut full = mono(AudioConfig {
        nodes: 2,
        ..AudioConfig::default()
    });
    full.control().insert(NodeKey::MASTER, Box::new(Gain::new()), None).unwrap();
    assert!(matches!(
        full.control().insert(NodeKey::MASTER, Box::new(Gain::new()), None),
        Err(AudioError::NodesExhausted(2))
    ));
}

#[test]
fn a_removed_bus_rings_out_its_tail_before_it_goes() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let room = bus(&mut context, NodeKey::MASTER, 1.0);
    let ringing = context.control().insert(room, ring(4, 0), None).unwrap();
    context.control().play(&click, &into(room, 0)).unwrap();
    let mut out = context.render(2);
    assert!(context.control().remove(room, None).unwrap());
    assert!(!context.control().node_live(ringing), "the processor goes with its bus");
    out.extend(context.render(6));
    assert_eq!(out, [1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0]);
    let stats = context.control().stats();
    assert_eq!(stats.allocated, 0);
    let reused = bus(&mut context, NodeKey::MASTER, 1.0);
    assert_ne!(reused, room);
    context.control().insert(reused, Box::new(Gain::new()), None).unwrap();
}

#[test]
fn latency_adds_up_along_the_longest_path_to_each_bus() {
    let mut context = mono(AudioConfig::default());
    let group = bus(&mut context, NodeKey::MASTER, 1.0);
    let inner = bus(&mut context, group, 1.0);
    let aux = bus(&mut context, NodeKey::MASTER, 1.0);
    context.control().insert(group, ring(0, 10), None).unwrap();
    let late = context.control().insert(inner, ring(0, 5), None).unwrap();
    context.control().send_to(inner, aux, 1.0, Tap::PostFader, None).unwrap();
    assert_eq!(context.control().latency(inner), Ok(5));
    assert_eq!(context.control().latency(group), Ok(15));
    assert_eq!(context.control().latency(aux), Ok(5));
    assert_eq!(context.control().latency(NodeKey::MASTER), Ok(15));
    context.control().remove(late, None).unwrap();
    assert_eq!(context.control().latency(NodeKey::MASTER), Ok(10));
}

/// Delays what it hears by its length, and says so.
struct Late {
    ring: Vec<f32>,
    next: usize,
}

fn late(frames: usize) -> Box<dyn Processor> {
    Box::new(Late {
        ring: vec![0.0; frames],
        next: 0,
    })
}

impl Processor for Late {
    fn kind(&self) -> &'static str {
        "late"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }

    fn set(&mut self, _index: usize, _value: f64) {}

    fn value(&self, _index: usize) -> f64 {
        0.0
    }

    fn latency(&self) -> u64 {
        self.ring.len() as u64
    }

    fn process(&mut self, io: &mut Io<'_>) {
        let mut block = io.main();
        let block = &mut block;
        for sample in block.channel(0) {
            std::mem::swap(sample, &mut self.ring[self.next]);
            self.next = (self.next + 1) % self.ring.len();
        }
    }
}

#[test]
fn paths_of_different_latency_line_up_where_they_meet() {
    let click = pcm(&[0.5], 48_000);
    let render = |sizes: &[usize]| {
        let mut context = mono(AudioConfig::default());
        let slow = bus(&mut context, NodeKey::MASTER, 1.0);
        let plain = bus(&mut context, NodeKey::MASTER, 1.0);
        let room = bus(&mut context, NodeKey::MASTER, 1.0);
        let lookahead = context.control().insert(slow, late(3), None).unwrap();
        // The room hears the slow bus and the plain one, so it lines them up too, and the master
        // waits for the room.
        context.control().send_to(slow, room, 1.0, Tap::PostFader, None).unwrap();
        context.control().send_to(plain, room, 1.0, Tap::PostFader, None).unwrap();
        for target in [slow, plain, NodeKey::MASTER] {
            context.control().play(&click, &into(target, 1)).unwrap();
        }
        // Without the slow bus's processor nothing waits, from the frame it goes.
        context.control().remove(lookahead, Some(20)).unwrap();
        for target in [slow, plain, NodeKey::MASTER] {
            context.control().play(&click, &into(target, 30)).unwrap();
        }
        let mut out = Vec::new();
        for size in sizes.iter().cycle() {
            if out.len() >= 40 {
                break;
            }
            let mut block = vec![0.0; *size];
            let (_, engine) = context.parts();
            assert_eq!(allocations(|| engine.render(&mut block)), 0);
            out.extend(block);
        }
        out.truncate(40);
        assert_eq!(context.control().latency(NodeKey::MASTER), Ok(0));
        out
    };
    let out = render(&[40]);
    let mut expected = [0.0; 40];
    // Each click reaches the master on its own, and the slow and plain ones again through the
    // room: five halves, all on the frame the slowest path arrives.
    expected[4] = 2.5;
    expected[30] = 2.5;
    assert_eq!(out, expected);
    assert_eq!(render(&[1, 3, 7, 128]), out);
}

#[test]
fn a_meter_reads_what_its_bus_put_out_since_it_was_last_read() {
    let ramp = pcm(&[0.5, -1.0, 0.5, -1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let group = bus(&mut context, NodeKey::MASTER, 0.5);
    context.control().play(&ramp, &into(group, 0)).unwrap();
    context.render(4);
    let meter = context.control().meter(group).unwrap();
    assert_eq!(meter.peak, 0.5);
    // Samples of 0.25 and 0.5 after the gain: the root of their mean square.
    assert!((meter.rms - (0.15625f32).sqrt()).abs() < 1e-6, "{}", meter.rms);
    assert_eq!(context.control().meter(NodeKey::MASTER).unwrap().peak, 0.5);
    context.render(4);
    assert_eq!(context.control().meter(group).unwrap(), Meter { peak: 0.0, rms: 0.0 });
}

fn glide(frames: u64, curve: Curve) -> Glide {
    Glide { frames, curve }
}

#[test]
fn a_voice_gain_ramps_sample_by_sample_from_its_frame() {
    let tone = pcm(&[1.0; 10], 48_000);
    let mut context = mono(AudioConfig::default());
    let voice = context.control().play(&tone, &at(0)).unwrap();
    let ramp = Some(glide(4, Curve::Linear));
    context.control().set(voice, VoiceParam::Gain, 0.0, Some(2), ramp).unwrap();
    assert_eq!(context.render(8), [1.0, 1.0, 1.0, 0.75, 0.5, 0.25, 0.0, 0.0]);
}

#[test]
fn a_bus_gain_ramps_exponentially_in_even_ratios() {
    let tone = pcm(&[1.0; 6], 48_000);
    let mut context = mono(AudioConfig::default());
    let group = bus(&mut context, NodeKey::MASTER, 1.0);
    context.control().play(&tone, &into(group, 0)).unwrap();
    let ramp = Some(glide(3, Curve::Exponential));
    context.control().set_param(group, "gain", 8.0, Some(1), ramp).unwrap();
    let out = context.render(5);
    for (sample, expected) in out.iter().zip([1.0, 1.0, 2.0, 4.0, 8.0]) {
        assert!((sample - expected).abs() < 1e-5, "{out:?}");
    }
    assert!(matches!(
        context
            .control()
            .set_param(group, "gain", 0.0, None, Some(glide(3, Curve::Exponential))),
        Err(AudioError::Parameter { .. })
    ));
}

#[test]
fn a_voice_fades_in_and_a_stop_fades_out_before_it_ends() {
    let tone = pcm(&[1.0; 16], 48_000);
    let mut context = mono(AudioConfig::default());
    let spec = PlaySpec { fade: 4, ..at(0) };
    let voice = context.control().play(&tone, &spec).unwrap();
    context.control().stop(voice, Some(6), 4).unwrap();
    let out = context.render(12);
    assert_eq!(out, [0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 0.75, 0.5, 0.25, 0.0, 0.0]);
    assert_eq!(context.control().state(voice), VoiceState::Ended);
}

#[test]
fn a_rate_glide_moves_through_the_source_at_the_rates_between() {
    let ramp: Vec<f32> = (0..1000u16).map(f32::from).collect();
    let mut context = mono(AudioConfig::default());
    let voice = context.control().play(&pcm(&ramp, 48_000), &at(0)).unwrap();
    context
        .control()
        .set(voice, VoiceParam::Rate, 2.0, None, Some(glide(100, Curve::Linear)))
        .unwrap();
    let out = context.render(201);
    // A hundred frames at rates rising from 1 to 2 cover about 150 source frames; after that, 2.
    assert!((out[100] - 149.5).abs() < 1.0, "{}", out[100]);
    assert!((out[200] - (out[100] + 200.0)).abs() < 1e-3, "{} {}", out[100], out[200]);
}

#[test]
fn a_loop_let_go_on_its_frame_plays_on_into_the_outro() {
    // Intro 10, loop 1 and 2, outro 30 and 31.
    let samples = [10.0, 1.0, 2.0, 30.0, 31.0];
    let mut context = mono(AudioConfig::default());
    let spec = PlaySpec {
        looping: Some(LoopRegion { start: 1, end: 3 }),
        ..at(0)
    };
    let voice = context.control().play(&pcm(&samples, 48_000), &spec).unwrap();
    context.control().set_loop(voice, None, Some(6)).unwrap();
    assert_eq!(context.render(10), [10.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 30.0, 31.0, 0.0]);
    assert_eq!(context.control().state(voice), VoiceState::Ended);
    let outside = Some(LoopRegion { start: 0, end: 9 });
    let again = context.control().play(&pcm(&samples, 48_000), &at(20)).unwrap();
    assert!(matches!(
        context.control().set_loop(again, outside, None),
        Err(AudioError::LoopRegion { .. })
    ));
}

#[test]
fn a_group_arrives_whole_or_not_at_all() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig {
        commands: 3,
        ..AudioConfig::default()
    });
    context.control().begin_group().unwrap();
    assert_eq!(context.control().begin_group(), Err(AudioError::GroupOpen));
    context.control().play(&click, &PlaySpec::default()).unwrap();
    context.control().play(&click, &PlaySpec::default()).unwrap();
    assert_eq!(
        context.control().play(&click, &PlaySpec::default()),
        Err(AudioError::QueueFull(3)),
        "the group and its members fill the queue"
    );
    context.control().end_group().unwrap();
    assert_eq!(context.render(2), [2.0, 0.0]);
    assert_eq!(context.control().room(), 3);
}

/// Multiplies by a control-rate level: it hears where a ramp has got to once a control block.
struct Level(f32);

impl Processor for Level {
    fn kind(&self) -> &'static str {
        "level"
    }

    fn params(&self) -> &'static [ParamInfo] {
        const PARAMS: &[ParamInfo] = &[ParamInfo::control("level", 0.0, 1.0, 1.0, "")];
        PARAMS
    }

    #[allow(clippy::cast_possible_truncation)] // Ranged on the control side.
    fn set(&mut self, _index: usize, value: f64) {
        self.0 = value as f32;
    }

    fn value(&self, _index: usize) -> f64 {
        f64::from(self.0)
    }

    fn process(&mut self, io: &mut Io<'_>) {
        for sample in io.main().channel(0) {
            *sample *= self.0;
        }
    }
}

#[test]
fn a_control_rate_parameter_ramps_every_control_block() {
    let tone = pcm(&[1.0; 128], 48_000);
    let mut context = mono(AudioConfig::default());
    let level = context.control().insert(NodeKey::MASTER, Box::new(Level(1.0)), None).unwrap();
    context.control().play(&tone, &at(0)).unwrap();
    let ramp = Some(glide(64, Curve::Linear));
    context.control().set_param(level, "level", 0.0, None, ramp).unwrap();
    let out = context.render(100);
    assert_eq!(out[..32], [1.0; 32]);
    assert_eq!(out[32..64], [0.5; 32]);
    assert_eq!(out[64..], [0.0; 36]);
}

#[test]
fn an_audio_rate_parameter_ramps_every_frame() {
    let tone = pcm(&[1.0; 128], 48_000);
    let mut context = mono(AudioConfig::default());
    let gain = context
        .control()
        .insert(NodeKey::MASTER, Box::new(Gain::new()), None)
        .unwrap();
    context.control().play(&tone, &at(0)).unwrap();
    let ramp = Some(glide(64, Curve::Linear));
    context.control().set_param(gain, "gain", 0.0, None, ramp).unwrap();
    let out = context.render(100);
    for (frame, sample) in out[..64].iter().enumerate() {
        #[allow(clippy::cast_precision_loss)] // A frame in a ramp.
        let expected = 1.0 - frame as f32 / 64.0;
        assert!((sample - expected).abs() < 1e-6, "frame {frame}: {sample}");
    }
    assert_eq!(out[64..], [0.0; 36]);
}

#[test]
fn a_compressor_listening_to_dialogue_ducks_the_music_while_it_speaks() {
    let mut context = mono(AudioConfig::default());
    let music = bus(&mut context, NodeKey::MASTER, 1.0);
    let dialogue = bus(&mut context, NodeKey::MASTER, 1.0);
    let settings = {
        let mut settings = crate::audio::processor::Settings::new();
        settings.insert("ratio", crate::audio::processor::Setting::Number(20.0));
        settings.insert("knee", crate::audio::processor::Setting::Number(0.0));
        settings
    };
    let setup = crate::audio::processor::Setup {
        rate: 48_000,
        channels: 1,
    };
    let ducker = crate::audio::Registry::builtin()
        .create("compressor", &setup, settings)
        .unwrap();
    let ducker = context.control().insert(music, ducker, None).unwrap();
    assert!(context.control().listen(ducker, Some(dialogue), None).unwrap());
    context
        .control()
        .play(&pcm(&vec![0.1; 48_000], 48_000), &into(music, 0))
        .unwrap();
    context
        .control()
        .play(&pcm(&vec![0.5; 24_000], 48_000), &into(dialogue, 12_000))
        .unwrap();
    context.render(12_000);
    assert!(
        (context.control().meter(music).unwrap().peak - 0.1).abs() < 1e-6,
        "untouched before anyone speaks"
    );
    context.render(12_000);
    let level = context.control().meter(music).unwrap().rms;
    assert!(level < 0.03, "{level}");

    // A key the music feeds would be a loop; a bus that feeds the music is run before it anyway.
    let inner = bus(&mut context, music, 1.0);
    assert_eq!(
        context.control().listen(ducker, Some(NodeKey::MASTER), None),
        Err(AudioError::Cycle)
    );
    assert!(context.control().listen(ducker, Some(inner), None).unwrap());
    assert_eq!(context.control().listen(ducker, Some(music), None), Err(AudioError::Cycle));
    // A key goes with its connection to the sidechain when it's removed.
    context.render(30_000);
    assert!(context.control().listen(ducker, Some(dialogue), None).unwrap());
    assert!(context.control().remove(dialogue, None).unwrap());
    context.render(1);
    context.control().remove(ducker, None).unwrap();
    context.render(1);
    assert_eq!(context.control().stats().allocated, 0);
}

fn placed(position: [f64; 3]) -> PlaySpec {
    PlaySpec {
        spatial: Some(Emitter {
            position,
            ..Emitter::default()
        }),
        ..at(0)
    }
}

#[test]
fn a_voice_in_space_pans_toward_its_side_and_fades_with_distance() {
    let tone = pcm(&[1.0; 256], 48_000);
    let mut context = OfflineContext::new(&AudioConfig::default()).unwrap();
    context.control().play(&tone, &placed([2.0, 0.0, 0.0])).unwrap();
    let out = context.render(4);
    // Hard right at half its level: inverse distance at twice the reference.
    assert!(out[0].abs() < 1e-6 && (out[1] - 0.5).abs() < 1e-6, "{out:?}");

    let mut context = OfflineContext::new(&AudioConfig::default()).unwrap();
    context.control().play(&tone, &placed([0.0, 0.0, -1.0])).unwrap();
    let out = context.render(4);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    assert!(
        (out[0] - half).abs() < 1e-6 && (out[1] - half).abs() < 1e-6,
        "ahead: both sides"
    );
}

#[test]
fn a_voice_moved_glides_to_its_new_place_over_a_block() {
    let tone = pcm(&[1.0; 1024], 48_000);
    let mut context = OfflineContext::new(&AudioConfig::default()).unwrap();
    let voice = context.control().play(&tone, &placed([-1.0, 0.0, 0.0])).unwrap();
    context.render(QUANTUM);
    let right = Emitter {
        position: [1.0, 0.0, 0.0],
        ..Emitter::default()
    };
    context.control().place(voice, right, Some(QUANTUM as u64)).unwrap();
    let out = context.render(QUANTUM * 2);
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    // From hard left to hard right across the block, without a step anywhere, then settled.
    assert!((left[0] - 1.0).abs() < 0.02 && left[QUANTUM - 1].abs() < 1e-6);
    assert!(left.windows(2).take(QUANTUM - 1).all(|pair| pair[1] <= pair[0] + 1e-6));
    assert!(left[QUANTUM..].iter().all(|sample| sample.abs() < 1e-6));
}

#[test]
fn an_approaching_voice_plays_faster_and_a_wall_darkens_it() {
    let ramp: Vec<f32> = (0..4000u16).map(f32::from).collect();
    let mut context = mono(AudioConfig::default());
    let approaching = PlaySpec {
        spatial: Some(Emitter {
            position: [0.0, 0.0, -100.0],
            velocity: [0.0, 0.0, 34.3],
            distance: Distance::None,
            ..Emitter::default()
        }),
        ..at(0)
    };
    context.control().play(&pcm(&ramp, 48_000), &approaching).unwrap();
    let out = context.render(1001);
    // A ninth faster: a thousand frames reach a thousand and a ninth of a thousand on.
    assert!((out[1000] - 1000.0 / 0.9).abs() < 0.5, "{}", out[1000]);

    let fizz: Vec<f32> = (0..4800).map(|frame| if frame % 2 == 0 { 1.0 } else { -1.0 }).collect();
    let mut context = mono(AudioConfig::default());
    let behind_a_wall = PlaySpec {
        spatial: Some(Emitter {
            distance: Distance::None,
            lowpass: 500.0,
            gain: 0.5,
            ..Emitter::default()
        }),
        ..at(0)
    };
    context.control().play(&pcm(&fizz, 48_000), &behind_a_wall).unwrap();
    let out = context.render(4800);
    assert!(out[2400..].iter().all(|sample| sample.abs() < 0.05), "the highs are gone");
}

#[test]
fn turning_the_listener_turns_the_voices_around_it() {
    let tone = pcm(&[1.0; 1024], 48_000);
    let mut context = OfflineContext::new(&AudioConfig::default()).unwrap();
    context.control().play(&tone, &placed([1.0, 0.0, 0.0])).unwrap();
    let about_face = Listener {
        forward: [0.0, 0.0, 1.0],
        ..Listener::default()
    };
    context.control().set_listener(about_face, Some(QUANTUM as u64)).unwrap();
    let out = context.render(QUANTUM * 3);
    assert!(out[0].abs() < 1e-6, "on the right at first");
    assert!((out[2 * (QUANTUM * 2 + 10)] - 1.0).abs() < 1e-6, "on the left once turned");
}

#[test]
fn rendering_voices_in_space_never_allocates() {
    let tone = pcm(&[0.5; 2000], 44_100);
    let mut context = OfflineContext::new(&AudioConfig::default()).unwrap();
    for index in 0..32u32 {
        let spec = PlaySpec {
            spatial: Some(Emitter {
                position: [f64::from(index) - 16.0, 0.0, -3.0],
                velocity: [1.0, 0.0, 0.0],
                lowpass: if index % 2 == 0 { 2000.0 } else { f64::INFINITY },
                ..Emitter::default()
            }),
            ..at(u64::from(index) * 11)
        };
        let voice = context.control().play(&tone, &spec).unwrap();
        let moved = Emitter {
            position: [0.0, 0.0, -1.0],
            ..Emitter::default()
        };
        context.control().place(voice, moved, Some(500)).unwrap();
    }
    let mut out = vec![0.0; 2 * 300];
    let (_, engine) = context.parts();
    assert_eq!(
        allocations(|| {
            for _ in 0..10 {
                engine.render(&mut out);
            }
        }),
        0
    );
}

fn budget(physical: usize) -> OfflineContext {
    mono(AudioConfig {
        physical: Some(physical),
        ..AudioConfig::default()
    })
}

#[test]
fn a_budget_mixes_the_loudest_voices_and_culls_the_rest() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = budget(3);
    for index in 1..=6u8 {
        let spec = PlaySpec {
            gain: f32::from(index) / 10.0,
            ..at(0)
        };
        context.control().play(&tone, &spec).unwrap();
    }
    let out = context.render(4);
    assert!((out[0] - 1.5).abs() < 1e-6, "0.4 + 0.5 + 0.6: {}", out[0]);
    assert_eq!(context.control().stats().culled, 3);
}

#[test]
fn priority_keeps_a_quiet_voice_over_louder_ones() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = budget(1);
    let dialogue = PlaySpec {
        gain: 0.1,
        priority: 10.0,
        ..at(0)
    };
    context.control().play(&tone, &dialogue).unwrap();
    for _ in 0..4 {
        context.control().play(&tone, &at(0)).unwrap();
    }
    assert!((context.render(1)[0] - 0.1).abs() < 1e-6);
}

#[test]
fn a_culled_voice_keeps_time_or_holds_its_place_or_is_gone() {
    let ramp: Vec<f32> = (0..1000u16).map(f32::from).collect();
    let ramp = pcm(&ramp, 48_000);
    let loud = pcm(&[100.0; 200], 48_000);
    for (culled, expected) in [
        (Culled::Advance, Some(301.0)),
        (Culled::Pause, Some(100.0)),
        (Culled::Stop, None),
    ] {
        let mut context = budget(1);
        let spec = PlaySpec { culled, ..at(0) };
        let voice = context.control().play(&ramp, &spec).unwrap();
        context.render(100);
        // Something that matters more takes the only place for two hundred frames, and is let go on
        // the frame after, when the culled voice comes back.
        let urgent = PlaySpec {
            priority: 1.0,
            ..PlaySpec::default()
        };
        context.control().play(&loud, &urgent).unwrap();
        context.render(201);
        let out = context.render(1);
        if let Some(frame) = expected {
            assert!((out[0] - frame).abs() < 1e-3, "{culled:?}: {}", out[0]);
            continue;
        }
        assert_eq!(out[0], 0.0);
        assert_eq!(context.control().state(voice), VoiceState::Ended);
        assert_eq!(context.control().stats().stolen, 1);
    }
}

#[test]
fn a_culled_stream_keeps_time_or_holds_its_place_as_a_voice_does() {
    let ramp: Vec<f32> = (0..1000u16).map(f32::from).collect();
    let loud = pcm(&[100.0; 200], 48_000);
    for (culled, expected) in [(Culled::Advance, 301.0), (Culled::Pause, 100.0)] {
        let mut context = budget(1);
        let stream = Stream::new(1, 48_000, 1.0).unwrap();
        assert_eq!(stream.write(&ramp).unwrap(), 1000);
        let spec = PlaySpec { culled, ..at(0) };
        context.control().play_stream(&stream, &spec).unwrap();
        context.render(100);
        let urgent = PlaySpec {
            priority: 1.0,
            ..PlaySpec::default()
        };
        context.control().play(&loud, &urgent).unwrap();
        context.render(201);
        let out = context.render(1);
        assert!((out[0] - expected).abs() < 1e-3, "{culled:?}: {}", out[0]);
        assert_eq!(stream.stats().missed, 0, "{culled:?}");
    }

    // A culled stream its writer is behind on keeps time through silence, as a mixed one does.
    let mut context = budget(1);
    let stream = Stream::new(1, 48_000, 1.0).unwrap();
    stream.write(&ramp[..150]).unwrap();
    context.control().play_stream(&stream, &at(0)).unwrap();
    context.render(100);
    let urgent = PlaySpec {
        priority: 1.0,
        ..PlaySpec::default()
    };
    context.control().play(&loud, &urgent).unwrap();
    context.render(201);
    assert_eq!(stream.stats().read, 150);
    assert!(stream.stats().missed > 100, "{:?}", stream.stats());

    // A finished one that runs out while culled ends there.
    let mut context = budget(1);
    let stream = Stream::new(1, 48_000, 1.0).unwrap();
    stream.write(&ramp[..150]).unwrap();
    stream.finish();
    let voice = context.control().play_stream(&stream, &at(0)).unwrap();
    context.render(100);
    context.control().play(&loud, &urgent).unwrap();
    context.render(201);
    assert_eq!(context.control().state(voice), VoiceState::Ended);
    assert_eq!(stream.stats().missed, 0);
}

#[test]
fn choosing_voices_over_a_budget_never_allocates() {
    let tone = pcm(&[0.5; 3000], 48_000);
    let mut context = OfflineContext::new(&AudioConfig {
        physical: Some(8),
        ..AudioConfig::default()
    })
    .unwrap();
    for index in 0..100u16 {
        let spec = PlaySpec {
            gain: f32::from(index % 7) / 7.0,
            priority: f32::from(index % 3),
            culled: if index % 5 == 0 { Culled::Stop } else { Culled::Advance },
            ..at(u64::from(index) * 3)
        };
        context.control().play(&tone, &spec).unwrap();
    }
    let mut out = vec![0.0; 2 * 256];
    let (_, engine) = context.parts();
    assert_eq!(
        allocations(|| {
            for _ in 0..20 {
                engine.render(&mut out);
            }
        }),
        0
    );
}

#[test]
fn a_stream_written_between_renders_plays_the_same_as_its_pcm() {
    let tone: Vec<f32> = (0..1000u16).map(|frame| (f32::from(frame) / 37.0).sin()).collect();
    let mut whole = mono(AudioConfig::default());
    whole.control().play(&pcm(&tone, 24_000), &at(0)).unwrap();
    let expected = whole.render(2000);

    let mut context = mono(AudioConfig::default());
    let stream = Stream::new(1, 24_000, 0.01).unwrap();
    context.control().play_stream(&stream, &at(0)).unwrap();
    assert_eq!(context.control().play_stream(&stream, &at(0)), Err(AudioError::StreamClaimed));
    let mut written = 0;
    let mut out = Vec::new();
    while out.len() < 2000 {
        // The writer keeps a little ahead of the engine, the way a generator or decoder would.
        while written < tone.len() && stream.room() > 0 {
            written += stream.write(&tone[written..(written + 64).min(tone.len())]).unwrap();
        }
        if written == tone.len() {
            stream.finish();
        }
        out.extend(context.render(100));
    }
    assert_eq!(out, expected);
    assert_eq!(stream.stats().missed, 0);
}

#[test]
fn a_stream_that_runs_dry_plays_counted_silence_and_ends_once_finished() {
    let mut context = mono(AudioConfig::default());
    let stream = Stream::new(1, 48_000, 0.1).unwrap();
    let voice = context.control().play_stream(&stream, &at(0)).unwrap();
    stream.write(&[1.0; 10]).unwrap();
    let out = context.render(20);
    assert_eq!(out[..10], [1.0; 10]);
    assert_eq!(out[10..], [0.0; 10]);
    assert!(stream.stats().missed >= 10);
    stream.write(&[0.5; 5]).unwrap();
    stream.finish();
    let out = context.render(20);
    assert!(out.contains(&0.5), "what came late plays after the gap");
    context.render(QUANTUM);
    assert_eq!(context.control().state(voice), VoiceState::Ended);
}

#[test]
fn rendering_a_stream_never_allocates() {
    let mut context = OfflineContext::new(&AudioConfig::default()).unwrap();
    let stream = Stream::new(2, 44_100, 1.0).unwrap();
    let spec = PlaySpec { rate: 1.5, ..at(0) };
    context.control().play_stream(&stream, &spec).unwrap();
    stream.write(&vec![0.25; 2 * 20_000]).unwrap();
    let mut out = vec![0.0; 2 * 300];
    let (_, engine) = context.parts();
    assert_eq!(
        allocations(|| {
            for _ in 0..20 {
                engine.render(&mut out);
            }
        }),
        0
    );
}

#[test]
fn a_stream_written_from_another_thread_arrives_whole_and_in_order() {
    const FRAMES: u16 = 40_000;
    let mut context = mono(AudioConfig::default());
    let stream = Stream::new(1, 48_000, 0.01).unwrap();
    let voice = context.control().play_stream(&stream, &at(0)).unwrap();
    let id = stream.id();
    let writer = std::thread::spawn(move || {
        // The writer has only the id, as a worker has only a handle.
        let stream = Stream::find(id).unwrap();
        let ramp: Vec<f32> = (0..FRAMES).map(f32::from).collect();
        let mut written = 0;
        while written < ramp.len() {
            assert!(stream.wait(64, std::time::Duration::from_secs(10)));
            written += stream.write(&ramp[written..(written + 64).min(ramp.len())]).unwrap();
        }
        stream.finish();
    });
    let mut out = Vec::new();
    while context.control().state(voice) != VoiceState::Ended {
        // Keep behind the writer, as a device's buffer would.
        let stats = stream.stats();
        if !stats.finished && stats.written - stats.read < 200 {
            std::thread::yield_now();
            continue;
        }
        out.extend(context.render(64));
    }
    writer.join().unwrap();
    let played: Vec<f32> = out.into_iter().take(usize::from(FRAMES)).collect();
    let expected: Vec<f32> = (0..FRAMES).map(f32::from).collect();
    assert_eq!(played, expected);
    assert_eq!(stream.stats().missed, 0);
}

#[test]
fn voices_come_and_go_without_revising_the_graph() {
    let shot = pcm(&[0.5; 40], 48_000);
    let mut context = mono(AudioConfig {
        voices: 64,
        physical: Some(16),
        ..AudioConfig::default()
    });
    let effects = bus(&mut context, NodeKey::MASTER, 1.0);
    let room = bus(&mut context, NodeKey::MASTER, 1.0);
    context
        .control()
        .insert(room, Box::new(crate::audio::dsp::reverb::Reverb::new(48_000)), None)
        .unwrap();
    context.render(1);
    let revised = context.control().stats().revisions;
    assert_eq!(revised, 3, "two buses and an insert");
    for round in 0..200u64 {
        let spec = PlaySpec {
            sends: vec![VoiceSend { bus: room, level: 0.5 }],
            ..into(effects, round * 16 + 1)
        };
        let voice = context.control().play(&shot, &spec).unwrap();
        context.control().play(&shot, &into(effects, round * 16 + 3)).unwrap();
        if round % 3 == 0 {
            context.control().stop(voice, Some(round * 16 + 9), 4).unwrap();
        }
        let (_, engine) = context.parts();
        let mut block = vec![0.0; 16];
        assert_eq!(allocations(|| engine.render(&mut block)), 0);
        context.control().collect();
    }
    assert_eq!(context.control().stats().revisions, revised);
}

#[test]
fn a_revision_leaves_every_node_it_does_not_touch_as_it_was() {
    let click = pcm(&[1.0], 48_000);
    let render = |revise: bool| {
        let mut context = mono(AudioConfig::default());
        let room = bus(&mut context, NodeKey::MASTER, 1.0);
        context
            .control()
            .insert(room, Box::new(crate::audio::dsp::reverb::Reverb::new(48_000)), None)
            .unwrap();
        context.control().play(&click, &into(room, 0)).unwrap();
        // The bus's fader is halfway down a ramp when the graph changes around it.
        context
            .control()
            .set_param(room, "gain", 0.25, Some(0), Some(glide(4000, Curve::Linear)))
            .unwrap();
        let mut out = context.render(1000);
        if revise {
            let other = bus(&mut context, NodeKey::MASTER, 1.0);
            let aux = bus(&mut context, other, 1.0);
            context
                .control()
                .send_to(aux, NodeKey::MASTER, 0.5, Tap::PostFader, None)
                .unwrap();
            context.control().remove(other, None).unwrap_err();
            context.control().unsend(aux, NodeKey::MASTER, None).unwrap();
            context.control().remove(aux, None).unwrap();
            context.control().remove(other, None).unwrap();
        }
        for _ in 0..30 {
            let (_, engine) = context.parts();
            let mut block = vec![0.0; 100];
            assert_eq!(allocations(|| engine.render(&mut block)), 0);
            out.extend(block);
            context.control().collect();
        }
        (out, context.control().stats().revisions)
    };
    let (still, before) = render(false);
    let (revised, after) = render(true);
    assert!(after > before, "the graph changed {after} times, {before} without");
    assert!(still[1500..].iter().any(|sample| *sample != 0.0), "the room rings on");
    assert_eq!(
        revised, still,
        "the reverb's tank and the fader's ramp carried on through every plan"
    );
}

#[test]
fn revisions_take_effect_in_the_order_they_are_made() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = mono(AudioConfig::default());
    let first = bus(&mut context, NodeKey::MASTER, 0.5);
    let second = bus(&mut context, NodeKey::MASTER, 0.25);
    context.control().play(&tone, &into(first, 0)).unwrap();
    context.control().route(first, second, Some(10)).unwrap();
    assert_eq!(
        context.control().route(first, NodeKey::MASTER, Some(5)),
        Err(AudioError::GraphOrder { pending: 10 })
    );
    assert_eq!(
        context.control().add_bus(&BusSpec::default()),
        Err(AudioError::GraphOrder { pending: 10 }),
        "a change for now waits for none"
    );
    // On or after the frame of the one waiting is fine, and the refused changes changed nothing.
    context.control().route(first, NodeKey::MASTER, Some(12)).unwrap();
    let out = context.render(16);
    assert_eq!(out[..10], [0.5; 10]);
    assert_eq!(out[10..12], [0.125; 2]);
    assert_eq!(out[12..], [0.5; 4]);
    assert!(context.control().add_bus(&BusSpec::default()).is_ok(), "nothing waits now");
}

mod graph;
mod modulation;
