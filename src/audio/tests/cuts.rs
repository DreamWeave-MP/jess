//! A render is the same however it is cut: every processor, ramp, modulation, voice in space,
//! and revision lands on the same frames whether the engine is asked for a thousand frames at
//! once or for blocks of any length.

use super::{allocations, pcm};
use crate::audio::processor::{Setting, Settings, Setup};
use crate::audio::{
    AudioConfig, BusSpec, Connection, Culled, Curve, Edit, Emitter, Glide, Listener, NodeKey, OfflineContext, PlaySpec,
    Registry, VoiceParam,
};

fn glide(frames: u64) -> Glide {
    Glide {
        frames,
        curve: Curve::Linear,
    }
}

fn connect(from: NodeKey, to: NodeKey, input: &str, connection: Connection) -> Edit {
    Edit::Connect {
        from,
        output: "out".to_owned(),
        to,
        input: input.to_owned(),
        connection,
    }
}

/// A source frame by frame unlike any other: a few sines that don't divide the rate, so any
/// frame played early or late shows.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // Small whole numbers, as f32.
fn tone(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|frame| {
            let time = frame as f64 / 48_000.0;
            let wave = |frequency: f64, level: f64| (time * frequency * std::f64::consts::TAU).sin() * level;
            let sum = wave(2_000.0, 0.4) + wave(317.0, 0.3) + wave(5_011.0, 0.2);
            sum as f32
        })
        .collect()
}

/// Every built-in processor, ramps of both rates starting between control frames, modulation of
/// both rates, a feedback loop, voices in space moving, more voices than the budget mixes, and a
/// patch, all stamped for frames that fall inside blocks.
#[allow(clippy::too_many_lines)] // One scene.
fn scene() -> OfflineContext {
    let mut context = OfflineContext::new(&AudioConfig {
        physical: Some(2),
        ..AudioConfig::default()
    })
    .unwrap();
    let registry = Registry::builtin();
    let make = |kind: &str, settings: Settings| {
        let setup = Setup {
            rate: 48_000,
            channels: 2,
        };
        registry.create(kind, &setup, settings).unwrap()
    };
    let control = context.control();

    // A bus with every effect that keeps state inserted, each ramped from a frame of its own.
    let music = control.add_bus(&BusSpec::default()).unwrap();
    let ramps: [(&str, &str, f64, u64, u64); 8] = [
        ("filter", "frequency", 300.0, 17, 700),
        ("compressor", "threshold", -40.0, 333, 50),
        ("drive", "drive", 30.0, 45, 1000),
        ("delay", "time", 0.002, 61, 900),
        ("reverb", "room", 0.9, 250, 1300),
        ("pan", "pan", -0.8, 5, 1700),
        ("limiter", "ceiling", -12.0, 1001, 3),
        ("gain", "gain", 0.25, 77, 999),
    ];
    let mut chain = Vec::new();
    for (kind, param, value, at, frames) in ramps {
        let mut settings = Settings::new();
        if kind == "delay" {
            settings.insert("longest", Setting::Number(0.01));
        }
        let node = control.insert(music, make(kind, settings), None).unwrap();
        control.set_param(node, param, value, Some(at), Some(glide(frames))).unwrap();
        chain.push(node);
    }
    // A ramp cut short, and a step between control frames.
    control.cancel(chain[0], "frequency", Some(500)).unwrap();
    control.set_param(chain[1], "ratio", 12.0, Some(613), None).unwrap();
    control.set_param(music, "gain", 0.5, Some(1201), Some(glide(300))).unwrap();

    // An oscillator sweeping a filter's cutoff (control rate), and an envelope opening an
    // amplifier (audio rate), its gate opened and closed between control frames.
    let sweep = control.add_node(make("filter", Settings::new()), None).unwrap();
    control.route(sweep, NodeKey::MASTER, None).unwrap();
    let mut lfo = Settings::new();
    lfo.insert("wave", Setting::Text("triangle".to_owned()));
    let lfo = control.add_node(make("oscillator", lfo), Some(1)).unwrap();
    control.set_param(lfo, "frequency", 90.0, None, None).unwrap();
    let amplifier = control.add_node(make("gain", Settings::new()), None).unwrap();
    control.set_param(amplifier, "gain", 0.0, None, None).unwrap();
    let envelope = control.add_node(make("envelope", Settings::new()), Some(1)).unwrap();
    control.set_param(envelope, "attack", 0.002, None, None).unwrap();
    control.set_param(envelope, "gate", 1.0, Some(101), None).unwrap();
    control.set_param(envelope, "gate", 0.0, Some(1403), None).unwrap();
    let mut saw = Settings::new();
    saw.insert("wave", Setting::Text("saw".to_owned()));
    let saw = control.add_node(make("oscillator", saw), None).unwrap();
    control
        .set_param(saw, "frequency", 2_500.0, Some(9), Some(glide(2000)))
        .unwrap();
    let blend = control.add_node(make("crossfade", Settings::new()), None).unwrap();
    control.route(blend, NodeKey::MASTER, None).unwrap();
    control.set_param(blend, "mix", 0.9, Some(150), Some(glide(1100))).unwrap();
    let modulation = |depth| Connection {
        level: depth,
        ..Connection::default()
    };
    control
        .patch(
            &[
                connect(lfo, sweep, "frequency", modulation(400.0)),
                connect(envelope, amplifier, "gain", modulation(1.0)),
                connect(saw, amplifier, "in", Connection::default()),
                connect(amplifier, blend, "a", Connection::default()),
                connect(music, blend, "b", modulation(0.5)),
            ],
            None,
        )
        .unwrap();

    // An echo: a bus feeding itself back through half its level.
    let echo = control.add_bus(&BusSpec::default()).unwrap();
    let half = control.add_node(make("gain", Settings::new()), None).unwrap();
    control.set_param(half, "gain", 0.5, None, None).unwrap();
    control
        .patch(
            &[
                connect(echo, half, "in", Connection::default()),
                connect(
                    half,
                    echo,
                    "in",
                    Connection {
                        feedback: Some(300),
                        ..Connection::default()
                    },
                ),
            ],
            None,
        )
        .unwrap();

    // Voices: one ramping, one resampled and moving in space, one too quiet to be mixed while
    // the budget is full, and one through the sweep and the echo.
    let source = pcm(&tone(4000), 48_000);
    let first = control
        .play(
            &source,
            &PlaySpec {
                bus: music,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    control.set(first, VoiceParam::Gain, 0.3, Some(37), Some(glide(500))).unwrap();
    let emitter = |x: f64| Emitter {
        position: [x, 0.0, -2.0],
        ..Emitter::default()
    };
    let moving = control
        .play(
            &source,
            &PlaySpec {
                at: Some(19),
                rate: 1.3,
                spatial: Some(emitter(-3.0)),
                ..PlaySpec::default()
            },
        )
        .unwrap();
    control
        .set(moving, VoiceParam::Rate, 0.8, Some(222), Some(glide(900)))
        .unwrap();
    for (at, x) in [(211, -1.0), (650, 2.0), (1333, 5.0)] {
        control.place(moving, emitter(x), Some(at)).unwrap();
    }
    control
        .set_listener(
            Listener {
                position: [0.5, 0.0, 0.0],
                ..Listener::default()
            },
            Some(999),
        )
        .unwrap();
    let quiet = control
        .play(
            &source,
            &PlaySpec {
                at: Some(400),
                gain: 0.05,
                spatial: Some(emitter(0.0)),
                priority: -1.0,
                culled: Culled::Advance,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    control.stop(quiet, Some(1501), 200).unwrap();
    control
        .play(
            &source,
            &PlaySpec {
                at: Some(777),
                bus: sweep,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    control
        .play(
            &source,
            &PlaySpec {
                at: Some(3),
                bus: echo,
                gain: 0.5,
                ..PlaySpec::default()
            },
        )
        .unwrap();

    // A revision mid-render: the echo also goes through the music chain from here.
    control
        .patch(&[connect(echo, music, "in", modulation(0.3))], Some(1111))
        .unwrap();
    context
}

/// Renders `frames` of a fresh scene in blocks of `sizes`, round and round, none allocating.
fn cut(frames: usize, sizes: impl IntoIterator<Item = usize>) -> Vec<f32> {
    let mut context = scene();
    let channels = 2;
    let mut out = Vec::new();
    for size in sizes {
        if out.len() >= frames * channels {
            break;
        }
        let (_, engine) = context.parts();
        let mut block = vec![0.0; size.min(frames - out.len() / channels) * channels];
        assert_eq!(allocations(|| engine.render(&mut block)), 0);
        out.extend(block);
        context.control().collect();
    }
    out
}

/// Block lengths from 1 to `longest`, from `seed`, without end.
fn random(seed: u64, longest: u64) -> impl Iterator<Item = usize> {
    let mut state = seed;
    std::iter::repeat_with(move || {
        // xorshift64*: plenty for picking lengths.
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let next = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
        usize::try_from(next % longest + 1).unwrap_or(1)
    })
}

#[test]
fn a_render_is_the_same_however_it_is_cut() {
    let whole = cut(2400, [2400]);
    assert!(whole.iter().any(|sample| *sample != 0.0) && whole.iter().all(|sample| sample.is_finite()));
    for seed in [1, 2, 3, 0x5eed, 0xdead_beef, 48_000, 1_000_003, 7] {
        assert!(cut(2400, random(seed, 300)) == whole, "random blocks from seed {seed}");
    }
    for sizes in [&[1][..], &[13], &[31, 33], &[441], &[127, 1, 128], &[1000]] {
        let sizes = sizes.iter().copied().cycle();
        assert!(
            cut(2400, sizes.clone()) == whole,
            "blocks of {:?}",
            sizes.take(3).collect::<Vec<_>>()
        );
    }
}
