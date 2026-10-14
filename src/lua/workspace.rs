//! Workspaces: Package Sources developed together, the host side of `stdlib/workspace.jsl`.
//!
//! A `jess-workspace.toml` names its members, folders below it that each hold a Package Source.
//! A package no workspace lists is a workspace of one. What a workspace is, which one a folder
//! belongs to, and what its members are is decided in Luau, through the package bootstrap; this
//! module hands it the folders to start from and reads back what it found. Opening a workspace
//! reads manifests and runs nothing.

use l3i::Runtime;
use serde::{Deserialize, Serialize};

use crate::lua::package::{Dependency, Target};

/// Where a workspace is.
#[derive(Clone, Debug)]
pub enum Locate {
    /// The workspace a folder is in: the one listing the package it is in, or the nearest one
    /// above it outside any package, or the package itself as a workspace of one.
    From(std::path::PathBuf),
    /// The workspace whose root is this folder.
    Root(std::path::PathBuf),
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
    crate::lua::package::bootstrap(runtime, "workspace", &workspace_shape(), at.args())
}
