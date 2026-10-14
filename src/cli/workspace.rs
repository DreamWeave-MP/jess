//! `jess workspace`: the workspace the current folder is in, or the one `--workspace` names, and
//! its members' targets and tests.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use jess::lua::workspace::{self, Locate};
use serde::Serialize;

use crate::output::{self, Format};

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
    /// Run a member's command target, with its package resolved over the workspace and exactly
    /// the capabilities the target declares, once granted.
    Run(RunArgs),
    /// Run members' tests, each from its source and then packed, with its dependencies and
    /// dev-dependencies resolved over the workspace.
    Test {
        /// The members to test, by package id or path. Without any, every member with tests.
        members: Vec<String>,
    },
}

#[derive(Debug, Args)]
struct RunArgs {
    /// The target: <package id>:<target id>; a target id or command name only one target in the
    /// workspace has; or a member's package id, for its default target. Without one, the default
    /// target of the member the current folder is in.
    target: Option<String>,
    #[command(flatten)]
    grant: crate::package::GrantArgs,
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
    args.workspace
        .as_ref()
        .map_or_else(|| current().map(Locate::From), |root| Ok(Locate::Root(root.clone())))
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

/// Runs a workspace target; its first returned number is the process's exit status.
fn run_target(at: Locate, args: &RunArgs) -> Result<u8, String> {
    let request = workspace::RunRequest {
        at,
        target: args.target.clone(),
        cwd: current()?,
        grant: args.grant.grant(),
        args: args.args.clone(),
    };
    let ran = workspace::run_target(&jess::lua::Options::default(), &request).map_err(|error| error.to_string())?;
    let status = crate::run::script_status(&ran.runtime, &ran.values);
    crate::leave(ran);
    status
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
    }
    Ok(0)
}
