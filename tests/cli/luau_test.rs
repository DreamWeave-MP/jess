//! `jess test` over Luau test modules.

use crate::common;

use common::*;

fn module(root: &Path, name: &str, source: &str) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, source).unwrap();
    path
}

#[test]
fn test_reports_every_case_and_fails_when_one_does() {
    let root = temp_dir_path("luau-test");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    let passing = module(
        &root,
        "passing.luau",
        "local test = require('@jess/test')\nlocal log = require('@jess/log')\nlocal suite = test.new()\n\
         suite.describe('numbers', function()\n  suite.it('add', function()\n    log.info('adding', { left = 1 })\n    test.expect(1 + 1).toBe(2)\n  end)\nend)\nreturn suite\n",
    );
    let failing = module(
        &root,
        "failing.luau",
        "local test = require('@jess/test')\nlocal suite = test.new()\nsuite.it('compares', function() test.expect({ 1 }).toEqual({ 2 }) end)\nreturn suite\n",
    );
    let broken = module(&root, "broken.luau", "return 'not a suite'\n");

    let output = jess().args(["--format", "json", "test"]).arg(&passing).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], 1);
    assert_eq!(report["failed"], 0);
    let case = &report["modules"][0]["report"]["tests"][0];
    assert_eq!(case["name"], "numbers / add");
    assert_eq!(case["passed"], true);
    assert_eq!(case["logs"][0]["message"], "adding");
    assert_eq!(case["logs"][0]["fields"]["left"], 1.0);

    let output = jess()
        .args(["--format", "json", "test"])
        .arg(&passing)
        .arg(&failing)
        .arg(&broken)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], 1);
    assert_eq!(report["failed"], 2);
    let compares = &report["modules"][1]["report"]["tests"][0];
    assert_eq!(compares["passed"], false);
    assert!(
        compares["error"].as_str().unwrap().contains("expected { 1 } to equal { 2 }"),
        "{compares}"
    );
    assert!(report["modules"][2]["report"].is_null());
    assert!(
        report["modules"][2]["error"]
            .as_str()
            .unwrap()
            .contains("must return a suite"),
        "{report}"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("2 failed"));
    drop(fs::remove_dir_all(&root));
}

/// A script reads and writes the host's files only with the capability for it, whether it runs
/// under `jess run`, as a `jess test` module, or as a package's command target.
#[test]
fn scripts_touch_files_only_with_the_capability_for_it() {
    let root = temp_dir_path("file-capabilities");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(root.join("tool/src")).unwrap();
    let note = root.join("note.txt");
    let script = module(
        &root,
        "script.luau",
        "local path = ...\nprint(jess.fs.read(path) or 'nothing')\njess.fs.write(path, 'written')\n",
    );
    let stderr = |output: &std::process::Output| String::from_utf8_lossy(&output.stderr).into_owned();

    let ungranted = jess().arg("run").arg(&script).arg(&note).output().unwrap();
    assert!(!ungranted.status.success());
    assert!(
        stderr(&ungranted).contains("jess.fs.read: needs the 'filesystem.read' capability"),
        "{}",
        stderr(&ungranted)
    );
    let reading = jess()
        .args(["run", "--capability", "filesystem.read"])
        .arg(&script)
        .arg(&note)
        .output()
        .unwrap();
    assert!(!reading.status.success());
    assert_eq!(String::from_utf8_lossy(&reading.stdout), "nothing\n");
    assert!(
        stderr(&reading).contains("jess.fs.write: needs the 'filesystem.write' capability"),
        "{}",
        stderr(&reading)
    );
    assert!(!note.exists(), "a script without filesystem.write wrote a file");
    let both = jess()
        .args(["run", "--capability", "filesystem.read", "--capability", "filesystem.write"])
        .arg(&script)
        .arg(&note)
        .output()
        .unwrap();
    assert!(both.status.success(), "{}", stderr(&both));
    assert_eq!(fs::read_to_string(&note).unwrap(), "written");

    let suite = module(
        &root,
        "suite.luau",
        &format!(
            "local test = require('@jess/test')\nlocal suite = test.new()\n\
             suite.it('reads', function() test.expect(jess.fs.read({:?})).toBe 'written' end)\nreturn suite\n",
            note.display().to_string()
        ),
    );
    let denied = jess().args(["--format", "json", "test"]).arg(&suite).output().unwrap();
    assert!(!denied.status.success());
    let report: serde_json::Value = serde_json::from_slice(&denied.stdout).unwrap();
    let case = &report["modules"][0]["report"]["tests"][0];
    assert!(
        case["error"]
            .as_str()
            .unwrap()
            .contains("needs the 'filesystem.read' capability"),
        "{case}"
    );
    let granted = jess()
        .args(["--format", "json", "test", "--capability", "filesystem.read"])
        .arg(&suite)
        .output()
        .unwrap();
    assert!(granted.status.success(), "{}", stderr(&granted));

    // A target that declares nothing is granted nothing, however it is trusted.
    fs::write(
        root.join("tool/jess.toml"),
        "manifest_version = 1\n[package]\nid = \"org.example.files\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
         default_target = \"quiet\"\ninclude = [\"src\"]\n\
         [targets.quiet]\nkind = \"command\"\nentry = \"src/main.luau\"\n\
         [targets.reader]\nkind = \"command\"\nentry = \"src/main.luau\"\ncapabilities = [\"filesystem.read\"]\n",
    )
    .unwrap();
    fs::write(root.join("tool/src/main.luau"), "print(jess.fs.read((...)))\n").unwrap();
    let target = |name: &str| {
        jess()
            .args(["package", "run"])
            .arg(root.join("tool"))
            .args(["--target", name, "--trusted"])
            .arg(&note)
            .output()
            .unwrap()
    };
    let quiet = target("quiet");
    assert!(!quiet.status.success());
    assert!(
        stderr(&quiet).contains("jess.fs.read: needs the 'filesystem.read' capability"),
        "{}",
        stderr(&quiet)
    );
    let reader = target("reader");
    assert!(reader.status.success(), "{}", stderr(&reader));
    assert_eq!(String::from_utf8_lossy(&reader.stdout), "written\n");
    drop(fs::remove_dir_all(&root));
}
