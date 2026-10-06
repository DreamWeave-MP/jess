//! Packages: the JPK container, Package Source and JPK parity, resolution, and `require` through
//! a mounted plan.

use std::path::Path;

use jess::lua::l3i::Runtime;
use jess::lua::l3i::value::Table;
use jess::lua::package::{self, Plan};

mod common;

/// Writes `files` (path, contents) under `root`.
fn write(root: &Path, files: &[(&str, &str)]) {
    for (path, contents) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

/// A package with two layers, a relative module, an `@self` module, and a module that reaches
/// for a file outside its package.
fn alpha(root: &Path) {
    write(
        root,
        &[
            (
                "jess.toml",
                r#"manifest_version = 1

[package]
id = "org.example.alpha"
version = "1.2.3"
jess_api = "1"
default_target = "greet"
include = ["src"]
tests = ["tests/run.luau"]

[modules]
"@alpha/core" = "src/core/init.luau"
"@alpha/tools/greet" = "src/tools/greet.luau"
"@alpha/escape" = "src/escape.luau"
"@alpha/fails" = "src/fails.luau"
"@alpha/tools/register" = "src/tools/register.luau"

[[layer]]
id = "example.core"
exports = ["@alpha/core"]

[[layer]]
id = "example.tools"
requires = ["example.core"]
exports = ["@alpha/tools/greet", "@alpha/escape", "@alpha/fails"]

[[layer.entrypoint]]
module = "@alpha/tools/register"
capabilities = ["vfs.read"]

[managed.core]
module = "src/core/init.luau"
interface = "example.core"

[targets.greet]
kind = "command"
entry = "src/tools/register.luau"
command = "alpha-greet"
capabilities = ["vfs.read"]
"#,
            ),
            (
                "src/core/init.luau",
                "local words = require '@self/words'\nreturn { greeting = words.hello, nested = require('./shared').name }\n",
            ),
            ("src/core/words.luau", "return { hello = 'hello' }\n"),
            ("src/shared.luau", "return { name = 'shared' }\n"),
            (
                "src/tools/greet.luau",
                "local core = require '@alpha/core'\nreturn function(name: string) return `{core.greeting}, {name}` end\n",
            ),
            ("src/escape.luau", "return require '../../outside'\n"),
            ("src/fails.luau", "error('boom')\n"),
            (
                "src/tools/register.luau",
                "require('@jess/log').info('registered', { greeting = require('@alpha/tools/greet')('entrypoint') })\n",
            ),
            ("outside.luau", "return 'escaped'\n"),
            ("tests/run.luau", "return require('@jess/test').new()\n"),
        ],
    );
}

fn plan_of(runtime: &Runtime, location: &Path, layers: &[&str]) -> Plan {
    package::activate(
        runtime,
        &[location.display().to_string()],
        &layers.iter().map(ToString::to_string).collect::<Vec<_>>(),
        None,
    )
    .unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn the_container_reads_writes_and_refuses() {
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lua/package/container.luau");
    let options = jess::lua::Options::default();
    let run = jess::lua::testing::run(&options, &[suite], false).unwrap();
    assert!(run.failed == 0 && run.passed > 0, "{:#?}", run.modules);
}

#[test]
fn a_package_source_and_its_jpk_are_one_package() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    let runtime = common::runtime();
    let jpk = temp.path().join("alpha.jpk");
    let packed = package::pack(&runtime, &source.display().to_string(), &jpk.display().to_string()).unwrap();
    let first = std::fs::read(&jpk).unwrap();
    package::pack(&runtime, &source.display().to_string(), &jpk.display().to_string()).unwrap();
    assert_eq!(first, std::fs::read(&jpk).unwrap(), "packing is deterministic");

    let from_source = package::inspect(&runtime, &source.display().to_string()).unwrap();
    let from_jpk = package::inspect(&runtime, &jpk.display().to_string()).unwrap();
    assert_eq!(from_source.kind, "source");
    assert_eq!(from_jpk.kind, "jpk");
    assert_eq!(from_source.digest, from_jpk.digest);
    assert_eq!(from_source.digest, packed.digest);
    assert_eq!(from_source.files, 8, "the manifest and src/, not tests/ or outside.luau");
    assert_eq!(from_source.files, from_jpk.files);
    assert_eq!(from_source.default_target.as_deref(), Some("greet"));
    assert_eq!(from_source.managed.len(), 1);
    assert_eq!(from_source.managed[0].id, "core");
    assert_eq!(
        from_source.managed[0].interface.as_deref(),
        Some("example.core")
    );
    assert_eq!(from_source.targets.len(), 1);
    assert_eq!(from_source.targets[0].id, "greet");
    assert_eq!(from_source.targets[0].kind, "command");
    assert_eq!(from_source.targets[0].command.as_deref(), Some("alpha-greet"));
    assert_eq!(from_source.targets[0].capabilities, ["vfs.read"]);
    assert_eq!(
        serde_json::to_value(&from_source.managed).unwrap(),
        serde_json::to_value(&from_jpk.managed).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&from_source.targets).unwrap(),
        serde_json::to_value(&from_jpk.targets).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&from_source.layers).unwrap(),
        serde_json::to_value(&from_jpk.layers).unwrap()
    );

    for location in [&source, &jpk] {
        let runtime = common::runtime();
        let plan = plan_of(&runtime, location, &["example.tools"]);
        assert_eq!(
            plan.layers.iter().map(|layer| layer.id.as_str()).collect::<Vec<_>>(),
            ["example.core", "example.tools"]
        );
        assert_eq!(plan.capabilities.len(), 1);
        common::run(
            &runtime,
            "local greet = require '@alpha/tools/greet'\n\
             assert(greet('vvardenfell') == 'hello, vvardenfell')\n\
             assert(require('@alpha/core').nested == 'shared')\n\
             assert(require('@alpha/core') == require('@alpha/core'))\n\
             local ok, problem = pcall(require, '@alpha/escape')\n\
             assert(not ok and string.find(problem, 'leaves the package root of org.example.alpha', 1, true), problem)\n\
             assert(string.find(problem, 'required by @org.example.alpha@1.2.3/src/escape.luau', 1, true), problem)\n\
             local failed, boom = pcall(require, '@alpha/fails')\n\
             assert(not failed and string.find(boom, 'org.example.alpha@1.2.3/src/fails.luau:1: boom', 1, true), boom)",
        );
    }
}

#[test]
fn an_edited_source_is_a_new_digest() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    let runtime = common::runtime();
    let before = package::inspect(&runtime, &source.display().to_string()).unwrap();
    write(&source, &[("src/shared.luau", "return { name = 'edited' }\n")]);
    let after = package::inspect(&runtime, &source.display().to_string()).unwrap();
    assert_eq!(before.version, after.version);
    assert_ne!(before.digest, after.digest);
}

#[test]
fn resolution_is_code_free_and_fails_loudly() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    write(
        &temp.path().join("beta"),
        &[
            (
                "jess.toml",
                r#"manifest_version = 1

[package]
id = "org.example.beta"
version = "2.0.0"
jess_api = "1"
include = ["src"]

[dependencies]
"org.example.alpha" = "^1.3"

[modules]
"@beta" = "src/init.luau"

[[layer]]
id = "example.beta"
requires = ["example.tools"]
exports = ["@beta"]
"#,
            ),
            ("src/init.luau", "error('package code ran during resolution')\n"),
        ],
    );
    write(
        &temp.path().join("cycle"),
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.cycle\"\nversion = \"0.1.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n\
                 [[layer]]\nid = \"cycle.a\"\nrequires = [\"cycle.b\"]\n[[layer]]\nid = \"cycle.b\"\nrequires = [\"cycle.a\"]\n",
            ),
            ("src/x.luau", "return 1\n"),
        ],
    );
    write(
        &temp.path().join("clash"),
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.clash\"\nversion = \"0.1.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n\
                 [modules]\n\"@dream/fs\" = \"src/x.luau\"\n\"@alpha/core\" = \"src/x.luau\"\n[[layer]]\nid = \"clash.layer\"\nexports = [\"@dream/fs\", \"@alpha/core\"]\n",
            ),
            ("src/x.luau", "return 1\n"),
        ],
    );
    let location = |name: &str| temp.path().join(name).display().to_string();
    let runtime = common::runtime();
    let failure = |locations: &[&str], layers: &[&str], grant: Option<&[&str]>| -> String {
        let locations = locations.iter().map(|name| location(name)).collect::<Vec<_>>();
        let layers = layers.iter().map(ToString::to_string).collect::<Vec<_>>();
        let grant = grant.map(|grant| grant.iter().map(ToString::to_string).collect::<Vec<_>>());
        let runtime = common::runtime();
        match package::activate(&runtime, &locations, &layers, grant.as_deref()) {
            Ok(plan) => panic!("expected a failure, got {plan:#?}"),
            Err(error) => error.to_string(),
        }
    };

    let unknown = failure(&["alpha"], &["example.nowhere"], None);
    assert!(unknown.contains("no provider found for example.nowhere"), "{unknown}");
    let missing = failure(&["beta"], &["example.beta"], None);
    assert!(
        missing.contains("example.beta requires example.tools: no provider found for example.tools"),
        "{missing}"
    );
    let version = failure(&["alpha", "beta"], &["example.beta"], None);
    assert!(
        version.contains("org.example.beta 2.0.0 requires org.example.alpha ^1.3"),
        "{version}"
    );
    assert!(version.contains("available: org.example.alpha 1.2.3"), "{version}");
    let cycle = failure(&["cycle"], &["cycle.a"], None);
    assert!(cycle.contains("layer cycle: cycle.a -> cycle.b -> cycle.a"), "{cycle}");
    let host = failure(&["clash"], &["clash.layer"], None);
    assert!(
        host.contains("exports '@dream/fs', which the host already provides"),
        "{host}"
    );
    let twice = failure(&["alpha", "alpha"], &["example.core"], None);
    assert!(twice.contains("org.example.alpha is offered twice"), "{twice}");
    let denied = failure(&["alpha"], &["example.tools"], Some(&["filesystem.write"]));
    assert!(denied.contains("cannot activate example.tools"), "{denied}");
    assert!(denied.contains("required capability denied:\n    vfs.read"), "{denied}");
    assert!(
        denied.contains("org.example.alpha 1.2.3\n    layer example.tools\n    entrypoint @alpha/tools/register"),
        "{denied}"
    );

    // A failed activation mounts nothing: the runtime can still activate a plan that resolves.
    let plan = package::graph(&runtime, &[location("alpha")], &["example.core".to_owned()]).unwrap();
    assert_eq!(plan.modules.len(), 4);
    common::run(&runtime, "assert(not pcall(require, '@alpha/core'), 'graph mounts nothing')");
}

#[test]
fn a_runtime_activates_one_plan() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    let runtime = common::runtime();
    plan_of(&runtime, &source, &["example.core"]);
    let again = package::activate(&runtime, &[source.display().to_string()], &["example.core".to_owned()], None);
    assert!(again.unwrap_err().to_string().contains("one plan per runtime"));
}

#[test]
fn manifests_and_payloads_are_checked_before_anything_runs() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = common::runtime();
    let open = |files: &[(&str, &str)]| -> String {
        let root = tempfile::tempdir_in(temp.path()).unwrap();
        write(root.path(), files);
        package::inspect(&runtime, &root.path().display().to_string())
            .map_or_else(|error| error.to_string(), |inspection| format!("{inspection:?}"))
    };
    let manifest = |extra: &str| {
        format!(
            "manifest_version = 1\n[package]\nid = \"org.example.check\"\nversion = \"1.0.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n{extra}"
        )
    };

    let unknown = open(&[("jess.toml", &manifest("license = \"MIT\"\n")), ("src/a.luau", "")]);
    assert!(unknown.contains("unknown key 'license'"), "{unknown}");
    let jess = open(&[
        ("jess.toml", &manifest("[modules]\n\"@jess/mine\" = \"src/a.luau\"\n")),
        ("src/a.luau", ""),
    ]);
    assert!(jess.contains("@jess/ is jess's own namespace"), "{jess}");
    let absent = open(&[
        (
            "jess.toml",
            &manifest("[modules]\n\"@x/y\" = \"src/missing.luau\"\n[[layer]]\nid = \"x.y\"\nexports = [\"@x/y\"]\n"),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(absent.contains("isn't in the payload"), "{absent}");
    let layer = open(&[
        ("jess.toml", &manifest("[[layer]]\nid = \"x.y\"\nexports = [\"@x/none\"]\n")),
        ("src/a.luau", ""),
    ]);
    assert!(layer.contains("exports '@x/none', which [modules] doesn't name"), "{layer}");
    let dead = open(&[
        ("jess.toml", &manifest("[modules]\n\"@x/y\" = \"src/a.luau\"\n")),
        ("src/a.luau", ""),
    ]);
    assert!(
        dead.contains("module '@x/y' is neither exported nor an entrypoint of any layer"),
        "{dead}"
    );
    let entrypoint = open(&[
        (
            "jess.toml",
            &manifest("[[layer]]\nid = \"x.y\"\n[[layer.entrypoint]]\nmodule = \"@x/nowhere\"\n"),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        entrypoint.contains("entrypoint module '@x/nowhere' isn't one [modules] names"),
        "{entrypoint}"
    );
    let old = open(&[
        ("jess.toml", &manifest("[[layer]]\nid = \"x.y\"\nmodules = []\n")),
        ("src/a.luau", ""),
    ]);
    assert!(old.contains("unknown key 'modules'"), "{old}");
    let api = open(&[
        ("jess.toml", &manifest("").replace("jess_api = \"1\"", "jess_api = \"2\"")),
        ("src/a.luau", ""),
    ]);
    assert!(api.contains("jess_api must be \"1\""), "{api}");
    let id = open(&[
        ("jess.toml", &manifest("").replace("org.example.check", "Example")),
        ("src/a.luau", ""),
    ]);
    assert!(id.contains("is not lowercase ASCII words joined by dots"), "{id}");

    let default_target = open(&[
        (
            "jess.toml",
            &manifest("default_target = \"missing\"\n[targets.real]\nkind = \"command\"\nentry = \"src/a.luau\"\n"),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        default_target.contains("default_target 'missing' does not name a [targets] entry"),
        "{default_target}"
    );

    let managed_interface = open(&[
        (
            "jess.toml",
            &manifest("[managed.worker]\nmodule = \"src/a.luau\"\ninterface = \"Worker\"\n"),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        managed_interface.contains("[managed.worker] interface 'Worker'"),
        "{managed_interface}"
    );

    let interface_name = open(&[
        (
            "jess.toml",
            &manifest(
                "[managed.worker]\nmodule = \"src/a.luau\"\ninterfaceName = \"example.worker\"\n",
            ),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        interface_name.contains("[managed.worker] has an unknown key 'interfaceName'"),
        "{interface_name}"
    );

    let target_kind = open(&[
        (
            "jess.toml",
            &manifest("[targets.tool]\nkind = \"service\"\nentry = \"src/a.luau\"\n"),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        target_kind.contains("kind must be \"application\" or \"command\""),
        "{target_kind}"
    );

    let app_command = open(&[
        (
            "jess.toml",
            &manifest("[targets.app]\nkind = \"application\"\nentry = \"src/a.luau\"\ncommand = \"oops\"\n"),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        app_command.contains("command is only valid when kind = \"command\""),
        "{app_command}"
    );

    let duplicate_command = open(&[
        (
            "jess.toml",
            &manifest(
                "[targets.one]\nkind = \"command\"\nentry = \"src/a.luau\"\ncommand = \"same\"\n\
                 [targets.two]\nkind = \"command\"\nentry = \"src/a.luau\"\ncommand = \"same\"\n",
            ),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        duplicate_command.contains("command 'same' is declared by both targets 'one' and 'two'"),
        "{duplicate_command}"
    );

    let target_capability = open(&[
        (
            "jess.toml",
            &manifest(
                "[targets.tool]\nkind = \"command\"\nentry = \"src/a.luau\"\ncapabilities = [\"filesystem\"]\n",
            ),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        target_capability.contains("capability 'filesystem' is not dotted lowercase words"),
        "{target_capability}"
    );

    let private_source = open(&[
        (
            "jess.toml",
            &manifest(
                "[managed.worker]\nmodule = \"src/missing.luau\"\n\
                 [targets.tool]\nkind = \"command\"\nentry = \"src/a.luau\"\n",
            ),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        private_source.contains("managed 'worker' is 'src/missing.luau', which isn't in the payload"),
        "{private_source}"
    );

    let target_source = open(&[
        (
            "jess.toml",
            &manifest(
                "[targets.tool]\nkind = \"command\"\nentry = \"src/missing.luau\"\n",
            ),
        ),
        ("src/a.luau", ""),
    ]);
    assert!(
        target_source.contains("target 'tool' entry is 'src/missing.luau', which isn't in the payload"),
        "{target_source}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_package_source_holds_no_links_and_no_case_twins() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = common::runtime();
    let manifest = "manifest_version = 1\n[package]\nid = \"org.example.links\"\nversion = \"1.0.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n";
    let linked = temp.path().join("linked");
    write(&linked, &[("jess.toml", manifest), ("src/a.luau", "")]);
    std::os::unix::fs::symlink("/etc/hostname", linked.join("src/link.luau")).unwrap();
    let problem = package::inspect(&runtime, &linked.display().to_string())
        .unwrap_err()
        .to_string();
    assert!(problem.contains("'src/link.luau' is a symlink"), "{problem}");

    let twins = temp.path().join("twins");
    write(
        &twins,
        &[("jess.toml", manifest), ("src/Name.luau", ""), ("src/name.luau", "")],
    );
    let problem = package::inspect(&runtime, &twins.display().to_string())
        .unwrap_err()
        .to_string();
    assert!(problem.contains("differ only in case"), "{problem}");
}

#[test]
fn a_jpk_holds_only_its_payload() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = common::runtime();
    let jpk = temp.path().join("extra.jpk");
    common::set_global(&runtime, "output", jpk.display().to_string().as_str());
    common::run(
        &runtime,
        "local jpk = require '@jess/jpk'\n\
         local manifest = 'manifest_version = 1\\n[package]\\nid = \"org.example.extra\"\\nversion = \"1.0.0\"\\njess_api = \"1\"\\ninclude = [\"src\"]\\n'\n\
         jess.fs.replace(output, jpk.write {\n\
           { path = 'jess.toml', data = buffer.fromstring(manifest) },\n\
           { path = 'src/a.luau', data = buffer.fromstring('return 1') },\n\
           { path = 'smuggled.luau', data = buffer.fromstring('return 2') },\n\
         })",
    );
    let problem = package::inspect(&runtime, &jpk.display().to_string())
        .unwrap_err()
        .to_string();
    assert!(problem.contains("'smuggled.luau' is outside the payload"), "{problem}");
}

/// An opened package is one generation: a file changed on disk, or a JPK replaced, after it was
/// opened is refused when read, never run under the digest it was opened with.
#[test]
fn an_opened_package_reads_only_the_bytes_it_was_opened_with() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    let runtime = common::runtime();
    plan_of(&runtime, &source, &["example.core"]);
    write(
        &source,
        &[("src/shared.luau", "return { name = 'edited after activation' }\n")],
    );
    let problem = runtime.exec("require '@alpha/core'").unwrap_err().to_string();
    assert!(
        problem.contains("'src/shared.luau' changed since the package was opened; open it again"),
        "{problem}"
    );

    alpha(&source);
    let jpk = temp.path().join("alpha.jpk");
    let jpk_text = jpk.display().to_string();
    package::pack(&common::runtime(), &source.display().to_string(), &jpk_text).unwrap();
    let runtime = common::runtime();
    plan_of(&runtime, &jpk, &["example.core"]);
    write(&source, &[("src/shared.luau", "return { name = 'repacked' }\n")]);
    package::pack(&common::runtime(), &source.display().to_string(), &jpk_text).unwrap();
    let problem = runtime.exec("require '@alpha/core'").unwrap_err().to_string();
    assert!(problem.contains("changed since the package was opened"), "{problem}");
}

/// The names a package may not export are the ones this runtime resolves, including a module its
/// host registered after the plan.
#[test]
fn a_package_cannot_export_a_module_the_host_registered() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("clash");
    write(
        &source,
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.clash\"\nversion = \"0.1.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n\
                 [modules]\n\"@host/extra\" = \"src/x.luau\"\n[[layer]]\nid = \"clash.layer\"\nexports = [\"@host/extra\"]\n",
            ),
            ("src/x.luau", "return 1\n"),
        ],
    );
    let runtime = common::runtime();
    let extra = Table::new(&runtime.stack(), 0, 0).unwrap();
    runtime.register_require_module("@host/extra", extra.value()).unwrap();
    let problem = package::activate(&runtime, &[source.display().to_string()], &["clash.layer".to_owned()], None)
        .unwrap_err()
        .to_string();
    assert!(
        problem.contains("exports '@host/extra', which the host already provides"),
        "{problem}"
    );
}

/// The built-in package `jpk/builtin`, as jess carries it.
fn builtin() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("jpk/builtin")
}

/// The embedded built-in package is its source: the manifest and exactly the payload files the
/// manifest includes, byte for byte. A file added to `jpk/builtin` and not to
/// `jess::lua::builtin::FILES` fails here.
#[test]
fn the_embedded_built_in_package_is_its_source() {
    fn walk(root: &Path, dir: &Path, files: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                files.push(path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let root = builtin();
    let mut files = vec!["jess.toml".to_owned()];
    walk(&root, &root.join("src"), &mut files);
    files.sort();
    let embedded = jess::lua::builtin::FILES
        .iter()
        .map(|(path, _)| (*path).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(embedded, files, "jess::lua::builtin::FILES names the payload's files");
    for (path, source) in jess::lua::builtin::FILES {
        assert_eq!(std::fs::read_to_string(root.join(path)).unwrap(), *source, "{path}");
    }
}

/// Analysis is available without activation and through either accepted built-in provider.
#[test]
fn analysis_is_embedded_and_loads_from_both_package_providers() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let temp = tempfile::tempdir_in(scratch).unwrap();
    let packed = temp.path().join("builtin.jpk");
    package::pack(
        &common::runtime(),
        &builtin().display().to_string(),
        &packed.display().to_string(),
    )
    .unwrap();
    for provider in [None, Some(builtin()), Some(packed)] {
        let runtime = common::runtime();
        if let Some(location) = provider {
            package::activate(&runtime, &[location.display().to_string()], &["jess.style".to_owned()], None).unwrap();
        }
        common::run(
            &runtime,
            "local analysis = require '@jess/analysis'\n\
             assert(table.isfrozen(analysis))\n\
             assert(analysis == require('@jess/analysis'))\n\
             local source = analysis.analyze(\"local fs = require '@dream/fs'\\nfs.readFile('example')\\n\")\n\
             assert(#source.requires == 1 and source.requires[1].module == '@dream/fs')\n\
             assert(#analysis.callsTo(source, '@dream/fs.readFile') == 1)",
        );
    }
}

#[test]
fn application_tools_are_not_embedded_script_aliases() {
    let runtime = common::runtime();
    for name in ["@jess/doctor", "@jess/tes3merge", "@jess/s3lightfixes", "@jess/mergedlands"] {
        let error = jess::lua::scripts::run_file(&runtime, Path::new(name), &[]).unwrap_err();
        assert!(error.to_string().contains("failed to read"), "{name}: {error}");
    }
}

/// `@jess/` holds the built-in package's exports in every runtime, with nothing activated; a
/// package claiming the built-in package's id is accepted only when it is the copy jess carries,
/// from its source or packed, and any other package exporting into `@jess/` is refused.
#[test]
fn only_the_built_in_package_exports_under_jess() {
    let temp = tempfile::tempdir().unwrap();
    let package_with = |name: &str, id: &str, module: &str| {
        let source = temp.path().join(name);
        write(
            &source,
            &[
                (
                    "jess.toml",
                    &format!(
                        "manifest_version = 1\n[package]\nid = \"{id}\"\nversion = \"0.1.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n\
                         [modules]\n\"{module}\" = \"src/x.luau\"\n[[layer]]\nid = \"{name}.layer\"\nexports = [\"{module}\"]\n"
                    ),
                ),
                ("src/x.luau", "return 'not jess'\n"),
            ],
        );
        source
    };
    let activate = |source: &Path, layer: &str| {
        let runtime = common::runtime();
        package::activate(&runtime, &[source.display().to_string()], &[layer.to_owned()], None)
            .map(|_| runtime)
            .map_err(|error| error.to_string())
    };
    let style = "local style = require '@jess/style'\nreturn style.check('print(\"x\")\\n')[1].rule";

    let runtime = common::runtime();
    let rule: String = runtime.eval(style).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(rule, "quotes.double");
    let missing = runtime.eval::<()>("require '@jess/nothing'").unwrap_err().to_string();
    assert!(missing.contains("@jess/ is jess's own namespace"), "{missing}");

    let runtime = activate(&builtin(), "jess.style").unwrap_or_else(|error| panic!("{error}"));
    let rule: String = runtime.eval(style).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(rule, "quotes.double");
    let packed = temp.path().join("builtin.jpk");
    package::pack(
        &common::runtime(),
        &builtin().display().to_string(),
        &packed.display().to_string(),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    activate(&packed, "jess.style").unwrap_or_else(|error| panic!("{error}"));

    let impostor = package_with("impostor", "org.dreamweave.jess", "@jess/style");
    let refused = activate(&impostor, "impostor.layer").err().unwrap_or_default();
    assert!(
        refused.contains("says it is jess's built-in package (org.dreamweave.jess)"),
        "{refused}"
    );

    let other = package_with("other", "org.example.other", "@jess/extra");
    let refused = activate(&other, "other.layer").err().unwrap_or_default();
    assert!(
        refused.contains("open only to its built-in package (org.dreamweave.jess)"),
        "{refused}"
    );
}

/// An entrypoint runs because its layer was activated, after the plan is mounted, and never
/// because a layer that merely depends on it was; its module isn't a name anyone can require.
#[test]
fn entrypoints_run_only_for_the_layers_activated() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    for (layers, ran) in [(&["example.core"][..], false), (&["example.tools"][..], true)] {
        let runtime = common::runtime();
        let records = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let captured = records.clone();
        jess::lua::stdlib::set_log_sink(&runtime, move |record| captured.borrow_mut().push(record.clone())).unwrap();
        let plan = plan_of(&runtime, &source, layers);
        assert_eq!(plan.entrypoints.len(), usize::from(ran));
        let records = records.borrow();
        assert_eq!(records.iter().any(|record| record.message == "registered"), ran, "{layers:?}");
        if ran {
            assert_eq!(records[0].fields["greeting"], "hello, entrypoint");
        }
        common::run(
            &runtime,
            "assert(not pcall(require, '@alpha/tools/register'), 'an entrypoint has no name')",
        );
    }
}
