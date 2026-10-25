//! The engine against renders worked out by hand.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use crate::audio::{
    AudioConfig, AudioError, BusKey, BusParam, BusSpec, LoopRegion, OfflineContext, Pcm, PlaySpec, QUANTUM, VoiceParam,
    VoiceState,
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
    assert!(context.control().set(voice, VoiceParam::Gain, 0.5, Some(20)).unwrap());
    assert!(context.control().stop(voice, Some(30)).unwrap());
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
    context.control().set(quiet, VoiceParam::Gain, 0.25, None).unwrap();
    context.control().stop(cancelled, None).unwrap();
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
    assert!(!context.control().stop(first, None).unwrap(), "already ended");
    let reused = context.control().play(&click, &at(10)).unwrap();
    assert_ne!(reused, first, "a reused slot is a new voice");
    assert_eq!(context.control().state(first), VoiceState::Ended);
    assert!(!context.control().set(first, VoiceParam::Gain, 0.0, None).unwrap());
    assert_eq!(context.render(12)[6], 1.0, "the stale set didn't reach the new voice");
}

#[test]
fn a_full_queue_and_bad_parameters_are_errors() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig {
        commands: 2,
        ..AudioConfig::default()
    });
    context.control().play(&click, &at(10)).unwrap();
    context.control().play(&click, &at(10)).unwrap();
    assert_eq!(context.control().play(&click, &at(10)), Err(AudioError::QueueFull(2)));
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
        context.control().set(*voice, VoiceParam::Gain, 0.5, Some(frame)).unwrap();
        context.control().stop(*voice, Some(frame + 400)).unwrap();
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

fn bus(context: &mut OfflineContext, parent: BusKey, gain: f32) -> BusKey {
    let spec = BusSpec {
        parent,
        gain,
        ..BusSpec::default()
    };
    context.control().add_bus(&spec).unwrap()
}

fn into(bus: BusKey, frame: u64) -> PlaySpec {
    PlaySpec { bus, ..at(frame) }
}

#[test]
fn voices_sum_through_nested_buses_at_each_bus_s_gain() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let music = bus(&mut context, BusKey::MASTER, 0.5);
    let strings = bus(&mut context, music, 0.5);
    let effects = bus(&mut context, BusKey::MASTER, 2.0);
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
    let group = bus(&mut context, BusKey::MASTER, 1.0);
    context.control().play(&tone, &into(group, 0)).unwrap();
    context.control().set_bus(group, BusParam::Gain, 0.5, Some(2)).unwrap();
    context.control().set_bus(group, BusParam::Mute, 1.0, Some(4)).unwrap();
    context.control().set_bus(group, BusParam::Mute, 0.0, Some(6)).unwrap();
    context
        .control()
        .set_bus(BusKey::MASTER, BusParam::Gain, 4.0, Some(7))
        .unwrap();
    assert_eq!(context.render(8), [1.0, 1.0, 0.5, 0.5, 0.0, 0.0, 0.5, 2.0]);
}

#[test]
fn rerouting_moves_a_bus_on_its_frame_and_a_loop_is_refused() {
    let tone = pcm(&[1.0; 4], 48_000);
    let mut context = mono(AudioConfig::default());
    let quiet = bus(&mut context, BusKey::MASTER, 0.25);
    let group = bus(&mut context, BusKey::MASTER, 1.0);
    context.control().play(&tone, &into(group, 0)).unwrap();
    context.control().route_bus(group, quiet, Some(2)).unwrap();
    assert_eq!(context.render(4), [1.0, 1.0, 0.25, 0.25]);

    assert_eq!(context.control().route_bus(quiet, group, None), Err(AudioError::Cycle));
    assert_eq!(context.control().route_bus(group, group, None), Err(AudioError::Cycle));
    assert_eq!(
        context.control().route_bus(BusKey::MASTER, group, None),
        Err(AudioError::Master)
    );
}

#[test]
fn a_bus_is_removed_only_once_nothing_routes_into_it() {
    let tone = pcm(&[1.0; 4], 48_000);
    let mut context = mono(AudioConfig {
        buses: 3,
        ..AudioConfig::default()
    });
    let outer = bus(&mut context, BusKey::MASTER, 1.0);
    let inner = bus(&mut context, outer, 1.0);
    assert_eq!(
        context.control().add_bus(&BusSpec::default()),
        Err(AudioError::BusesExhausted(3))
    );
    let voice = context.control().play(&tone, &into(inner, 0)).unwrap();
    assert_eq!(
        context.control().remove_bus(inner, None),
        Err(AudioError::BusInUse { voices: 1, buses: 0 })
    );
    assert_eq!(
        context.control().remove_bus(outer, None),
        Err(AudioError::BusInUse { voices: 0, buses: 1 })
    );
    context.control().stop(voice, None).unwrap();
    context.render(1);
    assert!(context.control().remove_bus(inner, None).unwrap());
    assert!(!context.control().bus_live(inner));
    assert_eq!(context.control().play(&tone, &into(inner, 0)), Err(AudioError::StaleBus));
    context.render(1);
    assert!(context.control().remove_bus(outer, None).unwrap());
    context.render(1);
    let reused = bus(&mut context, BusKey::MASTER, 1.0);
    assert_ne!(reused, outer, "a reused slot is a new bus");
    assert!(!context.control().set_bus(outer, BusParam::Gain, 0.0, None).unwrap());
    assert_eq!(context.control().remove_bus(BusKey::MASTER, None), Err(AudioError::Master));
}
