//! Packages at runtime: the host side of `@jess/package` and the package bootstrap.
//!
//! The package format, manifests, digests, and resolution are Luau (`stdlib/jpk.luau`,
//! `stdlib/package.luau`). Rust supplies the file reads they run over, runs the
//! bootstrap (`stdlib/bootstrap.luau`) for [`inspect`], [`pack`], [`graph`], and [`activate`], and
//! owns the one seam package code needs: the mount the bootstrap hands an accepted plan to, which
//! `require` resolves package modules through.
//!
//! A package module's chunk name is `@<id>@<version>/<path>`, which diagnostics show; its cache
//! identity is its package's id and content digest and its path, so two generations of one package
//! version never share a cached module.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use l3i::Runtime;
use l3i::bind::Call;
use l3i::convert::{Exact, NewBuffer};
use l3i::stack::Scope;
use l3i::value::{Function, Table, Value};
use serde::{Deserialize, Serialize};

use crate::lua::convert;
use crate::lua::{Bytes, Text, error};

const BOOTSTRAP: &str = include_str!("stdlib/bootstrap.luau");
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
}

/// The host functions `@jess/package` reads and writes through: the only file access it has.
pub(crate) fn host_table(runtime: &Runtime) -> l3i::Result<Table> {
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
    let decode_toml = runtime.bind_function("jess.package.decodeToml", |call: &Call<'_>, text: Text| {
        crate::lua::toml::decode(call, &text.0).map(|(value, _)| value)
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
    let table = {
        let stack = runtime.stack();
        let table = Table::new(&stack, 0, 9)?;
        for (name, function) in [
            ("listTree", list_tree),
            ("listDir", list_dir),
            ("readFile", read_file),
            ("readAt", read_at),
            ("fileSize", file_size),
            ("isDir", is_dir),
            ("replace", replace),
            ("decodeToml", decode_toml),
            ("builtin", builtin),
        ] {
            table.set(&stack, name, &function)?;
        }
        table
    };
    l3i::readonly::make_read_only(runtime, &table)?;
    Ok(table)
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
    /// Package ids selected, with their dependencies, whether or not a requested layer is theirs.
    pub roots: Vec<String>,
    /// Package ids whose `[dev-dependencies]` are selected with them: packages whose tests run.
    pub dev: Vec<String>,
}

impl Search {
    /// This search as the table the bootstrap reads.
    fn table(&self, scope: &impl Scope) -> l3i::Result<Table> {
        let table = Table::new(scope, 0, 3)?;
        for (key, items) in [("paths", &self.paths), ("roots", &self.roots), ("dev", &self.dev)] {
            table.set(scope, key, &string_list(scope, items)?)?;
        }
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
fn bootstrap<T: serde::de::DeserializeOwned + Serialize>(
    runtime: &Runtime,
    entry: &str,
    shape: &serde_json::Value,
    args: impl l3i::call::PushArgs,
) -> l3i::Result<T> {
    let jess = crate::lua::state(runtime)?;
    let stack = runtime.stack();
    let chunk = jess.load(&stack, BOOTSTRAP_CHUNK, BOOTSTRAP.as_bytes())?;
    let entries: Table = chunk.invoke(&stack, ())?;
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
    names.retain(|name| crate::lua::builtin::export(name).is_none());
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
            call.with_frame(|frame| {
                packages.push_to(frame)?.for_each_array(frame, |frame, _, row| {
                    let row = row.as_table()?;
                    let id = row.raw_get(frame, "id")?.read::<String>()?;
                    let version = row.raw_get(frame, "version")?.read::<String>()?;
                    let digest = row.raw_get(frame, "digest")?.read::<String>()?;
                    let read = Function::from_value(Value::store(row.raw_get(frame, "read")?)?)?;
                    mounted.packages.push(MountedPackage {
                        prefix: format!("@{id}@{version}/"),
                        id,
                        digest,
                        read,
                    });
                    Ok(())
                })?;
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
fn string_list(scope: &impl Scope, items: &[String]) -> l3i::Result<Table> {
    crate::lua::lua::stored_table(scope, |frame| {
        let table = frame.push_table(items.len(), 0)?;
        for (index, item) in items.iter().enumerate() {
            frame.push_string(item);
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
    /// Each provider's run, source first; none for a JPK, which carries no tests.
    pub providers: Vec<ProviderRun>,
    /// Cases that passed, over every provider.
    pub passed: usize,
    /// Cases that failed, over every provider, plus modules that didn't run, plus one when the
    /// providers' digests differ.
    pub failed: usize,
}

/// Runs the tests the manifest of the Package Source at `location` names, against the source and
/// then against it packed.
///
/// The JPK is written to `scratch` and removed afterwards. Each run activates every layer the
/// package declares, under the trusted policy, in a fresh runtime made with `options`. A JPK
/// carries no tests, so for one this reports none.
///
/// # Errors
///
/// Returns an error if the package can't be read, packed, or activated.
pub fn test(options: &crate::lua::Options, location: &Path, scratch: &Path) -> l3i::Result<PackageTest> {
    // Package tests are an explicit host action over a caller-chosen package, so their test code
    // gets scratch filesystem access and terminal/environment reporting. Process spawning remains
    // denied unless a host separately approves it.
    let test_capabilities = [
        l3i::fs::READ_CAPABILITY.to_owned(),
        l3i::fs::WRITE_CAPABILITY.to_owned(),
        l3i::process::ENVIRONMENT_CAPABILITY.to_owned(),
    ];
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
    std::fs::create_dir_all(scratch)
        .map_err(|source| error(format!("failed to create '{}': {source}", scratch.display())))?;
    let packed = scratch.join(format!("{}-{}.jpk", inspection.id, inspection.version));
    let packed_display = packed.display().to_string();
    let result = (|| -> l3i::Result<()> {
        pack(&runtime, &display, &packed_display)?;
        for (index, provider) in [&display, &packed_display].into_iter().enumerate() {
            let runtime = crate::lua::runtime_with_capabilities(options.clone(), &test_capabilities)?;
            let plan = activate_all(&runtime, std::slice::from_ref(provider), None)?;
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
