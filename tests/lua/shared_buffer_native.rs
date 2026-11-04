//! Lowered `SharedBuffer` reads against the method they replace: the same values, the same errors,
//! no calls, and a receiver that stays alive.

use jess::lua::Options;
use std::fmt::Write;

use jess::lua::l3i::convert::Integer;
use jess::lua::l3i::native_code::{AssemblyOptions, AssemblyTarget};
use jess::lua::l3i::source::{LoadScope, compile};
use jess::lua::l3i::value::{Table, Value};
use jess::lua::l3i::{Error, Runtime};
use jess::runtime::shared_buffer_lowering::{lowered_sites, method_reads};

use super::common::{run, set_global};

const LENGTH: usize = 64;

const READERS: &str = r"
function state.NAME_f64(view: jess_runtime_SharedBuffer, offset: any): number
  local value = view:readf64(offset)
  return value
end

function state.NAME_u32(view: jess_runtime_SharedBuffer, offset: any): number
  local value = view:readu32(offset)
  return value
end

function state.NAME_sum(view: jess_runtime_SharedBuffer, count: number): number
  local total = 0
  for _ = 1, count do
    local value = view:readf64(8)
    total += value
  end

  return total
end
";

/// A runtime with a `state` table, since a runtime's own globals are read-only to scripts.
fn with_state() -> Runtime {
    let runtime = jess::lua::runtime_with_capabilities(Options::default(), &["shared-buffer.create".to_owned()]).unwrap();
    let state = Table::from_value(runtime.eval::<Value>("return {}").unwrap()).unwrap();
    set_global(&runtime, "state", &state);
    runtime
}

/// Runs a chunk as a module is loaded, so `--!native` means what it does there. `Runtime::exec`
/// never compiles to machine code.
fn load(runtime: &Runtime, source: &str) {
    runtime
        .stack()
        .with_frame(|frame| {
            let bytecode = compile(source, &runtime.compile_options())?;
            let chunk = frame.load_bytecode("=chunk", &bytecode)?;
            let _result: Value = chunk.invoke(frame, ())?;
            Ok(())
        })
        .unwrap_or_else(|error: Error| panic!("{error}"));
}

/// A runtime with `view` over `LENGTH` bytes equal to their index, a native and a plain set of
/// the same readers, and the bytes for checking them independently.
fn fixture() -> (Runtime, Vec<u8>) {
    let runtime = with_state();
    run(
        &runtime,
        &format!(
            r"
local builder = require('@jess/sharedBuffer').create({LENGTH})
local bytes = buffer.create({LENGTH})
for index = 0, {} do
  buffer.writeu8(bytes, index, index)
end

builder:write(0, bytes)
state.view = builder:seal()
state.slice = state.view:slice(9, 20)
builder = require('@jess/sharedBuffer').create(8)
state.unsealed = builder
",
            LENGTH - 1
        ),
    );
    load(&runtime, &format!("--!native\n{}", READERS.replace("NAME", "native")));
    load(&runtime, &READERS.replace("NAME", "plain"));
    (runtime, (0..u8::try_from(LENGTH).unwrap()).collect())
}

fn f64_at(bytes: &[u8], offset: usize) -> u64 {
    f64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()).to_bits()
}

fn u32_at(bytes: &[u8], offset: usize) -> f64 {
    f64::from(u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()))
}

#[test]
fn reads_match_the_bytes_interpreted_and_native_at_every_offset() {
    let (runtime, bytes) = fixture();
    for offset in 0..=LENGTH - 8 {
        let expected = f64_at(&bytes, offset);
        for mode in ["native", "plain"] {
            let value: f64 = runtime
                .eval(&format!("return state.{mode}_f64(state.view, {offset})"))
                .unwrap();
            assert_eq!(value.to_bits(), expected, "{mode} readf64 at {offset}");
        }
    }

    for offset in 0..=LENGTH - 4 {
        let expected = u32_at(&bytes, offset);
        for mode in ["native", "plain"] {
            let value: f64 = runtime
                .eval(&format!("return state.{mode}_u32(state.view, {offset})"))
                .unwrap();
            assert_eq!(value, expected, "{mode} readu32 at {offset}");
        }
    }

    // A slice's offsets are its own, and its end is its own.
    let slice = &bytes[9..29];
    for offset in 0..=slice.len() - 8 {
        let expected = f64_at(slice, offset);
        for mode in ["native", "plain"] {
            let value: f64 = runtime
                .eval(&format!("return state.{mode}_f64(state.slice, {offset})"))
                .unwrap();
            assert_eq!(value.to_bits(), expected, "{mode} slice readf64 at {offset}");
        }
    }
}

#[test]
fn special_values_keep_their_bits() {
    let runtime = with_state();
    let values = [
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MIN_POSITIVE / 2.0,
        f64::MAX,
        1.5,
    ];
    let mut script = String::from("local builder = require('@jess/sharedBuffer').create(64)\n");
    for (index, value) in values.iter().enumerate() {
        let bits = value.to_bits();
        writeln!(
            script,
            "builder:writeu32({}, {})\nbuilder:writeu32({}, {})",
            index * 8,
            bits & 0xffff_ffff,
            index * 8 + 4,
            bits >> 32
        )
        .unwrap();
    }

    // A signalling NaN's payload, which a value passing through an x87 or a conversion would lose.
    script.push_str("builder:writeu32(56, 1)\nbuilder:writeu32(60, 2146435072)\nstate.view = builder:seal()\n");
    run(&runtime, &script);
    load(&runtime, &format!("--!native\n{}", READERS.replace("NAME", "native")));
    for (index, value) in values.iter().enumerate() {
        let read: f64 = runtime
            .eval(&format!("return state.native_f64(state.view, {})", index * 8))
            .unwrap();
        assert_eq!(read.to_bits(), value.to_bits());
    }

    let nan: f64 = runtime.eval("return state.native_f64(state.view, 56)").unwrap();
    assert!(nan.is_nan());
    assert_eq!(nan.to_bits() & 0xffff_ffff, 1);
}

/// What an error says past its location, which names the chunk.
fn message(error: &impl ToString) -> String {
    let text = error.to_string();
    text.rsplit_once(": ")
        .map_or_else(|| text.clone(), |(_, message)| message.to_owned())
}

#[test]
fn failed_guards_raise_what_the_method_raises() {
    let (runtime, _) = fixture();
    set_global(&runtime, "whole", &Integer(8));
    let offsets = [
        "-1",
        "-0.5",
        "0.5",
        "1.5",
        "57",
        "61",
        "64",
        "65",
        "2147483647",
        "2147483648",
        "4294967304",
        "9007199254740992",
        "9223372036854775808",
        "-9223372036854775808",
        "18446744073709551616",
        "0/0",
        "math.huge",
        "-math.huge",
        "'8'",
        "'x'",
        "nil",
        "true",
        "{}",
    ];
    for reader in ["f64", "u32"] {
        for offset in offsets {
            let native = runtime.exec(&format!("state.native_{reader}(state.view, {offset})"));
            let plain = runtime.exec(&format!("state.plain_{reader}(state.view, {offset})"));
            // 57 and 61 are in range for a four-byte read, and fail only as an eight-byte one.
            assert_eq!(
                native.is_ok(),
                plain.is_ok(),
                "{reader} at {offset}: {native:?} against {plain:?}"
            );
            if let (Err(native), Err(plain)) = (native, plain) {
                assert_eq!(message(&native), message(&plain), "{reader} at {offset}");
            }
        }
    }

    // What the guards pass to the method rather than reject: a Luau integer and a negative zero.
    for offset in ["whole", "-0"] {
        let native: f64 = runtime
            .eval(&format!("return state.native_f64(state.view, {offset})"))
            .unwrap();
        let plain: f64 = runtime
            .eval(&format!("return state.plain_f64(state.view, {offset})"))
            .unwrap();
        assert_eq!(native.to_bits(), plain.to_bits(), "{offset}");
    }

    for receiver in ["state.unsealed", "{}", "nil", "8", "'text'"] {
        let native = runtime.exec(&format!("state.native_f64({receiver}, 0)")).unwrap_err();
        let plain = runtime.exec(&format!("state.plain_f64({receiver}, 0)")).unwrap_err();
        assert_eq!(message(&native), message(&plain), "receiver {receiver}");
    }
}

// A release build compiles the method's counter out.
#[cfg(debug_assertions)]
#[test]
fn a_lowered_read_makes_no_call() {
    let before = lowered_sites();
    let (runtime, bytes) = fixture();
    assert!(lowered_sites() > before, "the native readers were lowered");

    let reads = method_reads();
    let _: f64 = runtime.eval("return state.native_sum(state.view, 100000)").unwrap();
    assert_eq!(method_reads(), reads, "a lowered read reached the method");

    let _: f64 = runtime.eval("return state.plain_sum(state.view, 1000)").unwrap();
    assert_eq!(method_reads(), reads + 1000, "an interpreted read takes the method");

    // A whole offset out of range passes the binder's conversion, so it reaches the method once,
    // with its arguments intact, and the method raises.
    let reads = method_reads();
    let error = runtime.exec("state.native_f64(state.view, 57)").unwrap_err();
    assert_eq!(message(&error), "buffer range is out of bounds");
    assert_eq!(method_reads(), reads + 1, "the guard's exit reached the method");

    // A fractional offset is refused by the binder's conversion, before the method's body runs.
    let reads = method_reads();
    let error = runtime.exec("state.native_f64(state.view, 0.5)").unwrap_err();
    let plain = runtime.exec("state.plain_f64(state.view, 0.5)").unwrap_err();
    assert_eq!(message(&error), message(&plain));
    assert_eq!(method_reads(), reads, "the conversion refuses it before the method");

    // Neither failure costs the function its native code, and a value read after one is right.
    let value: f64 = runtime.eval("return state.native_f64(state.view, 1)").unwrap();
    assert_eq!(value.to_bits(), f64_at(&bytes, 1));
    let _: f64 = runtime.eval("return state.native_sum(state.view, 1000)").unwrap();
    assert_eq!(method_reads(), reads, "native code stayed native after a failed guard");
}

// Where Luau has a code generator, it must be running here, or every test in this file would pass
// by comparing the interpreter with itself.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn native_code_runs_on_this_architecture() {
    let (runtime, _) = fixture();
    assert!(runtime.native_code().unwrap().is_available());
    assert!(lowered_sites() > 0, "this thread's fixture lowered its reads");
}

#[test]
fn the_generated_code_has_one_load_and_no_call_on_each_target() {
    let (runtime, _) = fixture();
    let generator = runtime.native_code().unwrap();
    let dump = |target| {
        let value = runtime.eval::<Value>("return state.native_f64").unwrap();
        runtime
            .stack()
            .with_frame(|frame| {
                value.with_value(frame, |frame, view| {
                    generator.assembly(
                        frame,
                        view.index(),
                        AssemblyOptions {
                            target,
                            include_ir: true,
                            ..AssemblyOptions::default()
                        },
                    )
                })
            })
            .unwrap()
    };
    for target in [AssemblyTarget::X64SystemV, AssemblyTarget::X64Windows, AssemblyTarget::A64] {
        let text = dump(target);
        assert!(text.contains("BUFFER_READF64"), "{target:?} has no load:\n{text}");
        // The namecall is gone from the fast path; what remains of it is the interpreter's exit.
        assert!(!text.contains("FALLBACK_NAMECALL"), "{target:?} kept the namecall:\n{text}");
        assert!(!text.contains("INVOKE"), "{target:?} kept a call:\n{text}");
    }
}

#[test]
fn a_view_outlives_collection_and_its_parent_while_native_code_reads_it() {
    let (runtime, bytes) = fixture();
    load(
        &runtime,
        r"--!native
function state.churn(view: jess_runtime_SharedBuffer): number
  local total = 0
  for index = 1, 200000 do
    local garbage = { index }
    total += view:readf64(0) * #garbage
    if index % 50000 == 0 then
      jess.bench.collect()
    end
  end

  return total
end
",
    );
    run(&runtime, "state.view = nil\njess.bench.collect()");
    let total: f64 = runtime.eval("return state.churn(state.slice)").unwrap();
    let expected = f64::from_bits(f64_at(&bytes[9..29], 0));
    let mut sum = 0.0;
    for _ in 0..200_000 {
        sum += expected;
    }

    assert_eq!(total.to_bits(), sum.to_bits());
}
