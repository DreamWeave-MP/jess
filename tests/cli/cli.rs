//! Top-level commands and generic script execution.

use crate::common;

use common::*;

#[test]
fn top_level_generators_write_to_stdout() {
    let completions = Command::new(env!("CARGO_BIN_EXE_jess"))
        .args(["completions", "bash"])
        .output()
        .unwrap();
    assert!(
        completions.status.success(),
        "{}",
        String::from_utf8_lossy(&completions.stderr)
    );
    let stdout = String::from_utf8(completions.stdout).unwrap();
    assert!(stdout.contains("jess"));
    assert!(stdout.contains("package"));
    assert!(stdout.contains("run"));
    for name in ["plugin", "save", "nif"] {
        assert!(!stdout.contains(&format!("jess-{name}")), "{stdout}");
    }

    let man = Command::new(env!("CARGO_BIN_EXE_jess")).args(["man"]).output().unwrap();
    assert!(man.status.success(), "{}", String::from_utf8_lossy(&man.stderr));
    let stdout = String::from_utf8(man.stdout).unwrap();
    assert!(stdout.contains("jess"));
    assert!(stdout.contains("Run Luau applications and manage packages"));

    let folder = temp_dir_path("man-pages");
    drop(fs::remove_dir_all(&folder));
    let pages = Command::new(env!("CARGO_BIN_EXE_jess"))
        .args(["--format", "json", "man", "--output"])
        .arg(&folder)
        .output()
        .unwrap();
    assert!(pages.status.success(), "{}", String::from_utf8_lossy(&pages.stderr));
    let dump = fs::read_to_string(folder.join("jess-package-inspect.1")).unwrap();
    assert!(dump.contains(concat!("\"jess ", env!("CARGO_PKG_VERSION"), "\"")), "{dump}");
    assert!(dump.contains("jess-package-inspect"));
    fs::remove_dir_all(&folder).unwrap();

    let folder = temp_dir_path("luau-types");
    drop(fs::remove_dir_all(&folder));
    let types = Command::new(env!("CARGO_BIN_EXE_jess"))
        .args(["--format", "json", "luau-types"])
        .arg(&folder)
        .output()
        .unwrap();
    assert!(types.status.success(), "{}", String::from_utf8_lossy(&types.stderr));
    let report: serde_json::Value = serde_json::from_slice(&types.stdout).unwrap();
    assert_eq!(
        report["aliases"]["jess"],
        format!("{}/jess", folder.file_name().unwrap().to_string_lossy())
    );
    let definitions = fs::read_to_string(folder.join("jess.d.jsl")).unwrap();
    assert!(definitions.contains("declare jess: {"));
    assert!(
        fs::read_to_string(folder.join("dream/fs.jsl"))
            .unwrap()
            .contains("local module")
    );
    fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn run_executes_jsl_scripts_with_arguments() {
    let root = temp_dir_path("run-script");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(root.join("tools")).unwrap();
    fs::write(root.join("tools/greet.jsl"), "return function(name) return 'hi ' .. name end").unwrap();
    fs::write(
        root.join("script.jsl"),
        "local name, value = ...\nlocal greet = require('./tools/greet')\nprint(greet(name), value, arg[0] ~= nil)\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_jess"))
        .arg("run")
        .arg(root.join("script.jsl"))
        .arg("jess")
        .arg("generic argument")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "hi jess\tgeneric argument\ttrue\n");

    fs::write(
        root.join("write.jsl"),
        "local process = require('@dream/process')\nprocess.write('stdout', 'raw')\nprocess.write('stderr', 'err')\n",
    )
    .unwrap();
    let written = Command::new(env!("CARGO_BIN_EXE_jess"))
        .arg("run")
        .arg(root.join("write.jsl"))
        .output()
        .unwrap();
    assert!(written.status.success(), "{}", String::from_utf8_lossy(&written.stderr));
    assert_eq!(
        (written.stdout.as_slice(), written.stderr.as_slice()),
        (&b"raw"[..], &b"err"[..])
    );

    fs::write(root.join("failing.jsl"), "error('nope')").unwrap();
    let failed = Command::new(env!("CARGO_BIN_EXE_jess"))
        .args(["run"])
        .arg(root.join("failing.jsl"))
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("nope"));
    drop(fs::remove_dir_all(&root));
}

#[test]
fn run_requires_an_explicit_network_capability_grant() {
    let root = temp_dir_path("run-network-capability");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    let script = root.join("network.jsl");
    fs::write(
        &script,
        r#"
        local udp = require("@dream/udp")
        local schema = udp.schema {
            version = 1,
            channels = {{ name = "reliable", delivery = "reliableOrdered" }},
            events = {{ name = "ping", channel = "reliable", maxPayload = 8 }},
        }
        udp.client { schema = schema }
        print("transport granted")
        "#,
    )
    .unwrap();

    let denied = Command::new(env!("CARGO_BIN_EXE_jess"))
        .arg("run")
        .arg(&script)
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("network.transport"));

    let granted = Command::new(env!("CARGO_BIN_EXE_jess"))
        .args(["run", "--capability", "network.transport"])
        .arg(&script)
        .output()
        .unwrap();
    assert!(granted.status.success(), "{}", String::from_utf8_lossy(&granted.stderr));
    assert_eq!(String::from_utf8(granted.stdout).unwrap(), "transport granted\n");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn run_uses_the_first_numeric_script_result_as_its_exit_status() {
    let root = temp_dir_path("run-script-exit-status");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    let script = root.join("status.jsl");
    fs::write(&script, "return 23").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_jess"))
        .arg("run")
        .arg(&script)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(23), "{}", String::from_utf8_lossy(&output.stderr));

    fs::write(&script, "return 'not a status'").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jess"))
        .arg("run")
        .arg(&script)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    fs::write(&script, "return 256").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jess"))
        .arg("run")
        .arg(&script)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("0 to 255"));
    drop(fs::remove_dir_all(&root));
}

#[test]
fn run_can_profile_and_disassemble_luau_without_external_tools() {
    let root = temp_dir_path("run-luau-debug-tools");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    let script = root.join("debug.jsl");
    let disassembly = root.join("debug.bytecode.txt");
    let assembly = root.join("debug.asm");
    fs::write(
        &script,
        "local function sum(a, b) return a + b end\nfor i = 1, 1000 do sum(i, 1) end\nprint('debugged')\n",
    )
    .unwrap();

    let profiled = Command::new(env!("CARGO_BIN_EXE_jess"))
        .args(["run", "--profile"])
        .arg(&script)
        .output()
        .unwrap();
    assert!(profiled.status.success(), "{}", String::from_utf8_lossy(&profiled.stderr));
    let report = String::from_utf8(profiled.stderr).unwrap();
    assert!(report.contains("jess profile:"), "{report}");
    assert!(report.contains("safepoint samples"), "{report}");
    assert_eq!(String::from_utf8(profiled.stdout).unwrap(), "debugged\n");

    let dumped = Command::new(env!("CARGO_BIN_EXE_jess"))
        .args(["run", "--disassemble"])
        .arg(&disassembly)
        .arg(&script)
        .output()
        .unwrap();
    assert!(dumped.status.success(), "{}", String::from_utf8_lossy(&dumped.stderr));
    let listing = fs::read_to_string(&disassembly).unwrap();
    assert!(listing.contains("Function"), "{listing}");
    assert!(listing.contains("ADD"), "{listing}");
    assert!(listing.contains("sum"), "{listing}");

    let dumped = Command::new(env!("CARGO_BIN_EXE_jess"))
        .args(["run", "--emit-asm"])
        .arg(&assembly)
        .arg(&script)
        .output()
        .unwrap();
    assert!(dumped.status.success(), "{}", String::from_utf8_lossy(&dumped.stderr));
    assert_ne!(fs::read_to_string(&assembly).unwrap(), "");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn bare_jess_prints_the_help_that_starts_people_off() {
    for args in [&[][..], &["--help"][..]] {
        let output = jess().args(args).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(
            help.contains("Start here:") && help.contains("https://DreamWeave-MP.github.io/jess"),
            "{help}"
        );
    }
}

#[test]
fn native_format_commands_are_rejected() {
    for name in ["plugin", "save", "nif"] {
        let output = jess().args([name, "--help"]).output().unwrap();
        assert!(!output.status.success(), "{name}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
    }
}

#[test]
fn generic_reports_support_yaml_json_and_toml() {
    let root = temp_dir_path("generic-output-formats");
    drop(fs::remove_dir_all(&root));
    for format in ["yaml", "json", "toml"] {
        let folder = root.join(format);
        let output = jess()
            .args(["--format", format, "man", "--output"])
            .arg(&folder)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8(output.stdout).unwrap();
        let report: serde_json::Value = match format {
            "yaml" => serde_yaml::from_str(&text).unwrap(),
            "json" => {
                assert_eq!(text.lines().count(), 1);
                serde_json::from_str(&text).unwrap()
            }
            "toml" => serde_json::to_value(toml::from_str::<toml::Value>(&text).unwrap()).unwrap(),
            _ => unreachable!(),
        };
        let written = report["written"].as_array().unwrap();
        assert!(
            written
                .iter()
                .any(|path| path.as_str() == Some(folder.join("jess.1").to_str().unwrap()))
        );
        for name in ["plugin", "save", "nif"] {
            assert!(!folder.join(format!("jess-{name}.1")).exists());
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_package_source_and_jpk_use_the_same_public_commands() {
    let root = temp_dir_path("generic-package-providers");
    drop(fs::remove_dir_all(&root));
    let source = root.join("source");
    fs::create_dir_all(source.join("src")).unwrap();
    fs::write(
        source.join("jess.toml"),
        r#"manifest_version = 1
[package]
id = "org.example.cli"
version = "1.0.0"
jess_api = "1"
include = ["src"]
[modules]
"@cli/example" = "src/example.jsl"
[[layer]]
id = "cli.example"
exports = ["@cli/example"]
"#,
    )
    .unwrap();
    fs::write(source.join("src/example.jsl"), "return 'generic package'\n").unwrap();
    let packed = root.join("example.jpk");
    let output = jess()
        .args(["--format", "json", "pack"])
        .arg(&source)
        .arg("--output")
        .arg(&packed)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["id"], "org.example.cli");
    let script = root.join("script.jsl");
    fs::write(&script, "print(require('@cli/example'))\n").unwrap();
    for provider in [&source, &packed] {
        let inspection = jess()
            .args(["--format", "json", "package", "inspect"])
            .arg(provider)
            .output()
            .unwrap();
        assert!(inspection.status.success(), "{}", String::from_utf8_lossy(&inspection.stderr));
        let inspection: serde_json::Value = serde_json::from_slice(&inspection.stdout).unwrap();
        assert_eq!(inspection["id"], report["id"]);
        assert_eq!(inspection["digest"], report["digest"]);
        let output = jess().args(["run", "--package"]).arg(provider).arg(&script).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "generic package\n");
    }
    fs::remove_dir_all(root).unwrap();
}

/// A package path holding `base`, a tool package depending on it with a test and a command
/// target, and a script, all under `root`.
fn tool_over_a_package_path(root: &Path) {
    let write = |path: &str, contents: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    };
    write(
        "packages/base/jess.toml",
        r#"manifest_version = 1
[package]
id = "org.example.base"
version = "1.0.0"
jess_api = "1"
include = ["src"]
[modules]
"@base" = "src/init.jsl"
[[layer]]
id = "example.base"
exports = ["@base"]
"#,
    );
    write("packages/base/src/init.jsl", "return { name = 'base' }\n");
    write(
        "tool/jess.toml",
        r#"manifest_version = 1
[package]
id = "org.example.tool"
version = "1.0.0"
jess_api = "1"
default_target = "greet"
include = ["src", "tests"]
tests = ["tests/run.jsl"]
[dependencies]
"org.example.base" = "1"
[modules]
"@tool" = "src/init.jsl"
[[layer]]
id = "example.tool"
exports = ["@tool"]
[targets.greet]
kind = "command"
entry = "src/greet.jsl"
capabilities = ["process.environment"]
"#,
    );
    write(
        "tool/src/init.jsl",
        "return { greeting = `hello from {require('@base').name}` }\n",
    );
    write(
        "tool/src/greet.jsl",
        "print(require('@tool').greeting, ...)\nreturn if select('#', ...) == 2 then 3 else 0\n",
    );
    write(
        "tool/tests/run.jsl",
        "local test = require '@jess/test'\nlocal suite = test.new()\n\
         suite.it('greets', function() test.expect(require('@tool').greeting).toBe 'hello from base' end)\n\
         return suite\n",
    );
    write("script.jsl", "print(require('@base').name)\n");
}

#[test]
fn packages_resolve_over_a_package_path_and_run_their_targets() {
    let root = temp_dir_path("package-path");
    drop(fs::remove_dir_all(&root));
    tool_over_a_package_path(&root);
    let (tool, packages) = (root.join("tool"), root.join("packages"));

    let graph = jess()
        .args(["--format", "json", "package", "graph"])
        .arg(&tool)
        .arg("--package-path")
        .arg(&packages)
        .output()
        .unwrap();
    assert!(graph.status.success(), "{}", String::from_utf8_lossy(&graph.stderr));
    let plan: serde_json::Value = serde_json::from_slice(&graph.stdout).unwrap();
    assert_eq!(plan["packages"][0]["id"], "org.example.base");
    assert_eq!(plan["packages"][1]["dependencies"][0], "org.example.base");

    let alone = jess().args(["package", "test"]).arg(&tool).output().unwrap();
    assert!(!alone.status.success());
    assert!(
        String::from_utf8_lossy(&alone.stderr).contains("org.example.base is not available"),
        "{}",
        String::from_utf8_lossy(&alone.stderr)
    );
    for flag in ["--package-path", "--package"] {
        let provider = if flag == "--package" {
            packages.join("base")
        } else {
            packages.clone()
        };
        let tested = jess()
            .args(["--format", "json", "package", "test"])
            .arg(&tool)
            .arg(flag)
            .arg(provider)
            .output()
            .unwrap();
        assert!(tested.status.success(), "{}", String::from_utf8_lossy(&tested.stderr));
        let report: serde_json::Value = serde_json::from_slice(&tested.stdout).unwrap();
        assert_eq!((report["passed"].as_u64(), report["failed"].as_u64()), (Some(2), Some(0)));
    }

    let layered = jess()
        .args(["run", "--package-path"])
        .arg(&packages)
        .args(["--layer", "example.base"])
        .arg(root.join("script.jsl"))
        .output()
        .unwrap();
    assert!(layered.status.success(), "{}", String::from_utf8_lossy(&layered.stderr));
    assert_eq!(String::from_utf8(layered.stdout).unwrap(), "base\n");
    let unchosen = jess()
        .args(["run", "--package-path"])
        .arg(&packages)
        .arg(root.join("script.jsl"))
        .output()
        .unwrap();
    assert!(!unchosen.status.success());
    assert!(String::from_utf8_lossy(&unchosen.stderr).contains("name a layer with --layer"));

    let run = |extra: &[&str]| {
        jess()
            .args(["package", "run"])
            .arg(&tool)
            .arg("--package-path")
            .arg(&packages)
            .args(extra)
            .output()
            .unwrap()
    };
    let denied = run(&[]);
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("required capability denied:\n    process.environment"),
        "{}",
        String::from_utf8_lossy(&denied.stderr)
    );
    let trusted = run(&["--trusted", "one", "--two"]);
    assert_eq!(trusted.status.code(), Some(3), "{}", String::from_utf8_lossy(&trusted.stderr));
    assert_eq!(String::from_utf8(trusted.stdout).unwrap(), "hello from base\tone\t--two\n");
    let granted = run(&["--capability", "process.environment", "--target", "greet"]);
    assert_eq!(granted.status.code(), Some(0), "{}", String::from_utf8_lossy(&granted.stderr));
    assert_eq!(String::from_utf8(granted.stdout).unwrap(), "hello from base\n");
    fs::remove_dir_all(root).unwrap();
}

/// `jess run` leaves the runtime for the process's exit to reclaim, but closes what buffers first:
/// a writer the script never closed still leaves the whole file.
#[test]
fn run_finishes_a_writer_the_script_never_closed() {
    let root = temp_dir_path("run-unclosed-writer");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("write.jsl"),
        "local fs = require('@dream/fs')\nlocal out = fs.openWrite(...)\nout:write(string.rep('x', 100))\n",
    )
    .unwrap();

    let output = jess()
        .args(["run", "--capability", "filesystem.write"])
        .arg(root.join("write.jsl"))
        .arg(root.join("out.txt"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(root.join("out.txt")).unwrap(), vec![b'x'; 100]);
    fs::remove_dir_all(root).unwrap();
}

/// A package module's export is frozen once it returns: no one patches it for the whole plan.
#[test]
fn a_package_export_is_frozen() {
    let root = temp_dir_path("package-export-frozen");
    drop(fs::remove_dir_all(&root));
    tool_over_a_package_path(&root);
    fs::write(
        root.join("patch.jsl"),
        "local base = require('@base')\nprint(pcall(function() base.name = 'patched' end))\nprint(base.name)\n",
    )
    .unwrap();

    let output = jess()
        .args(["run", "--package-path"])
        .arg(root.join("packages"))
        .args(["--layer", "example.base"])
        .arg(root.join("patch.jsl"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let printed = String::from_utf8(output.stdout).unwrap();
    assert!(printed.starts_with("false\t"), "{printed}");
    assert!(printed.contains("readonly"), "{printed}");
    assert!(printed.ends_with("base\n"), "{printed}");
    fs::remove_dir_all(root).unwrap();
}

/// `jess package check` prints the package's type errors and fails on any, and passes a package
/// that checks.
#[test]
fn package_check_fails_on_a_type_error_and_passes_without_one() {
    let root = temp_dir_path("package-check");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("jess.toml"),
        "manifest_version = 1\n[package]\nid = \"org.example.checked\"\nversion = \"0.1.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n\
         [modules]\n\"@checked\" = \"src/init.jsl\"\n[[layer]]\nid = \"checked.core\"\nexports = [\"@checked\"]\n",
    )
    .unwrap();
    fs::write(
        root.join("src/init.jsl"),
        "--!strict\nlocal count: number = 'three'\nreturn { count = count }\n",
    )
    .unwrap();
    let failed = jess()
        .args(["--format", "json", "package", "check"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("1 type error\n"),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(
        (report["files"].as_u64(), report["errors"][0]["file"].as_str()),
        (Some(1), Some("src/init.jsl"))
    );
    assert_eq!(report["errors"][0]["line"].as_u64(), Some(2));

    fs::write(
        root.join("src/init.jsl"),
        "--!strict\nlocal count: number = 3\nreturn { count = count }\n",
    )
    .unwrap();
    let passed = jess()
        .args(["--format", "json", "package", "check"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(passed.status.success(), "{}", String::from_utf8_lossy(&passed.stderr));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scripts_must_be_jsl_files() {
    let root = temp_dir_path("jsl-only");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    let lua = root.join("script.lua");
    fs::write(&lua, "print(1)\n").unwrap();
    let message = "isn't a .jsl file: jess runs .jsl sources, and nothing else";

    let run = jess().arg("run").arg(&lua).output().unwrap();
    assert!(!run.status.success());
    assert!(
        String::from_utf8_lossy(&run.stderr).contains(message),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let test = jess().args(["--format", "json", "test"]).arg(&lua).output().unwrap();
    assert!(!test.status.success());
    let report: serde_json::Value = serde_json::from_slice(&test.stdout).unwrap();
    assert!(report["modules"][0]["error"].as_str().unwrap().contains(message), "{report}");

    let check = jess().arg("check").arg(&lua).output().unwrap();
    assert!(!check.status.success());
    assert!(
        String::from_utf8_lossy(&check.stderr).contains(message),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );

    fs::remove_dir_all(root).unwrap();
}
