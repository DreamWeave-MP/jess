//! `jess check`: type-checks loose scripts in strict mode, the way an editor in a folder
//! `jess init` set up checks them, with the packages `jess run` would activate mounted so their
//! exports type too. Nothing runs.

use std::path::PathBuf;

use clap::Args;

use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// The scripts to check, .jsl files. A file one of them requires with ./ or ../ is checked
    /// with it.
    #[arg(required = true)]
    scripts: Vec<PathBuf>,
    /// A package whose exports the scripts require (repeatable): a Package Source folder or a
    /// .jpk, resolved as jess run --package resolves it. Nothing in it runs.
    #[arg(long = "package", value_name = "PACKAGE")]
    packages: Vec<PathBuf>,
    /// A layer to resolve (repeatable). Without any, every layer the packages declare.
    #[arg(long = "layer", value_name = "ID")]
    layers: Vec<String>,
    /// A folder of packages to find layers and dependencies in (repeatable), as jess run
    /// --package-path reads one.
    #[arg(long = "package-path", value_name = "FOLDER")]
    package_paths: Vec<PathBuf>,
}

pub fn run(args: &CheckArgs, format: Format) -> Result<(), String> {
    let texts = |paths: &[PathBuf]| paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>();
    let report = jess::lua::check::check_scripts(
        &jess::lua::Options::default(),
        &args.scripts,
        &texts(&args.packages),
        &args.layers,
        &jess::lua::package::Search {
            paths: texts(&args.package_paths),
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())?;
    output::print(&report, format)?;
    if !report.errors.is_empty() {
        let count = report.errors.len();
        return Err(format!("{count} type error{}", if count == 1 { "" } else { "s" }));
    }
    Ok(())
}
