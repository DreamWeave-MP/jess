//! Packages at runtime: the host side of `@jess/package` and the package bootstrap.
//!
//! The package format, manifests, digests, and resolution are Luau (`stdlib/jpk.jsl`,
//! `stdlib/package.jsl`). Rust supplies the file reads they run over, runs the
//! bootstrap (`stdlib/bootstrap.jsl`) for [`inspect`], [`pack`], [`graph`], and [`activate`], and
//! owns the one seam package code needs: the mount the bootstrap hands an accepted plan to, which
//! `require` resolves package modules through.
//!
//! A package module's chunk name is `@<id>@<version>/<path>`, which diagnostics show; its cache
//! identity is its package's id and content digest and its path, so two generations of one package
//! version never share a cached module.
//!
//! An activation plan is one trust zone: every package it mounts runs in the one runtime, with the
//! capabilities that runtime was granted. Admission refuses an entrypoint that requires more than
//! the grant, but nothing separates one admitted package from another; per-package isolation
//! (contexts or VMs of their own) is not built yet.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use l3i::Runtime;
use l3i::bind::{ArgView, Call};
use l3i::convert::{Exact, NewBuffer};
use l3i::stack::Scope;
use l3i::value::{Function, Table, Value};
use serde::{Deserialize, Serialize};

use crate::lua::convert;
use crate::lua::{Bytes, Text, error};

const BOOTSTRAP: &str = include_str!("stdlib/bootstrap.jsl");
const BOOTSTRAP_CHUNK: &str = "=jess.package.bootstrap";

/// The packages an accepted plan mounted, and the module names they export.
#[derive(Default)]
pub(crate) struct Mounted {
    packages: Vec<MountedPackage>,
    exports: HashMap<String, (usize, String)>,
    /// The resolved layers' entrypoints, in activation order: package, module name, path.
    entrypoints: Vec<(usize, String, String)>,
}

struct MountedPackage {
    id: String,
    /// The mounted packages whose exports its modules may require: its declared dependencies.
    dependencies: Vec<usize>,
    digest: String,
    /// `@<id>@<version>/`, the start of every chunk name this package's modules load with.
    prefix: String,
    read: Function,
}

/// A package module, ready to load: its human chunk name, its cache identity, and its source.
pub(crate) struct PackageModule {
    pub(crate) chunk: String,
    pub(crate) key: String,
    pub(crate) source: Vec<u8>,
}

impl Mounted {
    /// The package and path a module name is exported as.
    pub(crate) fn export(&self, name: &str) -> Option<(usize, &str)> {
        self.exports.get(name).map(|(index, path)| (*index, path.as_str()))
    }

    /// The mounted package a chunk belongs to, and the chunk's path inside it.
    pub(crate) fn owner<'c>(&self, chunk: &'c str) -> Option<(usize, &'c str)> {
        self.packages
            .iter()
            .enumerate()
            .find_map(|(index, package)| chunk.strip_prefix(package.prefix.as_str()).map(|path| (index, path)))
    }

    /// Reads `path` from package `index`, as its provider has it.
    pub(crate) fn load(&self, scope: &impl Scope, index: usize, path: &str) -> l3i::Result<Option<PackageModule>> {
        let package = &self.packages[index];
        let source = package.read.invoke_with(scope, (path,), |_, value| {
            if value.is_nil() {
                return Ok(None);
            }
            Ok(Some(value.read::<l3i::convert::BufferView<'_>>()?.to_vec()))
        })?;
        Ok(source.map(|source| PackageModule {
            chunk: format!("{}{path}", package.prefix),
            key: format!("\0package\0{}\0{}\0{path}", package.id, package.digest),
            source,
        }))
    }

    /// The id of package `index`, for messages.
    pub(crate) fn id(&self, index: usize) -> &str {
        &self.packages[index].id
    }

    /// Where the package with `id` is mounted.
    pub(crate) fn index_of(&self, id: &str) -> Option<usize> {
        self.packages.iter().position(|package| package.id == id)
    }

    /// Whether a module of package `caller` may require an export of package `owner`: its own,
    /// one of a dependency it declares, or one of the built-in package, which every runtime has.
    pub(crate) fn may_require(&self, caller: usize, owner: usize) -> bool {
        caller == owner
            || self.packages[caller].dependencies.contains(&owner)
            || self.packages[owner].id == crate::lua::builtin::ID
    }
}

/// The host functions `@jess/package` reads and writes through: the only file access it has.
///
/// `read` and `write` say whether whoever holds the table may read and write the host's files.
/// The module scripts require gets what the runtime's policy grants, so a script can't open or
/// pack a package it couldn't have read or written through `@dream/fs`; the host's own package
/// operations get both, since inspecting and activating packages is the host acting.
pub(crate) fn host_table(runtime: &Runtime, read: bool, write: bool) -> l3i::Result<Table> {
    let list_tree = runtime.bind_function("jess.package.listTree", |call: &Call<'_>, root: Text| {
        let mut entries = Vec::new();
        walk(Path::new(&root.0), "", &mut entries)?;
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        crate::lua::lua::push_list(call, entries.into_iter(), |frame, (path, kind)| {
            crate::lua::push_table(frame, 0, 2, |frame, row| {
                row.raw_set_value(frame, "path", &path)?;
                row.raw_set_value(frame, "kind", &kind)
            })
        })?;
        Ok::<_, l3i::Error>(l3i::bind::StackResults)
    })?;
    let read_file = runtime.bind_function("jess.package.readFile", |path: Text| -> l3i::Result<Option<NewBuffer>> {
        match std::fs::read(&path.0) {
            Ok(bytes) => Ok(Some(NewBuffer(bytes))),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(error(format!("failed to read '{}': {source}", path.0))),
        }
    })?;
    let read_at = runtime.bind_function(
        "jess.package.readAt",
        |path: Text, offset: Exact<i64>, length: Exact<i64>| -> l3i::Result<NewBuffer> {
            let failed = |source: std::io::Error| error(format!("failed to read '{}': {source}", path.0));
            let (Ok(offset), Ok(length)) = (u64::try_from(offset.0), usize::try_from(length.0)) else {
                return Err(error(format!("a read of '{}' asked for a negative range", path.0)));
            };
            let mut file = std::fs::File::open(&path.0).map_err(failed)?;
            file.seek(SeekFrom::Start(offset)).map_err(failed)?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes).map_err(failed)?;
            Ok(NewBuffer(bytes))
        },
    )?;
    let file_size = runtime.bind_function("jess.package.fileSize", |path: Text| {
        std::fs::metadata(&path.0)
            .ok()
            .filter(std::fs::Metadata::is_file)
            .map(|metadata| metadata.len())
    })?;
    let is_dir = runtime.bind_function("jess.package.isDir", |path: Text| Path::new(&path.0).is_dir())?;
    let list_dir = runtime.bind_function("jess.package.listDir", |call: &Call<'_>, root: Text| {
        let mut entries = children(Path::new(&root.0))?;
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        crate::lua::lua::push_list(call, entries.into_iter(), |frame, (name, kind)| {
            crate::lua::push_table(frame, 0, 2, |frame, row| {
                row.raw_set_value(frame, "name", &name)?;
                row.raw_set_value(frame, "kind", &kind)
            })
        })?;
        Ok::<_, l3i::Error>(l3i::bind::StackResults)
    })?;
    let replace = runtime.bind_function("jess.package.replace", |path: Text, data: Bytes| {
        crate::lua::files::replace_file(Path::new(&path.0), &data.0).map_err(error)
    })?;
    // jess reads TOML with one codec, the built-in package's, loaded the first time a manifest, lock
    // or workspace file is read.
    let decoder = std::rc::Rc::new(std::cell::RefCell::new(None));
    let decode_toml = runtime.bind_function("jess.package.decodeToml", move |call: &Call<'_>, text: Text| {
        let decode = crate::lua::builtin::decoder(call, &decoder)?;
        let document = decode
            .invoke_with(call, (text.0.as_str(),), |_, document| Value::store(document))
            .map_err(crate::lua::callback_error)?;
        document.push_to_scope(call)?;
        Ok::<_, l3i::Error>(l3i::bind::StackResults)
    })?;
    let builtin = runtime.bind_function("jess.package.builtin", |call: &Call<'_>| {
        crate::lua::lua::push_table(call, 0, crate::lua::builtin::FILES.len(), |frame, files| {
            for (path, source) in crate::lua::builtin::FILES {
                files.raw_set_value(frame, path, source)?;
            }
            Ok(())
        })?;
        Ok::<_, l3i::Error>(l3i::bind::StackResults)
    })?;
    let functions = [
        ("listTree", list_tree, Some((read, l3i::fs::READ_CAPABILITY))),
        ("listDir", list_dir, Some((read, l3i::fs::READ_CAPABILITY))),
        ("readFile", read_file, Some((read, l3i::fs::READ_CAPABILITY))),
        ("readAt", read_at, Some((read, l3i::fs::READ_CAPABILITY))),
        ("fileSize", file_size, Some((read, l3i::fs::READ_CAPABILITY))),
        ("isDir", is_dir, Some((read, l3i::fs::READ_CAPABILITY))),
        ("replace", replace, Some((write, l3i::fs::WRITE_CAPABILITY))),
        ("decodeToml", decode_toml, None),
        ("builtin", builtin, None),
    ];
    gated_table(
        runtime,
        functions.into_iter().chain(workspace_functions(runtime, read, write)?),
    )
}

/// A host function by name, and the capability it needs with whether the table's holder has it:
/// `None` for one that needs nothing.
type HostFunction = (&'static str, Function, Option<(bool, &'static str)>);

/// `functions` as a read-only table, each one whose holder lacks the capability it needs replaced
/// by one that raises the permission error for it.
fn gated_table(runtime: &Runtime, functions: impl Iterator<Item = HostFunction>) -> l3i::Result<Table> {
    let mut gated = Vec::new();
    for (name, function, needs) in functions {
        let function = match needs {
            Some((false, capability)) => {
                let message = crate::lua::plan::denied("@jess/package", capability);
                runtime.bind_function(&format!("jess.package.{name}"), move |_: ArgView<'_>| -> l3i::Result<()> {
                    Err(l3i::Error::permission(message.clone()))
                })?
            }
            _ => function,
        };
        gated.push((name, function));
    }
    let table = {
        let stack = runtime.stack();
        let table = Table::new(&stack, 0, gated.len())?;
        for (name, function) in &gated {
            table.set(&stack, name, function)?;
        }
        table
    };
    l3i::readonly::make_read_only(runtime, &table)?;
    Ok(table)
}

/// The functions workspaces find folders and keep packages with.
///
/// `canonical` is a path with every link and `..` resolved (nil when nothing is there); `parent`
/// the folder holding a path (nil at the top); `relative` a path below a base as `/`-joined names
/// (nil when it isn't below it); `createDir` makes a folder and those it is in; and `remove`
/// removes a file if it is there. `parent` and `relative` only work on paths, and need nothing.
fn workspace_functions(runtime: &Runtime, read: bool, write: bool) -> l3i::Result<[HostFunction; 5]> {
    let canonical = runtime.bind_function("jess.package.canonical", |path: Text| -> l3i::Result<Option<String>> {
        match std::fs::canonicalize(&path.0) {
            Ok(canonical) => plain(&canonical).map(Some),
            Err(source)
                if matches!(
                    source.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(None)
            }
            Err(source) => Err(error(format!("failed to resolve '{}': {source}", path.0))),
        }
    })?;
    let parent = runtime.bind_function("jess.package.parent", |path: Text| {
        Path::new(&path.0)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(plain)
            .transpose()
    })?;
    let relative = runtime.bind_function("jess.package.relative", |base: Text, path: Text| {
        let Ok(rest) = Path::new(&path.0).strip_prefix(&base.0) else {
            return Ok(None);
        };
        rest.components()
            .map(|component| plain(Path::new(component.as_os_str())))
            .collect::<l3i::Result<Vec<_>>>()
            .map(|names| Some(names.join("/")))
    })?;
    let create_dir = runtime.bind_function("jess.package.createDir", |path: Text| {
        std::fs::create_dir_all(&path.0).map_err(|source| error(format!("failed to create '{}': {source}", path.0)))
    })?;
    let remove = runtime.bind_function("jess.package.remove", |path: Text| match std::fs::remove_file(&path.0) {
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => {
            Err(error(format!("failed to remove '{}': {source}", path.0)))
        }
        _ => Ok(()),
    })?;
    Ok([
        ("canonical", canonical, Some((read, l3i::fs::READ_CAPABILITY))),
        ("parent", parent, None),
        ("relative", relative, None),
        ("createDir", create_dir, Some((write, l3i::fs::WRITE_CAPABILITY))),
        ("remove", remove, Some((write, l3i::fs::WRITE_CAPABILITY))),
    ])
}

/// `path` as text, without the `\\?\` Windows puts in front of a canonical path where the plain
/// form names the same place: Luau joins paths with `/`, which a `\\?\` path doesn't take.
fn plain(path: &Path) -> l3i::Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| error(format!("'{}' has a name that isn't UTF-8", path.display())))?;
    #[cfg(windows)]
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        if let Some(share) = rest.strip_prefix(r"UNC\") {
            return Ok(format!(r"\\{share}"));
        }
        if rest.as_bytes().get(1) == Some(&b':') {
            return Ok(rest.to_owned());
        }
    }
    Ok(text.to_owned())
}

/// Every entry under `folder`, as `/`-joined paths below the walk's root and what each is: `file`,
/// `dir`, `link` (never followed), or `other`.
fn walk(folder: &Path, prefix: &str, entries: &mut Vec<(String, &'static str)>) -> l3i::Result<()> {
    let failed = |source: std::io::Error| error(format!("failed to list '{}': {source}", folder.display()));
    for entry in std::fs::read_dir(folder).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| error(format!("'{}' has a name that isn't UTF-8", entry.path().display())))?;
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let kind = std::fs::symlink_metadata(entry.path()).map_err(failed)?.file_type();
        if kind.is_symlink() {
            entries.push((path, "link"));
        } else if kind.is_dir() {
            entries.push((path.clone(), "dir"));
            walk(&entry.path(), &path, entries)?;
        } else if kind.is_file() {
            entries.push((path, "file"));
        } else {
            entries.push((path, "other"));
        }
    }
    Ok(())
}

/// What is directly inside `folder`, by name, and what each is once any link is followed: `file`,
/// `dir`, or `other`.
fn children(folder: &Path) -> l3i::Result<Vec<(String, &'static str)>> {
    let failed = |source: std::io::Error| error(format!("failed to list '{}': {source}", folder.display()));
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(folder).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| error(format!("'{}' has a name that isn't UTF-8", entry.path().display())))?;
        // A link that points nowhere is neither a package folder nor a JPK.
        let kind = match std::fs::metadata(entry.path()) {
            Ok(metadata) if metadata.is_dir() => "dir",
            Ok(metadata) if metadata.is_file() => "file",
            _ => "other",
        };
        entries.push((name.to_owned(), kind));
    }
    Ok(entries)
}

/// Code jess runs because its layer was activated, and the capabilities it needs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Entrypoint {
    /// The module name, one the manifest's `[modules]` names.
    pub module: String,
    /// The capabilities it needs.
    pub capabilities: Vec<String>,
}

/// One layer as a manifest declares it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Layer {
    /// The layer's id.
    pub id: String,
    /// The layers it requires.
    pub requires: Vec<String>,
    /// The module names it offers as inert public API.
    pub exports: Vec<String>,
    /// What runs when it is activated.
    pub entrypoints: Vec<Entrypoint>,
}

/// A module name and the path it loads from.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Export {
    /// The module name.
    pub name: String,
    /// The package path.
    pub path: String,
}

/// A package dependency.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Dependency {
    /// The package id.
    pub id: String,
    /// The version requirement.
    pub requirement: String,
}

/// A capability an entrypoint requires.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LayerCapability {
    /// The capability.
    pub capability: String,
    /// The entrypoint's layer.
    pub layer: String,
    /// The entrypoint requiring it.
    pub entrypoint: String,
}

/// A managed implementation declared by a package, available for supervisors to load later.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Managed {
    /// Its package-local id.
    pub id: String,
    /// Its private source path in the package.
    pub module: String,
    /// The interface identity it provides, when it provides one.
    pub interface: Option<String>,
}

/// One runnable role declared by a package.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Target {
    /// Its package-local id.
    pub id: String,
    /// `application` or `command`.
    pub kind: String,
    /// Its private supervisor/entry source path in the package.
    pub entry: String,
    /// The maximum authority it requests.
    pub capabilities: Vec<String>,
    /// Its requested CLI alias, for command targets.
    pub command: Option<String>,
}

/// What `jess package inspect` reports: a package's manifest and identity, read without running it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Inspection {
    /// The package id.
    pub id: String,
    /// The package version.
    pub version: String,
    /// The content digest, the same from a Package Source and its JPK.
    pub digest: String,
    /// `source` or `jpk`.
    pub kind: String,
    /// Where it was opened from.
    pub location: String,
    /// The package API generation it expects.
    pub jess_api: String,
    /// The target a runner may select when none is named.
    pub default_target: Option<String>,
    /// How many files its payload holds, manifest included.
    pub files: usize,
    /// Its module exports.
    pub modules: Vec<Export>,
    /// Its layers.
    pub layers: Vec<Layer>,
    /// Its managed script definitions.
    pub managed: Vec<Managed>,
    /// Its runnable targets.
    pub targets: Vec<Target>,
    /// Its package dependencies.
    pub dependencies: Vec<Dependency>,
    /// The packages only its own tests need.
    pub dev_dependencies: Vec<Dependency>,
    /// Every capability its layers' entrypoints require.
    pub capabilities: Vec<LayerCapability>,
    /// Its source-only test modules.
    pub tests: Vec<String>,
}

/// What `jess pack` reports.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Packed {
    /// The JPK written.
    pub path: String,
    /// Its content digest.
    pub digest: String,
    /// How many files it holds.
    pub files: usize,
    /// Its size in bytes.
    pub size: u64,
}

/// A layer in a plan, in activation order.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanLayer {
    /// The layer id.
    pub id: String,
    /// The package providing it.
    pub package: String,
    /// That package's version.
    pub version: String,
    /// The layers it requires.
    pub requires: Vec<String>,
    /// The module names it offers.
    pub exports: Vec<String>,
    /// What runs when it is activated.
    pub entrypoints: Vec<Entrypoint>,
}

/// An entrypoint a plan runs, in order.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanEntrypoint {
    /// The module name.
    pub module: String,
    /// Its package.
    pub package: String,
    /// Its path in that package.
    pub path: String,
    /// The layer it belongs to.
    pub layer: String,
}

/// A package in a plan.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanPackage {
    /// The package id.
    pub id: String,
    /// Its version.
    pub version: String,
    /// Its content digest.
    pub digest: String,
    /// `source` or `jpk`.
    pub kind: String,
    /// Where it was opened from.
    pub location: String,
    /// The packages whose exports its modules may require: the dependencies it declares.
    pub dependencies: Vec<String>,
}

/// A module name in a plan's namespace.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanModule {
    /// The module name.
    pub name: String,
    /// The package exporting it.
    pub package: String,
    /// Its path in that package.
    pub path: String,
}

/// A capability a plan requires, and who requires it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanCapability {
    /// The capability.
    pub capability: String,
    /// The package requiring it.
    pub package: String,
    /// That package's version.
    pub version: String,
    /// The entrypoint's layer.
    pub layer: String,
    /// The entrypoint requiring it.
    pub entrypoint: String,
}

/// Where a resolution looks beyond the packages it is handed, and what it selects besides the
/// requested layers' providers.
#[derive(Clone, Debug, Default)]
pub struct Search {
    /// Package path folders. Every Package Source folder and `.jpk` directly inside one is a
    /// candidate, opened only when a layer or a dependency needs it, and never in place of a
    /// package handed over outright. Two candidates for one need are an error, not a choice.
    pub paths: Vec<String>,
    /// Packages offered as candidates by location, Package Source folders or `.jpk` files, as a
    /// package path's are: opened only when needed, never in place of a package handed over
    /// outright, and never chosen between.
    pub candidates: Vec<String>,
    /// Package ids selected, with their dependencies, whether or not a requested layer is theirs.
    pub roots: Vec<String>,
    /// Package ids whose `[dev-dependencies]` are selected with them: packages whose tests run.
    pub dev: Vec<String>,
    /// Content digests by package id: a package with one of these ids, handed over outright or a
    /// candidate, is refused unless it has that digest.
    pub pins: BTreeMap<String, String>,
}

impl Search {
    /// This search as the table the bootstrap reads.
    fn table(&self, scope: &impl Scope) -> l3i::Result<Table> {
        let table = Table::new(scope, 0, 5)?;
        for (key, items) in [
            ("paths", &self.paths),
            ("candidates", &self.candidates),
            ("roots", &self.roots),
            ("dev", &self.dev),
        ] {
            table.set(scope, key, &string_list(scope, items)?)?;
        }
        let pins = Table::new(scope, 0, self.pins.len())?;
        for (id, digest) in &self.pins {
            pins.set(scope, id.as_str(), digest.as_str())?;
        }
        table.set(scope, "pins", &pins)?;
        Ok(table)
    }
}

/// An activation plan: requested layers resolved over the packages offered, with nothing run.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Plan {
    /// The layers asked for, sorted.
    pub requested: Vec<String>,
    /// Every resolved layer, each after the layers it requires.
    pub layers: Vec<PlanLayer>,
    /// The packages selected: the layers' providers and their package dependencies.
    pub packages: Vec<PlanPackage>,
    /// The module namespace, sorted by name: every export of every selected package.
    pub modules: Vec<PlanModule>,
    /// The resolved layers' entrypoints, in the order activation runs them.
    pub entrypoints: Vec<PlanEntrypoint>,
    /// Every capability the entrypoints require.
    pub capabilities: Vec<PlanCapability>,
}

fn plan_shape() -> serde_json::Value {
    serde_json::json!({
        "requested": [""],
        "layers": [{
            "id": "", "package": "", "version": "", "requires": [""], "exports": [""],
            "entrypoints": [{ "module": "", "capabilities": [""] }],
        }],
        "packages": [{ "id": "", "version": "", "digest": "", "kind": "", "location": "", "dependencies": [""] }],
        "modules": [{ "name": "", "package": "", "path": "" }],
        "entrypoints": [{ "module": "", "package": "", "path": "", "layer": "" }],
        "capabilities": [{ "capability": "", "package": "", "version": "", "layer": "", "entrypoint": "" }],
    })
}

/// Runs one bootstrap entry with `args` and reads what it returns as `T`.
pub(crate) fn bootstrap<T: serde::de::DeserializeOwned + Serialize>(
    runtime: &Runtime,
    entry: &str,
    shape: &serde_json::Value,
    args: impl l3i::call::PushArgs,
) -> l3i::Result<T> {
    let jess = crate::lua::state(runtime)?;
    // The bootstrap is the host's own code, so it works over the host's `@jess/package` and
    // workspaces.
    let packages = crate::lua::stdlib::host_packages(runtime)?;
    let workspaces = crate::lua::stdlib::host_workspaces(runtime)?;
    let stack = runtime.stack();
    let chunk = jess.load(&stack, BOOTSTRAP_CHUNK, BOOTSTRAP.as_bytes())?;
    let entries: Table = chunk.invoke(&stack, (&packages, &workspaces))?;
    let function: Function = entries.get(&stack, entry)?;
    function
        .invoke_with(&stack, args, |frame, value| {
            convert::from_lua(frame, &jess, value, Some(shape))
        })
        .map_err(crate::lua::callback_error)
}

/// The module names `runtime` already resolves, which no package may export: whatever is
/// registered with its `require` (its plan's modules, and any its host added), and the `@` names
/// jess's own `require` hands out from its cache (`@jess/<member>`, the libraries' globals).
pub(crate) fn host_modules(runtime: &Runtime) -> l3i::Result<Vec<String>> {
    let mut names = runtime.registered_require_modules();
    let jess = crate::lua::state(runtime)?;
    let stack = runtime.stack();
    stack.with_frame(|frame| {
        jess.loaded.push_to(frame)?.for_each(frame, |_, key, _| {
            if key.is_string()
                && let Ok(name) = key.read::<&str>()
                && name.starts_with('@')
            {
                names.push(name.to_owned());
            }
            Ok(())
        })
    })?;
    names.push("@jess".to_owned());
    // The built-in package's exports stay open to the one package that may hold them, a copy of
    // the built-in package itself; `@jess/package` refuses any other.
    let manifest = crate::lua::builtin::manifest(&stack, &jess)?;
    names.retain(|name| manifest.export(name).is_none());
    names.sort();
    names.dedup();
    Ok(names)
}

/// Reads the package at `location`, a Package Source folder or a `.jpk`, without running it.
///
/// # Errors
///
/// Returns an error if it can't be read or its manifest or payload is invalid.
pub fn inspect(runtime: &Runtime, location: &str) -> l3i::Result<Inspection> {
    let shape = serde_json::json!({
        "id": "", "version": "", "digest": "", "kind": "", "location": "", "jessApi": "", "files": 0,
        "defaultTarget": null,
        "modules": [{ "name": "", "path": "" }],
        "layers": [{ "id": "", "requires": [""], "exports": [""], "entrypoints": [{ "module": "", "capabilities": [""] }] }],
        "managed": [{ "id": "", "module": "", "interface": null }],
        "targets": [{ "id": "", "kind": "", "entry": "", "capabilities": [""], "command": null }],
        "dependencies": [{ "id": "", "requirement": "" }],
        "devDependencies": [{ "id": "", "requirement": "" }],
        "capabilities": [{ "capability": "", "layer": "", "entrypoint": "" }],
        "tests": [""],
    });
    bootstrap(runtime, "inspect", &shape, (location,))
}

/// Packs the Package Source at `source` into the canonical JPK at `output`.
///
/// # Errors
///
/// Returns an error if the source is invalid or the JPK can't be written.
pub fn pack(runtime: &Runtime, source: &str, output: &str) -> l3i::Result<Packed> {
    let shape = serde_json::json!({ "path": "", "digest": "", "files": 0, "size": 0 });
    bootstrap(runtime, "pack", &shape, (source, output))
}

/// Resolves `layers` over the packages at `locations` into a plan, without running anything.
///
/// # Errors
///
/// Returns an error naming what failed to resolve.
pub fn graph(runtime: &Runtime, locations: &[String], layers: &[String]) -> l3i::Result<Plan> {
    graph_with(runtime, locations, layers, &Search::default())
}

/// [`graph`], resolving under `search`.
///
/// # Errors
///
/// As [`graph`], and if a package path can't be read.
pub fn graph_with(runtime: &Runtime, locations: &[String], layers: &[String], search: &Search) -> l3i::Result<Plan> {
    let reserved = host_modules(runtime)?;
    let stack = runtime.stack();
    let args = (
        string_list(&stack, locations)?,
        string_list(&stack, layers)?,
        string_list(&stack, &reserved)?,
        search.table(&stack)?,
    );
    drop(stack);
    bootstrap(runtime, "graph", &plan_shape(), args)
}

/// Resolves `layers` over the packages at `locations` and mounts the plan in `runtime`, so its
/// module names `require` from then on. Nothing a package holds runs here.
///
/// `grant` is the capabilities the host gives; a layer requiring one outside it fails the
/// activation before anything is mounted. `None` is the trusted, development policy: whatever
/// the plan requires is granted (and reported in the plan).
///
/// # Errors
///
/// Returns an error if the plan doesn't resolve, a capability is denied, or `runtime` already has
/// packages mounted.
pub fn activate(runtime: &Runtime, locations: &[String], layers: &[String], grant: Option<&[String]>) -> l3i::Result<Plan> {
    activate_with(runtime, locations, layers, grant, &Search::default())
}

/// [`activate`], resolving under `search`.
///
/// # Errors
///
/// As [`activate`], and if a package path can't be read.
pub fn activate_with(
    runtime: &Runtime,
    locations: &[String],
    layers: &[String],
    grant: Option<&[String]>,
    search: &Search,
) -> l3i::Result<Plan> {
    let state = crate::lua::state(runtime)?;
    if state.packages.borrow().is_some() {
        return Err(error("this runtime already has packages activated; one plan per runtime"));
    }
    let mount = runtime.bind_function(
        "jess.package.mount",
        move |call: &Call<'_>, modules: Table, entrypoints: Table, packages: Table| -> l3i::Result<()> {
            if state.packages.borrow().is_some() {
                return Err(error("this runtime already has packages activated; one plan per runtime"));
            }
            let mut mounted = Mounted::default();
            let mut dependencies = Vec::new();
            call.with_frame(|frame| {
                packages.push_to(frame)?.for_each_array(frame, |frame, _, row| {
                    let row = row.as_table()?;
                    let id = row.raw_get(frame, "id")?.read::<String>()?;
                    let version = row.raw_get(frame, "version")?.read::<String>()?;
                    let digest = row.raw_get(frame, "digest")?.read::<String>()?;
                    let read = Function::from_value(Value::store(row.raw_get(frame, "read")?)?)?;
                    let mut declared = Vec::new();
                    row.raw_get(frame, "dependencies")?
                        .as_table()?
                        .for_each_array(frame, |_, _, dependency| {
                            declared.push(dependency.read::<String>()?);
                            Ok(())
                        })?;
                    dependencies.push(declared);
                    mounted.packages.push(MountedPackage {
                        prefix: format!("@{id}@{version}/"),
                        id,
                        dependencies: Vec::new(),
                        digest,
                        read,
                    });
                    Ok(())
                })?;
                for (index, declared) in dependencies.iter().enumerate() {
                    // A dependency the plan didn't mount can't be required either way.
                    mounted.packages[index].dependencies = declared.iter().filter_map(|id| mounted.index_of(id)).collect();
                }
                for (list, key, exports) in [(&modules, "name", true), (&entrypoints, "module", false)] {
                    list.push_to(frame)?.for_each_array(frame, |frame, _, row| {
                        let row = row.as_table()?;
                        let name = row.raw_get(frame, key)?.read::<String>()?;
                        let package = row.raw_get(frame, "package")?.read::<String>()?;
                        let path = row.raw_get(frame, "path")?.read::<String>()?;
                        let owner = mounted
                            .packages
                            .iter()
                            .position(|candidate| candidate.id == package)
                            .ok_or_else(|| error(format!("'{name}' is {package}'s, which the plan didn't mount")))?;
                        if exports {
                            mounted.exports.insert(name, (owner, path));
                        } else {
                            mounted.entrypoints.push((owner, name, path));
                        }
                        Ok(())
                    })?;
                }
                Ok(())
            })?;
            state.forget_package_bytecode();
            *state.packages.borrow_mut() = Some(mounted);
            Ok(())
        },
    )?;
    let reserved = host_modules(runtime)?;
    let stack = runtime.stack();
    let grant = grant.map(|grant| string_list(&stack, grant)).transpose()?;
    let args = (
        mount,
        string_list(&stack, locations)?,
        string_list(&stack, layers)?,
        string_list(&stack, &reserved)?,
        grant,
        search.table(&stack)?,
    );
    drop(stack);
    let plan = bootstrap(runtime, "activate", &plan_shape(), args)?;
    run_entrypoints(runtime)?;
    Ok(plan)
}

/// Runs the mounted plan's entrypoints in order, each once, as `require` would load it: called
/// with its module name and cached by its package identity, so a module that is also exported is
/// one instance either way. An entrypoint's error fails the activation; what ran before it stays.
fn run_entrypoints(runtime: &Runtime) -> l3i::Result<()> {
    let jess = crate::lua::state(runtime)?;
    let mounted = jess.packages.borrow();
    let Some(mounted) = mounted.as_ref() else {
        return Ok(());
    };
    let stack = runtime.stack();
    for (index, name, path) in &mounted.entrypoints {
        let failed = |problem: l3i::Error| {
            error(format!(
                "entrypoint {name} of {} failed: {}",
                mounted.id(*index),
                crate::lua::error_message(&problem)
            ))
        };
        let module = mounted
            .load(&stack, *index, path)?
            .ok_or_else(|| error(format!("entrypoint {name}: {} doesn't hold '{path}'", mounted.id(*index))))?;
        stack.with_frame(|frame| {
            let loaded = jess.loaded.push_to(frame)?;
            if !loaded.raw_get(frame, &module.key)?.is_nil() {
                return Ok(());
            }
            let chunk = jess.load_as(frame, &module.key, &module.chunk, &module.source)?;
            let value = chunk
                .invoke_with(frame, (name.as_str(),), |_, value| Value::store(value))
                .map_err(failed)?;
            value.push_to(frame)?;
            loaded.raw_set(frame, &module.key)
        })?;
    }
    Ok(())
}

/// `items` as a Luau list of strings.
pub(crate) fn string_list(scope: &impl Scope, items: &[String]) -> l3i::Result<Table> {
    crate::lua::lua::stored_table(scope, |frame| {
        let table = frame.push_table(items.len(), 0)?;
        for (index, item) in items.iter().enumerate() {
            frame.push_string(item)?;
            table.raw_set_index(frame, index as i64 + 1)?;
        }
        Ok(())
    })
}

/// Activates every layer the packages at `locations` declare: what `jess run --package` does
/// without `--layer`.
///
/// # Errors
///
/// As [`activate`], and if a package can't be read.
pub fn activate_all(runtime: &Runtime, locations: &[String], grant: Option<&[String]>) -> l3i::Result<Plan> {
    let mut layers = Vec::new();
    for location in locations {
        layers.extend(inspect(runtime, location)?.layers.into_iter().map(|layer| layer.id));
    }
    activate(runtime, locations, &layers, grant)
}

/// One provider's run of a package's tests.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderRun {
    /// `source` or `jpk`.
    pub kind: String,
    /// Where the package was opened from.
    pub location: String,
    /// Its content digest, as that provider computed it.
    pub digest: String,
    /// The test modules' reports.
    pub run: crate::lua::testing::TestRun,
}

/// What `jess package test` reports: the package's tests from its Package Source, then the same
/// tests against that source packed into a JPK.
#[derive(Clone, Debug, Serialize)]
pub struct PackageTest {
    /// The package id.
    pub id: String,
    /// The package version.
    pub version: String,
    /// The test modules the manifest names, as given there.
    pub tests: Vec<String>,
    /// Each provider's run, source first; none for a JPK, whose tests run from its source.
    pub providers: Vec<ProviderRun>,
    /// Cases that passed, over every provider.
    pub passed: usize,
    /// Cases that failed, over every provider, plus modules that didn't run, plus one when the
    /// providers' digests differ.
    pub failed: usize,
}

/// Runs the tests the manifest of the Package Source at `location` names, against the source and
/// then against it packed, with no dependencies offered.
///
/// # Errors
///
/// As [`test_with`].
pub fn test(options: &crate::lua::Options, location: &Path, scratch: &Path) -> l3i::Result<PackageTest> {
    test_with(options, location, scratch, &[], &Search::default())
}

/// Runs the tests the manifest of the Package Source at `location` names, against the source and
/// then against it packed.
///
/// The JPK is written to `scratch` and removed afterwards. Each run activates every layer the
/// package declares, under the trusted policy, in a fresh runtime made with `options`, and selects
/// the package with its `[dependencies]` and `[dev-dependencies]`, which resolve over `packages`
/// (handed over outright) and the package paths and candidates of `search`; whatever its roots
/// and dev say, the package is the root, its dev-dependencies selected with it. The tests are the
/// host's scripts, so they hold the whole plan's namespace. A JPK's tests run from its source, so for one
/// this reports none.
///
/// # Errors
///
/// Returns an error if the package can't be read, packed, or activated.
pub fn test_with(
    options: &crate::lua::Options,
    location: &Path,
    scratch: &Path,
    packages: &[String],
    search: &Search,
) -> l3i::Result<PackageTest> {
    // Package tests are an explicit host action over a caller-chosen package, so their test code
    // gets scratch filesystem access and terminal/environment reporting. Process spawning remains
    // denied unless a host separately approves it.
    let test_capabilities = crate::lua::host_script_grant(&[
        l3i::fs::READ_CAPABILITY.to_owned(),
        l3i::fs::WRITE_CAPABILITY.to_owned(),
        l3i::process::ENVIRONMENT_CAPABILITY.to_owned(),
    ]);
    let runtime = crate::lua::runtime_with_capabilities(options.clone(), &test_capabilities)?;
    let display = location.display().to_string();
    let inspection = inspect(&runtime, &display)?;
    let mut report = PackageTest {
        id: inspection.id.clone(),
        version: inspection.version.clone(),
        tests: inspection.tests.clone(),
        providers: Vec::new(),
        passed: 0,
        failed: 0,
    };
    if inspection.kind != "source" {
        return Ok(report);
    }
    let modules = inspection
        .tests
        .iter()
        .map(|test| location.join(test))
        .collect::<Vec<PathBuf>>();
    let layers = inspection.layers.iter().map(|layer| layer.id.clone()).collect::<Vec<_>>();
    let search = Search {
        roots: vec![inspection.id.clone()],
        dev: vec![inspection.id.clone()],
        ..search.clone()
    };
    std::fs::create_dir_all(scratch)
        .map_err(|source| error(format!("failed to create '{}': {source}", scratch.display())))?;
    let packed = scratch.join(format!("{}-{}.jpk", inspection.id, inspection.version));
    let packed_display = packed.display().to_string();
    let result = (|| -> l3i::Result<()> {
        pack(&runtime, &display, &packed_display)?;
        for (index, provider) in [&display, &packed_display].into_iter().enumerate() {
            let runtime = crate::lua::runtime_with_capabilities(options.clone(), &test_capabilities)?;
            let mut locations = vec![provider.clone()];
            locations.extend_from_slice(packages);
            let plan = activate_with(&runtime, &locations, &layers, None, &search)?;
            let package = plan
                .packages
                .into_iter()
                .find(|package| package.id == inspection.id)
                .ok_or_else(|| error(format!("{} activated without itself", inspection.id)))?;
            let provider_scratch = scratch.join(if index == 0 { "source" } else { "jpk" });
            std::fs::create_dir_all(&provider_scratch)
                .map_err(|source| error(format!("failed to create '{}': {source}", provider_scratch.display())))?;
            let run = crate::lua::testing::run_in_with_scratch(&runtime, &modules, &provider_scratch)?;
            report.passed += run.passed;
            report.failed += run.failed;
            report.providers.push(ProviderRun {
                kind: package.kind,
                location: package.location,
                digest: package.digest,
                run,
            });
        }
        Ok(())
    })();
    drop(std::fs::remove_file(&packed));
    result?;
    if report.providers.windows(2).any(|pair| pair[0].digest != pair[1].digest) {
        report.failed += 1;
    }
    Ok(report)
}

/// A request to run one of a package's command targets.
#[derive(Clone, Debug, Default)]
pub struct TargetRequest {
    /// The Package Source folder or `.jpk` whose target runs.
    pub package: String,
    /// The target's id; `None` is the package's `default_target`.
    pub target: Option<String>,
    /// Packages its dependencies resolve over, handed over outright.
    pub packages: Vec<String>,
    /// Package path folders its dependencies resolve over.
    pub paths: Vec<String>,
    /// Packages its dependencies resolve over, offered as candidates: opened only when needed.
    pub candidates: Vec<String>,
    /// The content digest each package with an id here must have.
    pub pins: BTreeMap<String, String>,
    /// The capabilities the host grants. `None` is the trusted policy: what the target declares is
    /// granted. Either way the target runs with what it declares and nothing more.
    pub grant: Option<Vec<String>>,
    /// The target's arguments, which its entry gets as `...` and in the global `arg` table.
    pub args: Vec<String>,
}

/// A target that ran: its runtime, which owns whatever it returned, and those values.
pub struct TargetRun {
    /// The runtime the target ran in.
    pub runtime: Runtime,
    /// What the target's entry returned.
    pub values: Vec<Value>,
}

impl std::fmt::Debug for TargetRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TargetRun")
            .field("values", &self.values.len())
            .finish_non_exhaustive()
    }
}

/// The target of `inspection` that `name` picks, or its default target without one.
///
/// # Errors
///
/// Returns an error naming the targets the package does have.
pub fn target<'i>(inspection: &'i Inspection, name: Option<&str>) -> l3i::Result<&'i Target> {
    let package = &inspection.id;
    let has = || {
        let ids = inspection.targets.iter().map(|target| target.id.as_str()).collect::<Vec<_>>();
        if ids.is_empty() {
            "it declares no targets".to_owned()
        } else {
            format!("it has: {}", ids.join(", "))
        }
    };
    let Some(name) = name.or(inspection.default_target.as_deref()) else {
        return Err(error(format!("{package} has no default_target to run; {}", has())));
    };
    inspection
        .targets
        .iter()
        .find(|target| target.id == name)
        .ok_or_else(|| error(format!("{package} has no target '{name}'; {}", has())))
}

/// Runs a command target of the package `request` names.
///
/// The package resolves with its dependencies, every layer it declares is activated, and the
/// target's entry runs as a script, in a fresh runtime holding exactly the capabilities the target
/// declares. A capability the target declares and `request.grant` withholds fails the run before
/// anything is activated.
///
/// # Errors
///
/// Returns an error if the package or target can't be read, a capability is denied, the plan
/// doesn't resolve, or the entry fails.
pub fn run_target(options: &crate::lua::Options, request: &TargetRequest) -> l3i::Result<TargetRun> {
    let inspection = inspect(&crate::lua::runtime(options.clone())?, &request.package)?;
    let target = target(&inspection, request.target.as_deref())?;
    let (package, version, id) = (&inspection.id, &inspection.version, &target.id);
    if target.kind != "command" {
        return Err(error(format!(
            "target '{id}' of {package} is {} target, which needs a host of its own; only command targets run here",
            if target.kind == "application" {
                "an application"
            } else {
                "not a command"
            }
        )));
    }
    if let Some(grant) = &request.grant {
        let denied = target
            .capabilities
            .iter()
            .filter(|capability| !grant.contains(capability))
            .map(|capability| format!("    {capability}"))
            .collect::<Vec<_>>();
        if !denied.is_empty() {
            return Err(error(format!(
                "cannot run target {id}\n\nrequired capability denied:\n{}\n\nrequested by:\n    {package} {version}\n    target {id}",
                denied.join("\n")
            )));
        }
    }
    let runtime = crate::lua::runtime_with_capabilities(options.clone(), &target.capabilities)?;
    let layers = inspection.layers.iter().map(|layer| layer.id.clone()).collect::<Vec<_>>();
    let mut locations = vec![request.package.clone()];
    locations.extend_from_slice(&request.packages);
    let search = Search {
        paths: request.paths.clone(),
        candidates: request.candidates.clone(),
        pins: request.pins.clone(),
        roots: vec![package.clone()],
        dev: Vec::new(),
    };
    // The runtime holds what the target declares, so its entrypoints are admitted against exactly
    // that, whatever the request granted beyond it.
    activate_with(&runtime, &locations, &layers, Some(&target.capabilities), &search)?;
    let jess = crate::lua::state(&runtime)?;
    let (chunk, function) = {
        let mounted = jess.packages.borrow();
        let mounted = mounted
            .as_ref()
            .ok_or_else(|| error(format!("{package} activated without a plan")))?;
        let index = mounted
            .index_of(package)
            .ok_or_else(|| error(format!("{package} activated without itself")))?;
        let stack = runtime.stack();
        let module = mounted
            .load(&stack, index, &target.entry)?
            .ok_or_else(|| error(format!("target {id}: {package} doesn't hold '{}'", target.entry)))?;
        let function = jess.load_as(&stack, &module.key, &module.chunk, &module.source)?;
        (module.chunk, function)
    };
    let values = crate::lua::scripts::run_loaded(&runtime, chunk.trim_start_matches('@'), &function, &request.args)?;
    Ok(TargetRun { runtime, values })
}
