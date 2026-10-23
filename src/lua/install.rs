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
    /// A package id, from its publisher: the packed workspace its newest release carries, or else
    /// resolved from its packages, with the requirement a version must meet.
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
    /// For a package id: resolve it from its publisher's packages even when a release carries it
    /// as an application.
    pub resolve: bool,
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

/// A name an install takes over: from another install, from a file no install owns, or from an
/// executable of its own that has changed since it was written.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Take {
    /// The command name.
    pub name: String,
    /// `install` (another install's), `stray` (a file or link no install owns), or `changed` (this
    /// package's own executable, no longer the one written).
    pub kind: String,
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
    /// `publisher` (a package's own packed workspace), `resolved`, `file` or `workspace`.
    pub source: String,
    /// For a `publisher` or `resolved` install, the project manifest it came from.
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
    /// What whoever is asked should know: a newer release this jess can't run, or a package
    /// resolved because no release carries it as an application.
    pub notes: Vec<String>,
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
    /// Such commands left where they are, because what is there is no longer the executable the
    /// earlier install wrote: each path, and why.
    pub kept: Vec<String>,
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
    /// `publisher`, `resolved`, `file` or `workspace`.
    pub source: String,
    /// For a `publisher` or `resolved` install, the project manifest it came from.
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
    /// Its commands left where they are, because what is there is no longer the executable it
    /// wrote: each path, and why.
    pub kept: Vec<String>,
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
        "takes": [{ "name": "", "kind": "", "from": null }],
        "notes": [""],
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
        table.set(&stack, "resolve", &request.resolve)?;
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
/// Everything is written beside the staging folder first; then every file is put in place, or,
/// when one can't be, none is, and what was there before is back. What it replaces is removed only
/// once everything is in place.
///
/// # Errors
///
/// Returns an error if this jess can't carry a packed workspace, or a file can't be written or put
/// in place. Nothing outside the staging folder has changed then.
pub fn commit(runtime: &Runtime, prepared: &Prepared) -> l3i::Result<Installed> {
    let shape = serde_json::json!({
        "package": "", "version": "", "bin": "",
        "commands": [{ "name": "", "path": "" }],
        "removed": [""],
        "kept": [""],
    });
    let outbox = outbox(&prepared.staging);
    let args = (text(&prepared.root)?, text(&prepared.staging)?, text(&outbox)?, SUFFIX);
    let installed = call(runtime, "commit", &shape, args);
    if installed.is_err() {
        abandon(prepared);
    }
    drop(std::fs::remove_dir_all(&outbox));
    installed
}

/// Where what a commit puts in place is written first: beside the staging folder, on the same
/// filesystem as the root it goes into.
fn outbox(staging: &Path) -> PathBuf {
    let mut name = staging.file_name().unwrap_or_default().to_owned();
    name.push(".out");
    staging.with_file_name(name)
}

/// Drops a prepared install: its staging folder is removed and nothing else changes.
pub fn abandon(prepared: &Prepared) {
    drop(std::fs::remove_dir_all(&prepared.staging));
    drop(std::fs::remove_dir_all(outbox(&prepared.staging)));
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
    let shape = serde_json::json!([{ "package": "", "version": "", "commands": [""], "kept": [""] }]);
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

/// What is at `path`, a link not followed: `file`, `link`, `directory`, or `other`; `None` when
/// nothing is.
fn occupant(path: &Path) -> Option<&'static str> {
    let kind = std::fs::symlink_metadata(path).ok()?.file_type();
    Some(if kind.is_symlink() {
        "link"
    } else if kind.is_dir() {
        "directory"
    } else if kind.is_file() {
        "file"
    } else {
        "other"
    })
}

/// One step of putting an install in place, or taking one away: `from`, a file or folder staged
/// for it, put at `to`, or, with no `from`, what is at `to` moved away. Whatever is at `to` is
/// moved aside into the trash first, where it stays until every step has succeeded. Only a step
/// for one of the install root's own `folder`s may meet a directory at `to`.
#[derive(Debug, Deserialize)]
struct Step {
    to: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    folder: bool,
}

/// What one step did, so it can be undone.
struct Done {
    to: PathBuf,
    from: Option<PathBuf>,
    aside: Option<PathBuf>,
}

/// Takes every step in order, or none of them: a step that fails undoes the steps before it,
/// last first, so what was at each `to` is back where it was and what was staged is back where it
/// was staged. Before anything moves, a directory at a `to` that isn't a `folder`'s is refused.
///
/// # Errors
///
/// Returns why a step failed, and whether undoing the ones before it succeeded.
fn publish(steps: &[Step], trash: &Path) -> Result<(), String> {
    for step in steps {
        let to = Path::new(&step.to);
        if !step.folder && matches!(occupant(to), Some("directory" | "other")) {
            return Err(format!(
                "'{}' is a directory, and jess install never replaces or removes one; nothing was changed",
                to.display()
            ));
        }
    }
    std::fs::create_dir_all(trash).map_err(|source| format!("failed to create '{}': {source}", trash.display()))?;
    let mut done = Vec::new();
    for step in steps {
        match take(step, trash) {
            Ok(record) => done.push(record),
            Err(problem) => {
                return Err(match undo(done) {
                    Ok(()) => format!("{problem}; everything before it was put back"),
                    Err(more) => format!("{problem}; putting back what was done before it failed too: {more}"),
                });
            }
        }
    }
    Ok(())
}

/// Takes one step, or, when it fails halfway, puts back what it moved aside.
fn take(step: &Step, trash: &Path) -> Result<Done, String> {
    let to = PathBuf::from(&step.to);
    let aside = if occupant(&to).is_some() {
        let aside = retired_name(trash, &to);
        std::fs::rename(&to, &aside).map_err(|source| format!("failed to move '{}' aside: {source}", to.display()))?;
        Some(aside)
    } else {
        None
    };
    let from = step.from.as_ref().map(PathBuf::from);
    if let Some(from) = &from
        && let Err(source) = std::fs::rename(from, &to)
    {
        let problem = format!("failed to put '{}' in place: {source}", to.display());
        return match aside.map(|aside| std::fs::rename(&aside, &to)) {
            Some(Err(again)) => Err(format!("{problem}, and moving back what was there failed: {again}")),
            _ => Err(problem),
        };
    }
    Ok(Done { to, from, aside })
}

/// Undoes `done`, last first.
fn undo(done: Vec<Done>) -> Result<(), String> {
    let mut problems = Vec::new();
    for step in done.into_iter().rev() {
        if let Some(from) = &step.from
            && let Err(source) = std::fs::rename(&step.to, from)
        {
            problems.push(format!("'{}': {source}", step.to.display()));
            continue;
        }
        if let Some(aside) = &step.aside
            && let Err(source) = std::fs::rename(aside, &step.to)
        {
            problems.push(format!("'{}' (kept as '{}'): {source}", step.to.display(), aside.display()));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Writes this jess carrying the packed workspace at `payload` to `output`, a new file of the
/// staging folder's, executable, and answers its SHA-256.
fn standalone(payload: &Path, output: &Path) -> l3i::Result<String> {
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
    crate::lua::files::replace_executable(output, &bytes).map_err(|source| failed("write", output, source))?;
    Ok(crate::lua::files::sha256(&bytes))
}

/// Makes `link`, a new file of the staging folder's, the same executable as `existing`: a hard
/// link, or a copy where linking fails.
fn link(existing: &Path, link: &Path) -> l3i::Result<()> {
    if std::fs::hard_link(existing, link).is_ok() {
        return Ok(());
    }
    std::fs::copy(existing, link)
        .map(drop)
        .map_err(|source| error(format!("failed to write '{}': {source}", link.display())))
}

/// Removes what it can of what is in `trash`: an executable still running stays until it stops.
fn empty_trash(trash: &Path) {
    let Ok(entries) = std::fs::read_dir(trash) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if occupant(&path) == Some("directory") {
            drop(std::fs::remove_dir_all(&path));
        } else {
            drop(std::fs::remove_file(&path));
        }
    }
}

/// The SHA-256 of the file at `path`, a link not followed, or `None` when there is no file.
fn digest(path: &Path) -> l3i::Result<Option<String>> {
    if occupant(path) != Some("file") {
        return Ok(None);
    }
    std::fs::read(path)
        .map(|bytes| Some(crate::lua::files::sha256(&bytes)))
        .map_err(|source| error(format!("failed to read '{}': {source}", path.display())))
}

/// The file operations `stdlib/install.jsl` needs beyond the package host table's, as a read-only
/// table: `standalone`, `link`, `occupant`, `digest`, `publish` (its steps as JSON), and
/// `emptyTrash`.
pub(crate) fn host_table(runtime: &Runtime) -> l3i::Result<Table> {
    let functions = [
        (
            "standalone",
            runtime.bind_function("jess.install.standalone", |payload: Text, output: Text| {
                standalone(Path::new(&payload.0), Path::new(&output.0))
            })?,
        ),
        (
            "link",
            runtime.bind_function("jess.install.link", |existing: Text, to: Text| {
                link(Path::new(&existing.0), Path::new(&to.0))
            })?,
        ),
        (
            "occupant",
            runtime.bind_function("jess.install.occupant", |path: Text| occupant(Path::new(&path.0)))?,
        ),
        (
            "digest",
            runtime.bind_function("jess.install.digest", |path: Text| digest(Path::new(&path.0)))?,
        ),
        (
            "publish",
            runtime.bind_function("jess.install.publish", |steps: Text, trash: Text| {
                let steps: Vec<Step> =
                    serde_json::from_str(&steps.0).map_err(|problem| error(format!("install steps: {problem}")))?;
                publish(&steps, Path::new(&trash.0)).map_err(error)
            })?,
        ),
        (
            "emptyTrash",
            runtime.bind_function("jess.install.emptyTrash", |trash: Text| empty_trash(Path::new(&trash.0)))?,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn step(to: &Path, from: Option<&Path>) -> Step {
        Step {
            to: to.display().to_string(),
            from: from.map(|from| from.display().to_string()),
            folder: false,
        }
    }

    /// A step that fails undoes every step before it: each target holds what it held, and what
    /// was staged for it is back where it was staged.
    #[test]
    fn a_failed_step_puts_back_every_step_before_it() {
        let folder = tempfile::tempdir().unwrap();
        let (bin, staged, trash) = (
            folder.path().join("bin"),
            folder.path().join("staged"),
            folder.path().join("trash"),
        );
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(bin.join("first"), "old first").unwrap();
        std::fs::write(bin.join("retired"), "old retired").unwrap();
        std::fs::write(staged.join("first"), "new first").unwrap();
        let steps = [
            step(&bin.join("first"), Some(&staged.join("first"))),
            step(&bin.join("retired"), None),
            step(&bin.join("second"), Some(&staged.join("never-staged"))),
        ];

        let problem = publish(&steps, &trash).unwrap_err();
        assert!(problem.contains("everything before it was put back"), "{problem}");
        assert_eq!(std::fs::read_to_string(bin.join("first")).unwrap(), "old first");
        assert_eq!(std::fs::read_to_string(bin.join("retired")).unwrap(), "old retired");
        assert_eq!(std::fs::read_to_string(staged.join("first")).unwrap(), "new first");
        assert!(!bin.join("second").exists());
        assert_eq!(std::fs::read_dir(&trash).unwrap().count(), 0);
    }

    /// A directory where a command goes is never moved, replaced or emptied, before anything else
    /// is touched; one of the root's own folders may be.
    #[test]
    fn a_directory_where_a_command_goes_is_never_touched() {
        let folder = tempfile::tempdir().unwrap();
        let (bin, trash) = (folder.path().join("bin"), folder.path().join("trash"));
        std::fs::create_dir_all(bin.join("greet/important")).unwrap();
        std::fs::write(bin.join("greet/important/data.txt"), "keep me").unwrap();
        std::fs::write(bin.join("other"), "old other").unwrap();
        let staged = folder.path().join("staged");
        std::fs::write(&staged, "new other").unwrap();

        let steps = [step(&bin.join("other"), Some(&staged)), step(&bin.join("greet"), None)];
        let problem = publish(&steps, &trash).unwrap_err();
        assert!(problem.contains("never replaces or removes one"), "{problem}");
        assert_eq!(
            std::fs::read_to_string(bin.join("greet/important/data.txt")).unwrap(),
            "keep me"
        );
        assert_eq!(std::fs::read_to_string(bin.join("other")).unwrap(), "old other");

        let own = Step {
            folder: true,
            ..step(&bin.join("greet"), None)
        };
        publish(&[own], &trash).unwrap();
        assert!(!bin.join("greet").exists());
        assert_eq!(std::fs::read_dir(&trash).unwrap().count(), 1);
    }
}
