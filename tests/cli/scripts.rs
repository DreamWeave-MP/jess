//! Loose scripts under a workspace file's `[scripts]` table: `jess run`, `jess test` and
//! `jess check` with no package flags run with the packages the table names.

use super::common;

use common::*;

const USES: &str = "--!strict\nlocal greeter = require '@greeter'\nprint(greeter.greet 'world')\n";
const SUITE: &str = "local test = require('@jess/test')\nlocal greeter = require '@greeter'\nlocal suite = test.new()\n\
     suite.it('greets', function() test.expect(type(greeter.greet 'world')).toBe('string') end)\nreturn suite\n";

/// A folder holding a package under `packages/greeter`, a script and a test module two folders
/// down, and a workspace file whose `[scripts]` table is `scripts`.
fn folder(name: &str, scripts: &str) -> PathBuf {
    let root = temp_dir_path(name);
    drop(fs::remove_dir_all(&root));
    let init = jess()
        .args(["init", "--editor", "none", "--package", "org.example.greeter"])
        .arg(root.join("packages/greeter"))
        .output()
        .unwrap();
    assert!(init.status.success(), "{}", String::from_utf8_lossy(&init.stderr));
    fs::create_dir_all(root.join("tools/deep")).unwrap();
    fs::write(root.join("tools/deep/use.jsl"), USES).unwrap();
    fs::write(root.join("tools/deep/use.test.jsl"), SUITE).unwrap();
    workspace_file(&root, scripts);
    root
}

fn workspace_file(root: &Path, scripts: &str) {
    fs::write(
        root.join("jess-workspace.toml"),
        format!("manifest_version = 1\n\n[workspace]\nmembers = []\n{scripts}"),
    )
    .unwrap();
}

/// A command's success, standard output and standard error.
fn ran(arguments: &[&str], script: &Path) -> (bool, String, String) {
    let output = jess().args(arguments).arg(script).output().unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_loose_script_runs_tests_and_checks_with_the_packages_its_workspace_names() {
    let root = folder("scripts-named", "\n[scripts]\npackages = [\"packages/greeter\"]\n");
    let script = root.join("tools/deep/use.jsl");
    let module = root.join("tools/deep/use.test.jsl");

    let (passed, stdout, stderr) = ran(&["run"], &script);
    assert!(passed, "{stderr}");
    assert!(stdout.contains("world"), "{stdout}");

    let (passed, stdout, stderr) = ran(&["--format", "json", "test"], &module);
    assert!(passed, "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["passed"], 1, "{report}");

    let (passed, _, stderr) = ran(&["check"], &script);
    assert!(passed, "{stderr}");

    // Without the table the same commands know no such package.
    workspace_file(&root, "");
    let (passed, _, stderr) = ran(&["run"], &script);
    assert!(!passed && stderr.contains("@greeter"), "{stderr}");
    let (passed, _, _) = ran(&["test"], &module);
    assert!(!passed);
    let (passed, _, stderr) = ran(&["check"], &script);
    assert!(!passed && stderr.contains("type error"), "{stderr}");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_command_line_that_names_packages_is_the_whole_answer() {
    // The table names a package that isn't there, so reading it at all is an error.
    let root = folder("scripts-flags", "\n[scripts]\npackages = [\"packages/missing\"]\n");
    let script = root.join("tools/deep/use.jsl");
    let greeter = root.join("packages/greeter").display().to_string();

    let (passed, _, stderr) = ran(&["run"], &script);
    assert!(!passed && stderr.contains("missing"), "{stderr}");

    let (passed, stdout, stderr) = ran(&["run", "--package", &greeter], &script);
    assert!(passed, "{stderr}");
    assert!(stdout.contains("world"), "{stdout}");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_package_path_and_a_layer_are_named_as_the_flags_name_them() {
    let root = folder("scripts-path", "");
    let inspected = jess()
        .args(["--format", "json", "package", "inspect"])
        .arg(root.join("packages/greeter"))
        .output()
        .unwrap();
    let inspection: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    let layer = inspection["layers"][0]["id"].as_str().unwrap().to_owned();
    workspace_file(
        &root,
        &format!("\n[scripts]\npackage_path = [\"packages\"]\nlayers = [\"{layer}\"]\n"),
    );

    let (passed, stdout, stderr) = ran(&["run"], &root.join("tools/deep/use.jsl"));
    assert!(passed, "{stderr}");
    assert!(stdout.contains("world"), "{stdout}");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_scripts_table_nothing_could_use_is_refused() {
    let root = folder("scripts-refused", "");
    let script = root.join("tools/deep/use.jsl");
    for (table, message) in [
        ("[scripts]\n", "[scripts] names no packages"),
        (
            "[scripts]\npackages = [\"../outside\"]\n",
            "is not a path of names below the workspace",
        ),
        (
            "[scripts]\npackages = [\"/absolute\"]\n",
            "is not a path of names below the workspace",
        ),
        ("[scripts]\npackages = [1]\n", "[scripts] packages is a list of paths"),
        (
            "[scripts]\npackages = \"packages/greeter\"\n",
            "[scripts] packages is a list of paths",
        ),
        (
            "[scripts]\npackages = [\"packages/greeter\"]\nlayers = [\"\"]\n",
            "[scripts] layers is a list of layer ids",
        ),
        (
            "[scripts]\npackages = [\"packages/greeter\"]\nextra = 1\n",
            "[scripts] has an unknown key 'extra'",
        ),
    ] {
        workspace_file(&root, &format!("\n{table}"));
        let (passed, _, stderr) = ran(&["run"], &script);
        assert!(!passed && stderr.contains(message), "{table}: {stderr}");
        // The workspace says so too, whoever opens it.
        let info = jess()
            .args(["workspace", "--workspace"])
            .arg(&root)
            .arg("info")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&info.stderr);
        assert!(!info.status.success() && stderr.contains(message), "{table}: {stderr}");
    }
    fs::remove_dir_all(root).unwrap();
}
