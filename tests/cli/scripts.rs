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
    let path = root.join("jess-workspace.toml");
    fs::write(&path, format!("manifest_version = 1\n\n[workspace]\nmembers = []\n{scripts}")).unwrap();
    // Whatever umask the tests run under, the file is its owner's alone to write.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
}

const NAMED: &str = "\n[scripts]\npackages = [\"packages/greeter\"]\n";

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

/// The nearest workspace file is the one that says, and it is found from where the script is,
/// not from where the command was typed or from a link to the script.
#[test]
fn the_table_is_the_nearest_workspace_file_s_from_the_script_s_own_folder() {
    let root = folder("scripts-nearest", NAMED);
    let script = root.join("tools/deep/use.jsl");

    let relative = jess()
        .args(["run", "use.jsl"])
        .current_dir(root.join("tools/deep"))
        .output()
        .unwrap();
    assert!(relative.status.success(), "{}", String::from_utf8_lossy(&relative.stderr));

    #[cfg(unix)]
    {
        let elsewhere = temp_dir_path("scripts-nearest-link");
        drop(fs::remove_dir_all(&elsewhere));
        fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&script, elsewhere.join("linked.jsl")).unwrap();
        let (passed, stdout, stderr) = ran(&["run"], &elsewhere.join("linked.jsl"));
        assert!(passed && stdout.contains("world"), "{stderr}");
        fs::remove_dir_all(elsewhere).unwrap();
    }

    // A nearer file with no table says the scripts below it run with nothing.
    workspace_file(&root.join("tools"), "");
    let (passed, _, stderr) = ran(&["run"], &script);
    assert!(!passed && stderr.contains("@greeter"), "{stderr}");

    fs::remove_dir_all(root).unwrap();
}

/// A script run as a program holds everything, so no file it never named chooses code for it.
#[test]
fn a_script_run_as_a_program_reads_no_table() {
    let root = folder("scripts-program", NAMED);
    let script = root.join("tools/deep/use.jsl");

    let (passed, _, stderr) = ran(&["run", "--trusted"], &script);
    assert!(passed, "{stderr}");
    let (passed, _, stderr) = ran(&[], &script);
    assert!(!passed && stderr.contains("@greeter"), "{stderr}");

    fs::remove_dir_all(root).unwrap();
}

/// A workspace file anyone else may write is passed over, and a package a link leads out of the
/// workspace to is refused.
#[cfg(unix)]
#[test]
fn a_table_others_could_have_written_or_that_leaves_the_workspace_is_not_used() {
    use std::os::unix::fs::PermissionsExt;
    let root = folder("scripts-held", NAMED);
    let script = root.join("tools/deep/use.jsl");
    let file = root.join("jess-workspace.toml");

    for mode in [0o664, 0o646] {
        fs::set_permissions(&file, fs::Permissions::from_mode(mode)).unwrap();
        let (passed, _, stderr) = ran(&["run"], &script);
        assert!(!passed && stderr.contains("@greeter"), "{mode:o}: {stderr}");
    }

    let outside = temp_dir_path("scripts-held-outside");
    drop(fs::remove_dir_all(&outside));
    fs::rename(root.join("packages/greeter"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("packages/greeter")).unwrap();
    workspace_file(&root, NAMED);
    let (passed, _, stderr) = ran(&["run"], &script);
    assert!(!passed && stderr.contains("outside the workspace"), "{stderr}");

    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
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

    // A layer alone names something too, so it is the command line's error and not the table's.
    let (passed, _, stderr) = ran(&["run", "--layer", "example.greeter"], &script);
    assert!(!passed && stderr.contains("--layer needs packages"), "{stderr}");

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
        (
            "[scripts]\npackages = [\"packages/greeter\"]\n\n[later]\n",
            "has an unknown key 'later'",
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
