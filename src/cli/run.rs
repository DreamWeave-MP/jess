//! `jess run`: runs a Luau script with the `jess` API, optionally with packages activated.

use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use jess::lua::l3i::Runtime;
use jess::lua::l3i::native_code::AssemblyOptions;

#[derive(Debug, Args)]
pub struct RunArgs {
    /// The Luau script to run, a file.
    script: PathBuf,
    /// A package to activate before the script runs (repeatable): a Package Source folder or a
    /// .jpk. Its exports are then require-able by the script, and the activated layers'
    /// entrypoints have run.
    #[arg(long = "package", value_name = "PACKAGE")]
    packages: Vec<PathBuf>,
    /// A layer to activate (repeatable). Without any, every layer the packages declare.
    #[arg(long = "layer", value_name = "ID", requires = "packages")]
    layers: Vec<String>,
    /// Explicitly grant an optional runtime/package capability (repeatable).
    #[arg(long = "capability", value_name = "NAME")]
    capabilities: Vec<String>,
    /// Print script call/sampling/native-block statistics after execution.
    #[arg(long)]
    profile: bool,
    /// Write generated native ASM for this script without executing it.
    #[arg(long, value_name = "FILE", conflicts_with = "disassemble")]
    emit_asm: Option<PathBuf>,
    /// Write textual Luau bytecode disassembly without executing the script.
    #[arg(long, value_name = "FILE", conflicts_with = "emit_asm")]
    disassemble: Option<PathBuf>,
    /// Arguments for the script, which gets them as ... and in the global arg table.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/// A Luau runtime with the whole jess API and only the host-approved capabilities. It loads no
/// application configuration: a script that wants one loads it.
pub fn lua(capabilities: &[String]) -> Result<Runtime, String> {
    lua_with_profile(capabilities, false)
}

fn lua_with_profile(capabilities: &[String], profiling: bool) -> Result<Runtime, String> {
    jess::lua::runtime_with_profile(jess::lua::Options::default(), capabilities, profiling).map_err(|error| error.to_string())
}

/// A default-deny runtime for CLI commands that have no capability flags.
pub fn lua_default() -> Result<Runtime, String> {
    lua(&[])
}

/// Activates the packages `args` names in `lua`, if any.
fn activate(lua: &Runtime, args: &RunArgs) -> Result<(), String> {
    if args.packages.is_empty() {
        return Ok(());
    }
    let locations = args
        .packages
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let plan = if args.layers.is_empty() {
        jess::lua::package::activate_all(lua, &locations, Some(&args.capabilities))
    } else {
        jess::lua::package::activate(lua, &locations, &args.layers, Some(&args.capabilities))
    };
    plan.map(|_| ()).map_err(|error| error.to_string())
}

pub fn run(args: &RunArgs) -> Result<u8, String> {
    if !args.script.is_file() {
        return Err(format!("'{}' isn't a file", args.script.display()));
    }
    if (args.emit_asm.is_some() || args.disassemble.is_some()) && !args.args.is_empty() {
        return Err("script inspection modes don't execute the script and take no script arguments".to_owned());
    }
    let lua = lua_with_profile(&args.capabilities, args.profile)?;
    activate(&lua, args)?;
    let result = match (&args.emit_asm, &args.disassemble) {
        (Some(output), _) => emit_assembly(&lua, &args.script, output).map(|()| 0),
        (_, Some(output)) => disassemble(&lua, &args.script, output).map(|()| 0),
        (None, None) => jess::lua::scripts::run_file(&lua, &args.script, &args.args)
            .map(|values| script_status(&lua, &values))
            .map_err(|error| error.to_string())?,
    };
    if args.profile {
        print_profile(&lua);
    }
    // Nothing in the state needs dropping to finish its work: scripts write files as they go.
    crate::leave(lua);
    result
}

/// The first returned number as the process exit status; other return values mean success.
fn script_status(lua: &Runtime, values: &[l3i::value::Value]) -> Result<u8, String> {
    let Some(value) = values.first() else { return Ok(0) };
    let number = lua
        .stack()
        .with_frame(|frame| value.push_to(frame)?.read::<f64>())
        .map_err(|error| error.to_string());
    let Ok(number) = number else { return Ok(0) };
    if !number.is_finite() || number.fract() != 0.0 || !(0.0..=f64::from(u8::MAX)).contains(&number) {
        return Err(format!("script exit status must be an integer from 0 to 255, got {number}"));
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the guard above requires a finite integral value in 0..=255"
    )]
    let status = number as u8;
    Ok(status)
}

fn emit_assembly(lua: &Runtime, script: &Path, output: &Path) -> Result<(), String> {
    let source = fs::read(script).map_err(|error| format!("failed to read '{}': {error}", script.display()))?;
    let function =
        jess::lua::scripts::load(lua, &script.display().to_string(), &source).map_err(|error| error.to_string())?;
    let generator = lua
        .native_code()
        .ok_or_else(|| "this Jess runtime has no Luau native-code generator".to_owned())?;
    let assembly = lua
        .stack()
        .with_frame(|frame| {
            function.push_to(frame)?;
            generator.assembly(frame, -1, AssemblyOptions::default())
        })
        .map_err(|error| error.to_string())?;
    fs::write(output, assembly).map_err(|error| format!("failed to write '{}': {error}", output.display()))
}

fn disassemble(lua: &Runtime, script: &Path, output: &Path) -> Result<(), String> {
    let source = fs::read_to_string(script).map_err(|error| format!("failed to read '{}': {error}", script.display()))?;
    let listing = l3i::source::disassemble(&source, &lua.compile_options()).map_err(|error| error.to_string())?;
    fs::write(output, listing).map_err(|error| format!("failed to write '{}': {error}", output.display()))?;
    Ok(())
}

fn print_profile(lua: &Runtime) {
    let stats = lua.call_stats();
    let allocations = stats.allocation_by_context.get(&1).copied().unwrap_or_default();
    let context_ms = stats.time_by_context_ms.get(&1).copied().unwrap_or_default();
    eprintln!(
        "jess profile: {:.3} ms script self-time, {} timed calls, {} overhead samples ({:.3} ms), {} bytes grown",
        context_ms, stats.timed_calls, stats.overhead_samples, stats.overhead_total_ms, allocations
    );
    let samples = lua.samples();
    eprintln!("jess profile: {} safepoint samples", samples.count);
    let mut lines: Vec<_> = samples.lines.into_iter().collect();
    lines.sort_by_key(|(_, location)| Reverse(location.samples));
    for (line, location) in lines.into_iter().take(20) {
        eprintln!("  {:>6}  {line}", location.samples);
    }
    if let Some(generator) = lua.native_code() {
        let native = generator.execution_stats(&lua.stack());
        eprintln!(
            "jess profile: {} native blocks executed, {} VM exits",
            native.regular_blocks_executed, native.vm_exits_taken
        );
    }
}
