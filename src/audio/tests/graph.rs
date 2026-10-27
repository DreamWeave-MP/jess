//! The graph: any node into any other, patches made whole or not at all, and insert chains that
//! render as the graph they stand for.

use std::sync::Arc;

use super::{Offset, allocations, at, bus, into, late, mono, pcm};
use crate::audio::dsp::reverb::Reverb;
use crate::audio::processor::Gain;
use crate::audio::{
    AudioConfig, AudioError, BusSpec, Connection, Edit, NodeKey, OfflineContext, PlaySpec, Processor, Tap, VoiceSend,
};

fn gain(value: f32) -> Box<dyn Processor> {
    let mut gain = Gain::new();
    gain.set(0, f64::from(value));
    Box::new(gain)
}

fn connect(from: NodeKey, to: NodeKey, level: f32) -> Edit {
    Edit::Connect {
        from,
        output: "out".to_owned(),
        to,
        input: "in".to_owned(),
        connection: Connection {
            level,
            ..Connection::default()
        },
    }
}

fn disconnect(from: NodeKey, to: NodeKey) -> Edit {
    Edit::Disconnect {
        from,
        output: "out".to_owned(),
        to,
        input: "in".to_owned(),
    }
}

/// A standalone node running `processor`, routed into `parent`.
fn node(context: &mut OfflineContext, processor: Box<dyn Processor>, parent: NodeKey) -> NodeKey {
    let key = context.control().add_node(processor, None).unwrap();
    context.control().route(key, parent, None).unwrap();
    key
}

#[test]
fn signal_fans_in_to_one_node_and_out_to_many() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let double = node(&mut context, gain(2.0), NodeKey::MASTER);
    let triple = node(&mut context, gain(3.0), NodeKey::MASTER);
    // One voice into both, and two more into one: out is the sum of every path.
    let spec = PlaySpec {
        sends: vec![VoiceSend { bus: triple, level: 1.0 }],
        ..into(double, 0)
    };
    context.control().play(&click, &spec).unwrap();
    context.control().play(&click, &into(double, 1)).unwrap();
    context.control().play(&click, &into(double, 1)).unwrap();
    assert_eq!(context.render(3), [5.0, 4.0, 0.0]);
    // A node fans out to two buses that meet again at the master.
    let first = bus(&mut context, NodeKey::MASTER, 1.0);
    let second = bus(&mut context, NodeKey::MASTER, 0.5);
    let source = node(&mut context, gain(1.0), first);
    context.control().send_to(source, second, 1.0, Tap::PostFader, None).unwrap();
    context.control().play(&click, &into(source, 3)).unwrap();
    assert_eq!(context.render(1), [1.5]);
}

#[test]
fn parallel_paths_of_different_latency_meet_in_line() {
    let click = pcm(&[1.0], 48_000);
    let render = |sizes: &[usize]| {
        let mut context = mono(AudioConfig::default());
        let sum = bus(&mut context, NodeKey::MASTER, 1.0);
        // Dry straight through, wet through five frames of lookahead and a drive's worth of gain.
        let dry = node(&mut context, gain(1.0), sum);
        let wet = context.control().add_node(late(5), None).unwrap();
        let louder = node(&mut context, gain(2.0), sum);
        context.control().patch(&[connect(wet, louder, 1.0)], None).unwrap();
        let spec = PlaySpec {
            sends: vec![VoiceSend { bus: wet, level: 1.0 }],
            ..into(dry, 1)
        };
        context.control().play(&click, &spec).unwrap();
        assert_eq!(context.control().latency(sum), Ok(5));
        let mut out = Vec::new();
        for size in sizes.iter().cycle() {
            if out.len() >= 12 {
                break;
            }
            let (_, engine) = context.parts();
            let mut block = vec![0.0; *size];
            assert_eq!(allocations(|| engine.render(&mut block)), 0);
            out.extend(block);
        }
        out.truncate(12);
        out
    };
    let out = render(&[12]);
    let mut expected = [0.0; 12];
    // One impulse, all of it, on the frame the slower path arrives.
    expected[6] = 3.0;
    assert_eq!(out, expected);
    assert_eq!(render(&[1, 3, 5]), out);
}

#[test]
fn an_insert_chain_renders_as_the_graph_it_stands_for() {
    let source = pcm(
        &(0..2000u16)
            .map(|frame| f32::from(frame % 37) / 37.0 - 0.5)
            .collect::<Vec<_>>(),
        48_000,
    );
    let chained = {
        let mut context = mono(AudioConfig::default());
        let group = bus(&mut context, NodeKey::MASTER, 0.5);
        context.control().insert(group, Box::new(Reverb::new(48_000)), None).unwrap();
        context.control().insert(group, Box::new(Offset(0.25)), None).unwrap();
        context.control().insert(group, gain(2.0), None).unwrap();
        context.control().play(&source, &into(group, 3)).unwrap();
        context.render(3000)
    };
    let explicit = {
        let mut context = mono(AudioConfig::default());
        let fader = bus(&mut context, NodeKey::MASTER, 0.5);
        let doubled = node(&mut context, gain(2.0), fader);
        let offset = node(&mut context, Box::new(Offset(0.25)), doubled);
        let room = node(&mut context, Box::new(Reverb::new(48_000)), offset);
        let sum = bus(&mut context, room, 1.0);
        context.control().play(&source, &into(sum, 3)).unwrap();
        context.render(3000)
    };
    assert!(chained.iter().any(|sample| *sample != 0.0));
    assert_eq!(chained, explicit);
}

#[test]
fn a_patch_takes_effect_whole_or_not_at_all() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = mono(AudioConfig::default());
    let source = bus(&mut context, NodeKey::MASTER, 1.0);
    let halved = node(&mut context, gain(0.5), NodeKey::MASTER);
    context.control().play(&tone, &into(source, 0)).unwrap();
    // A good edit and a bad one: nothing changes.
    let bad = Edit::Connect {
        from: source,
        output: "out".to_owned(),
        to: halved,
        input: "volume".to_owned(),
        connection: Connection::default(),
    };
    assert!(matches!(
        context.control().patch(&[disconnect(source, NodeKey::MASTER), bad], Some(4)),
        Err(AudioError::UnknownPort { .. })
    ));
    // A loop is found when the whole patch compiles, after its edits each looked fine.
    assert_eq!(
        context
            .control()
            .patch(&[connect(halved, source, 1.0), connect(source, halved, 1.0)], Some(4)),
        Err(AudioError::Cycle)
    );
    // Moving the source from the master to the halving node in one patch: on its frame, and never
    // through both or neither.
    context
        .control()
        .patch(&[disconnect(source, NodeKey::MASTER), connect(source, halved, 1.0)], Some(4))
        .unwrap();
    assert_eq!(context.render(8), [1.0, 1.0, 1.0, 1.0, 0.5, 0.5, 0.5, 0.5]);
}

#[test]
fn a_patch_the_queue_has_no_room_for_leaves_the_graph_as_it_was() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = mono(AudioConfig {
        commands: 4,
        ..AudioConfig::default()
    });
    let source = bus(&mut context, NodeKey::MASTER, 1.0);
    let halved = node(&mut context, gain(0.5), NodeKey::MASTER);
    context.control().play(&tone, &into(source, 0)).unwrap();
    while context.control().set_param(source, "gain", 1.0, None, None).is_ok() {}
    let moved = [disconnect(source, NodeKey::MASTER), connect(source, halved, 1.0)];
    assert_eq!(context.control().patch(&moved, None), Err(AudioError::QueueFull(4)));
    assert_eq!(context.render(2), [1.0, 1.0], "the patch never happened");
    // The graph the control side checks against is still the one playing: the patch goes now.
    context.control().patch(&moved, None).unwrap();
    assert_eq!(context.render(1), [0.5]);
}

#[test]
fn a_stale_node_names_nothing_the_slot_holds_now() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let gone = node(&mut context, gain(2.0), NodeKey::MASTER);
    assert!(context.control().remove(gone, None).unwrap());
    context.render(1);
    let reused = node(&mut context, gain(3.0), NodeKey::MASTER);
    assert_eq!(reused.slot, gone.slot, "the slot came back");
    assert_eq!(
        context.control().patch(&[connect(gone, NodeKey::MASTER, 1.0)], None),
        Err(AudioError::StaleNode)
    );
    assert!(!context.control().set_param(gone, "gain", 1.0, None, None).unwrap());
    assert_eq!(context.control().play(&click, &into(gone, 0)), Err(AudioError::StaleNode));
    context.control().play(&click, &into(reused, 0)).unwrap();
    assert_eq!(context.render(1), [3.0]);
}

#[test]
fn a_removed_node_rings_out_and_gives_its_slot_back_only_then() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let ringing = context.control().add_node(super::ring(4, 0), None).unwrap();
    context.control().route(ringing, NodeKey::MASTER, None).unwrap();
    context.control().play(&click, &into(ringing, 0)).unwrap();
    // The click ends in the block after its last frame; the node is unfed from then.
    let mut out = context.render(2);
    assert!(context.control().remove(ringing, None).unwrap());
    for _ in 0..5 {
        let (_, engine) = context.parts();
        let mut block = vec![0.0; 1];
        assert_eq!(
            allocations(|| engine.render(&mut block)),
            0,
            "nothing is freed on the render thread"
        );
        out.extend(block);
        context.control().collect();
    }
    assert_eq!(out, [1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0]);
    let next = context.control().add_node(gain(1.0), None).unwrap();
    assert_eq!(next.slot, ringing.slot, "free once it had rung out");
}

#[test]
fn channel_counts_convert_one_way_only() {
    let stereo = Arc::new(crate::audio::Pcm::from_planar(&[&[1.0], &[0.5]], 48_000).unwrap());
    let mut context = OfflineContext::new(&AudioConfig::default()).unwrap();
    // A stereo voice into a mono node is their average, and a mono node into the stereo master
    // is the same on both sides.
    let narrow = context.control().add_node(gain(1.0), Some(1)).unwrap();
    context.control().route(narrow, NodeKey::MASTER, None).unwrap();
    context.control().play(&stereo, &into(narrow, 0)).unwrap();
    assert_eq!(context.render(1), [0.75, 0.75]);
    let mut wide = OfflineContext::new(&AudioConfig {
        channels: 4,
        ..AudioConfig::default()
    })
    .unwrap();
    let pair = wide.control().add_node(gain(1.0), Some(2)).unwrap();
    let triple = wide.control().add_node(gain(1.0), Some(3)).unwrap();
    assert_eq!(
        wide.control().route(pair, NodeKey::MASTER, None),
        Err(AudioError::Layout { source: 2, output: 4 }),
        "nor two into four"
    );
    assert_eq!(
        wide.control().patch(&[connect(pair, triple, 1.0)], None),
        Err(AudioError::Layout { source: 2, output: 3 }),
        "two channels into three has no one way"
    );
}

#[test]
fn a_voice_plays_through_a_chain_of_its_own_into_a_shared_bus() {
    let tone = pcm(&[1.0; 32], 48_000);
    let mut context = mono(AudioConfig::default());
    let effects = bus(&mut context, NodeKey::MASTER, 0.5);
    // Three voices of one sound, each through its own processing, none of it a bus.
    for (index, level) in [1.0f32, 2.0, 4.0].into_iter().enumerate() {
        let drive = node(&mut context, gain(level), effects);
        let lift = node(&mut context, gain(1.5), drive);
        context.control().play(&tone, &into(lift, index as u64)).unwrap();
    }
    // One and a half times each level, from each voice's frame, through the bus at a half.
    let out = context.render(3);
    assert_eq!(out, [1.5 * 1.0 * 0.5, 1.5 * 3.0 * 0.5, 1.5 * 7.0 * 0.5]);
}

#[test]
fn a_voice_moves_between_nodes_on_its_frame() {
    let tone = pcm(&[1.0; 64], 48_000);
    let mut context = mono(AudioConfig::default());
    let quiet = bus(&mut context, NodeKey::MASTER, 0.25);
    let loud = bus(&mut context, NodeKey::MASTER, 2.0);
    let voice = context.control().play(&tone, &at(0)).unwrap();
    assert!(context.control().route_voice(voice, quiet, Some(2)).unwrap());
    assert!(context.control().set_send(voice, loud, 0.5, Some(4)).unwrap());
    assert!(context.control().unsend_voice(voice, quiet, Some(6)).unwrap());
    assert_eq!(context.render(8), [1.0, 1.0, 0.25, 0.25, 1.25, 1.25, 1.0, 1.0]);
    assert_eq!(
        context.control().stats().revisions,
        2,
        "the two buses: moving a voice revises nothing"
    );
    // Moved inside a patch, a voice goes on the patch's frame with the graph.
    let doubled = node(&mut context, gain(2.0), NodeKey::MASTER);
    let edits = [
        Edit::Unsend { voice, to: loud },
        Edit::Send {
            voice,
            to: doubled,
            input: None,
            level: 1.0,
        },
        connect(doubled, quiet, 1.0),
    ];
    context.control().patch(&edits, Some(12)).unwrap();
    assert_eq!(context.render(6), [1.0, 1.0, 1.0, 1.0, 2.5, 2.5]);
    // A node a voice still plays into can't go, even in a patch that would also move it later.
    assert!(matches!(
        context
            .control()
            .patch(&[Edit::Remove(doubled), Edit::Unsend { voice, to: doubled }], None),
        Err(AudioError::NodeInUse { voices: 1, .. })
    ));
    context
        .control()
        .patch(&[Edit::Unsend { voice, to: doubled }, Edit::Remove(doubled)], None)
        .unwrap();
}

#[test]
fn the_engine_renders_on_while_another_thread_revises_its_graph() {
    let (mut control, mut engine) = crate::audio::context(&AudioConfig {
        channels: 1,
        ..AudioConfig::default()
    })
    .unwrap();
    let tone = pcm(&vec![0.25; 4800], 48_000);
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let renderer = {
        let running = Arc::clone(&running);
        std::thread::spawn(move || {
            let mut block = vec![0.0; 64];
            let mut rendered = 0u64;
            while running.load(std::sync::atomic::Ordering::Relaxed) {
                engine.render(&mut block);
                assert!(block.iter().all(|sample| sample.is_finite()));
                rendered += 1;
            }
            rendered
        })
    };
    let first = control.add_bus(&BusSpec::default()).unwrap();
    let second = control.add_bus(&BusSpec::default()).unwrap();
    let mut made = 0;
    for round in 0..300 {
        let (from, to) = if round % 2 == 0 { (first, second) } else { (second, first) };
        let edits = [disconnect(to, from), connect(from, to, 0.5)];
        if control.patch(&edits, None).is_ok() {
            made += 1;
        }
        if round % 10 == 0 {
            control
                .play(
                    &tone,
                    &PlaySpec {
                        bus: from,
                        ..PlaySpec::default()
                    },
                )
                .ok();
        }
        control.collect();
        std::thread::yield_now();
    }
    running.store(false, std::sync::atomic::Ordering::Relaxed);
    assert!(renderer.join().unwrap() > 0);
    control.collect();
    assert!(made > 0);
    assert_eq!(control.stats().nonfinite, 0);
}

#[test]
fn late_lines_up_whatever_is_late() {
    // The test processor delays by its length, and says so: a sanity check for the rest.
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let slow = context.control().add_node(late(3), None).unwrap();
    context.control().route(slow, NodeKey::MASTER, None).unwrap();
    context.control().play(&click, &into(slow, 0)).unwrap();
    assert_eq!(context.render(5), [0.0, 0.0, 0.0, 1.0, 0.0]);
}

/// Renders `frames` of `context` in blocks of `sizes`, round and round, checking no block
/// allocates.
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

fn feedback(from: NodeKey, to: NodeKey, frames: u64) -> Edit {
    Edit::Connect {
        from,
        output: "out".to_owned(),
        to,
        input: "in".to_owned(),
        connection: Connection {
            feedback: Some(frames),
            ..Connection::default()
        },
    }
}

#[test]
fn a_loop_needs_a_feedback_delay_of_a_quantum_or_more() {
    let mut context = mono(AudioConfig::default());
    let echo = bus(&mut context, NodeKey::MASTER, 1.0);
    let half = context.control().add_node(gain(0.5), None).unwrap();
    assert_eq!(
        context
            .control()
            .patch(&[connect(echo, half, 1.0), connect(half, echo, 1.0)], None),
        Err(AudioError::Cycle)
    );
    assert!(matches!(
        context
            .control()
            .patch(&[connect(echo, half, 1.0), feedback(half, echo, 64)], None),
        Err(AudioError::Parameter { name: "feedback", .. })
    ));
    context
        .control()
        .patch(&[connect(echo, half, 1.0), feedback(half, echo, 128)], None)
        .unwrap();
}

#[test]
fn a_feedback_loop_echoes_the_same_however_the_render_is_cut() {
    let click = pcm(&[1.0], 48_000);
    let render = |sizes: &[usize]| {
        let mut context = mono(AudioConfig::default());
        let echo = bus(&mut context, NodeKey::MASTER, 1.0);
        let half = context.control().add_node(gain(0.5), None).unwrap();
        context
            .control()
            .patch(&[connect(echo, half, 1.0), feedback(half, echo, 200)], None)
            .unwrap();
        context.control().play(&click, &into(echo, 3)).unwrap();
        // A command mid-loop splits the blocks too.
        context.control().set_param(echo, "gain", 1.0, Some(301), None).unwrap();
        render_in(&mut context, 1000, sizes)
    };
    let out = render(&[1000]);
    let mut expected = vec![0.0; 1000];
    for (index, level) in [1.0, 0.5, 0.25, 0.125, 0.0625].into_iter().enumerate() {
        expected[3 + 200 * index] = level;
    }
    assert_eq!(out, expected);
    for sizes in [&[1][..], &[13], &[128], &[7, 300, 64, 1]] {
        assert_eq!(render(sizes), out, "blocks of {sizes:?}");
    }
}

#[test]
fn latency_in_a_loop_lengthens_its_period_and_leaves_compensation_alone() {
    let click = pcm(&[1.0], 48_000);
    let mut context = mono(AudioConfig::default());
    let sum = bus(&mut context, NodeKey::MASTER, 1.0);
    let entry = bus(&mut context, sum, 1.0);
    // The entry feeds the sum two ways, through ten frames of lookahead and straight, and the sum
    // feeds the entry back, half as loud, two hundred frames later.
    let slow = context.control().add_node(late(10), None).unwrap();
    let half = context.control().add_node(gain(0.5), None).unwrap();
    context
        .control()
        .patch(
            &[
                connect(entry, slow, 1.0),
                connect(slow, sum, 1.0),
                connect(sum, half, 1.0),
                feedback(half, entry, 200),
            ],
            None,
        )
        .unwrap();
    // The feedback is deliberate: it isn't latency, and nothing waits for it.
    assert_eq!(context.control().latency(sum), Ok(10));
    assert_eq!(context.control().latency(NodeKey::MASTER), Ok(10));
    context.control().play(&click, &into(entry, 0)).unwrap();
    let out = render_in(&mut context, 700, &[128]);
    let heard: Vec<(usize, f32)> = out
        .iter()
        .enumerate()
        .filter(|(_, sample)| **sample != 0.0)
        .map(|(frame, sample)| (frame, *sample))
        .collect();
    // Both paths meet on frame 10, and each trip round the loop takes its 200 frames and the
    // lookahead's 10 again: the straight path waits for the slow one inside the loop as well.
    assert_eq!(heard, [(10, 2.0), (220, 2.0), (430, 2.0), (640, 2.0)]);
}
