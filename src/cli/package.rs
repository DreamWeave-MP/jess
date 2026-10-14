//! `jess pack` and `jess package`: packing a Package Source into a JPK, and inspecting, resolving,
//! testing, and running packages. Opening, packing, and resolving run no package code; `test` runs
//! the package's own tests, against its source and then against it packed, and `run` runs one of
//! its command targets.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::output::{self, Format, Written};

#[derive(Debug, Args)]
pub struct PackArgs {
    /// The Package Source folder, the one holding jess.toml.
    source: PathBuf,
    /// The JPK to write. It is replaced atomically, and must not be inside the source.
    #[arg(short, long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
pub struct PackageArgs {
    #[command(subcommand)]
    command: PackageCommand,
}

#[derive(Debug, Subcommand)]
enum PackageCommand {
    /// Show a package's identity, digest, exports, layers, dependencies, and the capabilities its
    /// layers require, from a Package Source folder or a .jpk. Runs nothing.
    Inspect {
        /// A Package Source folder or a .jpk file.
        package: PathBuf,
    },
    /// Resolve layers over packages into an activation plan: the layer order, the packages
    /// selected, the module names, and the capabilities required. Runs nothing.
    Graph {
        /// Package Source folders or .jpk files to resolve over.
        #[arg(required = true)]
        packages: Vec<PathBuf>,
        /// A layer to resolve (repeatable). Without any, every layer the packages declare.
        #[arg(long = "layer", value_name = "ID")]
        layers: Vec<String>,
        /// A folder of packages to find layers and dependencies in (repeatable): each Package
        /// Source folder and .jpk directly inside it, opened only when one is needed. A package
        /// named outright is chosen over one found here; two found for one need are an error.
        #[arg(long = "package-path", value_name = "FOLDER")]
        package_paths: Vec<PathBuf>,
    },
    /// Run the tests a Package Source's manifest names, against the source and then against it
    /// packed into a temporary JPK, and check both providers give the same digest. The package's
    /// [dependencies] and [dev-dependencies] are selected with it. A .jpk carries no tests and
    /// reports none.
    Test {
        /// A Package Source folder (or a .jpk, which has no tests to run).
        package: PathBuf,
        /// A package its dependencies resolve over (repeatable): a Package Source folder or a .jpk.
        #[arg(long = "package", value_name = "PACKAGE")]
        packages: Vec<PathBuf>,
        /// A folder of packages its dependencies resolve over (repeatable), read as
        /// package graph reads one.
        #[arg(long = "package-path", value_name = "FOLDER")]
        package_paths: Vec<PathBuf>,
    },
    /// Run one of a package's command targets: its entry, as a script, with the package's layers
    /// activated and exactly the capabilities the target declares.
    Run(RunTargetArgs),
}

#[derive(Debug, Args)]
pub struct RunTargetArgs {
    /// A Package Source folder or a .jpk file: the package whose target runs.
    package: PathBuf,
    /// The target to run. Without one, the package's default target.
    #[arg(long, value_name = "ID")]
    target: Option<String>,
    /// A package its dependencies resolve over (repeatable): a Package Source folder or a .jpk.
    #[arg(long = "package", value_name = "PACKAGE")]
    packages: Vec<PathBuf>,
    /// A folder of packages its dependencies resolve over (repeatable), read as package graph
    /// reads one.
    #[arg(long = "package-path", value_name = "FOLDER")]
    package_paths: Vec<PathBuf>,
    #[command(flatten)]
    grant: GrantArgs,
    /// Arguments for the target, which gets them as ... and in the global arg table.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/// What a command target is granted: what it declares, by name or all of it.
#[derive(Debug, Args)]
pub struct GrantArgs {
    /// Grant a capability the target declares (repeatable). One it declares and isn't granted
    /// stops the run before anything is activated.
    #[arg(long = "capability", value_name = "NAME", conflicts_with = "trusted")]
    capabilities: Vec<String>,
    /// Grant everything the target declares; package inspect shows what that is.
    #[arg(long)]
    trusted: bool,
}

impl GrantArgs {
    /// The grant a target request carries: `None`, the trusted policy, or the capabilities named.
    pub fn grant(&self) -> Option<Vec<String>> {
        (!self.trusted).then(|| self.capabilities.clone())
    }
}

#[derive(Serialize)]
struct Packed {
    input: String,
    id: String,
    version: String,
    digest: String,
    files: usize,
    size: u64,
    written: Written,
}

#[derive(Serialize)]
struct Inspected {
    input: String,
    #[serde(flatten)]
    package: jess::lua::package::Inspection,
}

fn runtime() -> Result<l3i::Runtime, String> {
    crate::run::lua_default()
}

fn text(path: &std::path::Path) -> String {
    path.display().to_string()
}

fn texts(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| text(path)).collect()
}

pub fn pack(args: &PackArgs, format: Format) -> Result<(), String> {
    let source = std::fs::canonicalize(&args.source)
        .map_err(|error| format!("failed to read '{}': {error}", args.source.display()))?;
    let output_folder = args
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(std::env::current_dir, std::fs::canonicalize)
        .map_err(|error| format!("failed to read the folder of '{}': {error}", args.output.display()))?;
    if output_folder.starts_with(&source) {
        return Err(format!(
            "'{}' is inside the package source; write the JPK somewhere else",
            args.output.display()
        ));
    }
    let lua = runtime()?;
    let inspection = jess::lua::package::inspect(&lua, &text(&args.source)).map_err(|error| error.to_string())?;
    let packed =
        jess::lua::package::pack(&lua, &text(&args.source), &text(&args.output)).map_err(|error| error.to_string())?;
    output::print(
        &Packed {
            input: text(&args.source),
            id: inspection.id,
            version: inspection.version,
            digest: packed.digest,
            files: packed.files,
            size: packed.size,
            written: Written {
                path: text(&args.output),
                backup: None,
            },
        },
        format,
    )
}

/// Runs a command target; its first returned number is the process's exit status.
fn run_target(args: &RunTargetArgs) -> Result<u8, String> {
    let request = jess::lua::package::TargetRequest {
        package: text(&args.package),
        target: args.target.clone(),
        packages: texts(&args.packages),
        paths: texts(&args.package_paths),
        candidates: Vec::new(),
        grant: args.grant.grant(),
        args: args.args.clone(),
    };
    let ran = jess::lua::package::run_target(&jess::lua::Options::default(), &request).map_err(|error| error.to_string())?;
    let status = crate::run::script_status(&ran.runtime, &ran.values);
    // Nothing in the state needs dropping to finish its work: targets write files as they go.
    crate::leave(ran);
    status
}

pub fn run(args: &PackageArgs, format: Format) -> Result<u8, String> {
    let done = match &args.command {
        PackageCommand::Run(args) => return run_target(args),
        PackageCommand::Inspect { package } => {
            let lua = runtime()?;
            let inspection = jess::lua::package::inspect(&lua, &text(package)).map_err(|error| error.to_string())?;
            output::print(
                &Inspected {
                    input: text(package),
                    package: inspection,
                },
                format,
            )
        }
        PackageCommand::Graph {
            packages,
            layers,
            package_paths,
        } => {
            let lua = runtime()?;
            let locations = texts(packages);
            let mut layers = layers.clone();
            if layers.is_empty() {
                for location in &locations {
                    let inspection = jess::lua::package::inspect(&lua, location).map_err(|error| error.to_string())?;
                    layers.extend(inspection.layers.into_iter().map(|layer| layer.id));
                }
            }
            let search = jess::lua::package::Search {
                paths: texts(package_paths),
                ..Default::default()
            };
            let plan =
                jess::lua::package::graph_with(&lua, &locations, &layers, &search).map_err(|error| error.to_string())?;
            output::print(&plan, format)
        }
        PackageCommand::Test {
            package,
            packages,
            package_paths,
        } => {
            let scratch = std::env::temp_dir().join(format!("jess-package-test-{}", std::process::id()));
            let report = jess::lua::package::test_with(
                &jess::lua::Options::default(),
                package,
                &scratch,
                &texts(packages),
                &jess::lua::package::Search {
                    paths: texts(package_paths),
                    ..Default::default()
                },
            );
            drop(std::fs::remove_dir_all(&scratch));
            let report = report.map_err(|error| error.to_string())?;
            output::print(&report, format)?;
            if report.providers.is_empty() {
                eprintln!(
                    "jess: {} carries no tests; run `jess package test` on its Package Source",
                    package.display()
                );
            }
            if report.failed > 0 {
                return Err(format!("{} failed", report.failed));
            }
            Ok(())
        }
    };
    done.map(|()| 0)
}
