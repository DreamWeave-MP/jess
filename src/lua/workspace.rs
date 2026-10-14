//! Workspaces: Package Sources developed together, the host side of `stdlib/workspace.jsl`.
//!
//! A `jess-workspace.toml` names its members, folders below it that each hold a Package Source.
//! A package no workspace lists is a workspace of one. What a workspace is, which one a folder
//! belongs to, and what its members are is decided in Luau, through the package bootstrap; this
//! module hands it the folders to start from and reads back what it found. Opening a workspace
//! reads manifests and runs nothing.

use std::path::{Path, PathBuf};

use l3i::Runtime;
use serde::{Deserialize, Serialize};

use crate::lua::package::{self, Dependency, Plan, Search, Target, TargetRun};

/// Where a workspace is.
#[derive(Clone, Debug)]
pub enum Locate {
    /// The workspace a folder is in: the one listing the package it is in, or the nearest one
    /// above it outside any package, or the package itself as a workspace of one.
    From(PathBuf),
    /// The workspace whose root is this folder.
    Root(PathBuf),
}

impl Locate {
    fn args(&self) -> (String, bool) {
        match self {
            Self::From(path) => (path.display().to_string(), false),
            Self::Root(path) => (path.display().to_string(), true),
        }
    }
}

/// A package the workspace develops.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Member {
    /// Its package id.
    pub id: String,
    /// Its version.
    pub version: String,
    /// Its folder as the workspace file names it: `.` for the root, else `/`-joined names.
    pub path: String,
    /// Its folder on disk.
    pub location: String,
    /// The target a runner selects when the member is named without one.
    pub default_target: Option<String>,
    /// Its runnable targets, by id.
    pub targets: Vec<Target>,
    /// Its package dependencies.
    pub dependencies: Vec<Dependency>,
    /// The packages only its tests need.
    pub dev_dependencies: Vec<Dependency>,
    /// Its test modules.
    pub tests: Vec<String>,
}

/// An opened workspace.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Workspace {
    /// The folder its `jess-workspace.toml` is in, or the package's own for a workspace of one.
    pub root: String,
    /// `package` when the root folder's package is a member, `virtual` when the workspace file
    /// names members only, `implicit` for a package no workspace lists.
    pub kind: String,
    /// Its members, by path.
    pub members: Vec<Member>,
    /// Why a package with a workspace above it is a workspace of its own.
    pub note: Option<String>,
}

fn target_shape() -> serde_json::Value {
    serde_json::json!({ "id": "", "kind": "", "entry": "", "capabilities": [""], "command": null })
}

fn workspace_shape() -> serde_json::Value {
    serde_json::json!({
        "root": "", "kind": "", "note": null,
        "members": [{
            "id": "", "version": "", "path": "", "location": "", "defaultTarget": null,
            "targets": [target_shape()],
            "dependencies": [{ "id": "", "requirement": "" }],
            "devDependencies": [{ "id": "", "requirement": "" }],
            "tests": [""],
        }],
    })
}

/// Opens the workspace `at` names, reading its file and its members' manifests.
///
/// # Errors
///
/// Returns an error if there is no workspace there, or its file or a member is invalid.
pub fn open(runtime: &Runtime, at: &Locate) -> l3i::Result<Workspace> {
    package::bootstrap(runtime, "workspace", &workspace_shape(), at.args())
}

/// A target a workspace picked, and what its package resolves over.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Chosen {
    /// The member's package id.
    pub package: String,
    /// Its version.
    pub version: String,
    /// Its folder.
    pub location: String,
    /// The target's id.
    pub target: String,
    /// The packages its dependencies resolve over, offered as candidates: the other members.
    pub candidates: Vec<String>,
}

/// Picks a target in the workspace `at` names.
///
/// `name` is `<package id>:<target id>`, a target id or command name only one target in the
/// workspace has, or a member's package id for its default target; without one, the default
/// target of the member holding `cwd`.
///
/// # Errors
///
/// Returns an error if the workspace doesn't open, or `name` picks no target or more than one.
pub fn choose(runtime: &Runtime, at: &Locate, name: Option<&str>, cwd: &Path) -> l3i::Result<Chosen> {
    let shape = serde_json::json!({
        "package": "", "version": "", "location": "", "target": "", "candidates": [""],
    });
    let (start, explicit) = at.args();
    let args = (start, explicit, name, cwd.display().to_string());
    package::bootstrap(runtime, "workspaceTarget", &shape, args)
}

/// A workspace target resolved, with nothing run: the package and target, what it resolved over,
/// and the activation plan running it would mount.
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedTarget {
    /// The member's package id.
    pub package: String,
    /// Its version.
    pub version: String,
    /// Its folder.
    pub location: String,
    /// The target, as its manifest declares it.
    pub target: Target,
    /// The packages its dependencies resolved over, offered as candidates.
    pub candidates: Vec<String>,
    /// What running it activates: every layer its package declares, over those candidates.
    pub plan: Plan,
}

/// Resolves the target `name` picks, as [`choose`] reads it, without running anything.
///
/// # Errors
///
/// As [`choose`], and if the plan doesn't resolve.
pub fn resolve_target(runtime: &Runtime, at: &Locate, name: Option<&str>, cwd: &Path) -> l3i::Result<ResolvedTarget> {
    let chosen = choose(runtime, at, name, cwd)?;
    let inspection = package::inspect(runtime, &chosen.location)?;
    let target = package::target(&inspection, Some(&chosen.target))?.clone();
    let layers = inspection.layers.iter().map(|layer| layer.id.clone()).collect::<Vec<_>>();
    let search = Search {
        candidates: chosen.candidates.clone(),
        roots: vec![chosen.package.clone()],
        ..Search::default()
    };
    let plan = package::graph_with(runtime, std::slice::from_ref(&chosen.location), &layers, &search)?;
    Ok(ResolvedTarget {
        package: chosen.package,
        version: chosen.version,
        location: chosen.location,
        target,
        candidates: chosen.candidates,
        plan,
    })
}

/// A request to run a workspace target.
#[derive(Clone, Debug)]
pub struct RunRequest {
    /// The workspace.
    pub at: Locate,
    /// The target, as [`choose`] reads a name.
    pub target: Option<String>,
    /// The folder it is run from, which picks the member when no target is named.
    pub cwd: PathBuf,
    /// The capabilities the host grants; `None` is the trusted policy, as for
    /// [`package::TargetRequest`].
    pub grant: Option<Vec<String>>,
    /// The target's arguments.
    pub args: Vec<String>,
}

/// Runs a workspace target, as [`package::run_target`] runs a package's.
///
/// Its package resolves over the workspace, and it runs with exactly the capabilities it declares,
/// once granted. Being in a workspace grants nothing.
///
/// # Errors
///
/// As [`choose`] and [`package::run_target`].
pub fn run_target(options: &crate::lua::Options, request: &RunRequest) -> l3i::Result<TargetRun> {
    let chosen = choose(
        &crate::lua::runtime(options.clone())?,
        &request.at,
        request.target.as_deref(),
        &request.cwd,
    )?;
    package::run_target(
        options,
        &package::TargetRequest {
            package: chosen.location,
            target: Some(chosen.target),
            candidates: chosen.candidates,
            grant: request.grant.clone(),
            args: request.args.clone(),
            ..package::TargetRequest::default()
        },
    )
}

/// What a workspace member's tests resolve over.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Tested {
    package: String,
    location: String,
    candidates: Vec<String>,
}

/// Runs the tests of the members `names` picks, as [`package::test_with`] runs a package's.
///
/// `names` picks members by package id or path; without any, every member that has tests. Each
/// runs from its source and then packed, with its dependencies and dev-dependencies resolved over
/// the workspace, and its scratch files in a folder of its own under `scratch`.
///
/// # Errors
///
/// Returns an error if the workspace doesn't open, a name picks no member, or a member's tests
/// can't be resolved or run.
pub fn test(
    options: &crate::lua::Options,
    at: &Locate,
    names: &[String],
    scratch: &Path,
) -> l3i::Result<Vec<package::PackageTest>> {
    let shape = serde_json::json!([{ "package": "", "location": "", "candidates": [""] }]);
    let (start, explicit) = at.args();
    let runtime = crate::lua::runtime(options.clone())?;
    let names = package::string_list(&runtime.stack(), names)?;
    let picked: Vec<Tested> = package::bootstrap(&runtime, "workspaceTests", &shape, (start, explicit, names))?;
    picked
        .into_iter()
        .map(|member| {
            let search = Search {
                candidates: member.candidates,
                ..Search::default()
            };
            package::test_with(
                options,
                Path::new(&member.location),
                &scratch.join(&member.package),
                &[],
                &search,
            )
        })
        .collect()
}
