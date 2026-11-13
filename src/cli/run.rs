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
    #[arg(long = "layer", value_name = "ID")]
    layers: Vec<String>,
    /// A folder of packages to find layers and dependencies in (repeatable): each Package Source
    /// folder and .jpk directly inside it, opened only when one is needed. A package named with
    /// --package is chosen over one found here; two found for one need are an error.
    #[arg(long = "package-path", value_name = "FOLDER")]
    package_paths: Vec<PathBuf>,
    /// Explicitly grant an optional runtime/package capability (repeatable).
    #[arg(long = "capability", value_name = "NAME", conflicts_with = "trusted")]
    capabilities: Vec<String>,
    /// Grant the script every capability, as running it as a program (jess script.jsl) does:
    /// files, the environment, processes, and the network included.
    #[arg(long)]
    trusted: bool,
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
    jess::lua::runtime_with_profile(jess::lua::Options::default(), capabilities, profiling)
        .map_err(|error| error.to_string())
}

/// A default-deny runtime for CLI commands that have no capability flags.
pub fn lua_default() -> Result<Runtime, String> {
    lua(&[])
}

/// The packages a command line names for a loose script or test module: `--package`, `--layer`,
/// and `--package-path`.
pub struct Named<'a> {
    pub packages: &'a [PathBuf],
    pub layers: &'a [String],
    pub paths: &'a [PathBuf],
}

/// What the package paths hold, for whoever named a path and chose nothing from it: each
/// package's file, and its layers.
fn offered(lua: &Runtime, paths: &[PathBuf]) -> String {
    let mut found = Vec::new();
    for path in paths {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for entry in entries {
            let is_package = entry.join("jess.toml").is_file() || entry.extension().is_some_and(|kind| kind == "jpk");
            if !is_package {
                continue;
            }
            let Ok(inspection) = jess::lua::package::inspect(lua, &entry.display().to_string()) else {
                continue;
            };
            let layers: Vec<String> = inspection.layers.into_iter().map(|layer| layer.id).collect();
            found.push(if layers.is_empty() {
                format!("{} (no layers)", entry.display())
            } else {
                format!("{} (layers: {})", entry.display(), layers.join(", "))
            });
        }
    }
    if found.is_empty() {
        return "no package was found there".to_owned();
    }
    format!("found {}", found.join("; "))
}

/// Activates the packages `named` in `lua`, if any, admitting entrypoints against
/// `capabilities`, what the runtime was granted.
pub fn activate(lua: &Runtime, named: &Named<'_>, capabilities: &[String]) -> Result<(), String> {
    if named.packages.is_empty() && named.paths.is_empty() {
        return if named.layers.is_empty() {
            Ok(())
        } else {
            Err("--layer needs packages to resolve over: name them with --package or --package-path".to_owned())
        };
    }
    let text = |paths: &[PathBuf]| paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>();
    let locations = text(named.packages);
    let mut layers = named.layers.to_vec();
    let mut roots = Vec::new();
    // Without --layer, every layer the named packages declare, and the packages themselves, so
    // one with no layers still brings its dependencies' exports.
    if layers.is_empty() {
        for location in &locations {
            let inspection = jess::lua::package::inspect(lua, location).map_err(|error| error.to_string())?;
            layers.extend(inspection.layers.into_iter().map(|layer| layer.id));
            roots.push(inspection.id);
        }
    }
    if layers.is_empty() && roots.is_empty() {
        return Err(format!(
            "a package path offers packages without choosing any: name one with --package, or a layer with --layer; \
             {}",
            offered(lua, named.paths)
        ));
    }
    let search = jess::lua::package::Search {
        paths: text(named.paths),
        roots,
        ..Default::default()
    };
    jess::lua::package::activate_with(lua, &locations, &layers, Some(capabilities), &search)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// `jess <script>.jsl [ARGS]...`: the script run as a program, trusted, with no packages activated.
/// It is what a shebang line, `#!/usr/bin/env jess`, makes an executable script into.
pub fn run_as_program(script: PathBuf, args: Vec<String>) -> Result<u8, String> {
    run(&RunArgs {
        script,
        packages: Vec::new(),
        layers: Vec::new(),
        package_paths: Vec::new(),
        capabilities: Vec::new(),
        trusted: true,
        profile: false,
        emit_asm: None,
        disassemble: None,
        args,
    })
}

pub fn run(args: &RunArgs) -> Result<u8, String> {
    if !args.script.is_file() {
        return Err(format!("'{}' isn't a file", args.script.display()));
    }
    jess::lua::scripts::require_jsl(&args.script).map_err(|error| error.to_string())?;
    if (args.emit_asm.is_some() || args.disassemble.is_some()) && !args.args.is_empty() {
        return Err("script inspection modes don't execute the script and take no script arguments".to_owned());
    }
    // A script run from the command line is its user's: it holds their terminal and their input
    // devices, and what they grant. Run as a program, `jess script.jsl` or `--trusted`, it holds
    // everything, as any program they chose to run would.
    let capabilities = if args.trusted {
        jess::lua::known_capabilities().map_err(|error| error.to_string())?
    } else {
        jess::lua::host_script_grant(&args.capabilities)
    };
    let lua = lua_with_profile(&capabilities, args.profile)?;
    let named = Named {
        packages: &args.packages,
        layers: &args.layers,
        paths: &args.package_paths,
    };
    activate(&lua, &named, &capabilities)?;
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
    crate::leave(lua, ());
    result
}

/// The first returned number as the process exit status; other return values mean success.
pub fn script_status(lua: &Runtime, values: &[l3i::value::Value]) -> Result<u8, String> {
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

/// How many source files and lines a profile lists; `JESS_PROFILE_LINES` asks for more lines.
const PROFILE_FILES: usize = 24;
const PROFILE_LINES: usize = 20;

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
    // By source file as well: twenty lines seldom say where a long run went, and the files do.
    let mut files: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for (line, location) in &lines {
        let file = line.rsplit_once(':').map_or(line.as_str(), |(file, _)| file);
        *files.entry(file.to_owned()).or_default() += location.samples;
    }
    let mut by_file: Vec<_> = files.into_iter().collect();
    by_file.sort_by_key(|(file, samples)| (Reverse(*samples), file.clone()));
    // A share in tenths of a percent, kept in whole numbers: a count of samples has no business
    // in a float.
    let total = samples.count.max(1);
    for (file, count) in by_file.into_iter().take(PROFILE_FILES) {
        let tenths = count.saturating_mul(1000) / total;
        eprintln!("  {count:>8}  {:>3}.{}%  {file}", tenths / 10, tenths % 10);
    }
    let shown = std::env::var("JESS_PROFILE_LINES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(PROFILE_LINES);
    for (line, location) in lines.into_iter().take(shown) {
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
