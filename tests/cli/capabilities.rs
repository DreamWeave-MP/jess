//! Capabilities along the real CLI paths: what `jess run` holds without asking, what it needs a
//! flag for, and that a package target holds what it declares and nothing the request adds.

use crate::common::*;

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_misspelled_capability_is_refused_instead_of_granting_nothing() {
    let root = temp_dir_path("capability-misspelled");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("script.jsl"), "print('ran')\n").unwrap();

    let output = jess()
        .args(["run", "--capability", "filesystem.raed"])
        .arg(root.join("script.jsl"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("unknown capability 'filesystem.raed'; a jess runtime can be granted"),
        "{}",
        stderr(&output)
    );
    assert!(stderr(&output).contains("filesystem.read"), "{}", stderr(&output));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn run_holds_the_terminal_and_input_and_reads_files_only_when_granted() {
    let root = temp_dir_path("capability-run");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(root.join("l10n/common")).unwrap();
    fs::write(root.join("l10n/common/en.toml"), "hello = 'Hello, {name}!'\n").unwrap();
    // The user's own terminal and devices: no flag needed.
    fs::write(
        root.join("host.jsl"),
        "local process = require('@dream/process')\n\
         local runtime = jess.input.runtime()\n\
         runtime:connect 'keyboard'\n\
         process.write('stdout', 'held\\n')\n",
    )
    .unwrap();
    // A folder of bundles is read through @dream/fs, so it needs filesystem.read like any read.
    fs::write(
        root.join("l10n.jsl"),
        "local l10n = require('@jess/l10n')\n\
         l10n.addDirectory(...)\n\
         l10n.reload()\n\
         print(l10n.bundle 'common'('hello', { name = 'Fargoth' }))\n",
    )
    .unwrap();

    let held = jess().arg("run").arg(root.join("host.jsl")).output().unwrap();
    assert!(held.status.success(), "{}", stderr(&held));
    assert_eq!(String::from_utf8(held.stdout).unwrap(), "held\n");

    let denied = jess()
        .arg("run")
        .arg(root.join("l10n.jsl"))
        .arg(root.join("l10n"))
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert!(
        stderr(&denied).contains("needs the 'filesystem.read' capability, which this runtime does not grant"),
        "{}",
        stderr(&denied)
    );

    let granted = jess()
        .args(["run", "--capability", "filesystem.read"])
        .arg(root.join("l10n.jsl"))
        .arg(root.join("l10n"))
        .output()
        .unwrap();
    assert!(granted.status.success(), "{}", stderr(&granted));
    assert_eq!(String::from_utf8(granted.stdout).unwrap(), "Hello, Fargoth!\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_worker_needs_a_domain_and_says_so_under_run() {
    let root = temp_dir_path("capability-worker");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("worker.jsl"),
        "require('@jess/worker').spawn { source = 'return function() end' }\n",
    )
    .unwrap();

    let output = jess().arg("run").arg(root.join("worker.jsl")).output().unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("worker.spawn: needs a ScriptDomain, and none owns this runtime"),
        "{}",
        stderr(&output)
    );
    fs::remove_dir_all(root).unwrap();
}

/// A package whose target declares only the terminal, and whose layer's entrypoint needs the
/// environment: neither its entry nor its entrypoint may hold more than the target declares.
fn narrow_tool(root: &Path, entrypoint: bool) {
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let (exports, layer) = if entrypoint {
        (
            "",
            "[[layer.entrypoint]]\nmodule = \"@narrow/start\"\ncapabilities = [\"process.environment\"]\n",
        )
    } else {
        ("\"@narrow/start\"", "")
    };
    write(
        "jess.toml",
        &format!(
            r#"manifest_version = 1
[package]
id = "org.example.narrow"
version = "1.0.0"
jess_api = "1"
default_target = "go"
include = ["src"]
[modules]
"@narrow/start" = "src/start.jsl"
[[layer]]
id = "example.narrow"
exports = [{exports}]
{layer}[targets.go]
kind = "command"
entry = "src/go.jsl"
capabilities = ["process.stdio"]
"#
        ),
    );
    write("src/start.jsl", "return nil\n");
    write(
        "src/go.jsl",
        "local process = require('@dream/process')\n\
         process.write('stdout', 'go\\n')\n\
         local ok, problem = pcall(function() jess.input.runtime():connect 'keyboard' end)\n\
         process.write('stdout', `{ok} {problem}\\n`)\n",
    );
}

#[test]
fn a_target_holds_what_it_declares_and_nothing_a_request_adds() {
    let root = temp_dir_path("capability-target");
    drop(fs::remove_dir_all(&root));
    narrow_tool(&root, false);

    // The terminal is the person's: a target that declares process.stdio prints without asking.
    for extra in [
        &[][..],
        &["--trusted"][..],
        &["--capability", "process.stdio", "--capability", "input.configure"][..],
    ] {
        let output = jess().args(["package", "run"]).arg(&root).args(extra).output().unwrap();
        assert!(output.status.success(), "{extra:?}: {}", stderr(&output));
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "go\nfalse jess.input: needs the 'input.configure' capability, which this runtime does not grant\n",
            "{extra:?}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_entrypoint_needing_more_than_its_target_declares_is_refused_at_activation() {
    let root = temp_dir_path("capability-entrypoint");
    drop(fs::remove_dir_all(&root));
    narrow_tool(&root, true);

    for extra in [
        &["--trusted"][..],
        &["--capability", "process.stdio", "--capability", "process.environment"][..],
    ] {
        let output = jess().args(["package", "run"]).arg(&root).args(extra).output().unwrap();
        assert!(!output.status.success(), "{extra:?}");
        assert!(
            stderr(&output)
                .contains("cannot activate example.narrow\n\nrequired capability denied:\n    process.environment"),
            "{extra:?}: {}",
            stderr(&output)
        );
    }
    fs::remove_dir_all(root).unwrap();
}

/// A script whose path reads like a package module's (`<id>@<version>/...`) is still a loose
/// script: it gets no package's scope, so the built-in package's unexported modules stay out of reach.
#[test]
fn a_script_named_like_a_package_module_is_a_loose_script() {
    let root = temp_dir_path("capability-chunk-name");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(root.join("org.dreamweave.jess@0.1.0")).unwrap();
    fs::write(
        root.join("org.dreamweave.jess@0.1.0/x.jsl"),
        "local ok, problem = pcall(require, './src/data/common')\nprint(ok, problem)\n",
    )
    .unwrap();

    let output = jess()
        .current_dir(&root)
        .args(["run", "org.dreamweave.jess@0.1.0/x.jsl"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let printed = String::from_utf8(output.stdout).unwrap();
    assert!(printed.starts_with("false\tmodule not found: ./src/data/common"), "{printed}");
    fs::remove_dir_all(root).unwrap();
}
