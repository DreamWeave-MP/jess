//! `jess package check` and `jess check`: a Package Source's Luau, or loose scripts, type-checked
//! in strict mode.
//!
//! A package is checked the way an editor in a folder `jess init` set up checks, with every `require`
//! resolved the way the runtime would resolve it under the package's plan. The plan is resolved
//! and mounted in a runtime of the check's own, and nothing a package holds runs. The package's own
//! files are read from its folder, every `.jsl` under it, tests and benches included; a
//! dependency's modules come from the mounted providers, a Package Source's or a JPK's alike; the
//! built-in package's exports from the copy jess embeds; jess's own `@jess/` and `@dream/` modules
//! are the types folder's stubs, as `jess luau-types` writes them. Only the package's own files are
//! reported: a dependency's errors are its own.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use l3i::Runtime;
use l3i::analysis::{Analysis, AnalysisOptions, Definitions, Mode, ModuleConfig, SourceCode, SourceProvider};
use serde::Serialize;

use crate::lua::package::{self, Search};

/// One type error in the package's own Luau.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CheckError {
    /// The file, relative to the package's folder, with `/` separators.
    pub file: String,
    /// The 1-based line.
    pub line: u32,
    /// The 1-based column.
    pub column: u32,
    /// What the checker says.
    pub message: String,
}

/// What [`check_with`] found.
#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    /// The package id.
    pub id: String,
    /// Its version.
    pub version: String,
    /// How many of its files were checked.
    pub files: usize,
    /// Every type error in them, sorted by file and position.
    pub errors: Vec<CheckError>,
}

/// The prefix a module of the types folder goes by.
const TYPES: &str = "types/";

/// Where the checker's modules come from. A module is named `<package id>/<path in it>`, or
/// `types/<path>` for the types folder.
struct Sources {
    runtime: Runtime,
    root: String,
    folder: PathBuf,
    types: crate::lua::types::Types,
    read: RefCell<HashMap<String, Option<String>>>,
}

impl Sources {
    /// The package id and the path within it of the module `name`.
    fn split(name: &str) -> Option<(&str, &str)> {
        // Package ids hold dots, never slashes, so the first slash ends the id.
        name.split_once('/')
    }

    /// The text of the package `id`'s file at `path`: the root package's from its folder, the
    /// built-in package's from the copy jess embeds, any other's from the provider the plan
    /// mounted.
    fn file(&self, id: &str, path: &str) -> Option<String> {
        if id == self.root {
            return std::fs::read_to_string(self.folder.join(path)).ok();
        }
        package_file(&self.runtime, id, path)
    }

    /// [`Self::file`], read once.
    fn cached(&self, id: &str, path: &str) -> Option<String> {
        let name = format!("{id}/{path}");
        if let Some(text) = self.read.borrow().get(&name) {
            return text.clone();
        }
        let text = self.file(id, path);
        self.read.borrow_mut().insert(name, text.clone());
        text
    }

    /// The module an export name stands for: the plan's namespace has it, or the built-in package
    /// exports it in every runtime.
    fn export(&self, required: &str) -> Option<String> {
        export_module(&self.runtime, required)
    }
}

/// The module `required`, an export name, stands for in `runtime`: `<package id>/<path>` from the
/// plan's namespace, or from the built-in package, which exports in every runtime.
fn export_module(runtime: &Runtime, required: &str) -> Option<String> {
    let jess = crate::lua::state(runtime).ok()?;
    if let Some(mounted) = jess.packages.borrow().as_ref()
        && let Some((owner, path)) = mounted.export(required)
    {
        return Some(format!("{}/{path}", mounted.id(owner)));
    }
    let builtin = crate::lua::builtin::manifest(&runtime.stack(), &jess).ok()?;
    builtin
        .export(required)
        .map(|path| format!("{}/{path}", crate::lua::builtin::ID))
}

/// The text of package `id`'s file at `path`, from the built-in package's embedded copy or the
/// provider the plan mounted.
fn package_file(runtime: &Runtime, id: &str, path: &str) -> Option<String> {
    if id == crate::lua::builtin::ID {
        return crate::lua::builtin::file(path).map(str::to_owned);
    }
    let jess = crate::lua::state(runtime).ok()?;
    let mounted = jess.packages.borrow();
    let mounted = mounted.as_ref()?;
    let index = mounted.index_of(id)?;
    let module = mounted.load(&runtime.stack(), index, path).ok()??;
    String::from_utf8(module.source).ok()
}

impl SourceProvider for Sources {
    fn read_source(&self, name: &str) -> Option<SourceCode> {
        let text = if let Some(path) = name.strip_prefix(TYPES) {
            self.types.files.iter().find(|file| file.path == path)?.source.clone()
        } else {
            let (id, path) = Self::split(name)?;
            crate::lua::scripts::strip_shebang(&self.cached(id, path)?).to_owned()
        };
        Some(SourceCode { text, is_script: false })
    }

    fn resolve_module(&self, from: &str, required: &str) -> Option<String> {
        if required.starts_with('@') && !required.starts_with("@self/") {
            if let Some(module) = self.export(required) {
                return Some(module);
            }
            return self.types.module_file(required).map(|file| format!("{TYPES}{}", file.path));
        }
        let (id, path) = Self::split(from)?;
        let candidates = crate::lua::require::relative_candidates(required, from, path, id).ok()?;
        candidates
            .into_iter()
            .find(|candidate| self.cached(id, candidate).is_some())
            .map(|candidate| format!("{id}/{candidate}"))
    }

    /// Strict, as the `.jessrc` `jess init` writes sets every module.
    fn module_config(&self, _: &str) -> ModuleConfig {
        ModuleConfig {
            mode: Mode::Strict,
            ..ModuleConfig::default()
        }
    }
}

/// Every `.jsl` file under `folder`, relative to it with `/` separators, sorted. A folder whose
/// name starts with `.`, `target`, and the types folder `jess init` writes (a `types` holding
/// `jess.d.jsl`) are skipped: they hold a tool's state and the editor's types, not the package.
fn sources(folder: &Path) -> std::io::Result<Vec<String>> {
    fn is_types_folder(name: &str, path: &Path) -> bool {
        name == crate::lua::init::TYPES_FOLDER && path.join(crate::lua::types::DEFINITIONS).is_file()
    }
    fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_dir() {
                if !name.starts_with('.') && name != "target" && !is_types_folder(&name, &path) {
                    walk(root, &path, found)?;
                }
            } else if path.extension().is_some_and(|extension| extension == "jsl")
                && let Ok(relative) = path.strip_prefix(root)
            {
                let parts = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>();
                found.push(parts.join("/"));
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(folder, folder, &mut found)?;
    found.sort();
    Ok(found)
}

/// Type-checks the Package Source at `location` in strict mode, and runs nothing.
///
/// Its `[dependencies]` and `[dev-dependencies]` resolve over `packages` (handed over outright)
/// and the package paths and candidates of `search`, as `jess package test` resolves them.
///
/// # Errors
///
/// Returns an error if `location` isn't a Package Source, the plan doesn't resolve, its folder
/// can't be read, or the checker can't load jess's types.
pub fn check_with(
    options: &crate::lua::Options,
    location: &Path,
    packages: &[String],
    search: &Search,
) -> l3i::Result<CheckReport> {
    let error = |message: String| l3i::Error::runtime(message);
    let runtime = crate::lua::runtime(options.clone())?;
    let display = location.display().to_string();
    let inspection = package::inspect(&runtime, &display)?;
    if inspection.kind != "source" {
        return Err(error(format!(
            "{display}: a check reads a Package Source; this is a {}",
            inspection.kind
        )));
    }
    let layers = inspection.layers.iter().map(|layer| layer.id.clone()).collect::<Vec<_>>();
    let mut locations = vec![display.clone()];
    locations.extend_from_slice(packages);
    let search = Search {
        roots: vec![inspection.id.clone()],
        dev: vec![inspection.id.clone()],
        ..search.clone()
    };
    package::mount_with(&runtime, &locations, &layers, None, &search)?;
    let files = sources(location).map_err(|source| error(format!("failed to read '{display}': {source}")))?;
    let types = crate::lua::types::types()?;
    let definitions = types.definitions().to_owned();
    let root = inspection.id.clone();
    let analysis = Analysis::new(
        Sources {
            runtime,
            root: root.clone(),
            folder: location.to_path_buf(),
            types,
            read: RefCell::new(HashMap::new()),
        },
        AnalysisOptions {
            definitions: vec![Definitions {
                name: "@jess".to_owned(),
                source: definitions,
            }],
            ..AnalysisOptions::default()
        },
    )?;
    let prefix = format!("{root}/");
    let mut errors = BTreeSet::new();
    for file in &files {
        for diagnostic in analysis.check(&format!("{prefix}{file}"), false).errors() {
            let Some(file) = diagnostic.module.strip_prefix(&prefix) else {
                continue;
            };
            errors.insert(CheckError {
                file: file.to_owned(),
                line: diagnostic.span.begin_line + 1,
                column: diagnostic.span.begin_column + 1,
                message: diagnostic.text.clone(),
            });
        }
    }
    Ok(CheckReport {
        id: inspection.id,
        version: inspection.version,
        files: files.len(),
        errors: errors.into_iter().collect(),
    })
}

/// What [`check_scripts`] found.
#[derive(Clone, Debug, Serialize)]
pub struct ScriptCheckReport {
    /// How many scripts were given.
    pub files: usize,
    /// Every type error in them and in the files their relative requires reached, sorted by file
    /// and position. `file` is the script's path as given, or the required file's as the
    /// `require` reached it.
    pub errors: Vec<CheckError>,
}

/// Where a loose script's modules come from: the scripts themselves and the files their relative
/// requires reach, by path; the types folder; and the packages mounted for the check.
struct ScriptSources {
    runtime: Rc<Runtime>,
    types: crate::lua::types::Types,
    /// The scripts given, by the path they were given as, `/`-separated.
    scripts: BTreeSet<String>,
    /// The files relative requires reached, by the path the `require` resolved to; shared with
    /// the check, which reports errors in them.
    files: Rc<RefCell<BTreeSet<String>>>,
    read: RefCell<HashMap<String, Option<String>>>,
}

impl ScriptSources {
    /// Whether `name` is a script or a file a script's `require` reached.
    fn is_file(&self, name: &str) -> bool {
        self.scripts.contains(name) || self.files.borrow().contains(name)
    }

    /// The file the relative `required` names from the file `from`, if it is there, by jess's
    /// rule: `./` starts beside the file, or beside its folder for an `init.jsl`; `@self/` starts
    /// inside an init file's folder; `.jsl` is tried, then `/init.jsl`.
    fn relative(&self, from: &str, required: &str) -> Option<String> {
        let from = Path::new(from);
        let init = from.file_stem().is_some_and(|stem| stem.eq_ignore_ascii_case("init"));
        let folder = from.parent().map_or_else(PathBuf::new, Path::to_path_buf);
        let (relative, mut base) = match required.strip_prefix("@self/") {
            Some(_) if !init => return None,
            Some(rest) => (rest, folder),
            None if init => (required, folder.parent().map_or_else(PathBuf::new, Path::to_path_buf)),
            None => (required, folder),
        };
        for part in relative.split('/') {
            match part {
                "." | "" => {}
                ".." => {
                    if !base.pop() {
                        base.push("..");
                    }
                }
                part => base.push(part),
            }
        }
        let base = base.to_string_lossy().replace('\\', "/");
        let found = [format!("{base}.jsl"), format!("{base}/init.jsl")]
            .into_iter()
            .find(|candidate| Path::new(candidate).is_file())?;
        self.files.borrow_mut().insert(found.clone());
        Some(found)
    }
}

impl SourceProvider for ScriptSources {
    fn read_source(&self, name: &str) -> Option<SourceCode> {
        if let Some(path) = name.strip_prefix(TYPES) {
            let text = self.types.files.iter().find(|file| file.path == path)?.source.clone();
            return Some(SourceCode { text, is_script: false });
        }
        if self.is_file(name) {
            let text = std::fs::read_to_string(name).ok()?;
            return Some(SourceCode {
                text: crate::lua::scripts::strip_shebang(&text).to_owned(),
                is_script: self.scripts.contains(name),
            });
        }
        let (id, path) = name.split_once('/')?;
        if let Some(text) = self.read.borrow().get(name) {
            return text.clone().map(|text| SourceCode { text, is_script: false });
        }
        let text = package_file(&self.runtime, id, path).map(|text| crate::lua::scripts::strip_shebang(&text).to_owned());
        self.read.borrow_mut().insert(name.to_owned(), text.clone());
        text.map(|text| SourceCode { text, is_script: false })
    }

    fn resolve_module(&self, from: &str, required: &str) -> Option<String> {
        if required.starts_with('@') && !required.starts_with("@self/") {
            if let Some(module) = export_module(&self.runtime, required) {
                return Some(module);
            }
            return self.types.module_file(required).map(|file| format!("{TYPES}{}", file.path));
        }
        if self.is_file(from) {
            return self.relative(from, required);
        }
        // Inside a package's module, a relative name resolves inside its payload.
        let (id, path) = from.split_once('/')?;
        let candidates = crate::lua::require::relative_candidates(required, from, path, id).ok()?;
        candidates
            .into_iter()
            .find(|candidate| package_file(&self.runtime, id, candidate).is_some())
            .map(|candidate| format!("{id}/{candidate}"))
    }

    /// Strict, as the `.jessrc` `jess init` writes sets every module; a file's own directive
    /// still wins.
    fn module_config(&self, _: &str) -> ModuleConfig {
        ModuleConfig {
            mode: Mode::Strict,
            ..ModuleConfig::default()
        }
    }
}

/// Type-checks loose scripts in strict mode, as `jess check` does, and runs nothing.
///
/// Each script is checked as an editor in a folder `jess init` set up would check it: `@jess/`
/// and `@dream/` names through the types folder, `./` and `../` names beside the script, and the
/// exports of `packages`, whose `layers` (every layer they declare, without any) are resolved over
/// `search` and mounted the way `jess run --package` would activate them. Only the scripts and
/// the files their relative requires reach are reported.
///
/// # Errors
///
/// Returns an error if a script isn't a `.jsl` file, the packages don't resolve, or the checker
/// can't load jess's types.
pub fn check_scripts(
    options: &crate::lua::Options,
    scripts: &[PathBuf],
    packages: &[String],
    layers: &[String],
    search: &Search,
) -> l3i::Result<ScriptCheckReport> {
    let error = |message: String| l3i::Error::runtime(message);
    for script in scripts {
        if !script.is_file() {
            return Err(error(format!("'{}' isn't a file", script.display())));
        }
        crate::lua::scripts::require_jsl(script)?;
    }
    let runtime = crate::lua::runtime(options.clone())?;
    if packages.is_empty() && search.paths.is_empty() {
        if !layers.is_empty() {
            return Err(error(
                "--layer needs packages to resolve over: name them with --package or --package-path".to_owned(),
            ));
        }
    } else {
        let mut layers = layers.to_vec();
        let mut roots = Vec::new();
        if layers.is_empty() {
            for location in packages {
                let inspection = package::inspect(&runtime, location)?;
                layers.extend(inspection.layers.into_iter().map(|layer| layer.id));
                roots.push(inspection.id);
            }
        }
        if layers.is_empty() && roots.is_empty() {
            return Err(error(
                "a package path offers packages without choosing any: name a layer with --layer".to_owned(),
            ));
        }
        let search = Search { roots, ..search.clone() };
        package::mount_with(&runtime, packages, &layers, None, &search)?;
    }
    let types = crate::lua::types::types()?;
    let definitions = types.definitions().to_owned();
    let names = scripts
        .iter()
        .map(|script| script.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();
    let runtime = Rc::new(runtime);
    let mut errors = BTreeSet::new();
    for name in &names {
        // A file explicitly checked as a script may also be required by another script. Give
        // each entry its own analysis so a cached script cannot masquerade as that module.
        let files = Rc::new(RefCell::new(BTreeSet::new()));
        let analysis = Analysis::new(
            ScriptSources {
                runtime: Rc::clone(&runtime),
                types: types.clone(),
                scripts: BTreeSet::from([name.clone()]),
                files: Rc::clone(&files),
                read: RefCell::new(HashMap::new()),
            },
            AnalysisOptions {
                definitions: vec![Definitions {
                    name: "@jess".to_owned(),
                    source: definitions.clone(),
                }],
                ..AnalysisOptions::default()
            },
        )?;
        for diagnostic in analysis.check(name, false).errors() {
            let reported = names.contains(&diagnostic.module) || files.borrow().contains(&diagnostic.module);
            if !reported {
                continue;
            }
            errors.insert(CheckError {
                file: diagnostic.module.clone(),
                line: diagnostic.span.begin_line + 1,
                column: diagnostic.span.begin_column + 1,
                message: diagnostic.text.clone(),
            });
        }
    }
    Ok(ScriptCheckReport {
        files: names.len(),
        errors: errors.into_iter().collect(),
    })
}
