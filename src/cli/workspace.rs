//! `jess workspace`: the workspace the current folder is in, or the one `--workspace` names, and
//! its members' targets.

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
    args.workspace.as_ref().map_or_else(
        || {
            std::env::current_dir()
                .map(Locate::From)
                .map_err(|error| format!("failed to read the current folder: {error}"))
        },
        |root| Ok(Locate::Root(root.clone())),
    )
}

pub fn run(args: &WorkspaceArgs, format: Format) -> Result<u8, String> {
    let lua = crate::run::lua_default()?;
    let opened = workspace::open(&lua, &locate(args)?).map_err(|error| error.to_string())?;
    match args.command {
        WorkspaceCommand::Info => output::print(&opened, format)?,
        WorkspaceCommand::Targets => {
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
            output::print(&listed, format)?;
        }
    }
    Ok(0)
}
