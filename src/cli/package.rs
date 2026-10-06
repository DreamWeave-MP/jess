//! `jess pack` and `jess package`: packing a Package Source into a JPK, and inspecting, resolving,
//! and testing packages. Opening, packing, and resolving run no package code; `test` runs the
//! package's own tests, against its source and then against it packed.

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
    },
    /// Run the tests a Package Source's manifest names, against the source and then against it
    /// packed into a temporary JPK, and check both providers give the same digest. A .jpk carries
    /// no tests and reports none.
    Test {
        /// A Package Source folder (or a .jpk, which has no tests to run).
        package: PathBuf,
    },
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

pub fn run(args: &PackageArgs, format: Format) -> Result<(), String> {
    match &args.command {
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
        PackageCommand::Graph { packages, layers } => {
            let lua = runtime()?;
            let locations = packages.iter().map(|path| text(path)).collect::<Vec<_>>();
            let mut layers = layers.clone();
            if layers.is_empty() {
                for location in &locations {
                    let inspection = jess::lua::package::inspect(&lua, location).map_err(|error| error.to_string())?;
                    layers.extend(inspection.layers.into_iter().map(|layer| layer.id));
                }
            }
            let plan = jess::lua::package::graph(&lua, &locations, &layers).map_err(|error| error.to_string())?;
            output::print(&plan, format)
        }
        PackageCommand::Test { package } => {
            let scratch = std::env::temp_dir().join(format!("jess-package-test-{}", std::process::id()));
            let report = jess::lua::package::test(&jess::lua::Options::default(), package, &scratch);
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
    }
}
