//! Installs: applications put on the PATH, the host side of `stdlib/install.jsl`.
//!
//! An install is a packed workspace embedded in this jess and written as an executable for every
//! command it exports, into an install root's `bin` (`$HOME/.jess` unless named). What to install,
//! from where, under which names, and what an install records is decided in Luau; this module
//! hands it the root and a folder to stage in, and gives it the few file operations writing
//! executables needs: this jess's own bytes, links, and moving aside what a running executable
//! won't let anything overwrite.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use l3i::Runtime;
use l3i::value::{Function, Table};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::lua::image::Image;
use crate::lua::{Text, error};

/// What an executable's name ends with on this platform.
const SUFFIX: &str = std::env::consts::EXE_SUFFIX;

/// The install root used when none is named: `.jess` in the home folder.
#[must_use]
pub fn default_root() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join(".jess"))
}

/// Where an install comes from.
#[derive(Clone, Debug)]
pub enum Source {
    /// A package id, resolved from its publisher, with the requirement a version must meet.
    Package {
        /// The package id.
        id: String,
        /// A version requirement, or an exact version.
        requirement: Option<String>,
    },
    /// The packed workspace a file holds: a `.jsw`, or an executable carrying one.
    File(Image),
    /// The workspace a folder is in, packed.
    Workspace(PathBuf),
}

/// A request to install.
#[derive(Clone, Debug)]
pub struct InstallRequest {
    /// The install root: `bin`, `installs` and the rest are under it.
    pub root: PathBuf,
    /// What to install.
    pub source: Source,
    /// Take over command names another install owns, or files no install owns, and install again
    /// what is already installed.
    pub force: bool,
    /// Ask no publisher.
    pub offline: bool,
    /// Package indexes that say who publishes an id.
    pub indexes: Vec<String>,
    /// Publishers named outright.
    pub publishers: Vec<String>,
}

/// A command an install writes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Command {
    /// The executable's name, without the platform's suffix.
    pub name: String,
    /// The target it runs, `<package id>:<target id>`.
    pub target: String,
    /// `command` or `application`.
    pub kind: String,
    /// The capabilities the target declares, which it holds when run.
    pub capabilities: Vec<String>,
}

/// What an earlier install of the same package had.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Previous {
    /// Its version.
    pub version: String,
    /// Its commands.
    pub commands: Vec<Command>,
}

/// A name an install takes over: from another install, or from a file no install owns.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Take {
    /// The command name.
    pub name: String,
    /// The package whose install owned it, if one did.
    pub from: Option<String>,
}

/// An install staged and not yet put in place, and what putting it in place would do.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Prepared {
    /// The package that names the install.
    pub package: String,
    /// Its version.
    pub version: String,
    /// `resolved`, `file` or `workspace`.
    pub source: String,
    /// For a resolved install, the project manifest the package came from.
    pub manifest: Option<String>,
    /// The commands it writes.
    pub commands: Vec<Command>,
    /// What the install it replaces had, if there is one.
    pub previous: Option<Previous>,
    /// Whether it is exactly what is installed already, executables and all.
    pub unchanged: bool,
    /// Whether it asks for anything the install it replaces wasn't granted: a new install, a
    /// command it didn't have, or a capability one of its commands didn't declare.
    pub consent: bool,
    /// The names it takes over.
    pub takes: Vec<Take>,
    /// Where it is staged.
    #[serde(skip)]
    pub staging: PathBuf,
    /// The install root.
    #[serde(skip)]
    pub root: PathBuf,
}

/// An executable an install wrote.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Written {
    /// The command's name.
    pub name: String,
    /// Where it is.
    pub path: String,
}

/// What [`commit`] did.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Installed {
    /// The package that names the install.
    pub package: String,
    /// Its version.
    pub version: String,
    /// The folder the executables are in.
    pub bin: String,
    /// The executables written.
    pub commands: Vec<Written>,
    /// The commands an earlier install of it had that this one doesn't, now removed.
    pub removed: Vec<String>,
}

/// An installed command, and whether its executable is still the one written.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ListedCommand {
    /// Its name.
    pub name: String,
    /// Where its executable is.
    pub path: String,
    /// The target it runs.
    pub target: String,
    /// `installed`, `missing`, or `changed` when something else is there now.
    pub state: String,
}

/// An install.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Listed {
    /// The package that names it.
    pub package: String,
    /// Its version.
    pub version: String,
    /// `resolved`, `file` or `workspace`.
    pub source: String,
    /// For a resolved install, the project manifest the package came from.
    pub manifest: Option<String>,
    /// The jess version its executables carry.
    pub runtime: String,
    /// Its commands.
    pub commands: Vec<ListedCommand>,
}

/// An install [`uninstall`] removed.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Removed {
    /// The package that named it.
    pub package: String,
    /// Its version.
    pub version: String,
    /// The commands removed with it.
    pub commands: Vec<String>,
}

fn command_shape() -> serde_json::Value {
    serde_json::json!({ "name": "", "target": "", "kind": "", "capabilities": [""] })
}

/// Calls `entry` of the host's installs with `args`, reading what it returns as `T`.
fn call<T: serde::de::DeserializeOwned + Serialize>(
    runtime: &Runtime,
    entry: &str,
    shape: &serde_json::Value,
    args: impl l3i::call::PushArgs,
) -> l3i::Result<T> {
    let jess = crate::lua::state(runtime)?;
    let module = crate::lua::stdlib::host_installs(runtime)?;
    let stack = runtime.stack();
    let function: Function = module.get(&stack, entry)?;
    function
        .invoke_with(&stack, args, |frame, value| {
            crate::lua::convert::from_lua(frame, &jess, value, Some(shape))
        })
        .map_err(crate::lua::callback_error)
}

fn text(path: &Path) -> l3i::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| error(format!("'{}' has a name that isn't UTF-8", path.display())))
}

/// Stages what `request` names under its root and says what installing it would do. Nothing
/// outside the staging folder changes; [`commit`] puts it in place and [`abandon`] drops it.
///
/// # Errors
///
/// Returns an error if the source can't be resolved, packed, or read, the packed workspace doesn't
/// hold what it pins or was packed for an incompatible jess, it has nothing to install, or a
/// command it would write is taken and `request.force` isn't set. The staging folder is removed
/// then.
pub fn prepare(runtime: &Runtime, request: &InstallRequest) -> l3i::Result<Prepared> {
    let staging = request.root.join("staging").join(std::process::id().to_string());
    let failed =
        |what: &str, path: &Path, source: std::io::Error| error(format!("failed to {what} '{}': {source}", path.display()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|source| failed("clear", &staging, source))?;
    }
    std::fs::create_dir_all(&staging).map_err(|source| failed("create", &staging, source))?;
    let prepared = stage(runtime, request, &staging);
    if prepared.is_err() {
        drop(std::fs::remove_dir_all(&staging));
    }
    let mut prepared = prepared?;
    prepared.staging = staging;
    prepared.root.clone_from(&request.root);
    Ok(prepared)
}

fn stage(runtime: &Runtime, request: &InstallRequest, staging: &Path) -> l3i::Result<Prepared> {
    let shape = serde_json::json!({
        "package": "", "version": "", "source": "", "manifest": null,
        "commands": [command_shape()],
        "previous": { "version": "", "commands": [command_shape()] },
        "unchanged": false, "consent": false,
        "takes": [{ "name": "", "from": null }],
    });
    let host_modules = crate::lua::package::host_modules(runtime)?;
    let table = {
        let stack = runtime.stack();
        let remote = Table::new(&stack, 0, 3)?;
        remote.set(&stack, "offline", &request.offline)?;
        remote.set(
            &stack,
            "indexes",
            &crate::lua::package::string_list(&stack, &request.indexes)?,
        )?;
        remote.set(
            &stack,
            "publishers",
            &crate::lua::package::string_list(&stack, &request.publishers)?,
        )?;
        let table = Table::new(&stack, 0, 12)?;
        table.set(&stack, "root", &text(&request.root)?)?;
        table.set(&stack, "staging", &text(staging)?)?;
        match &request.source {
            Source::Package { id, requirement } => {
                table.set(&stack, "source", "package")?;
                table.set(&stack, "name", id)?;
                if let Some(requirement) = requirement {
                    table.set(&stack, "requirement", requirement)?;
                }
            }
            Source::File(image) => {
                table.set(&stack, "source", "file")?;
                table.set(&stack, "image", &image.table(&stack)?)?;
            }
            Source::Workspace(path) => {
                table.set(&stack, "source", "workspace")?;
                table.set(&stack, "path", &text(path)?)?;
            }
        }
        table.set(&stack, "force", &request.force)?;
        table.set(&stack, "remote", &remote)?;
        table.set(
            &stack,
            "hostModules",
            &crate::lua::package::string_list(&stack, &host_modules)?,
        )?;
        table.set(&stack, "runtime", env!("CARGO_PKG_VERSION"))?;
        table.set(&stack, "suffix", SUFFIX)?;
        table
    };
    call(runtime, "prepare", &shape, (table,))
}

/// Puts a prepared install in place: its executables in the root's `bin`, and its folder, with
/// the packed workspace they carry and the record of where it came from, under `installs`.
///
/// # Errors
///
/// Returns an error if this jess can't carry a packed workspace or an executable can't be written.
pub fn commit(runtime: &Runtime, prepared: &Prepared) -> l3i::Result<Installed> {
    let shape = serde_json::json!({
        "package": "", "version": "", "bin": "",
        "commands": [{ "name": "", "path": "" }],
        "removed": [""],
    });
    let args = (text(&prepared.root)?, text(&prepared.staging)?, SUFFIX);
    let installed = call(runtime, "commit", &shape, args);
    if installed.is_err() {
        abandon(prepared);
    }
    installed
}

/// Drops a prepared install: its staging folder is removed and nothing else changes.
pub fn abandon(prepared: &Prepared) {
    drop(std::fs::remove_dir_all(&prepared.staging));
}

/// Every install under `root`.
///
/// # Errors
///
/// Returns an error if an install's record can't be read.
pub fn list(runtime: &Runtime, root: &Path) -> l3i::Result<Vec<Listed>> {
    let shape = serde_json::json!([{
        "package": "", "version": "", "source": "", "manifest": null, "runtime": "",
        "commands": [{ "name": "", "path": "", "target": "", "state": "" }],
    }]);
    call(runtime, "list", &shape, (text(root)?, SUFFIX))
}

/// Removes the installs `names` picks under `root`, each named by its package id or one of its
/// commands.
///
/// # Errors
///
/// Returns an error, removing nothing, if a name picks no install.
pub fn uninstall(runtime: &Runtime, root: &Path, names: &[String]) -> l3i::Result<Vec<Removed>> {
    let shape = serde_json::json!([{ "package": "", "version": "", "commands": [""] }]);
    let names = crate::lua::package::string_list(&runtime.stack(), names)?;
    call(runtime, "uninstall", &shape, (text(root)?, names, SUFFIX))
}

/// A name in `trash` nothing else has, for something moved there.
fn retired_name(trash: &Path, path: &Path) -> PathBuf {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let count = COUNT.fetch_add(1, Ordering::Relaxed);
    trash.join(format!("{name}.{}.{count}", std::process::id()))
}

/// Moves what is at `path`, if anything, into `trash`. A running executable can be moved where it
/// can't be overwritten or removed.
fn retire(path: &Path, trash: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    std::fs::create_dir_all(trash)?;
    std::fs::rename(path, retired_name(trash, path))
}

/// Puts the file `staged` at `path`, what was there moved into `trash` first.
fn place(staged: &Path, path: &Path, trash: &Path) -> std::io::Result<()> {
    retire(path, trash)?;
    std::fs::rename(staged, path)
}

/// A name beside `path` to write to before it is put there.
fn beside(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(format!(".jess-install-{}", std::process::id()));
    path.with_file_name(name)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Writes this jess carrying the packed workspace at `payload` to `output`, executable, and
/// answers its SHA-256.
fn standalone(payload: &Path, output: &Path, trash: &Path) -> l3i::Result<String> {
    let failed =
        |what: &str, path: &Path, source: std::io::Error| error(format!("failed to {what} '{}': {source}", path.display()));
    let jess = std::env::current_exe().map_err(|source| error(format!("failed to find this jess: {source}")))?;
    let runtime = std::fs::read(&jess).map_err(|source| failed("read", &jess, source))?;
    let payload_bytes = std::fs::read(payload).map_err(|source| failed("read", payload, source))?;
    let bytes = crate::lua::image::standalone(&runtime, &payload_bytes).map_err(|problem| {
        error(format!(
            "this jess, '{}', can't carry a packed workspace: {problem}",
            jess.display()
        ))
    })?;
    let staged = beside(output);
    crate::lua::files::replace_executable(&staged, &bytes).map_err(|source| failed("write", &staged, source))?;
    place(&staged, output, trash).map_err(|source| failed("write", output, source))?;
    Ok(hex(&sha2::Sha256::digest(&bytes)))
}

/// Makes `link` the same executable as `existing`: a hard link, or a copy where linking fails.
fn link(existing: &Path, link: &Path, trash: &Path) -> l3i::Result<()> {
    let failed = |source: std::io::Error| error(format!("failed to write '{}': {source}", link.display()));
    let staged = beside(link);
    drop(std::fs::remove_file(&staged));
    if std::fs::hard_link(existing, &staged).is_err() {
        std::fs::copy(existing, &staged).map_err(failed)?;
    }
    place(&staged, link, trash).map_err(failed)
}

/// Removes what it can of what is in `trash`: an executable still running stays until it stops.
fn empty_trash(trash: &Path) {
    let Ok(entries) = std::fs::read_dir(trash) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            drop(std::fs::remove_dir_all(&path));
        } else {
            drop(std::fs::remove_file(&path));
        }
    }
}

/// The SHA-256 of the file at `path`, or `None` when there is no file.
fn digest(path: &Path) -> l3i::Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(hex(&sha2::Sha256::digest(&bytes)))),
        Err(source) if matches!(source.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::IsADirectory) => Ok(None),
        Err(source) => Err(error(format!("failed to read '{}': {source}", path.display()))),
    }
}

/// The file operations `stdlib/install.jsl` needs beyond the package host table's, as a read-only
/// table: `standalone`, `link`, `retire`, `rename`, `emptyTrash`, and `digest`.
pub(crate) fn host_table(runtime: &Runtime) -> l3i::Result<Table> {
    let functions = [
        (
            "standalone",
            runtime.bind_function("jess.install.standalone", |payload: Text, output: Text, trash: Text| {
                standalone(Path::new(&payload.0), Path::new(&output.0), Path::new(&trash.0))
            })?,
        ),
        (
            "link",
            runtime.bind_function("jess.install.link", |existing: Text, to: Text, trash: Text| {
                link(Path::new(&existing.0), Path::new(&to.0), Path::new(&trash.0))
            })?,
        ),
        (
            "retire",
            runtime.bind_function("jess.install.retire", |path: Text, trash: Text| {
                retire(Path::new(&path.0), Path::new(&trash.0))
                    .map_err(|source| error(format!("failed to move '{}' aside: {source}", path.0)))
            })?,
        ),
        (
            "rename",
            runtime.bind_function("jess.install.rename", |from: Text, to: Text| {
                std::fs::rename(&from.0, &to.0)
                    .map_err(|source| error(format!("failed to move '{}' to '{}': {source}", from.0, to.0)))
            })?,
        ),
        (
            "emptyTrash",
            runtime.bind_function("jess.install.emptyTrash", |trash: Text| empty_trash(Path::new(&trash.0)))?,
        ),
        (
            "digest",
            runtime.bind_function("jess.install.digest", |path: Text| digest(Path::new(&path.0)))?,
        ),
    ];
    let table = {
        let stack = runtime.stack();
        let table = Table::new(&stack, 0, functions.len())?;
        for (name, function) in &functions {
            table.set(&stack, name, function)?;
        }
        table
    };
    l3i::readonly::make_read_only(runtime, &table)?;
    Ok(table)
}
