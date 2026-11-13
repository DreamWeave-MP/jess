//! `jess test`: runs Luau test modules, each returning a suite made with `@jess/test`, and
//! prints one report. The cases, hooks, and expectations are the module's; jess loads, runs,
//! and counts.

use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;

use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct TestArgs {
    /// Test modules: Luau files that each return a suite from require('@jess/test').new(). A
    /// folder stands for every *.test.jsl file in it and the folders below it.
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

/// What marks a file in a folder as a test module. A file named outright needs no mark.
const TEST_SUFFIX: &str = ".test.jsl";

/// The test modules `folder` holds: every `*.test.jsl` file in it and below it, folder by folder
/// and each in order of name, so a run is the same on every machine. A link to a file is the
/// file; a link to a folder is not entered, which is what keeps a link back up the tree from
/// never ending. A folder whose name starts with a dot is nobody's tests: version control's, a
/// workspace's kept packages.
fn modules_in(folder: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("'{}': {error}", folder.display());
    let mut entries = fs::read_dir(folder)
        .map_err(failed)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(failed)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type().map_err(failed)?.is_dir() {
            if !name.starts_with('.') {
                modules_in(&entry.path(), found)?;
            }
        } else if name.ends_with(TEST_SUFFIX) && entry.path().is_file() {
            found.push(entry.path());
        }
    }
    Ok(())
}

/// The modules a command line names: each file as given, as often as it is given, and each
/// folder as the test modules it holds that nothing earlier on the line has named. A folder
/// holding none at all is an error, since a run that tested nothing would pass.
fn named_modules(named: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut modules = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for path in named {
        if !path.is_dir() {
            seen.extend(fs::canonicalize(path));
            modules.push(path.clone());
            continue;
        }
        let mut held = Vec::new();
        modules_in(path, &mut held)?;
        if held.is_empty() {
            return Err(format!(
                "'{}' holds no test modules: a folder's are its *{TEST_SUFFIX} files, and a package's are the ones its manifest lists, which jess package test runs",
                path.display()
            ));
        }
        // Two folders that overlap, or a link beside the file it names, are one module each.
        held.retain(|module| fs::canonicalize(module).map_or(true, |real| seen.insert(real)));
        modules.append(&mut held);
    }
    Ok(modules)
}

pub fn run(args: &TestArgs, format: Format) -> Result<(), String> {
    let modules = named_modules(&args.modules)?;
    let grant = jess::lua::host_script_grant(&args.capabilities);
    // The modules of one run are one folder's tests: what the first runs with, they all do.
    let named = crate::run::Named {
        packages: &args.packages,
        layers: &args.layers,
        paths: &args.package_paths,
    }
    .resolve(None, &modules[0])?;
    let report =
        jess::lua::testing::run_prepared(&jess::lua::Options::default(), &grant, &modules, !args.shared, &|runtime| {
            crate::run::activate(runtime, &named, &grant).map_err(l3i::Error::runtime)
        })
        .map_err(|error| error.to_string())?;
    output::print(&report, format)?;
    if report.failed > 0 {
        return Err(format!("{} failed", report.failed));
    }
    Ok(())
}
