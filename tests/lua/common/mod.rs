//! What the tests share: a jess runtime and the values they hand scripts, and luau-analyze over
//! the Luau definitions, a module that declares everything `jess::lua::runtime` installs, for
//! type-checking scripts against.

#![allow(dead_code)] // Each test binary uses the part it needs.

use std::cell::RefCell;
use std::rc::Rc;

use jess::lua::Options;
use jess::lua::l3i;
use jess::lua::l3i::Runtime;
use jess::lua::l3i::convert::{NewBuffer, Push};
use jess::lua::l3i::value::Table;

/// A runtime with config resolution off, as most tests want.
pub fn runtime() -> Runtime {
    runtime_with(Options::default())
}

pub fn runtime_with(options: Options) -> Runtime {
    jess::lua::runtime(options).unwrap_or_else(|error| panic!("{error}"))
}

/// A fixture runtime explicitly authorized to read and write scratch filesystem data.
pub fn filesystem_runtime() -> Runtime {
    jess::lua::runtime_with_capabilities(
        Options::default(),
        &["filesystem.read".to_owned(), "filesystem.write".to_owned()],
    )
    .unwrap_or_else(|error| panic!("{error}"))
}

/// Runs a script, failing the test with its error.
pub fn run(runtime: &Runtime, script: &str) {
    runtime.exec(script).unwrap_or_else(|error| panic!("{error}"));
}

/// Sets a global for a script to read; the globals are read-only otherwise.
pub fn set_global<T: Push + ?Sized>(runtime: &Runtime, name: &str, value: &T) {
    let globals = Table::from_value(runtime.global("_G").unwrap()).unwrap();
    l3i::readonly::set_read_only_field(runtime, &globals, name, value).unwrap();
}

/// Sets a global holding `bytes` as a buffer.
pub fn set_buffer(runtime: &Runtime, name: &str, bytes: Vec<u8>) {
    set_global(runtime, name, &NewBuffer(bytes));
}

thread_local! {
    /// One checker per test thread: jess's types folder, its definitions loaded once, and the
    /// files of the check under way.
    static CHECKER: (jess::lua::l3i::analysis::Analysis, Rc<RefCell<Checked>>) = {
        use jess::lua::l3i::analysis::{Analysis, AnalysisOptions, Definitions};

        let types = Rc::new(jess::lua::types::types().unwrap_or_else(|error| panic!("{error}")));
        let checked = Rc::new(RefCell::new(Checked::default()));
        let sources = Sources { types: types.clone(), checked: checked.clone() };
        let analysis = Analysis::new(
            sources,
            AnalysisOptions {
                definitions: vec![Definitions {
                    name: "@jess".to_owned(),
                    source: types.definitions().to_owned(),
                }],
                ..AnalysisOptions::default()
            },
        )
        .unwrap_or_else(|error| panic!("{error}"));
        (analysis, checked)
    };
}

/// Type-checks `source` as a script in a folder `jess init` set up, and returns its errors, one
/// per line; empty when it checks.
pub fn type_check(source: &str) -> String {
    type_check_files(&["main.luau"], &[("main.luau".to_owned(), source.to_owned())], &[])
}

/// Every `.luau` file under `root`, as (`prefix/` and its path below `root`, source), sorted.
pub fn luau_tree(root: &std::path::Path, prefix: &str) -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, name: &str, files: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let child = format!("{name}/{}", path.file_name().unwrap().to_string_lossy());
            if path.is_dir() {
                walk(&path, &child, files);
            } else if path.extension().is_some_and(|extension| extension == "luau") {
                files.push((child, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, prefix, &mut files);
    files.sort();
    files
}

/// Type-checks the modules `mains` among `files` (path, source), the way luau-lsp does in a folder
/// `jess init` set up: `types/jess.d.luau` loaded as definitions, `@scope/...` resolved through
/// the types folder's stubs as its `.luaurc` aliases resolve them, `aliases` (a module name prefix
/// and the path it stands for, as a package's exports) next, and `./`, `../`, and `@self/`
/// relative to the requiring file, an `init.luau` standing for its folder. Returns every error in
/// them and what they require, one per line, as `path:line:column: message`.
pub fn type_check_files(mains: &[&str], files: &[(String, String)], aliases: &[(&str, &str)]) -> String {
    CHECKER.with(|(analysis, checked)| {
        *checked.borrow_mut() = Checked {
            files: files.iter().cloned().collect(),
            aliases: aliases
                .iter()
                .map(|(name, path)| ((*name).to_owned(), (*path).to_owned()))
                .collect(),
        };
        // Forget the last check's modules; the definitions stay loaded.
        analysis.clear();
        let mut errors = std::collections::BTreeSet::new();
        for main in mains {
            for d in analysis.check(main, false).errors() {
                errors.insert(format!(
                    "{}:{}:{}: {}",
                    d.module,
                    d.span.begin_line + 1,
                    d.span.begin_column + 1,
                    d.text
                ));
            }
        }
        errors.into_iter().collect::<Vec<_>>().join("\n")
    })
}

/// The files and aliases of the check under way.
#[derive(Default)]
struct Checked {
    files: std::collections::HashMap<String, String>,
    aliases: Vec<(String, String)>,
}

struct Sources {
    types: Rc<jess::lua::types::Types>,
    checked: Rc<RefCell<Checked>>,
}

impl Sources {
    /// The file `base` names: itself with `.luau`, or its folder's `init.luau`.
    fn find(&self, base: &str) -> Option<String> {
        let checked = self.checked.borrow();
        [format!("{base}.luau"), format!("{base}/init.luau")]
            .into_iter()
            .find(|candidate| checked.files.contains_key(candidate))
    }
}

impl jess::lua::l3i::analysis::SourceProvider for Sources {
    /// Strict, as the `.luaurc` `jess init` writes sets every module.
    fn module_config(&self, _: &str) -> jess::lua::l3i::analysis::ModuleConfig {
        jess::lua::l3i::analysis::ModuleConfig {
            mode: jess::lua::l3i::analysis::Mode::Strict,
            ..jess::lua::l3i::analysis::ModuleConfig::default()
        }
    }

    fn read_source(&self, name: &str) -> Option<jess::lua::l3i::analysis::SourceCode> {
        let text = match name.strip_prefix("types/") {
            Some(path) => self.types.files.iter().find(|file| file.path == path)?.source.clone(),
            None => self.checked.borrow().files.get(name)?.clone(),
        };
        Some(jess::lua::l3i::analysis::SourceCode { text, is_script: false })
    }

    fn resolve_module(&self, from: &str, required: &str) -> Option<String> {
        if required.starts_with('@') && !required.starts_with("@self/") {
            let aliased = self.checked.borrow().aliases.iter().find_map(|(name, path)| {
                let rest = required.strip_prefix(name.as_str())?;
                (rest.is_empty() || rest.starts_with('/')).then(|| format!("{path}{rest}"))
            });
            if let Some(base) = aliased {
                return self.find(&base);
            }
            return self.types.module_file(required).map(|file| format!("types/{}", file.path));
        }
        let mut parts = from.split('/').map(str::to_owned).collect::<Vec<_>>();
        let init = parts.last().is_some_and(|last| last == "init.luau");
        let (relative, levels) = required.strip_prefix("@self/").map_or_else(
            || (required.to_owned(), if init { 2 } else { 1 }),
            |rest| (format!("./{rest}"), 1),
        );
        for _ in 0..levels {
            parts.pop();
        }
        for part in relative.split('/') {
            match part {
                "." | "" => {}
                ".." => {
                    parts.pop()?;
                }
                part => parts.push(part.to_owned()),
            }
        }
        self.find(&parts.join("/"))
    }
}
