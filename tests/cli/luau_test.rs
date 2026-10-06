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
