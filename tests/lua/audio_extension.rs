//! A processor from outside jess: defined here, in another crate, registered with the runtime a
//! host makes, and made, set, automated, and modulated from Luau like a built-in one. Adding it
//! touches nothing in jess.

use jess::audio::AudioError;
use jess::audio::processor::{Io, ParamInfo, Processor, Registry};
use jess::lua::Options;
use jess::lua::l3i::Runtime;

use super::common;

/// Reduces a signal to so many `bits`: each sample rounded to the nearest of the levels that
/// many bits hold between -1 and 1.
struct Crush {
    steps: f32,
    bits: f64,
}

impl Crush {
    const PARAMS: &'static [ParamInfo] = &[ParamInfo::control("bits", 1.0, 24.0, 8.0, "")];

    fn new() -> Crush {
        let mut crush = Crush { steps: 0.0, bits: 0.0 };
        crush.set(0, 8.0);
        crush
    }
}

impl Processor for Crush {
    fn kind(&self) -> &'static str {
        "crush"
    }

    fn params(&self) -> &'static [ParamInfo] {
        Crush::PARAMS
    }

    #[allow(clippy::cast_possible_truncation)] // A power of two of at most 2^23.
    fn set(&mut self, _index: usize, value: f64) {
        self.bits = value;
        self.steps = (value - 1.0).exp2() as f32;
    }

    fn value(&self, _index: usize) -> f64 {
        self.bits
    }

    fn process(&mut self, io: &mut Io<'_>) {
        let (channels, steps) = (io.channels(), self.steps);
        let mut block = io.main();
        for channel in 0..channels {
            for sample in block.channel(channel) {
                *sample = (*sample * steps).round() / steps;
            }
        }
    }
}

fn runtime() -> Runtime {
    let mut processors = Registry::builtin();
    processors.register("crush", |_, _| Ok(Box::new(Crush::new()))).unwrap();
    common::runtime_with(Options { processors })
}

#[test]
fn a_processor_the_host_registers_is_made_and_driven_from_luau() {
    common::run(
        &runtime(),
        r"
        local audio = require '@jess/audio'
        assert(table.find(audio.processors(), 'crush'), 'listed with the built-in kinds')
        local context = audio.offline { channels = 1 }
        local data = buffer.create(4 * 4096)
        for frame = 0, 4095 do
            buffer.writef32(data, frame * 4, 0.3)
        end
        local level = audio.pcm(data, { format = 'f32', channels = 1, rate = 48000 })
        local crush = context:master():insert('crush', { bits = 2 })
        assert(crush:kind() == 'crush' and crush:params()[1].name == 'bits')
        context:play(level, { at = 0 })
        local function first(out: buffer): number return buffer.readf32(out, 0) end
        -- Two bits: halves. 0.3 rounds to 0.5.
        assert(first(context:render(32)) == 0.5)
        -- Four: eighths, on its frame. 0.3 rounds to 0.25.
        crush:set('bits', 4, context:now() + 16)
        local out = context:render(32)
        assert(buffer.readf32(out, 15 * 4) == 0.5 and buffer.readf32(out, 16 * 4) == 0.25)
        -- An oscillator modulates it like any parameter: a sine of 6 bits around its 4.
        local lfo = context:node('oscillator', { frequency = 10, channels = 1 })
        context:patch(function(graph) graph:connect(lfo, crush, { to = 'bits', level = 6 }) end)
        out = context:render(4800)
        local heard = {}
        local levels = 0
        for frame = 0, 4799 do
            local sample = buffer.readf32(out, frame * 4)
            if not heard[sample] then
                heard[sample] = true
                levels += 1
            end
        end
        assert(heard[0.25] and levels > 3, 'it crushed to four bits, and to others as the sine moved')
        ",
    );
}

#[test]
fn a_registered_kind_s_parameters_are_checked_before_its_code_sees_them() {
    let problem = runtime()
        .exec(
            r"
            local context = (require '@jess/audio').offline()
            context:master():insert('crush'):set('bits', 30)
            ",
        )
        .unwrap_err();
    assert!(problem.to_string().contains("inside its range"), "{problem}");
}

#[test]
fn a_kind_registers_once_and_must_make_what_it_says() {
    let mut processors = Registry::builtin();
    assert_eq!(
        processors.register("gain", |_, _| Ok(Box::new(Crush::new()))),
        Err(AudioError::KindTaken("gain".to_owned()))
    );
    processors.register("liar", |_, _| Ok(Box::new(Crush::new()))).unwrap();
    let setup = jess::audio::processor::Setup {
        rate: 48_000,
        channels: 1,
    };
    assert!(matches!(
        processors.create("liar", &setup, jess::audio::processor::Settings::new()),
        Err(AudioError::Shape { kind: "crush", .. })
    ));
}
