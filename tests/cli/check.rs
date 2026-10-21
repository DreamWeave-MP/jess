//! `jess check`: loose scripts type-checked in strict mode.

use super::common;

use common::*;
use serde_json::Value;

fn check(args: &[&str]) -> (bool, Value, String) {
    let output = jess().args(["--format", "json", "check"]).args(args).output().unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (
        output.status.success(),
        report,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn check_passes_a_clean_script_and_reports_errors_in_it_and_the_files_it_requires() {
    let root = temp_dir_path("check-scripts");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(
        root.join("clean.jsl"),
        "--!strict\nlocal json = require '@jess/json'\nlocal util = require './lib/util'\nprint(json.encodeTable({ answer = util.answer }))\n",
    )
    .unwrap();
    fs::write(root.join("lib/util.jsl"), "--!strict\nreturn { answer = 42 }\n").unwrap();
    fs::write(
        root.join("wrong.jsl"),
        "--!strict\nlocal count: number = 'three'\nlocal broken = require './lib/broken'\nprint(count, broken)\n",
    )
    .unwrap();
    fs::write(
        root.join("lib/broken.jsl"),
        "--!strict\nlocal answer: string = 42\nreturn answer\n",
    )
    .unwrap();

    let clean = root.join("clean.jsl").display().to_string();
    let (passed, report, stderr) = check(&[&clean]);
    assert!(passed, "{stderr}");
    assert_eq!(
        (report["files"].as_u64(), report["errors"].as_array().map(Vec::len)),
        (Some(1), Some(0))
    );

    let wrong = root.join("wrong.jsl").display().to_string();
    let (passed, report, stderr) = check(&[&wrong]);
    assert!(!passed);
    assert!(stderr.contains("2 type errors\n"), "{stderr}");
    // Errors sort by file, so the required file comes first or last depending on the folder's
    // name; both are reported, each on its line 2.
    let mut files = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| error["file"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    files.sort();
    let mut expected = [
        wrong.replace('\\', "/"),
        root.join("lib/broken.jsl").display().to_string().replace('\\', "/"),
    ];
    expected.sort();
    assert_eq!(files, expected, "{report}");
    for error in report["errors"].as_array().unwrap() {
        assert_eq!(error["line"].as_u64(), Some(2), "{error}");
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_types_a_named_package_s_exports() {
    let root = temp_dir_path("check-package");
    drop(fs::remove_dir_all(&root));
    let package = root.join("greeter");
    let init = jess()
        .args(["init", "--editor", "none", "--package", "org.example.greeter"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(init.status.success(), "{}", String::from_utf8_lossy(&init.stderr));
    fs::write(
        root.join("use.jsl"),
        "--!strict\nlocal greeter = require '@greeter'\nlocal count: number = greeter.greet 'world'\nprint(count)\n",
    )
    .unwrap();

    let script = root.join("use.jsl").display().to_string();
    let (passed, report, stderr) = check(&["--package", &package.display().to_string(), &script]);
    assert!(!passed);
    assert!(stderr.contains("1 type error\n"), "{stderr}");
    assert!(
        report["errors"][0]["message"].as_str().unwrap().contains("number"),
        "{report}"
    );

    let (passed, _, stderr) = check(&[&script]);
    assert!(!passed, "a package export the check wasn't told about must not type");
    assert!(stderr.contains("1 type error\n"), "{stderr}");

    fs::remove_dir_all(root).unwrap();
}
