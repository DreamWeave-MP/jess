//! jess's Luau types as a folder, what `jess luau-types` and `jess init` write.
//!
//! One definitions file declares the globals and every type: `jess`, the libraries' globals,
//! generic services. Each module `require` resolves natively (`@jess/log`,
//! `@jess/shader`, ...) gets a stub module of its own, a file that returns a value
//! of that module's type, and a `.jessrc` aliases each scope (`jess`, `dream`, ...) to its folder.
//! A checker then types `require('@jess/log')` the way Luau types any require: by resolving it to
//! a file. Nothing is overloaded, so nothing limits how many modules there are.
//!
//! The built-in package's exports (`@jess/json`, `@jess/style`, ...) are Luau, and a Luau module's
//! type is its source: the package's payload is copied under the `jess` scope at its own paths,
//! `src/` stripped, so `@jess/style` resolves to `jess/style/init.jsl`, the module itself, and its
//! relative and `@self/` requires resolve beside it as they do in the package. An export whose
//! name doesn't match its path (`@jess/json` is `src/data/json.jsl`) gets a one-line stub that
//! requires the copy.
//!
//! ```text
//! types/
//!   jess.d.jsl          the definitions: globals and types
//!   jess/init.jsl       @jess
//!   jess/log.jsl        @jess/log
//!   jess/json.jsl       @jess/json: `return require('./data/json')`
//!   jess/data/json.jsl  the built-in package's src/data/json.jsl
//!   jess/style/init.jsl @jess/style, the built-in package's src/style/init.jsl
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

/// The built-in package's payload folder, left off the copies' paths under the `jess` scope.
const PAYLOAD_SOURCE: &str = "src/";

/// The definitions file, at the folder's top.
pub const DEFINITIONS: &str = "jess.d.jsl";

/// One generated file: its path inside the types folder, `/`-separated, and its contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeFile {
    /// The path inside the types folder.
    pub path: String,
    /// The file's text.
    pub source: String,
}

/// The types folder's contents: the definitions file first, then each module's stub by path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Types {
    /// Every file, the definitions first, then the stubs sorted by path.
    pub files: Vec<TypeFile>,
    /// Every module typed, by the name `require` takes, sorted.
    pub modules: Vec<String>,
    /// Where each typed module's file is inside the folder, by the name `require` takes.
    pub paths: BTreeMap<String, String>,
    /// The `.jessrc` aliases' names, sorted: the first segment of every module name.
    pub scopes: Vec<String>,
}

impl Types {
    /// The file `require(module)` resolves to through the aliases, if `module` is typed.
    #[must_use]
    pub fn module_file(&self, module: &str) -> Option<&TypeFile> {
        let path = self.paths.get(module)?;
        self.files.iter().find(|file| &file.path == path)
    }

    /// The definitions file.
    #[must_use]
    pub fn definitions(&self) -> &str {
        &self.files[0].source
    }

    /// A `.jessrc` for a folder whose types live in `folder` (relative to it): strict, with an
    /// alias for every scope.
    #[must_use]
    pub fn jessrc(&self, folder: &str) -> String {
        let aliases = self
            .scopes
            .iter()
            .map(|scope| format!("    \"{scope}\": \"{folder}/{scope}\""))
            .collect::<Vec<_>>()
            .join(",\n");
        format!("{{\n  \"languageMode\": \"strict\",\n  \"aliases\": {{\n{aliases}\n  }}\n}}\n")
    }
}

/// jess's types: the definitions, and a stub for every `@jess` and plan module a jess runtime's
/// `require` resolves natively.
///
/// # Errors
///
/// Returns an error if a runtime can't be made to read the namespace from.
pub fn types() -> l3i::Result<Types> {
    let runtime = crate::lua::runtime(crate::lua::Options::default())?;
    let plan = crate::lua::plan()?;
    let members = crate::lua::plan::MEMBERS;
    let mut modules = Vec::new();
    let mut stubs = Vec::new();
    for name in crate::lua::package::host_modules(&runtime)? {
        let Some(body) = name.strip_prefix('@') else {
            continue;
        };
        let source = if body == "jess" {
            Some(stub(&name, "jess"))
        } else if let Some(member) = body.strip_prefix("jess/").filter(|member| members.contains(member)) {
            Some(stub(&name, &format!("jess.{member}")))
        } else {
            plan.module_stub(&name)
        };
        if let Some(source) = source {
            modules.push(name);
            stubs.push(source);
        }
    }
    let mut paths = BTreeMap::new();
    let mut stub_files = modules
        .iter()
        .zip(stubs)
        .filter_map(|(name, source)| module_path(name, &modules).map(|path| TypeFile { path, source }))
        .collect::<Vec<_>>();
    for (name, file) in modules.iter().zip(&stub_files) {
        paths.insert(name.clone(), file.path.clone());
    }
    // The built-in package's modules, verbatim: a Luau module's type is its source.
    let jess = crate::lua::state(&runtime)?;
    let manifest = crate::lua::builtin::manifest(&runtime.stack(), &jess)?;
    for (path, source) in crate::lua::builtin::FILES {
        if let Some(rest) = path.strip_prefix(PAYLOAD_SOURCE).filter(|rest| is_jsl(rest)) {
            stub_files.push(TypeFile {
                path: format!("jess/{rest}"),
                source: (*source).to_owned(),
            });
        }
    }
    for (name, path) in manifest.exports() {
        let Some(body) = name.strip_prefix('@') else {
            continue;
        };
        let Some(rest) = path.strip_prefix(PAYLOAD_SOURCE) else {
            continue;
        };
        let copy = format!("jess/{rest}");
        let natural = [format!("{body}.jsl"), format!("{body}/init.jsl")];
        let resolved = if natural.contains(&copy) {
            copy
        } else {
            let stub = format!("{body}.jsl");
            stub_files.push(TypeFile {
                source: reexport(name, &stub, &copy),
                path: stub.clone(),
            });
            stub
        };
        modules.push(name.to_owned());
        paths.insert(name.to_owned(), resolved);
    }
    modules.sort();
    let mut files = vec![TypeFile {
        path: DEFINITIONS.to_owned(),
        source: crate::lua::definitions(),
    }];
    stub_files.sort_by(|a, b| a.path.cmp(&b.path));
    files.extend(stub_files);
    let scopes = modules
        .iter()
        .filter_map(|name| name.strip_prefix('@'))
        .map(|body| body.split('/').next().unwrap_or(body).to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(Types {
        files,
        modules,
        paths,
        scopes,
    })
}

/// A stub that returns `expression`, a global or one of its members, as module `name`.
fn stub(name: &str, expression: &str) -> String {
    format!("--!strict\n-- jess's stub for {name}: the runtime module is `{expression}`.\nreturn {expression}\n")
}

/// A stub at `stub` that requires the module copied at `target`, relative to the stub's folder, as
/// module `name`: `jess/json.jsl` returning `require('./data/json')`.
fn reexport(name: &str, stub: &str, target: &str) -> String {
    let folder = stub.rsplit_once('/').map_or("", |(folder, _)| folder);
    let mut up = folder.split('/').filter(|part| !part.is_empty()).collect::<Vec<_>>();
    let mut down = target.strip_suffix(".jsl").unwrap_or(target).split('/').collect::<Vec<_>>();
    while let (Some(first), Some(second)) = (up.first(), down.first())
        && first == second
    {
        up.remove(0);
        down.remove(0);
    }
    let mut relative = if up.is_empty() { vec!["."] } else { vec![".."; up.len()] };
    relative.extend(down);
    let relative = relative.join("/");
    format!(
        "--!strict\n-- jess's stub for {name}: the built-in package's module, copied beside it at {target}.\nreturn require('{relative}')\n"
    )
}

/// Where module `name` lives in the folder: `@scope/a/b` is `scope/a/b.jsl`, or
/// `scope/a/b/init.jsl` when another module lives under it; `@scope` is `scope/init.jsl`.
fn module_path(name: &str, modules: &[String]) -> Option<String> {
    let body = name.strip_prefix('@')?;
    if !modules.iter().any(|module| module == name) {
        return None;
    }
    let parent = format!("{name}/");
    let has_children = modules.iter().any(|module| module.starts_with(&parent));
    Some(if has_children || !body.contains('/') {
        format!("{body}/init.jsl")
    } else {
        format!("{body}.jsl")
    })
}

/// What [`write()`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Written {
    /// Every file written.
    pub written: Vec<PathBuf>,
    /// Stale stubs removed: `.jsl` files in a scope's folder that jess no longer generates.
    pub removed: Vec<PathBuf>,
    /// The scopes written, one folder and one `.jessrc` alias each.
    pub scopes: Vec<String>,
}

/// Writes [`types`] into `folder`, creating it.
///
/// Stubs a previous jess left in a scope's folder that this one doesn't generate are removed.
/// Nothing outside the definitions file and the scopes' folders is touched.
///
/// # Errors
///
/// Returns an error if the types can't be made, or a file can't be written or removed.
pub fn write(folder: &Path, types: &Types) -> io::Result<Written> {
    let mut report = Written {
        scopes: types.scopes.clone(),
        ..Written::default()
    };
    for file in &types.files {
        let path = folder.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.source)?;
        report.written.push(path);
    }
    let generated = types
        .files
        .iter()
        .map(|file| folder.join(&file.path))
        .collect::<BTreeSet<_>>();
    for scope in &types.scopes {
        remove_stale(&folder.join(scope), &generated, &mut report.removed)?;
    }
    Ok(report)
}

fn remove_stale(dir: &Path, generated: &BTreeSet<PathBuf>, removed: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries = std::fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            remove_stale(&path, generated, removed)?;
            if std::fs::read_dir(&path)?.next().is_none() {
                std::fs::remove_dir(&path)?;
            }
        } else if path.extension().is_some_and(|extension| extension == "jsl") && !generated.contains(&path) {
            std::fs::remove_file(&path)?;
            removed.push(path);
        }
    }
    Ok(())
}

/// Whether `path` names a `.jsl` file, whatever the case of its extension.
fn is_jsl(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("jsl"))
}
