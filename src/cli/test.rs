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
}

pub fn run(args: &TestArgs, format: Format) -> Result<(), String> {
    let report = jess::lua::testing::run_with(
        &jess::lua::Options::default(),
        &args.capabilities,
        &args.modules,
        !args.shared,
    )
    .map_err(|error| error.to_string())?;
    output::print(&report, format)?;
    if report.failed > 0 {
        return Err(format!("{} failed", report.failed));
    }
    Ok(())
}
