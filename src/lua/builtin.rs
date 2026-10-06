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

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::lua::package::PackageModule;

/// The built-in package's id.
pub const ID: &str = "org.dreamweave.jess";

/// The manifest and every payload file, by package path, sorted.
pub const FILES: &[(&str, &str)] = &[
    ("jess.toml", include_str!("../../jpk/builtin/jess.toml")),
    (
        "src/analysis/init.luau",
        include_str!("../../jpk/builtin/src/analysis/init.luau"),
    ),
    (
        "src/data/common.luau",
        include_str!("../../jpk/builtin/src/data/common.luau"),
    ),
    ("src/data/json.luau", include_str!("../../jpk/builtin/src/data/json.luau")),
    ("src/data/toml.luau", include_str!("../../jpk/builtin/src/data/toml.luau")),
    ("src/data/yaml.luau", include_str!("../../jpk/builtin/src/data/yaml.luau")),
    (
        "src/style/check.luau",
        include_str!("../../jpk/builtin/src/style/check.luau"),
    ),
    (
        "src/style/init.luau",
        include_str!("../../jpk/builtin/src/style/init.luau"),
    ),
    (
        "src/style/rules.luau",
        include_str!("../../jpk/builtin/src/style/rules.luau"),
    ),
];

struct Manifest {
    prefix: String,
    exports: HashMap<String, String>,
}

/// What `require` needs from the manifest: the version, for chunk names, and the modules.
fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        let text = file("jess.toml").expect("the built-in package has a manifest");
        let document: toml::Table = text.parse().expect("the built-in package's manifest is TOML");
        let package = document["package"].as_table().expect("[package]");
        assert_eq!(package["id"].as_str(), Some(ID), "the built-in package's id");
        let version = package["version"].as_str().expect("package.version");
        let exports = document
            .get("modules")
            .and_then(toml::Value::as_table)
            .map(|modules| {
                modules
                    .iter()
                    .filter_map(|(name, path)| Some((name.clone(), path.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        Manifest {
            prefix: format!("@{ID}@{version}/"),
            exports,
        }
    })
}

/// The embedded file at `path`.
#[must_use]
pub fn file(path: &str) -> Option<&'static str> {
    FILES.iter().find(|(name, _)| *name == path).map(|(_, source)| *source)
}

/// `@<id>@<version>/`, the start of every chunk name its modules load with.
pub(crate) fn prefix() -> &'static str {
    &manifest().prefix
}

/// The path the package exports `name` from.
pub(crate) fn export(name: &str) -> Option<&'static str> {
    manifest().exports.get(name).map(String::as_str)
}

/// The module at `path`, ready to load.
pub(crate) fn module(path: &str) -> Option<PackageModule> {
    file(path).map(|source| PackageModule {
        chunk: format!("{}{path}", prefix()),
        key: format!("\0builtin\0{path}"),
        source: source.as_bytes().to_vec(),
    })
}
