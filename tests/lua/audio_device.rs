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
