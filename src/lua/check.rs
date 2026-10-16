//! `jess package check`: a Package Source's Luau, type-checked in strict mode.
//!
//! It checks the way an editor in a folder `jess init` set up checks, with every `require`
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
        if id == crate::lua::builtin::ID {
            return crate::lua::builtin::file(path).map(str::to_owned);
        }
        let jess = crate::lua::state(&self.runtime).ok()?;
        let mounted = jess.packages.borrow();
        let mounted = mounted.as_ref()?;
        let index = mounted.index_of(id)?;
        let module = mounted.load(&self.runtime.stack(), index, path).ok()??;
        String::from_utf8(module.source).ok()
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
        let jess = crate::lua::state(&self.runtime).ok()?;
        if let Some(mounted) = jess.packages.borrow().as_ref()
            && let Some((owner, path)) = mounted.export(required)
        {
            return Some(format!("{}/{path}", mounted.id(owner)));
        }
        let builtin = crate::lua::builtin::manifest(&self.runtime.stack(), &jess).ok()?;
        builtin
            .export(required)
            .map(|path| format!("{}/{path}", crate::lua::builtin::ID))
    }
}

impl SourceProvider for Sources {
    fn read_source(&self, name: &str) -> Option<SourceCode> {
        let text = if let Some(path) = name.strip_prefix(TYPES) {
            self.types.files.iter().find(|file| file.path == path)?.source.clone()
        } else {
            let (id, path) = Self::split(name)?;
            self.cached(id, path)?
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
/// name starts with `.`, and `target`, are skipped: they hold a tool's state, not the package.
fn sources(folder: &Path) -> std::io::Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_dir() {
                if !name.starts_with('.') && name != "target" {
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
