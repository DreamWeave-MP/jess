//! Workspaces: Package Sources developed together, the host side of `stdlib/workspace.jsl`.
//!
//! A `jess-workspace.toml` names its members, folders below it that each hold a Package Source.
//! A package no workspace lists is a workspace of one. What a workspace is, which one a folder
//! belongs to, and what its members are is decided in Luau, through the package bootstrap; this
//! module hands it the folders to start from and reads back what it found. Opening a workspace
//! reads manifests and runs nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use l3i::Runtime;
use serde::{Deserialize, Serialize};

use crate::lua::package::{self, Dependency, Launched, Plan, Search, Target, TargetRun};

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
    /// The tools it is developed with, from its file's `[dev-dependencies]`: pinned by `sync`, run
    /// by `run`, and no member's dependency.
    pub dev_dependencies: Vec<Dependency>,
    /// What its `jess.lock` pins, and whether that is what the members need.
    pub lock: Lock,
    /// Why a package with a workspace above it is a workspace of its own.
    pub note: Option<String>,
}

/// A package a workspace's `jess.lock` pins.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Locked {
    /// Its package id.
    pub id: String,
    /// Its exact version.
    pub version: String,
    /// Its content digest.
    pub digest: String,
    /// Where it came from: `local`, a package offered to `sync`, or `publisher`, the manifest of
    /// the `DreamWeave` project that publishes it.
    pub source: String,
    /// For a `publisher` source, the project manifest's URL.
    pub manifest: Option<String>,
    /// For a `publisher` source, the SHA-256 of the archive the manifest lists.
    pub artifact_sha256: Option<String>,
    /// The package ids it depends on.
    pub dependencies: Vec<String>,
    /// Where the workspace keeps it: a JPK under `.jess/packages`.
    pub location: String,
}

/// What a workspace's `jess.lock` pins, and whether that is what its members need.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Lock {
    /// Whether there is a `jess.lock`. A workspace that needs nothing beyond its members needs none.
    pub file: bool,
    /// `fresh` when it pins what the members need; `stale` when it doesn't; `missing` when it does,
    /// but `.jess/packages` lacks a package it pins.
    pub state: String,
    /// What is stale or missing, one line each.
    pub problems: Vec<String>,
    /// The packages it pins, by id.
    pub packages: Vec<Locked>,
}

/// A package id and the content digest a resolution holds it to.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Pin {
    /// The package id.
    pub id: String,
    /// The content digest.
    pub digest: String,
}

fn pin_map(pins: &[Pin]) -> BTreeMap<String, String> {
    pins.iter().map(|pin| (pin.id.clone(), pin.digest.clone())).collect()
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
        "devDependencies": [{ "id": "", "requirement": "" }],
        "lock": {
            "file": false, "state": "", "problems": [""],
            "packages": [{
                "id": "", "version": "", "digest": "", "source": "", "manifest": null, "artifactSha256": null,
                "dependencies": [""], "location": "",
            }],
        },
    })
}

/// Opens the workspace `at` names, reading its file, its members' manifests, and its lock, and
/// opening every package the lock keeps to check it has the digest the lock pins.
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
    /// The packages its dependencies resolve over, offered as candidates: the other members, and
    /// the packages the workspace's `jess.lock` pins.
    pub candidates: Vec<String>,
    /// The digests the pinned packages are held to.
    pub pins: Vec<Pin>,
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
        "pins": [{ "id": "", "digest": "" }],
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
    /// The digests the pinned packages among them are held to.
    pub pins: Vec<Pin>,
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
        pins: pin_map(&chosen.pins),
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
        pins: chosen.pins,
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
    package::run_target(options, &target_request(options, request)?)
}

/// Launches a workspace target of either kind, as [`package::launch`] launches a package's: an
/// application target resolves over the workspace exactly as a command target does.
///
/// # Errors
///
/// As [`choose`] and [`package::launch`].
pub fn launch(options: &crate::lua::Options, request: &RunRequest) -> l3i::Result<Launched> {
    package::launch(options, &target_request(options, request)?)
}

/// The package request a workspace target resolves to: its package, the workspace's members and
/// kept packages as candidates, and the lock's pins.
fn target_request(options: &crate::lua::Options, request: &RunRequest) -> l3i::Result<package::TargetRequest> {
    let chosen = choose(
        &crate::lua::runtime(options.clone())?,
        &request.at,
        request.target.as_deref(),
        &request.cwd,
    )?;
    Ok(package::TargetRequest {
        package: chosen.location,
        target: Some(chosen.target),
        candidates: chosen.candidates,
        pins: pin_map(&chosen.pins),
        grant: request.grant.clone(),
        args: request.args.clone(),
        ..package::TargetRequest::default()
    })
}

/// What a workspace member's tests resolve over.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Tested {
    package: String,
    location: String,
    candidates: Vec<String>,
    pins: Vec<Pin>,
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
    let shape = serde_json::json!([{
        "package": "", "location": "", "candidates": [""], "pins": [{ "id": "", "digest": "" }],
    }]);
    let (start, explicit) = at.args();
    let runtime = crate::lua::runtime(options.clone())?;
    let names = package::string_list(&runtime.stack(), names)?;
    let picked: Vec<Tested> = package::bootstrap(&runtime, "workspaceTests", &shape, (start, explicit, names))?;
    picked
        .into_iter()
        .map(|member| {
            let search = Search {
                candidates: member.candidates,
                pins: pin_map(&member.pins),
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

/// A request to bring a workspace's `jess.lock` and `.jess/packages` into line with what its
/// members need beyond each other.
#[derive(Clone, Debug)]
pub struct SyncRequest {
    /// The workspace.
    pub at: Locate,
    /// Packages offered outright, Package Source folders or `.jpk` files: for its id, the one.
    pub artifacts: Vec<String>,
    /// Package path folders whose Package Sources and JPKs are offered.
    pub paths: Vec<String>,
    /// Fail rather than change `jess.lock`.
    pub locked: bool,
    /// Ask no publisher: only what is offered and kept.
    pub offline: bool,
    /// Package indexes, such as the `jess-packages-v1.json` the `AsHyAmS` index publishes, which
    /// say who publishes an id.
    pub indexes: Vec<String>,
    /// Publishers named outright, a site's `dreamweave.json` or one project manifest, which are
    /// authorities for the packages their projects publish.
    pub publishers: Vec<String>,
}

/// A package a sync pinned.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncedPackage {
    /// Its package id.
    pub id: String,
    /// Its exact version.
    pub version: String,
    /// Its content digest.
    pub digest: String,
    /// `local` or `publisher`.
    pub source: String,
    /// For a `publisher` source, the project manifest it was taken from.
    pub manifest: Option<String>,
    /// Whether this sync put it in `.jess/packages`, rather than finding it there.
    pub installed: bool,
}

/// What a sync did.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Synced {
    /// `written` or `unchanged`.
    pub lock: String,
    /// The packages pinned, by id.
    pub packages: Vec<SyncedPackage>,
}

/// Brings a workspace's `jess.lock` and `.jess/packages` into line with what its members need
/// beyond each other.
///
/// Each package id needed gets one version, which every requirement on it must accept. A package
/// offered outright is the one for its id; otherwise what the lock already pins, while it still
/// meets every requirement and is kept, offered, or, for a publisher source, still listed by its
/// publisher's manifest with the pinned digests; otherwise the one package offered that does;
/// otherwise, unless offline, the newest version the one publisher named or indexed offers. Two
/// that would each do are an error, never a choice, and so are two publishers of one id. A choice
/// that a package chosen later rules out is made again with its requirements, until nothing
/// changes. Package Sources are packed into `.jess/packages`, JPKs copied there and archives
/// downloaded there, each checked against its digests before it is kept, and `jess.lock` is
/// written last. The runtime needs `network.dns.resolve` and `network.tcp.connect` for anything
/// to be fetched; offline, nothing is.
///
/// # Errors
///
/// Returns an error if the workspace doesn't open, a requirement can't be met by exactly one
/// package offered or published, a download isn't the bytes its manifest describes, a package changes
/// while it is installed, or `request.locked` and the lock would change.
pub fn sync(runtime: &Runtime, request: &SyncRequest) -> l3i::Result<Synced> {
    let shape = serde_json::json!({
        "lock": "",
        "packages": [{ "id": "", "version": "", "digest": "", "source": "", "manifest": null, "installed": false }],
    });
    let (start, explicit) = request.at.args();
    let (artifacts, paths, indexes, publishers) = {
        let stack = runtime.stack();
        (
            package::string_list(&stack, &request.artifacts)?,
            package::string_list(&stack, &request.paths)?,
            package::string_list(&stack, &request.indexes)?,
            package::string_list(&stack, &request.publishers)?,
        )
    };
    package::bootstrap(
        runtime,
        "workspaceSync",
        &shape,
        (
            start,
            explicit,
            artifacts,
            paths,
            request.locked,
            request.offline,
            indexes,
            publishers,
        ),
    )
}
