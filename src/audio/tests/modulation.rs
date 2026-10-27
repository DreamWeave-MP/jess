//! Modulation, generators, and processors of more than one input, output, or buffer.

use super::{Level, mono, pcm};
use crate::audio::dsp::crossfade::Crossfade;
use crate::audio::dsp::generators::{Envelope, Oscillator, Wave};
use crate::audio::processor::{Gain, Io, ParamInfo, Port, PortKind, Ports, Processor};
use crate::audio::{AudioConfig, Connection, Curve, Edit, Glide, NodeKey, OfflineContext, PlaySpec, QUANTUM};

fn modulate(from: NodeKey, to: NodeKey, param: &str, depth: f32) -> Edit {
    Edit::Connect {
        from,
        output: "out".to_owned(),
        to,
        input: param.to_owned(),
        connection: Connection {
            level: depth,
            ..Connection::default()
        },
    }
}

fn connect(from: NodeKey, output: &str, to: NodeKey, input: &str) -> Edit {
    Edit::Connect {
        from,
        output: output.to_owned(),
        to,
        input: input.to_owned(),
        connection: Connection::default(),
    }
}

/// A sine at a quarter of the rate: 0, 1, 0, -1, round and round.
fn quarter(context: &mut OfflineContext) -> NodeKey {
    let mut sine = Oscillator::new(Wave::Sine, 48_000);
    sine.set(0, 12_000.0);
    context.control().add_node(Box::new(sine), Some(1)).unwrap()
}

/// A gain node routed into the master, at `level`.
fn amplifier(context: &mut OfflineContext, level: f32) -> NodeKey {
    let mut gain = Gain::new();
    gain.set(0, f64::from(level));
    let key = context.control().add_node(Box::new(gain), None).unwrap();
    context.control().route(key, NodeKey::MASTER, None).unwrap();
    key
}

fn close(out: &[f32], expected: &[f32]) {
    assert_eq!(out.len(), expected.len());
    for (frame, (sample, expected)) in out.iter().zip(expected).enumerate() {
        assert!((sample - expected).abs() < 1e-5, "frame {frame}: {out:?}");
    }
}

#[test]
fn an_oscillator_modulates_an_audio_rate_parameter_every_sample() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = mono(AudioConfig::default());
    // The amplifier is made before the oscillator, and still runs after it.
    let amplifier = amplifier(&mut context, 1.0);
    let lfo = quarter(&mut context);
    context
        .control()
        .patch(&[modulate(lfo, amplifier, "gain", 0.5)], None)
        .unwrap();
    context
        .control()
        .play(
            &tone,
            &PlaySpec {
                bus: amplifier,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    close(&context.render(8), &[1.0, 1.5, 1.0, 0.5, 1.0, 1.5, 1.0, 0.5]);
    // It adds to the automation, which ramps underneath it.
    let ramp = Some(Glide {
        frames: 4,
        curve: Curve::Linear,
    });
    context.control().set_param(amplifier, "gain", 2.0, None, ramp).unwrap();
    close(&context.render(6), &[1.0, 1.75, 1.5, 1.25, 2.0, 2.5]);
    // Taken away, it leaves the automation as it was.
    context
        .control()
        .patch(
            &[Edit::Disconnect {
                from: lfo,
                output: "out".to_owned(),
                to: amplifier,
                input: "gain".to_owned(),
            }],
            None,
        )
        .unwrap();
    close(&context.render(3), &[2.0, 2.0, 2.0]);
}

#[test]
fn a_control_rate_parameter_hears_its_modulation_once_a_control_block() {
    let tone = pcm(&[1.0; 256], 48_000);
    let mut context = mono(AudioConfig::default());
    let level = context.control().add_node(Box::new(Level(0.0)), None).unwrap();
    context.control().route(level, NodeKey::MASTER, None).unwrap();
    // An envelope rising a 128th a frame.
    let mut rising = Envelope::new(48_000);
    rising.set(0, 128.0 / 48_000.0);
    rising.set(4, 1.0);
    let rising = context.control().add_node(Box::new(rising), Some(1)).unwrap();
    context
        .control()
        .patch(&[modulate(rising, level, "level", 1.0)], None)
        .unwrap();
    context
        .control()
        .play(
            &tone,
            &PlaySpec {
                bus: level,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    let out = context.render(128);
    for (block, samples) in out.chunks(32).enumerate() {
        #[allow(clippy::cast_precision_loss)] // A small frame number.
        let heard = (block * 32 + 1) as f32 / 128.0;
        close(samples, &[heard; 32]);
    }
}

/// Writes what it is set to on every sample, not-a-number included.
struct Constant(f32);

impl Processor for Constant {
    fn kind(&self) -> &'static str {
        "constant"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }

    fn ports(&self) -> &'static Ports {
        &Ports::SOURCE
    }

    fn set(&mut self, _index: usize, _value: f64) {}

    fn value(&self, _index: usize) -> f64 {
        0.0
    }

    fn process(&mut self, io: &mut Io<'_>) {
        io.main().channel(0).fill(self.0);
    }
}

#[test]
fn modulation_stays_inside_its_parameter_s_range_and_ignores_what_isn_t_a_number() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = mono(AudioConfig::default());
    let amplifier = amplifier(&mut context, 1.0);
    let huge = context.control().add_node(Box::new(Constant(1e9)), Some(1)).unwrap();
    let broken = context.control().add_node(Box::new(Constant(f32::NAN)), Some(1)).unwrap();
    context
        .control()
        .patch(&[modulate(huge, amplifier, "gain", 1.0)], None)
        .unwrap();
    context
        .control()
        .play(
            &tone,
            &PlaySpec {
                bus: amplifier,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    assert_eq!(context.render(2), [64.0, 64.0], "the gain's ceiling");
    context
        .control()
        .patch(
            &[
                Edit::Disconnect {
                    from: huge,
                    output: "out".to_owned(),
                    to: amplifier,
                    input: "gain".to_owned(),
                },
                modulate(broken, amplifier, "gain", 1.0),
            ],
            None,
        )
        .unwrap();
    assert_eq!(context.render(2), [1.0, 1.0], "the automation, untouched");
    assert_eq!(context.control().stats().nonfinite, 0);
}

#[test]
fn an_oscillator_drives_another_s_frequency_without_a_script_in_the_loop() {
    let mut context = mono(AudioConfig::default());
    let carrier = context
        .control()
        .add_node(Box::new(Oscillator::new(Wave::Sine, 48_000)), None)
        .unwrap();
    context.control().route(carrier, NodeKey::MASTER, None).unwrap();
    let modulator = quarter(&mut context);
    context
        .control()
        .patch(&[modulate(modulator, carrier, "frequency", 220.0)], None)
        .unwrap();
    let mut out = vec![0.0; 4800];
    let (_, engine) = context.parts();
    assert_eq!(super::allocations(|| engine.render(&mut out)), 0);
    // The carrier's step each frame is its frequency then: 440, 660, 440, 220 hertz, round again.
    let mut phase = 0.0f64;
    for (frame, sample) in out.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)] // A comparison at f32.
        let expected = (std::f64::consts::TAU * phase).sin() as f32;
        assert!((sample - expected).abs() < 1e-4, "frame {frame}");
        phase += [440.0, 660.0, 440.0, 220.0][frame % 4] / 48_000.0;
        phase -= phase.floor();
    }
}

#[test]
fn a_crossfade_blends_its_two_inputs() {
    let one = pcm(&[1.0; 8], 48_000);
    let two = pcm(&[2.0; 8], 48_000);
    let mut context = mono(AudioConfig::default());
    let blend = context.control().add_node(Box::new(Crossfade::new()), None).unwrap();
    context.control().route(blend, NodeKey::MASTER, None).unwrap();
    context.control().set_param(blend, "mix", 0.25, None, None).unwrap();
    context
        .control()
        .play(
            &one,
            &PlaySpec {
                bus: blend,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    // The second input is a node's: a voice plays into the first unless told otherwise.
    let feeder = amplifier(&mut context, 1.0);
    context
        .control()
        .patch(
            &[
                Edit::Disconnect {
                    from: feeder,
                    output: "out".to_owned(),
                    to: NodeKey::MASTER,
                    input: "in".to_owned(),
                },
                connect(feeder, "out", blend, "b"),
            ],
            None,
        )
        .unwrap();
    context
        .control()
        .play(
            &two,
            &PlaySpec {
                bus: feeder,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    close(&context.render(2), &[1.25, 1.25]);
}

/// Reads its input and writes it upside down into an output of its own, and twice as loud into
/// a second: a processor that doesn't work in place, with two outputs.
struct Flip;

impl Processor for Flip {
    fn kind(&self) -> &'static str {
        "flip"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }

    fn ports(&self) -> &'static Ports {
        const PORTS: Ports = Ports {
            inputs: &[Port {
                name: "in",
                kind: PortKind::Audio,
            }],
            outputs: &["out", "loud"],
            in_place: false,
        };
        &PORTS
    }

    fn set(&mut self, _index: usize, _value: f64) {}

    fn value(&self, _index: usize) -> f64 {
        0.0
    }

    fn process(&mut self, io: &mut Io<'_>) {
        let frames = io.frames();
        let mut input = [0.0f32; QUANTUM];
        if let Some(samples) = io.input(0) {
            input[..frames].copy_from_slice(&samples[..frames]);
        }
        for (out, sample) in io.main().channel(0).iter_mut().zip(&input) {
            *out = -sample;
        }
        if let Some(mut loud) = io.output(1) {
            for (out, sample) in loud.channel(0).iter_mut().zip(&input) {
                *out = sample * 2.0;
            }
        }
    }
}

#[test]
fn a_processor_out_of_place_with_two_outputs_feeds_each_where_it_goes() {
    let tone = pcm(&[0.5; 8], 48_000);
    let mut context = mono(AudioConfig::default());
    let quiet = amplifier(&mut context, 0.25);
    let flip = context.control().add_node(Box::new(Flip), None).unwrap();
    // Two of them, so their scratch outputs live at once and apart.
    let other = context.control().add_node(Box::new(Flip), None).unwrap();
    context
        .control()
        .patch(
            &[
                connect(flip, "out", NodeKey::MASTER, "in"),
                connect(flip, "loud", quiet, "in"),
                connect(other, "loud", NodeKey::MASTER, "in"),
            ],
            None,
        )
        .unwrap();
    context
        .control()
        .play(
            &tone,
            &PlaySpec {
                bus: flip,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    context
        .control()
        .play(
            &tone,
            &PlaySpec {
                bus: other,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    // -0.5 straight, 1 through the quarter gain, and 1 more from the other's loud output.
    close(&context.render(2), &[0.75, 0.75]);
}

#[test]
fn a_cancel_drops_what_was_scheduled_before_it_and_holds_the_ramp_where_it_got_to() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = mono(AudioConfig::default());
    let amplifier = amplifier(&mut context, 1.0);
    context
        .control()
        .play(
            &tone,
            &PlaySpec {
                bus: amplifier,
                ..PlaySpec::default()
            },
        )
        .unwrap();
    let ramp = Some(Glide {
        frames: 8,
        curve: Curve::Linear,
    });
    context.control().set_param(amplifier, "gain", 0.0, Some(0), ramp).unwrap();
    context.control().set_param(amplifier, "gain", 4.0, Some(6), None).unwrap();
    // Cancelled on frame 4: the ramp stops halfway, and the jump stamped for 6 never happens; a
    // change sent after the cancel still does.
    context.control().cancel(amplifier, "gain", Some(4)).unwrap();
    context.control().set_param(amplifier, "gain", 2.0, Some(7), None).unwrap();
    close(&context.render(8), &[1.0, 0.875, 0.75, 0.625, 0.5, 0.5, 0.5, 2.0]);
}
