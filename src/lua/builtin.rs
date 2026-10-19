//! jess's built-in package, `jpk/builtin` (`org.dreamweave.jess`), embedded in the binary.
//!
//! The manifest and payload are the source tree's files, taken with `include_str!` when jess is
//! built, so editing one rebuilds jess with it and the embedded copy can't fall behind the
//! source. A test checks [`FILES`] names exactly the files the manifest's payload holds.
//!
//! Its exports are `@jess/` names `require` resolves in every runtime, with nothing activated. And
//! it is the only package whose modules may be `@jess/` names: `@jess/package` accepts a package
//! on disk that claims [`ID`] only when its manifest and payload are byte for byte these files, so
//! naming a package `org.dreamweave.jess` gets it nothing.
//!
//! jess reads TOML with one codec, the package's Luau `@jess/data/toml`, and that includes its own
//! manifest. The decoder needs nothing but `./common`, so a runtime loads it under
//! `BOOTSTRAP_PREFIX`, where relative requires resolve against the embedded files with no
//! manifest read yet, and decodes the manifest with it the first time anything asks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use l3i::source::LoadScope;
use l3i::stack::Scope;
use l3i::value::{Function, Table, Value};

use crate::lua::package::PackageModule;
use crate::lua::{Jess, error};

/// The built-in package's id.
pub const ID: &str = "org.dreamweave.jess";

/// The chunk-name prefix of the modules a runtime loads to read TOML before any manifest is
/// read: one no package (`@<id>@<version>/`) and no script spells.
pub(crate) const BOOTSTRAP_PREFIX: &str = "@jess:bootstrap/";

/// The decoder module, which requires nothing but `./common`.
const DECODER: &str = "src/data/tomldecode.jsl";

/// The manifest and every payload file, by package path, sorted.
pub const FILES: &[(&str, &str)] = &[
    ("jess.toml", include_str!("../../jpk/builtin/jess.toml")),
    (
        "src/analysis/init.jsl",
        include_str!("../../jpk/builtin/src/analysis/init.jsl"),
    ),
    ("src/backup/init.jsl", include_str!("../../jpk/builtin/src/backup/init.jsl")),
    ("src/data/common.jsl", include_str!("../../jpk/builtin/src/data/common.jsl")),
    ("src/data/json.jsl", include_str!("../../jpk/builtin/src/data/json.jsl")),
    ("src/data/toml.jsl", include_str!("../../jpk/builtin/src/data/toml.jsl")),
    (
        "src/data/tomldecode.jsl",
        include_str!("../../jpk/builtin/src/data/tomldecode.jsl"),
    ),
    ("src/data/value.jsl", include_str!("../../jpk/builtin/src/data/value.jsl")),
    ("src/data/yaml.jsl", include_str!("../../jpk/builtin/src/data/yaml.jsl")),
    ("src/http/init.jsl", include_str!("../../jpk/builtin/src/http/init.jsl")),
    ("src/input/init.jsl", include_str!("../../jpk/builtin/src/input/init.jsl")),
    ("src/l10n/file.jsl", include_str!("../../jpk/builtin/src/l10n/file.jsl")),
    ("src/l10n/format.jsl", include_str!("../../jpk/builtin/src/l10n/format.jsl")),
    ("src/l10n/init.jsl", include_str!("../../jpk/builtin/src/l10n/init.jsl")),
    ("src/l10n/locale.jsl", include_str!("../../jpk/builtin/src/l10n/locale.jsl")),
    ("src/l10n/pattern.jsl", include_str!("../../jpk/builtin/src/l10n/pattern.jsl")),
    ("src/style/check.jsl", include_str!("../../jpk/builtin/src/style/check.jsl")),
    ("src/style/init.jsl", include_str!("../../jpk/builtin/src/style/init.jsl")),
    ("src/style/rules.jsl", include_str!("../../jpk/builtin/src/style/rules.jsl")),
    (
        "tests/analysis/run.jsl",
        include_str!("../../jpk/builtin/tests/analysis/run.jsl"),
    ),
    ("tests/backup/run.jsl", include_str!("../../jpk/builtin/tests/backup/run.jsl")),
    ("tests/data/run.jsl", include_str!("../../jpk/builtin/tests/data/run.jsl")),
    ("tests/http/run.jsl", include_str!("../../jpk/builtin/tests/http/run.jsl")),
    ("tests/input/run.jsl", include_str!("../../jpk/builtin/tests/input/run.jsl")),
    (
        "tests/l10n/fixtures.jsl",
        include_str!("../../jpk/builtin/tests/l10n/fixtures.jsl"),
    ),
    ("tests/l10n/run.jsl", include_str!("../../jpk/builtin/tests/l10n/run.jsl")),
    ("tests/style/run.jsl", include_str!("../../jpk/builtin/tests/style/run.jsl")),
];

/// What `require` needs from the manifest: the version, for chunk names, and the modules.
pub(crate) struct Manifest {
    prefix: String,
    exports: HashMap<String, String>,
}

impl Manifest {
    /// `@<id>@<version>/`, the start of every chunk name its modules load with.
    pub(crate) fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The path the package exports `name` from.
    pub(crate) fn export(&self, name: &str) -> Option<&str> {
        self.exports.get(name).map(String::as_str)
    }

    /// The module at `path`, ready to load.
    pub(crate) fn module(&self, path: &str) -> Option<PackageModule> {
        file(path).map(|source| PackageModule {
            chunk: format!("{}{path}", self.prefix),
            key: format!("\0builtin\0{path}"),
            source: source.as_bytes().to_vec(),
        })
    }
}

/// The embedded file at `path`.
#[must_use]
pub fn file(path: &str) -> Option<&'static str> {
    FILES.iter().find(|(name, _)| *name == path).map(|(_, source)| *source)
}

/// The module at `path` as the bootstrap loads it, under [`BOOTSTRAP_PREFIX`].
pub(crate) fn bootstrap_module(path: &str) -> Option<PackageModule> {
    file(path).map(|source| PackageModule {
        chunk: format!("{BOOTSTRAP_PREFIX}{path}"),
        key: format!("\0bootstrap\0{path}"),
        source: source.as_bytes().to_vec(),
    })
}

thread_local! {
    /// The decoder's bytecode, compiled once per thread: the source never changes.
    static DECODER_BYTECODE: std::cell::OnceCell<Rc<Vec<u8>>> = const { std::cell::OnceCell::new() };
}

/// A TOML decoder, `decode(text) -> (value, layout)`: the built-in package's, loaded under
/// [`BOOTSTRAP_PREFIX`] into `scope`'s runtime and kept in `cache` once it is. It needs no runtime
/// state of its own, so the host functions `@jess/package` reads manifests through can hold one, and
/// it never enters `require`'s cache by an export name, which a package activated later owns.
pub(crate) fn decoder(scope: &impl Scope, cache: &RefCell<Option<Function>>) -> l3i::Result<Function> {
    if let Some(decode) = cache.borrow().clone() {
        return Ok(decode);
    }
    let source = file(DECODER).ok_or_else(|| error("the built-in package lost its TOML decoder"))?;
    let bytecode = DECODER_BYTECODE.with(|cell| -> l3i::Result<Rc<Vec<u8>>> {
        if let Some(bytecode) = cell.get() {
            return Ok(Rc::clone(bytecode));
        }
        let bytecode = Rc::new(l3i::source::compile(source, &l3i::source::CompileOptions::default())?);
        Ok(Rc::clone(cell.get_or_init(|| bytecode)))
    })?;
    let chunk = scope.load_bytecode(&format!("{BOOTSTRAP_PREFIX}{DECODER}"), &bytecode)?;
    let decode = scope.with_frame(|frame| {
        let exports = chunk
            .invoke_with(frame, (DECODER,), |_, value| Value::store(value))
            .map_err(crate::lua::callback_error)?;
        let exports = Table::from_value(exports)?;
        Function::from_value(exports.get::<Value>(frame, "decode")?)
    })?;
    *cache.borrow_mut() = Some(decode.clone());
    Ok(decode)
}

/// The built-in package's manifest as `require` needs it, decoded once per runtime.
pub(crate) fn manifest(scope: &impl Scope, jess: &Jess) -> l3i::Result<Rc<Manifest>> {
    if let Some(manifest) = jess.builtin.borrow().clone() {
        return Ok(manifest);
    }
    let decode = decoder(scope, &jess.toml_decode)?;
    let text = file("jess.toml").ok_or_else(|| error("the built-in package has no manifest"))?;
    let manifest = scope.with_frame(|frame| {
        let document = decode
            .invoke_with(frame, (text,), |_, value| Value::store(value))
            .map_err(crate::lua::callback_error)?;
        let document = Table::from_value(document)?;
        let package = Table::from_value(document.get::<Value>(frame, "package")?)?;
        let id: String = package.get(frame, "id")?;
        if id != ID {
            return Err(error(format!("the built-in package's manifest names {id}, not {ID}")));
        }
        let version: String = package.get(frame, "version")?;
        let mut exports = HashMap::new();
        let modules = document.get::<Value>(frame, "modules")?;
        if modules.is_table() {
            frame.with_frame(|frame| {
                modules.push_to(frame)?.as_table()?.for_each(frame, |_, name, path| {
                    if let (Ok(name), Ok(path)) = (name.read::<&str>(), path.read::<&str>()) {
                        exports.insert(name.to_owned(), path.to_owned());
                    }
                    Ok(())
                })
            })?;
        }
        Ok(Manifest {
            prefix: format!("@{ID}@{version}/"),
            exports,
        })
    })?;
    let manifest = Rc::new(manifest);
    *jess.builtin.borrow_mut() = Some(Rc::clone(&manifest));
    Ok(manifest)
}
