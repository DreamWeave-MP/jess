//! `jess test`: runs Luau test modules, each returning a suite made with `@jess/test`, and
//! prints one report. The cases, hooks, and expectations are the module's; jess loads, runs,
//! and counts.

use std::path::PathBuf;

use clap::Args;

use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct TestArgs {
    /// Test modules: Luau files that each return a suite from require('@jess/test').new().
    #[arg(required = true)]
    modules: Vec<PathBuf>,
    /// Run every module in one VM, sharing the module cache, instead of a fresh VM each.
    #[arg(long)]
    shared: bool,
    /// Explicitly grant an optional runtime capability to the modules (repeatable): what a suite
    /// that reads or writes files needs.
    #[arg(long = "capability", value_name = "NAME")]
    capabilities: Vec<String>,
    /// A package to activate before each module runs (repeatable), as jess run does: a Package
    /// Source folder or a .jpk, whose exports the modules can then require.
    #[arg(long = "package", value_name = "PACKAGE")]
    packages: Vec<PathBuf>,
    /// A layer to activate (repeatable). Without any, every layer the packages named with
    /// --package declare.
    #[arg(long = "layer", value_name = "ID")]
    layers: Vec<String>,
    /// A folder of packages to find layers and dependencies in (repeatable), as jess run takes
    /// it.
    #[arg(long = "package-path", value_name = "FOLDER")]
    package_paths: Vec<PathBuf>,
}

pub fn run(args: &TestArgs, format: Format) -> Result<(), String> {
    let grant = jess::lua::host_script_grant(&args.capabilities);
    // The modules of one run are one folder's tests: what the first runs with, they all do.
    let named = crate::run::Named {
        packages: &args.packages,
        layers: &args.layers,
        paths: &args.package_paths,
    }
    .resolve(None, &args.modules[0])?;
    let report = jess::lua::testing::run_prepared(
        &jess::lua::Options::default(),
        &grant,
        &args.modules,
        !args.shared,
        &|runtime| crate::run::activate(runtime, &named, &grant).map_err(l3i::Error::runtime),
    )
    .map_err(|error| error.to_string())?;
    output::print(&report, format)?;
    if report.failed > 0 {
        return Err(format!("{} failed", report.failed));
    }
    Ok(())
}
