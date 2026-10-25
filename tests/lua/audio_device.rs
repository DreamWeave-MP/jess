//! `jess.audio.output`: the capability it needs, and, built with the audio-device feature, a
//! context on the null device that renders itself while the script only plays into it.

use jess::lua::Options;
use jess::lua::l3i::Runtime;

fn runtime(capabilities: &[&str]) -> Runtime {
    let capabilities: Vec<String> = capabilities.iter().map(|name| (*name).to_owned()).collect();
    jess::lua::runtime_with_capabilities(Options::default(), &capabilities).unwrap()
}

const OPEN: &str = "return jess.audio.output { device = 'null' }";

#[test]
fn an_output_needs_the_audio_output_capability() {
    let error = runtime(&[]).eval::<()>(OPEN).unwrap_err().to_string();
    assert!(error.contains("audio.output"), "{error}");
}

#[cfg(feature = "audio-device")]
#[test]
fn a_script_plays_into_the_null_device_and_the_device_renders_it() {
    let script = r"
        local context = jess.audio.output { device = 'null', channels = 2 }
        local device = context:device()
        assert(device.backend == 'Null', device.backend)
        assert(device.rate > 0 and device.channels == 2 and device.latency > 0)
        local click = jess.audio.pcm(buffer.create(4 * 64), { format = 'f32', channels = 1, rate = 48000 })
        local voice = context:play(click)
        assert(not pcall(function() return context:render(10) end), 'a device renders an output')
        -- Nothing here renders: the device's thread does, as time passes.
        local deadline = os.clock() + 10
        while context:now() < device.rate // 10 do
          assert(os.clock() < deadline, 'the device never rendered')
        end
        assert(voice:state() == 'ended', voice:state())
        assert(not context:device().failed)
        assert(jess.audio.offline():device() == nil)
    ";
    runtime(&["audio.output"])
        .exec(script)
        .unwrap_or_else(|error| panic!("{error}"));
}

#[cfg(not(feature = "audio-device"))]
#[test]
fn without_the_feature_an_output_says_what_it_needs() {
    let error = runtime(&["audio.output"]).eval::<()>(OPEN).unwrap_err().to_string();
    assert!(error.contains("audio-device feature"), "{error}");
}

/// Writes three seconds of a stream ahead, opens a null output in a block of its own and plays
/// the stream there, collects everything the block held, and returns the stream's id.
#[cfg(feature = "audio-device")]
const LET_GO: &str = r"
    local stream = jess.audio.stream { channels = 2, rate = 48000, seconds = 4 }
    stream:write(buffer.create(3 * 48000 * 2 * 4))
    do
      local context = jess.audio.output { device = 'null', rate = 48000 }
      context:play(stream)
    end
    local id = stream:handle().id
    stream = nil
    jess.bench.collect()
    return id
";

/// The stream an output still plays, or `None` once its engine is gone.
#[cfg(feature = "audio-device")]
fn playing(id: f64) -> Option<std::sync::Arc<jess::audio::Stream>> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // An id from `handle().id`.
    jess::audio::Stream::find(id as u64)
}

#[cfg(feature = "audio-device")]
#[test]
fn an_output_plays_on_when_the_script_lets_go_of_it_and_through_a_stall() {
    let runtime = runtime(&["audio.output"]);
    let id: f64 = runtime.eval(LET_GO).unwrap_or_else(|error| panic!("{error}"));
    // A stall: no Luau runs, and the device plays what was written ahead.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let stream = playing(id).expect("the output let go of its stream when the script did");
    let stats = stream.stats();
    assert!(stats.read > 0, "the device stopped when the script let go of it: {stats:?}");
    assert!(stats.read < stats.written, "{stats:?}");
    assert_eq!(stats.missed, 0, "the stall reached the device: {stats:?}");
    drop(stream);
    drop(runtime);
    assert!(playing(id).is_none(), "the output outlived its runtime");
}

#[cfg(feature = "audio-device")]
#[test]
fn a_domain_restart_stops_the_outputs_its_generation_opened() {
    use jess::runtime::{CapabilitySet, DomainSpec, RootAuthority, RuntimeFactory, ScriptDomain};
    use std::sync::Arc;

    let factory: Arc<dyn RuntimeFactory> =
        Arc::new(|| jess::lua::runtime_with_capabilities(Options::default(), &["audio.output".to_owned()]));
    let grant = RootAuthority::new().grant(CapabilitySet::from_names(["audio.output"]).unwrap());
    let domain = ScriptDomain::create(factory, DomainSpec::new("audio", grant)).unwrap();
    domain.load_source("=audio", "").unwrap();
    let id: f64 = domain
        .with_runtime(|runtime| runtime.eval(LET_GO))
        .unwrap()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(playing(id).is_some(), "the output stopped before the restart");
    domain.restart(|_| Ok(())).unwrap();
    assert!(playing(id).is_none(), "the old generation's output plays on");
    domain.stop().unwrap();
}

#[cfg(feature = "audio-device")]
#[test]
fn a_closed_output_stops_and_refuses_commands() {
    let script = r"
        local context = jess.audio.output { device = 'null' }
        local click = jess.audio.pcm(buffer.create(4 * 64), { format = 'f32', channels = 1, rate = 48000 })
        context:play(click, { loop = true })
        context:close()
        assert(context:device().closed)
        context:close()
        context:play(click)
    ";
    let runtime = runtime(&["audio.output"]);
    let error = runtime.exec(script).unwrap_err().to_string();
    assert!(error.contains("its output was closed"), "{error}");
    let error = runtime.exec("jess.audio.offline():close()").unwrap_err().to_string();
    assert!(error.contains("closes an output"), "{error}");
}
