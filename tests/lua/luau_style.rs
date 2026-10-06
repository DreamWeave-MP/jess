//! The repo's Luau style. `@jess/style`, the checker in jess's built-in package (`jpk/builtin`),
//! runs over every `.luau` file owned by this repo under `src/` and `tools/`, the ```` ```luau ```` blocks
//! in the site's pages, and `jess luau-types`' definitions, and any finding fails the test. It is
//! activated like any package, from the Package Source.

use std::path::{Path, PathBuf};
use jess::lua::l3i::Runtime;
use jess::lua::l3i::value::Table;

mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The built-in package's Package Source.
fn builtin() -> PathBuf {
    repo_root().join("jpk/builtin")
}

/// Every file with one of `extensions` under `folders` of the repo, as paths relative to it,
/// sorted; `target` folders are skipped.
fn files(folders: &[&str], extensions: &[&str]) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, extensions: &[&str], files: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "target") {
                    walk(root, &path, extensions, files);
                }
            } else if path
                .extension()
                .is_some_and(|extension| extensions.iter().any(|wanted| extension == *wanted))
            {
                files.push(path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let root = repo_root().canonicalize().unwrap();
    let mut files = Vec::new();
    for folder in folders {
        walk(&root, &root.join(folder), extensions, &mut files);
    }
    files.sort();
    files
}

/// A runtime with the checker activated, as the global `style`.
fn checker() -> Runtime {
    let runtime = common::runtime();
    jess::lua::package::activate(&runtime, &[builtin().display().to_string()], &["jess.style".to_owned()], None)
        .unwrap_or_else(|error| panic!("{error}"));
    let style: Table = runtime
        .eval("return require '@jess/style'")
        .unwrap_or_else(|error| panic!("{error}"));
    common::set_global(&runtime, "style", &style);
    runtime
}

/// What a file is held to beyond the style every file follows, as a Luau options table.
fn options(path: &str) -> &'static str {
    // The bundled scripts and jess's own Luau: strict, native, and never `any`.
    let bundled = ["jpk/builtin/src/", "src/lua/stdlib/"];
    if bundled.iter().any(|prefix| path.starts_with(prefix)) {
        return "{ directives = { 'strict', 'native' }, noAny = true }";
    }
    if path.starts_with("src/lua/templates/") {
        return "{ directives = { 'strict' } }";
    }
    if path == "src/lua/definitions.luau" {
        return "{ declarations = true }";
    }
    "nil"
}

/// Every finding in the file at `path`, relative to the repo, formatted.
fn check_file(runtime: &Runtime, path: &str) -> Vec<String> {
    let full = repo_root().join(path).display().to_string();
    common::set_global(runtime, "path", full.as_str());
    common::set_global(runtime, "name", path);
    let script = format!(
        "local lines = {{}}\nfor _, finding in style.checkFile(path, {}) do\n  table.insert(lines, style.format(finding, name))\nend\nreturn table.concat(lines, '\\n')",
        options(path)
    );
    let found: String = runtime.eval(&script).unwrap_or_else(|error| panic!("{path}: {error}"));
    found.lines().map(str::to_owned).collect()
}

#[test]
fn luau_sources_follow_the_style() {
    let runtime = checker();
    let mut findings = Vec::new();
    for path in files(&["src", "tools"], &["luau"]) {
        findings.extend(check_file(&runtime, &path));
    }
    assert!(findings.is_empty(), "{}", findings.join("\n"));
}

#[test]
fn site_luau_blocks_follow_the_style() {
    let runtime = checker();
    let mut findings = Vec::new();
    for path in files(&["site/content"], &["md"]) {
        findings.extend(check_file(&runtime, &path));
    }
    assert!(findings.is_empty(), "{}", findings.join("\n"));
}

/// `jess luau-types`' output: the hand-written definitions plus generated library declarations.
#[test]
fn the_generated_definitions_follow_the_style() {
    let runtime = checker();
    common::set_global(&runtime, "source", jess::lua::definitions().as_str());
    let found: String = runtime
        .eval("local lines = {}\nfor _, finding in style.check(source, { declarations = true }) do\n  table.insert(lines, style.format(finding, 'jess.d.luau'))\nend\nreturn table.concat(lines, '\\n')")
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(found.is_empty(), "{found}");
}

/// `jess package test`: the checker's own suite, from the Package Source and from it packed.
#[test]
fn the_checker_passes_its_own_suite_from_both_providers() {
    let scratch = tempfile::tempdir_in(repo_root().join("target")).unwrap();
    let report = jess::lua::package::test(&jess::lua::Options::default(), &builtin(), scratch.path())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(report.failed, 0, "{report:#?}");
    assert_eq!(report.providers.len(), 2);
    assert_eq!(report.providers[0].digest, report.providers[1].digest);
}

/// The checker and its suite type-check against the types folder, as luau-lsp sees them in the
/// package's source: `@jess/style` is the module, and `@jess/` and `@dream/` names are the types
/// folder's stubs.
#[test]
fn the_checker_type_checks_against_the_definitions() {
    let files = common::luau_tree(&builtin(), "builtin");
    let mains = files.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>();
    let report = common::type_check_files(
        &mains,
        &files,
        &[
            ("@jess/style", "builtin/src/style"),
            ("@jess/analysis", "builtin/src/analysis"),
            ("@jess/json", "builtin/src/data/json"),
            ("@jess/yaml", "builtin/src/data/yaml"),
            ("@jess/data/toml", "builtin/src/data/toml"),
        ],
    );
    assert!(report.is_empty(), "{report}");
}

/// Keep the new built-in analysis module strict even while older packages have style debt.
#[test]
fn the_analysis_sources_follow_strict_style() {
    let runtime = checker();
    for path in ["jpk/builtin/src/analysis/init.luau", "jpk/builtin/tests/analysis/run.luau"] {
        let findings = check_file(&runtime, path);
        assert!(findings.is_empty(), "{}", findings.join("\n"));
    }
}
