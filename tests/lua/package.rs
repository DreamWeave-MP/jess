//! Packages: the JPK container, Package Source and JPK parity, resolution, and `require` through
//! a mounted plan.

use std::path::{Path, PathBuf};

use jess::lua::l3i::Runtime;
use jess::lua::l3i::value::Table;
use jess::lua::package::{self, Plan};

mod application;
use super::common;
mod workspace;

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
include = ["src", "tests"]
tests = ["tests/run.jsl"]

[modules]
"@alpha/core" = "src/core/init.jsl"
"@alpha/tools/greet" = "src/tools/greet.jsl"
"@alpha/escape" = "src/escape.jsl"
"@alpha/fails" = "src/fails.jsl"
"@alpha/tools/register" = "src/tools/register.jsl"

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
module = "src/core/init.jsl"
interface = "example.core"

[targets.greet]
kind = "command"
entry = "src/tools/register.jsl"
command = "alpha-greet"
capabilities = ["vfs.read"]
"#,
            ),
            (
                "src/core/init.jsl",
                "local words = require '@self/words'\nreturn { greeting = words.hello, nested = require('./shared').name }\n",
            ),
            ("src/core/words.jsl", "return { hello = 'hello' }\n"),
            ("src/shared.jsl", "return { name = 'shared' }\n"),
            (
                "src/tools/greet.jsl",
                "local core = require '@alpha/core'\nreturn function(name: string) return `{core.greeting}, {name}` end\n",
            ),
            ("src/escape.jsl", "return require '../../outside'\n"),
            ("src/fails.jsl", "error('boom')\n"),
            (
                "src/tools/register.jsl",
                "require('@jess/log').info('registered', { greeting = require('@alpha/tools/greet')('entrypoint') })\n",
            ),
            ("outside.jsl", "return 'escaped'\n"),
            ("tests/run.jsl", "return require('@jess/test').new()\n"),
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
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lua/package/container.jsl");
    let options = jess::lua::Options::default();
    let run = jess::lua::testing::run(&options, &[suite], false).unwrap();
    assert!(run.failed == 0 && run.passed > 0, "{:#?}", run.modules);
}

#[test]
fn a_package_source_and_its_jpk_are_one_package() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    // An editor's and a merge tool's leftovers are never payload.
    for leftover in ["src/init.jsl.bak", "src/init.jsl~", "src/init.jsl.orig"] {
        std::fs::write(source.join(leftover), "return nil\n").unwrap();
    }
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
    assert_eq!(
        from_source.files, 9,
        "the manifest, src/, and tests/, not outside.jsl or leftovers"
    );
    assert_eq!(from_source.files, from_jpk.files);
    assert_eq!(from_source.default_target.as_deref(), Some("greet"));
    assert_eq!(from_source.managed.len(), 1);
    assert_eq!(from_source.managed[0].id, "core");
    assert_eq!(from_source.managed[0].interface.as_deref(), Some("example.core"));
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
             assert(string.find(problem, 'required by @org.example.alpha@1.2.3/src/escape.jsl', 1, true), problem)\n\
             local failed, boom = pcall(require, '@alpha/fails')\n\
             assert(not failed and string.find(boom, 'org.example.alpha@1.2.3/src/fails.jsl:1: boom', 1, true), boom)",
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
    write(&source, &[("src/shared.jsl", "return { name = 'edited' }\n")]);
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
"@beta" = "src/init.jsl"

[[layer]]
id = "example.beta"
requires = ["example.tools"]
exports = ["@beta"]
"#,
            ),
            ("src/init.jsl", "error('package code ran during resolution')\n"),
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
            ("src/x.jsl", "return 1\n"),
        ],
    );
    write(
        &temp.path().join("clash"),
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.clash\"\nversion = \"0.1.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n\
                 [modules]\n\"@dream/fs\" = \"src/x.jsl\"\n\"@alpha/core\" = \"src/x.jsl\"\n[[layer]]\nid = \"clash.layer\"\nexports = [\"@dream/fs\", \"@alpha/core\"]\n",
            ),
            ("src/x.jsl", "return 1\n"),
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
#[allow(clippy::too_many_lines)] // one case per rule a manifest or payload can break
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

    let unknown = open(&[("jess.toml", &manifest("license = \"MIT\"\n")), ("src/a.jsl", "")]);
    assert!(unknown.contains("unknown key 'license'"), "{unknown}");
    let jess = open(&[
        ("jess.toml", &manifest("[modules]\n\"@jess/mine\" = \"src/a.jsl\"\n")),
        ("src/a.jsl", ""),
    ]);
    assert!(jess.contains("@jess/ is jess's own namespace"), "{jess}");
    let absent = open(&[
        (
            "jess.toml",
            &manifest("[modules]\n\"@x/y\" = \"src/missing.jsl\"\n[[layer]]\nid = \"x.y\"\nexports = [\"@x/y\"]\n"),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(absent.contains("isn't in the payload"), "{absent}");
    let layer = open(&[
        ("jess.toml", &manifest("[[layer]]\nid = \"x.y\"\nexports = [\"@x/none\"]\n")),
        ("src/a.jsl", ""),
    ]);
    assert!(layer.contains("exports '@x/none', which [modules] doesn't name"), "{layer}");
    let dead = open(&[
        ("jess.toml", &manifest("[modules]\n\"@x/y\" = \"src/a.jsl\"\n")),
        ("src/a.jsl", ""),
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
        ("src/a.jsl", ""),
    ]);
    assert!(
        entrypoint.contains("entrypoint module '@x/nowhere' isn't one [modules] names"),
        "{entrypoint}"
    );
    let old = open(&[
        ("jess.toml", &manifest("[[layer]]\nid = \"x.y\"\nmodules = []\n")),
        ("src/a.jsl", ""),
    ]);
    assert!(old.contains("unknown key 'modules'"), "{old}");
    let api = open(&[
        ("jess.toml", &manifest("").replace("jess_api = \"1\"", "jess_api = \"2\"")),
        ("src/a.jsl", ""),
    ]);
    assert!(api.contains("jess_api must be \"1\""), "{api}");
    let id = open(&[
        ("jess.toml", &manifest("").replace("org.example.check", "Example")),
        ("src/a.jsl", ""),
    ]);
    assert!(id.contains("is not lowercase ASCII words joined by dots"), "{id}");

    let default_target = open(&[
        (
            "jess.toml",
            &manifest("default_target = \"missing\"\n[targets.real]\nkind = \"command\"\nentry = \"src/a.jsl\"\n"),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(
        default_target.contains("default_target 'missing' does not name a [targets] entry"),
        "{default_target}"
    );

    let managed_interface = open(&[
        (
            "jess.toml",
            &manifest("[managed.worker]\nmodule = \"src/a.jsl\"\ninterface = \"Worker\"\n"),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(
        managed_interface.contains("[managed.worker] interface 'Worker'"),
        "{managed_interface}"
    );

    let interface_name = open(&[
        (
            "jess.toml",
            &manifest("[managed.worker]\nmodule = \"src/a.jsl\"\ninterfaceName = \"example.worker\"\n"),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(
        interface_name.contains("[managed.worker] has an unknown key 'interfaceName'"),
        "{interface_name}"
    );

    let target_kind = open(&[
        (
            "jess.toml",
            &manifest("[targets.tool]\nkind = \"service\"\nentry = \"src/a.jsl\"\n"),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(
        target_kind.contains("kind must be \"application\" or \"command\""),
        "{target_kind}"
    );

    let app_command = open(&[
        (
            "jess.toml",
            &manifest("[targets.app]\nkind = \"application\"\nentry = \"src/a.jsl\"\ncommand = \"oops\"\n"),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(
        app_command.contains("command is only valid when kind = \"command\""),
        "{app_command}"
    );

    let duplicate_command = open(&[
        (
            "jess.toml",
            &manifest(
                "[targets.one]\nkind = \"command\"\nentry = \"src/a.jsl\"\ncommand = \"same\"\n\
                 [targets.two]\nkind = \"command\"\nentry = \"src/a.jsl\"\ncommand = \"same\"\n",
            ),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(
        duplicate_command.contains("command 'same' is declared by both targets 'one' and 'two'"),
        "{duplicate_command}"
    );

    let target_capability = open(&[
        (
            "jess.toml",
            &manifest("[targets.tool]\nkind = \"command\"\nentry = \"src/a.jsl\"\ncapabilities = [\"filesystem\"]\n"),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(
        target_capability.contains("capability 'filesystem' is not dotted lowercase words"),
        "{target_capability}"
    );

    let private_source = open(&[
        (
            "jess.toml",
            &manifest(
                "[managed.worker]\nmodule = \"src/missing.jsl\"\n\
                 [targets.tool]\nkind = \"command\"\nentry = \"src/a.jsl\"\n",
            ),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(
        private_source.contains("managed 'worker' is 'src/missing.jsl', which isn't in the payload"),
        "{private_source}"
    );

    let target_source = open(&[
        (
            "jess.toml",
            &manifest("[targets.tool]\nkind = \"command\"\nentry = \"src/missing.jsl\"\n"),
        ),
        ("src/a.jsl", ""),
    ]);
    assert!(
        target_source.contains("target 'tool' entry is 'src/missing.jsl', which isn't in the payload"),
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
    write(&linked, &[("jess.toml", manifest), ("src/a.jsl", "")]);
    std::os::unix::fs::symlink("/etc/hostname", linked.join("src/link.jsl")).unwrap();
    let problem = package::inspect(&runtime, &linked.display().to_string())
        .unwrap_err()
        .to_string();
    assert!(problem.contains("'src/link.jsl' is a symlink"), "{problem}");

    let twins = temp.path().join("twins");
    write(&twins, &[("jess.toml", manifest), ("src/Name.jsl", ""), ("src/name.jsl", "")]);
    let problem = package::inspect(&runtime, &twins.display().to_string())
        .unwrap_err()
        .to_string();
    assert!(problem.contains("differ only in case"), "{problem}");
}

/// A script's `@jess/package` opens and packs only what the runtime's policy lets it read and
/// write. The host's own package operations are the host acting, and need no grant.
#[test]
fn a_script_opens_and_packs_only_what_it_may_read_and_write() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    let output = temp.path().join("alpha.jpk");
    let denied =
        |capability: &str| format!("@jess/package: needs the '{capability}' capability, which this runtime does not grant");
    for (capabilities, may_read, may_write) in [
        (&[][..], false, false),
        (&["filesystem.read"][..], true, false),
        (&["filesystem.read", "filesystem.write"][..], true, true),
    ] {
        let capabilities = capabilities.iter().map(ToString::to_string).collect::<Vec<_>>();
        let runtime = jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &capabilities)
            .unwrap_or_else(|error| panic!("{error}"));
        common::set_global(&runtime, "source", source.display().to_string().as_str());
        common::set_global(&runtime, "folder", temp.path().display().to_string().as_str());
        common::set_global(&runtime, "output", output.display().to_string().as_str());
        common::set_global(&runtime, "mayRead", &may_read);
        common::set_global(&runtime, "mayWrite", &may_write);
        common::set_global(&runtime, "noRead", denied("filesystem.read").as_str());
        common::set_global(&runtime, "noWrite", denied("filesystem.write").as_str());
        common::run(
            &runtime,
            "local package = require '@jess/package'\n\
             local function refused(expected, call, ...)\n  \
               local ok, problem = pcall(call, ...)\n  \
               assert(not ok and string.find(tostring(problem), expected, 1, true), tostring(problem))\n\
             end\n\
             if mayRead then\n  \
               assert(package.open(source).id == 'org.example.alpha')\n  \
               assert(package.peek(source).version == '1.2.3')\n  \
               assert(#package.candidates(folder) == 1)\n\
             else\n  \
               refused(noRead, package.open, source)\n  \
               refused(noRead, package.peek, source)\n  \
               refused(noRead, package.candidates, folder)\n\
             end\n\
             if mayWrite then\n  \
               assert(package.pack(source, output).files == 9)\n\
             else\n  \
               refused(if mayRead then noWrite else noRead, package.pack, source, output)\n\
             end",
        );
        assert_eq!(output.exists(), may_write, "{capabilities:?}");
        drop(std::fs::remove_file(&output));
        // The host inspects, packs, and activates whatever the script's policy is.
        assert_eq!(
            package::inspect(&runtime, &source.display().to_string()).unwrap().id,
            "org.example.alpha"
        );
        package::pack(&runtime, &source.display().to_string(), &output.display().to_string()).unwrap();
        std::fs::remove_file(&output).unwrap();
        plan_of(&runtime, &source, &["example.core"]);
        common::run(&runtime, "assert(require('@alpha/core').greeting == 'hello')");
    }
}

#[test]
fn a_jpk_holds_only_its_payload() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = common::filesystem_runtime();
    let jpk = temp.path().join("extra.jpk");
    common::set_global(&runtime, "output", jpk.display().to_string().as_str());
    common::run(
        &runtime,
        "local jpk = require '@jess/jpk'\n\
         local manifest = 'manifest_version = 1\\n[package]\\nid = \"org.example.extra\"\\nversion = \"1.0.0\"\\njess_api = \"1\"\\ninclude = [\"src\"]\\n'\n\
         assert(require('@dream/fs').writeFile(output, jpk.write {\n\
           { path = 'jess.toml', data = buffer.fromstring(manifest) },\n\
           { path = 'src/a.jsl', data = buffer.fromstring('return 1') },\n\
           { path = 'smuggled.jsl', data = buffer.fromstring('return 2') },\n\
         }, { atomic = true }))",
    );
    let problem = package::inspect(&runtime, &jpk.display().to_string())
        .unwrap_err()
        .to_string();
    assert!(problem.contains("'smuggled.jsl' is outside the payload"), "{problem}");
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
        &[("src/shared.jsl", "return { name = 'edited after activation' }\n")],
    );
    let problem = runtime.exec("require '@alpha/core'").unwrap_err().to_string();
    assert!(
        problem.contains("'src/shared.jsl' changed since the package was opened; open it again"),
        "{problem}"
    );

    alpha(&source);
    let jpk = temp.path().join("alpha.jpk");
    let jpk_text = jpk.display().to_string();
    package::pack(&common::runtime(), &source.display().to_string(), &jpk_text).unwrap();
    let runtime = common::runtime();
    plan_of(&runtime, &jpk, &["example.core"]);
    write(&source, &[("src/shared.jsl", "return { name = 'repacked' }\n")]);
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
                 [modules]\n\"@host/extra\" = \"src/x.jsl\"\n[[layer]]\nid = \"clash.layer\"\nexports = [\"@host/extra\"]\n",
            ),
            ("src/x.jsl", "return 1\n"),
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
fn builtin() -> PathBuf {
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
    for folder in ["src", "tests"] {
        walk(&root, &root.join(folder), &mut files);
    }
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
    let scratch = env!("CARGO_TARGET_TMPDIR");
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
fn names_under_jess_are_paths_to_run_file_not_script_aliases() {
    let runtime = common::runtime();
    for name in ["@jess/anything", "@jess/tools/merge", "@jess/fix-lights", "@jess/report"] {
        let error = jess::lua::scripts::run_file(&runtime, Path::new(name), &[]).unwrap_err();
        assert!(error.to_string().contains("isn't a .jsl file"), "{name}: {error}");
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
                         [modules]\n\"{module}\" = \"src/x.jsl\"\n[[layer]]\nid = \"{name}.layer\"\nexports = [\"{module}\"]\n"
                    ),
                ),
                ("src/x.jsl", "return 'not jess'\n"),
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

/// A package `org.example.<name>` at `root`: one layer, `example.<name>`, exporting `@<name>` from
/// `source`. `extra` is the rest of the manifest: more `[package]` fields, then other tables.
fn library(root: &Path, name: &str, extra: &str, source: &str) {
    // Tests ship in the payload.
    let include = if extra.contains("tests = [") {
        r#"["src", "tests"]"#
    } else {
        r#"["src"]"#
    };
    write(
        root,
        &[
            (
                "jess.toml",
                &format!(
                    "manifest_version = 1\n[package]\nid = \"org.example.{name}\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
                     include = {include}\n{extra}\n[modules]\n\"@{name}\" = \"src/init.jsl\"\n\
                     [[layer]]\nid = \"example.{name}\"\nexports = [\"@{name}\"]\n"
                ),
            ),
            ("src/init.jsl", source),
        ],
    );
}

fn text(path: &Path) -> String {
    path.display().to_string()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(ToString::to_string).collect()
}

/// A package's modules reach another package's exports only through a dependency their own
/// manifest declares: not through a dependency's dependency, and not because something else
/// already loaded the module. The host's own script holds the whole plan.
#[test]
fn a_package_requires_only_what_it_declares() {
    let temp = tempfile::tempdir().unwrap();
    let at = |name: &str| temp.path().join(name);
    library(&at("base"), "base", "", "return { name = 'base' }\n");
    library(
        &at("middle"),
        "middle",
        "[dependencies]\n\"org.example.base\" = \"1\"\n",
        "return { base = require('@base').name }\n",
    );
    library(
        &at("top"),
        "top",
        "[dependencies]\n\"org.example.middle\" = \"1\"\n",
        "return {\n  middle = require('@middle').base,\n  direct = function() return require '@base' end,\n  \
         guarded = function() return pcall(require, '@base') end,\n}\n",
    );
    let runtime = common::runtime();
    let plan = package::activate(
        &runtime,
        &[text(&at("top")), text(&at("middle")), text(&at("base"))],
        &strings(&["example.top"]),
        None,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let allowed = |id: &str| -> Vec<String> {
        plan.packages
            .iter()
            .find(|package| package.id == id)
            .unwrap()
            .dependencies
            .clone()
    };
    assert_eq!(allowed("org.example.top"), ["org.example.middle"]);
    assert_eq!(allowed("org.example.base"), [""; 0]);
    common::run(
        &runtime,
        "local top = require '@top'\n\
         assert(top.middle == 'base')\n\
         local function denied(problem)\n  \
           assert(string.find(problem, \"module not found: @base (required by @org.example.top@1.0.0/src/init.jsl): \
         org.example.base exports it, and org.example.top doesn't declare org.example.base as a dependency\", 1, true), problem)\n\
         end\n\
         local ok, problem = pcall(top.direct)\n\
         assert(not ok)\n\
         denied(problem)\n\
         assert(require('@base').name == 'base', 'the host script holds the whole plan')\n\
         local again, still = pcall(top.direct)\n\
         assert(not again, 'a module the host loaded is still not the package\\'s')\n\
         denied(still)\n\
         local guarded, through = top.guarded()\n\
         assert(not guarded)\n\
         denied(through)",
    );
}

/// A package path's packages are candidates: opened when a layer or a dependency needs one, never
/// in place of a package handed over outright, and never chosen between.
#[test]
fn a_package_path_offers_what_a_plan_needs() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("packages");
    let top = temp.path().join("top");
    library(&path.join("base"), "base", "", "return { name = 'base' }\n");
    library(&path.join("unrelated"), "unrelated", "", "error('nothing asked for this')\n");
    write(&path, &[("notes/readme.txt", "not a package\n"), ("stray.jsl", "return 1\n")]);
    library(
        &top,
        "top",
        "[dependencies]\n\"org.example.base\" = \"1\"\n",
        "return { base = require('@base').name }\n",
    );
    let search = |roots: &[&str]| package::Search {
        paths: vec![text(&path)],
        roots: strings(roots),
        ..Default::default()
    };
    let graph = |locations: &[&Path], layers: &[&str], roots: &[&str]| {
        let locations = locations.iter().map(|location| text(location)).collect::<Vec<_>>();
        package::graph_with(&common::runtime(), &locations, &strings(layers), &search(roots))
    };
    let ids = |plan: &Plan| plan.packages.iter().map(|package| package.id.clone()).collect::<Vec<_>>();
    let failure = |locations: &[&Path], layers: &[&str], roots: &[&str]| -> String {
        match graph(locations, layers, roots) {
            Ok(plan) => panic!("expected a failure, got {plan:#?}"),
            Err(error) => error.to_string(),
        }
    };

    // A dependency, a layer, and a root each bring a candidate in; nothing brings the rest.
    let plan = graph(&[&top], &["example.top"], &[]).unwrap();
    assert_eq!(ids(&plan), ["org.example.base", "org.example.top"]);
    // As paths: the resolver joins a search path and an entry with a slash, which Windows takes
    // as a separator too but writes differently from a joined path.
    assert_eq!(PathBuf::from(&plan.packages[0].location), path.join("base"));
    assert_eq!(ids(&graph(&[], &["example.base"], &[]).unwrap()), ["org.example.base"]);
    let rooted = graph(&[&top], &[], &["org.example.top"]).unwrap();
    assert_eq!(ids(&rooted), ["org.example.base", "org.example.top"]);
    assert!(rooted.layers.is_empty() && rooted.modules.len() == 2, "{rooted:#?}");
    assert_eq!(ids(&graph(&[], &[], &["org.example.base"]).unwrap()), ["org.example.base"]);
    let absent = failure(&[&top], &[], &["org.example.nowhere"]);
    assert!(
        absent.contains("org.example.nowhere was asked for, and is not available"),
        "{absent}"
    );
    let nothing = failure(&[&top], &[], &[]);
    assert!(nothing.contains("no layers were requested"), "{nothing}");

    // The plan mounts, and its packages hold what they declared.
    let runtime = common::runtime();
    package::activate_with(&runtime, &[text(&top)], &strings(&["example.top"]), None, &search(&[]))
        .unwrap_or_else(|error| panic!("{error}"));
    common::run(&runtime, "assert(require('@top').base == 'base')");

    // Two candidates for one package are an error; one handed over outright is the answer.
    let packed = path.join("base.jpk");
    package::pack(&common::runtime(), &text(&path.join("base")), &text(&packed)).unwrap();
    let twice = failure(&[&top], &["example.top"], &[]);
    assert!(
        twice.contains("package org.example.base has more than one candidate"),
        "{twice}"
    );
    // A candidate is named as the search folder, a slash, and its entry, on every platform.
    assert!(twice.contains(&format!("(jpk {}/base.jpk)", text(&path))), "{twice}");
    assert!(twice.contains(&format!("(source {}/base)", text(&path))), "{twice}");
    assert!(twice.contains("offer one of them outright"), "{twice}");
    let chosen = graph(&[&top, &packed], &["example.top"], &[]).unwrap();
    assert_eq!(chosen.packages[0].kind, "jpk");
    std::fs::remove_file(&packed).unwrap();

    // Two packages offering one layer are no choice either, until something else picks one.
    write(
        &path.join("twin"),
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.twin\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
                 include = [\"src\"]\n[[layer]]\nid = \"example.base\"\n",
            ),
            ("src/x.jsl", "return 1\n"),
        ],
    );
    let twins = failure(&[], &["example.base"], &[]);
    assert!(twins.contains("layer example.base has more than one candidate"), "{twins}");
    assert_eq!(
        ids(&graph(&[&top], &["example.top"], &[]).unwrap()),
        ["org.example.base", "org.example.top"]
    );

    // A candidate that can't be read fails the path, by name.
    write(&path.join("broken"), &[("jess.toml", "manifest_version = 2\n")]);
    let broken = failure(&[&top], &["example.top"], &[]);
    assert!(
        broken.contains(&format!(
            "package path '{}': '{}': jess.toml: manifest_version must be 1",
            text(&path),
            text(&path.join("broken"))
        )),
        "{broken}"
    );
    let missing = package::graph_with(
        &common::runtime(),
        &[text(&top)],
        &strings(&["example.top"]),
        &package::Search {
            paths: vec![text(&temp.path().join("nowhere"))],
            ..Default::default()
        },
    );
    assert!(missing.unwrap_err().to_string().contains("is not a folder"));
}

/// `[dev-dependencies]` are a package's tests' needs: `package test` selects them, with the
/// package's dependencies, over the packages and package paths it is given, from the source and
/// from the JPK alike. Nothing else selects them.
#[test]
fn package_tests_resolve_dependencies_and_dev_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("packages");
    let subject = temp.path().join("subject");
    library(&path.join("base"), "base", "", "return { name = 'base' }\n");
    library(&path.join("fixtures"), "fixtures", "", "return { expected = 'base' }\n");
    library(
        &subject,
        "subject",
        "tests = [\"tests/run.jsl\"]\n[dependencies]\n\"org.example.base\" = \"1\"\n\
         [dev-dependencies]\n\"org.example.fixtures\" = \"1\"\n",
        "return { base = require('@base').name, fixtures = function() return require '@fixtures' end }\n",
    );
    write(
        &subject,
        &[(
            "tests/run.jsl",
            "local test = require '@jess/test'\nlocal suite, expect = test.new(), test.expect\n\
             suite.it('the subject sees its dependency', function()\n  \
               expect(require('@subject').base).toBe(require('@fixtures').expected)\n\
             end)\n\
             suite.it('a dev-dependency is the tests\\', not the package\\'s', function()\n  \
               expect(require('@subject').fixtures).toThrow \"doesn't declare org.example.fixtures\"\n\
             end)\n\
             return suite\n",
        )],
    );
    let options = jess::lua::Options::default();
    let runtime = common::runtime();
    let inspection = package::inspect(&runtime, &text(&subject)).unwrap();
    assert_eq!(inspection.dependencies[0].id, "org.example.base");
    assert_eq!(inspection.dev_dependencies.len(), 1);
    assert_eq!(inspection.dev_dependencies[0].id, "org.example.fixtures");
    assert_eq!(inspection.dev_dependencies[0].requirement, "1");

    // Over a package path, handed over outright, and offered as candidates by location.
    let (base, fixtures) = (text(&path.join("base")), text(&path.join("fixtures")));
    for (packages, search) in [
        (
            vec![],
            package::Search {
                paths: vec![text(&path)],
                ..Default::default()
            },
        ),
        (vec![base.clone(), fixtures.clone()], package::Search::default()),
        (
            vec![],
            package::Search {
                candidates: vec![base, fixtures],
                ..Default::default()
            },
        ),
    ] {
        let report = package::test_with(&options, &subject, &temp.path().join("scratch"), &packages, &search).unwrap();
        assert_eq!(report.providers.len(), 2, "{report:#?}");
        assert_eq!((report.passed, report.failed), (4, 0), "{report:#?}");
        assert_eq!(report.providers[0].kind, "source");
        assert_eq!(report.providers[1].kind, "jpk");
    }

    // Without its providers a tested package doesn't resolve, and the chain says who asked.
    let alone = package::test(&options, &subject, &temp.path().join("scratch"))
        .unwrap_err()
        .to_string();
    assert!(
        alone.contains("org.example.subject 1.0.0 requires org.example.base 1"),
        "{alone}"
    );
    let half = package::test_with(
        &options,
        &subject,
        &temp.path().join("scratch"),
        &[text(&path.join("base"))],
        &package::Search::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        half.contains("org.example.subject 1.0.0 requires org.example.fixtures 1 for its tests"),
        "{half}"
    );
    assert!(
        half.contains("resolution failed: org.example.fixtures is not available"),
        "{half}"
    );

    // Outside its tests a package's dev-dependencies are nobody's.
    let search = package::Search {
        paths: vec![text(&path)],
        ..Default::default()
    };
    let plan = package::graph_with(&runtime, &[text(&subject)], &strings(&["example.subject"]), &search).unwrap();
    assert_eq!(
        plan.packages.iter().map(|package| package.id.as_str()).collect::<Vec<_>>(),
        ["org.example.base", "org.example.subject"]
    );
}

/// A command target's entry runs as a script, with its package's dependencies resolved and
/// exactly the capabilities the target declares, once the host grants them.
#[test]
fn a_command_target_runs_with_what_it_declares() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("packages");
    let tool = temp.path().join("tool");
    library(&path.join("base"), "base", "", "return { name = 'base' }\n");
    write(
        &tool,
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.tool\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
                 default_target = \"report\"\ninclude = [\"src\"]\n[dependencies]\n\"org.example.base\" = \"1\"\n\
                 [targets.report]\nkind = \"command\"\nentry = \"src/report.jsl\"\ncapabilities = [\"filesystem.read\"]\n\
                 [targets.quiet]\nkind = \"command\"\nentry = \"src/quiet.jsl\"\n\
                 [targets.editor]\nkind = \"application\"\nentry = \"src/quiet.jsl\"\n",
            ),
            (
                "src/report.jsl",
                "local fs = require '@dream/fs'\nlocal first, second = ...\n\
                 assert(first == arg[1] and second == arg[2] and #arg == 2)\n\
                 assert(require('./shared') == 'shared', 'an entry is its package\\'s module')\n\
                 assert(fs.readFileString(first), 'the target was granted what it declares')\n\
                 assert(not pcall(fs.writeFile, first, 'x'), 'and nothing it does not declare')\n\
                 return if require('@base').name == second then 7 else 1\n",
            ),
            ("src/shared.jsl", "return 'shared'\n"),
            (
                "src/quiet.jsl",
                "assert(not pcall(require('@dream/fs').readFileString, (...)), 'no capability, no filesystem')\n",
            ),
        ],
    );
    let options = jess::lua::Options::default();
    let request = |target: Option<&str>, grant: Option<&[&str]>| package::TargetRequest {
        package: text(&tool),
        target: target.map(ToOwned::to_owned),
        packages: Vec::new(),
        paths: vec![text(&path)],
        grant: grant.map(strings),
        args: vec![text(&tool.join("jess.toml")), "base".to_owned()],
        ..Default::default()
    };
    let failure = |target: Option<&str>, grant: Option<&[&str]>| -> String {
        match package::run_target(&options, &request(target, grant)) {
            Ok(run) => panic!("expected a failure, got {run:?}"),
            Err(error) => error.to_string(),
        }
    };
    let status = |run: &package::TargetRun| -> Option<f64> {
        let value = run.values.first()?;
        run.runtime
            .stack()
            .with_frame(|frame| value.push_to(frame)?.read::<f64>())
            .ok()
    };

    // The default target, trusted and then granted by name.
    for grant in [None, Some(&["filesystem.read", "filesystem.write"][..])] {
        let run = package::run_target(&options, &request(None, grant)).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(status(&run), Some(7.0));
    }
    let quiet = package::run_target(&options, &request(Some("quiet"), Some(&[]))).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(quiet.values.len(), 0);

    let denied = failure(None, Some(&["filesystem.write"]));
    assert!(denied.contains("cannot run target report"), "{denied}");
    assert!(
        denied.contains("required capability denied:\n    filesystem.read"),
        "{denied}"
    );
    assert!(
        denied.contains("requested by:\n    org.example.tool 1.0.0\n    target report"),
        "{denied}"
    );
    let unknown = failure(Some("nowhere"), None);
    assert!(
        unknown.contains("org.example.tool has no target 'nowhere'; it has: editor, quiet, report"),
        "{unknown}"
    );
    let application = failure(Some("editor"), None);
    assert!(
        application.contains("target 'editor' of org.example.tool is an application target"),
        "{application}"
    );
    let inspection = package::inspect(&common::runtime(), &text(&path.join("base"))).unwrap();
    let none = package::target(&inspection, None).unwrap_err().to_string();
    assert!(
        none.contains("org.example.base has no default_target to run; it declares no targets"),
        "{none}"
    );
}

/// A name nothing registered and no activated package exports is reported as that, whether or not
/// a plan is mounted, and a module jess does provide still loads.
#[test]
fn a_name_nothing_provides_is_reported_as_not_found() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    let bare = common::runtime();
    let planned = common::runtime();
    plan_of(&planned, &source, &["example.core"]);
    for runtime in [&bare, &planned] {
        common::run(
            runtime,
            "local ok, problem = pcall(require, '@nowhere/module')\n\
             local expected = \"module not found: @nowhere/module: no package activated in this run exports it, \
             and it isn't one of jess's own modules\"\n\
             assert(not ok and string.find(tostring(problem), expected, 1, true), tostring(problem))\n\
             assert(type(require('@dream/bytes')) == 'table')",
        );
    }
}

/// A package pinned to a content digest is refused unless it has that digest, whether it is handed
/// over outright or found as a candidate, and before anything it holds runs.
#[test]
fn a_pinned_package_has_the_digest_it_is_pinned_to() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    write(
        &base,
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.base\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
                 include = [\"src\"]\n[modules]\n\"@base/start\" = \"src/start.jsl\"\n\
                 [[layer]]\nid = \"example.base\"\n[[layer.entrypoint]]\nmodule = \"@base/start\"\n",
            ),
            ("src/start.jsl", "error('the entrypoint ran')\n"),
        ],
    );
    let runtime = common::runtime();
    let digest = package::inspect(&runtime, &text(&base)).unwrap().digest;
    let pinned = |digest: &str, outright: bool| -> (Vec<String>, package::Search) {
        let (locations, candidates) = if outright {
            (vec![text(&base)], Vec::new())
        } else {
            (Vec::new(), vec![text(&base)])
        };
        let search = package::Search {
            candidates,
            pins: [("org.example.base".to_owned(), digest.to_owned())].into(),
            ..Default::default()
        };
        (locations, search)
    };
    for outright in [true, false] {
        let (locations, search) = pinned(&digest, outright);
        let plan = package::graph_with(&runtime, &locations, &strings(&["example.base"]), &search).unwrap();
        assert_eq!(plan.packages[0].digest, digest);
        // With the pin it has, activating it gets as far as running its entrypoint.
        let ran = package::activate_with(&common::runtime(), &locations, &strings(&["example.base"]), None, &search)
            .unwrap_err()
            .to_string();
        assert!(ran.contains("the entrypoint ran"), "{ran}");

        let (locations, search) = pinned("0123", outright);
        let refused = package::activate_with(&common::runtime(), &locations, &strings(&["example.base"]), None, &search)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains(&format!(
                "org.example.base 1.0.0 (source {}) has digest {digest}, not the 0123 it is pinned to",
                text(&base)
            )),
            "{refused}"
        );
    }
}

/// A Package Source holds the tests its manifest names, so its JPK carries them.
#[test]
fn a_source_whose_tests_are_outside_its_payload_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("outside");
    std::fs::create_dir_all(source.join("src")).unwrap();
    std::fs::create_dir_all(source.join("tests")).unwrap();
    std::fs::write(source.join("src/init.jsl"), "return {}\n").unwrap();
    std::fs::write(source.join("tests/run.jsl"), "return nil\n").unwrap();
    std::fs::write(
        source.join("jess.toml"),
        "manifest_version = 1\n[package]\nid = \"org.example.outside\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
         include = [\"src\"]\ntests = [\"tests/run.jsl\"]\n[modules]\n\"@outside\" = \"src/init.jsl\"\n\
         [[layer]]\nid = \"example.outside\"\nexports = [\"@outside\"]\n",
    )
    .unwrap();
    let error = package::inspect(&common::runtime(), &source.display().to_string()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("[package] tests names 'tests/run.jsl', which the payload doesn't hold"),
        "{error}"
    );
}

/// A module that is both a layer's export and its entrypoint is loaded by the activation, and
/// `require` hands that same value out afterwards, so it is frozen there as an export would be.
#[test]
fn an_export_its_entrypoint_loaded_first_is_frozen_too() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("beta");
    write(
        &root,
        &[
            (
                "jess.toml",
                r#"manifest_version = 1

[package]
id = "org.example.beta"
version = "0.1.0"
jess_api = "1"
include = ["src"]

[modules]
"@beta/api" = "src/api.jsl"

[[layer]]
id = "beta.api"
exports = ["@beta/api"]

[[layer.entrypoint]]
module = "@beta/api"
"#,
            ),
            ("src/api.jsl", "return { version = 1 }\n"),
        ],
    );
    let runtime = common::runtime();
    plan_of(&runtime, &root, &["beta.api"]);
    common::run(
        &runtime,
        "local api = require '@beta/api'\n\
         assert(table.isfrozen(api), 'the export is frozen')\n\
         assert(not pcall(function() api.version = 2 end), 'and refuses a write')",
    );
}

/// A library whose module has a type error of its own, and an app that requires it, a relative
/// module, and a built-in export, with one type error in its tests.
fn checked(root: &Path) {
    write(
        &root.join("lib"),
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.lib\"\nversion = \"0.1.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n\
                 [modules]\n\"@lib\" = \"src/init.jsl\"\n[[layer]]\nid = \"lib.core\"\nexports = [\"@lib\"]\n",
            ),
            (
                "src/init.jsl",
                "--!strict\nlocal wrong: number = 'not a number'\nlocal function double(value: number): number return value * 2 end\nreturn { double = double, wrong = wrong }\n",
            ),
        ],
    );
    write(
        &root.join("app"),
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.app\"\nversion = \"0.1.0\"\njess_api = \"1\"\ninclude = [\"src\", \"tests\"]\n\
                 tests = [\"tests/run.jsl\"]\n[dependencies]\n\"org.example.lib\" = \"0.1\"\n[modules]\n\"@app\" = \"src/init.jsl\"\n\
                 [[layer]]\nid = \"app.core\"\nrequires = [\"lib.core\"]\nexports = [\"@app\"]\n",
            ),
            (
                "src/init.jsl",
                "--!strict\nlocal lib = require '@lib'\nlocal helper = require '@self/helper'\nlocal json = require '@jess/json'\n\
                 return { twice = function(value: number): number return helper.add(lib.double(value), 0) end, encode = json.encodeTable }\n",
            ),
            (
                "src/helper.jsl",
                "--!strict\nreturn { add = function(left: number, right: number): number return left + right end }\n",
            ),
            (
                "tests/run.jsl",
                "--!strict\nlocal app = require '@app'\nlocal text: string = app.twice(2)\nreturn require('@jess/test').new()\n",
            ),
        ],
    );
}

/// `jess package check` reports the package's own type errors, in its tests too, with requires
/// resolved as the runtime resolves them: the dependency's errors are the dependency's, and a
/// dependency packed into a JPK checks the same as its source.
#[test]
fn a_check_reports_a_package_s_own_type_errors_with_its_requires_resolved() {
    let temp = tempfile::tempdir().unwrap();
    checked(temp.path());
    let options = jess::lua::Options::default();
    let packed = temp.path().join("lib.jpk");
    package::pack(
        &common::runtime(),
        &temp.path().join("lib").display().to_string(),
        &packed.display().to_string(),
    )
    .unwrap();
    for dependency in [temp.path().join("lib"), packed] {
        let report = jess::lua::check::check_with(
            &options,
            &temp.path().join("app"),
            &[dependency.display().to_string()],
            &package::Search::default(),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!((report.id.as_str(), report.files), ("org.example.app", 3), "{report:#?}");
        let errors = report
            .errors
            .iter()
            .map(|error| (error.file.as_str(), error.line))
            .collect::<Vec<_>>();
        assert_eq!(errors, [("tests/run.jsl", 3)], "{report:#?}");
    }
}

/// A package's module can hand out its own payload with `jess.payload()`: every path and a read
/// of each, from a Package Source and from the same package packed. A script has none, and
/// nothing reads another package's payload uninvited.
#[test]
fn a_package_hands_out_its_own_payload() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    write(
        root,
        &[
            (
                "jess.toml",
                r#"manifest_version = 1

[package]
id = "org.example.theme"
version = "1.0.0"
jess_api = "1"
include = ["src", "templates"]

[modules]
"@theme" = "src/init.jsl"

[[layer]]
id = "example.theme"
exports = ["@theme"]
"#,
            ),
            (
                "src/init.jsl",
                "local payload = jess.payload()\nreturn { files = payload.files, read = payload.read }\n",
            ),
            ("templates/base.html", "<html>{{ body }}</html>\n"),
        ],
    );
    let check = "local theme = require('@theme')\n\
                 assert(theme.files[1] == 'jess.toml', theme.files[1])\n\
                 assert(table.find(theme.files, 'templates/base.html'), 'the template is in the payload')\n\
                 assert(buffer.tostring(theme.read('templates/base.html')) == '<html>{{ body }}</html>\\n')\n\
                 assert(theme.read('templates/missing.html') == nil)\n\
                 local ok, message = pcall(function() return jess.payload() end)\n\
                 assert(not ok and string.find(message, 'not a package', 1, true), message)\n\
                 local direct, reason = pcall(jess.payload)\n\
                 assert(not direct and string.find(reason, 'through no module', 1, true), reason)";
    let runtime = common::runtime();
    package::activate_with(
        &runtime,
        &[text(root)],
        &strings(&["example.theme"]),
        None,
        &package::Search::default(),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    common::run(&runtime, check);

    let packed = root.join("theme.jpk");
    package::pack(&common::runtime(), &text(root), &text(&packed)).unwrap();
    let runtime = common::runtime();
    package::activate_with(
        &runtime,
        &[text(&packed)],
        &strings(&["example.theme"]),
        None,
        &package::Search::default(),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    common::run(&runtime, check);

    let script = common::runtime();
    let refused = script.eval::<()>("jess.payload()").unwrap_err().to_string();
    assert!(
        refused.contains("has no payload") || refused.contains("not a package"),
        "{refused}"
    );
}

/// A version is `major.minor.patch`, with a pre-release after a hyphen. A pre-release meets only the
/// exact requirement that names it, so no range picks a development build; a malformed version or
/// requirement is refused where it is read.
#[test]
fn a_version_may_be_a_pre_release_that_only_an_exact_pin_picks() {
    let runtime = common::runtime();
    common::run(
        &runtime,
        "local package = require '@jess/package'\n\
         local function refused(expected, ...)\n  \
           local ok, problem = pcall(package.satisfies, ...)\n  \
           assert(not ok and string.find(tostring(problem), expected, 1, true), tostring(problem))\n\
         end\n\
         assert(package.satisfies('1.2.1-dev.57', '=1.2.1-dev.57'))\n\
         assert(package.satisfies('1.2.1-dev.57', '=1.2.1-dev.57') == true)\n\
         assert(not package.satisfies('1.2.1-dev.57', '^1.2'))\n\
         assert(not package.satisfies('1.2.1-dev.57', '1'))\n\
         assert(not package.satisfies('1.2.1-dev.57', '=1.2.1'))\n\
         assert(not package.satisfies('1.2.1', '=1.2.1-dev.57'))\n\
         assert(not package.satisfies('1.2.1-dev.58', '=1.2.1-dev.57'))\n\
         assert(not package.satisfies('1.2.1-alpha', '=1.2.1-dev.57'))\n\
         assert(package.satisfies('1.2.1', '^1.2'))\n\
         refused(\"'1.2' is not a version: major.minor.patch, with -identifiers for a pre-release\", '1.2', '1')\n\
         refused('is not a version: major.minor.patch', '1.2.3-', '1')\n\
         refused('a pre-release is dot-separated identifiers of ASCII letters, digits and hyphens', '1.2.3-a..b', '1')\n\
         refused('a pre-release is dot-separated identifiers', '1.2.3-dev.5!', '1')\n\
         refused('a numeric pre-release identifier has no leading zero', '1.2.3-dev.07', '1')\n\
         refused('is not a version requirement (^1.2, =1.2.3, =1.2.3-dev.4)', '1.2.3', '^1.2.3-dev.1')\n\
         refused('is not a version requirement', '1.2.3', '1.2.3-dev.1')\n\
         refused('a numeric pre-release identifier has no leading zero', '1.2.3', '=1.2.3-00')\n\
         local manifest = package.manifest('manifest_version = 1\\n[package]\\nid = \"org.example.pre\"\\n\
         version = \"0.3.1-dev.57\"\\njess_api = \"1\"\\ninclude = [\"src\"]\\n')\n\
         assert(manifest.version == '0.3.1-dev.57')\n\
         local ok, problem = pcall(package.manifest, 'manifest_version = 1\\n[package]\\nid = \"org.example.pre\"\\n\
         version = \"0.3.1-dev.07\"\\njess_api = \"1\"\\ninclude = [\"src\"]\\n')\n\
         assert(not ok and string.find(tostring(problem), 'package version', 1, true), tostring(problem))",
    );
}

/// Packing at another version packs the same payload with the manifest's version line revised: the
/// JPK reads as that version, with a digest of its own, and the source is untouched.
#[test]
fn a_source_packs_at_another_version_as_a_development_build() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("alpha");
    alpha(&source);
    let capabilities = ["filesystem.read".to_owned(), "filesystem.write".to_owned()];
    let runtime = jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &capabilities)
        .unwrap_or_else(|error| panic!("{error}"));
    let manifest_before = std::fs::read_to_string(source.join("jess.toml")).unwrap();
    let release = temp.path().join("release.jpk");
    let development = temp.path().join("development.jpk");
    let released = package::pack(&runtime, &text(&source), &text(&release)).unwrap();
    let developed = package::pack_as(&runtime, &text(&source), &text(&development), Some("1.2.4-dev.7")).unwrap();
    assert_eq!(
        (released.version.as_str(), developed.version.as_str()),
        ("1.2.3", "1.2.4-dev.7")
    );
    assert_eq!(released.files, developed.files);
    assert_ne!(released.digest, developed.digest);
    assert_eq!(std::fs::read_to_string(source.join("jess.toml")).unwrap(), manifest_before);

    let opened = package::inspect(&runtime, &text(&development)).unwrap();
    assert_eq!(
        (opened.id.as_str(), opened.version.as_str()),
        ("org.example.alpha", "1.2.4-dev.7")
    );
    assert_eq!(opened.digest, developed.digest);
    assert_eq!(package::inspect(&runtime, &text(&source)).unwrap().version, "1.2.3");

    // The same from a script, through the options table, and the same digest: a development build
    // is a function of the source and the version.
    common::set_global(&runtime, "source", text(&source).as_str());
    common::set_global(&runtime, "output", text(&temp.path().join("again.jpk")).as_str());
    common::set_global(&runtime, "digest", developed.digest.as_str());
    common::run(
        &runtime,
        "local package = require '@jess/package'\n\
         local packed = package.pack(source, output, { version = '1.2.4-dev.7' })\n\
         assert(packed.version == '1.2.4-dev.7' and packed.digest == digest, packed.digest)\n\
         assert(package.open(output).manifest.version == '1.2.4-dev.7')\n\
         local ok, problem = pcall(package.pack, source, output, { version = '1.2' })\n\
         assert(not ok and string.find(tostring(problem), 'is not a version', 1, true), tostring(problem))",
    );

    // A manifest whose version isn't on a `version = "..."` line of [package] can't be revised.
    let odd = temp.path().join("odd");
    alpha(&odd);
    let text_of = std::fs::read_to_string(odd.join("jess.toml")).unwrap();
    std::fs::write(
        odd.join("jess.toml"),
        text_of.replace("version = \"1.2.3\"\n", "version = '1.2.3'\n"),
    )
    .unwrap();
    let refused = package::pack_as(
        &runtime,
        &text(&odd),
        &text(&temp.path().join("odd.jpk")),
        Some("1.2.4-dev.1"),
    )
    .unwrap_err()
    .to_string();
    assert!(
        refused.contains("to pack at another version, [package] needs a line version = \"...\""),
        "{refused}"
    );
}
