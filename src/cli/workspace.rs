//! `jess workspace`: the workspace the current folder is in, or the one `--workspace` names, and
//! its members' targets and tests.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use jess::lua::image::Image;
use jess::lua::workspace::{self, Locate};
use serde::Serialize;

use crate::output::{self, Format};

/// The package index asked when no `--index` or `JESS_REGISTRY_URL` names one: the `AsHyAmS`
/// index's listing of every Jess package the sites it reads publish, which only says who publishes
/// an id.
pub const DEFAULT_INDEX: &str = "https://dreamweave-mp.github.io/AsHyAmS/network-data/jess-packages-v1.json";

#[derive(Debug, Args)]
pub struct WorkspaceArgs {
    /// The workspace's root: the folder holding its jess-workspace.toml, or a package's folder for
    /// a workspace of one. Without it, the workspace the current folder is in.
    #[arg(long, global = true, value_name = "FOLDER")]
    workspace: Option<PathBuf>,
    #[command(subcommand)]
    command: WorkspaceCommand,
}

#[derive(Debug, Subcommand)]
enum WorkspaceCommand {
    /// Show the workspace: its root, its members, and what each depends on. Runs nothing.
    Info,
    /// List the members' targets, each as <package id>:<target id>.
    Targets,
    /// Resolve a target without running it: its package, what that resolved over, and the
    /// activation plan running it would mount.
    Graph {
        /// The target, named as for workspace run.
        target: Option<String>,
    },
    /// Run a target, a command or an application, of a member or of a package jess.lock pins, with
    /// its package resolved over the workspace and exactly the capabilities the target declares,
    /// once granted.
    Run(RunArgs),
    /// Pin what the members depend on beyond each other in jess.lock, and keep those packages,
    /// as JPKs, in .jess/packages: from the packages offered here, those already kept, and, for
    /// what neither holds, the manifest of the DreamWeave project publishing the package, found
    /// through an index or a publisher named with --publisher. Every download is checked against
    /// the manifest's SHA-256 and the content digest before it is kept.
    Sync {
        /// A package to offer outright (repeatable): a Package Source folder or a .jpk, the one
        /// for its id.
        artifacts: Vec<PathBuf>,
        /// A folder of packages to offer (repeatable): each Package Source folder and .jpk directly
        /// inside it. Two that would each do for one id are an error; name one outright.
        #[arg(long = "package-path", value_name = "FOLDER")]
        package_paths: Vec<PathBuf>,
        /// Fail rather than change jess.lock: packages it pins may still be put in .jess/packages,
        /// from their publishers or from what is offered.
        #[arg(long)]
        locked: bool,
        /// Ask no publisher: only what is offered and kept. JESS_OFFLINE=1 says the same.
        #[arg(long, env = "JESS_OFFLINE", value_parser = clap::builder::FalseyValueParser::new())]
        offline: bool,
        /// A publisher to take packages from (repeatable): a DreamWeave site's dreamweave.json, or
        /// one project manifest. It is the authority for what it lists, and the way to pick one
        /// publisher when an index knows two.
        #[arg(long, value_name = "URL")]
        publisher: Vec<String>,
        /// A package index that says who publishes an id (repeatable). Replaces the default,
        /// AsHyAmS's; JESS_REGISTRY_URL does the same.
        #[arg(long, value_name = "URL", env = "JESS_REGISTRY_URL", value_delimiter = ',')]
        index: Vec<String>,
    },
    /// Run members' tests, each from its source and then packed, with its dependencies and
    /// dev-dependencies resolved over the workspace.
    Test {
        /// The members to test, by package id or path. Without any, every member with tests.
        members: Vec<String>,
    },
    /// Freeze the workspace into a packed workspace, a .jsw: the targets its [pack] table names,
    /// or every member's, and every package they run, pinned. Runs offline from the one file:
    /// jess app.jsw, or jess workspace --workspace app.jsw run TARGET.
    Pack {
        /// The file to write.
        #[arg(long, value_name = "FILE")]
        output: PathBuf,
        /// Write a standalone executable instead: this jess executable, for any platform, carrying
        /// the packed workspace, which runs its default target with every argument.
        #[arg(long, value_name = "JESS")]
        runtime: Option<PathBuf>,
    },
}

#[derive(Debug, Args)]
struct RunArgs {
    /// The target: <package id>:<target id>; a target id or command name only one target in the
    /// workspace has; or a package id, for its default target. Members and the packages jess.lock
    /// pins both count. Without one, the default target of the member the current folder is in.
    target: Option<String>,
    #[command(flatten)]
    grant: crate::package::GrantArgs,
    /// For an application target: stop after N frames, shutdown handlers and all, unless it
    /// stops itself first.
    #[arg(long, value_name = "N")]
    frames: Option<u64>,
    /// Arguments for the target, after --, which gets them as ... and in the global arg table.
    #[arg(last = true)]
    args: Vec<String>,
}

/// One member's target, named so it can be run from anywhere in the workspace.
#[derive(Serialize)]
struct Listed {
    target: String,
    kind: String,
    command: Option<String>,
    default: bool,
    capabilities: Vec<String>,
}

fn locate(args: &WorkspaceArgs) -> Result<Locate, String> {
    let Some(root) = &args.workspace else {
        return current().map(Locate::From);
    };
    Locate::at(root.clone()).map_err(|error| format!("failed to open '{}': {error}", root.display()))
}

fn texts(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| path.display().to_string()).collect()
}

fn current() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|error| format!("failed to read the current folder: {error}"))
}

fn info(args: &WorkspaceArgs, format: Format) -> Result<(), String> {
    let lua = crate::run::lua_default()?;
    let opened = workspace::open(&lua, &locate(args)?).map_err(|error| error.to_string())?;
    if !matches!(args.command, WorkspaceCommand::Targets) {
        return output::print(&opened, format);
    }
    let listed = opened
        .members
        .iter()
        .flat_map(|member| {
            member.targets.iter().map(|target| Listed {
                target: format!("{}:{}", member.id, target.id),
                kind: target.kind.clone(),
                command: target.command.clone(),
                default: member.default_target.as_ref() == Some(&target.id),
                capabilities: target.capabilities.clone(),
            })
        })
        .collect::<Vec<_>>();
    output::print(&listed, format)
}

/// Runs a workspace target: a command target's first returned number is the process's exit
/// status, and an application's the status it asked to stop with.
fn run_target(at: Locate, args: &RunArgs) -> Result<u8, String> {
    let request = workspace::RunRequest {
        at,
        target: args.target.clone(),
        cwd: current()?,
        grant: args.grant.grant(),
        args: args.args.clone(),
    };
    if args.frames.is_some() {
        let lua = crate::run::lua_default()?;
        let chosen = workspace::choose(&lua, &request.at, request.target.as_deref(), &request.cwd)
            .map_err(|error| error.to_string())?;
        let inspection = jess::lua::package::inspect_with(&lua, &chosen.location, request.at.image())
            .map_err(|error| error.to_string())?;
        let target = jess::lua::package::target(&inspection, Some(&chosen.target)).map_err(|error| error.to_string())?;
        crate::application::check_frames(args.frames, target, &inspection.id)?;
    }
    let launched = workspace::launch(&jess::lua::Options::default(), &request).map_err(|error| error.to_string())?;
    crate::application::finish(launched, args.frames)
}

/// Runs a target of a packed workspace as a program, `jess app.jsw` or a standalone executable
/// does: `target`, or its default without one, holding what it declares, with `args`.
pub fn run_packed(image: Image, target: Option<String>, args: Vec<String>) -> Result<u8, String> {
    let request = workspace::RunRequest {
        at: Locate::Packed(image),
        target,
        cwd: current()?,
        grant: None,
        args,
    };
    let launched = workspace::launch(&jess::lua::Options::default(), &request).map_err(|error| error.to_string())?;
    crate::application::finish(launched, None)
}

/// Runs the packed workspace this executable carries: the target whose command is the name it
/// was run by, when one has it, so one payload can ship under several names; else its default
/// target. The name is argv[0]'s, which a link keeps and the executable's own path resolves away.
pub fn run_standalone(image: Image, args: Vec<String>) -> Result<u8, String> {
    let name = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()));
    let lua = crate::run::lua_default()?;
    let opened = workspace::open(&lua, &Locate::Packed(image.clone())).map_err(|error| error.to_string())?;
    let target = name.filter(|name| {
        opened.members.iter().any(|member| {
            member
                .targets
                .iter()
                .any(|target| target.command.as_deref() == Some(name.as_str()))
        })
    });
    run_packed(image, target, args)
}

fn test(at: &Locate, members: &[String], format: Format) -> Result<(), String> {
    let scratch = std::env::temp_dir().join(format!("jess-workspace-test-{}", std::process::id()));
    let reports = workspace::test(&jess::lua::Options::default(), at, members, &scratch);
    drop(std::fs::remove_dir_all(&scratch));
    let reports = reports.map_err(|error| error.to_string())?;
    output::print(&reports, format)?;
    let failed = reports.iter().map(|report| report.failed).sum::<usize>();
    if failed > 0 {
        return Err(format!("{failed} failed"));
    }
    Ok(())
}

pub fn run(args: &WorkspaceArgs, format: Format) -> Result<u8, String> {
    let at = locate(args)?;
    match &args.command {
        WorkspaceCommand::Info | WorkspaceCommand::Targets => info(args, format)?,
        WorkspaceCommand::Graph { target } => {
            let lua = crate::run::lua_default()?;
            let resolved =
                workspace::resolve_target(&lua, &at, target.as_deref(), &current()?).map_err(|error| error.to_string())?;
            output::print(&resolved, format)?;
        }
        WorkspaceCommand::Run(run) => return run_target(at, run),
        WorkspaceCommand::Test { members } => test(&at, members, format)?,
        WorkspaceCommand::Pack { output, runtime } => {
            let lua = crate::run::lua_default()?;
            let packed = workspace::pack(&lua, &at, output, runtime.as_deref()).map_err(|error| error.to_string())?;
            output::print(&packed, format)?;
        }
        WorkspaceCommand::Sync {
            artifacts,
            package_paths,
            locked,
            offline,
            publisher,
            index,
        } => {
            // Syncing is the host acting, in a runtime that runs no package code; reaching
            // publishers is its own grant and never a package's. Offline, there is none.
            let lua = if *offline {
                crate::run::lua_default()?
            } else {
                crate::run::lua(&["network.dns.resolve".to_owned(), "network.tcp.connect".to_owned()])?
            };
            let indexes = if index.is_empty() {
                vec![DEFAULT_INDEX.to_owned()]
            } else {
                index.clone()
            };
            let request = workspace::SyncRequest {
                at,
                artifacts: texts(artifacts),
                paths: texts(package_paths),
                locked: *locked,
                offline: *offline,
                indexes,
                publishers: publisher.clone(),
            };
            let synced = workspace::sync(&lua, &request).map_err(|error| error.to_string())?;
            output::print(&synced, format)?;
        }
    }
    Ok(0)
}
