//! Packed workspaces from the command line, over the showcase: `jess workspace pack`, `jess
//! app.jsw`, and a packed workspace named with `--workspace`.

use super::common::{Path, fs, jess};
use super::workspace::{Showcase, fixture, json};

/// The showcase with its JPKs synced into the lock, ready to pack.
fn synced(name: &str) -> Showcase {
    let showcase = Showcase::new(name);
    let jpks = showcase.root.join("jpks");
    fs::create_dir_all(&jpks).unwrap();
    let mut sync = jess();
    sync.current_dir(showcase.workspace()).args(["workspace", "sync"]);
    for name in ["words", "style"] {
        let jpk = jpks.join(format!("{name}.jpk"));
        let packed = jess()
            .arg("pack")
            .arg(fixture("workspace-vendor").join(name))
            .arg("--output")
            .arg(&jpk)
            .output()
            .unwrap();
        assert!(packed.status.success(), "{}", String::from_utf8_lossy(&packed.stderr));
        sync.arg(jpk);
    }
    let synced = sync.output().unwrap();
    assert!(synced.status.success(), "{}", String::from_utf8_lossy(&synced.stderr));
    fs::remove_dir_all(&jpks).unwrap();
    showcase
}

fn pack(workspace: &Path, output: &Path) -> serde_json::Value {
    json(
        &jess()
            .current_dir(workspace)
            .args(["--format", "json", "workspace", "pack", "--output"])
            .arg(output)
            .output()
            .unwrap(),
    )
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A packed workspace runs as a program from anywhere, the workspace gone: its default target as
/// `jess app.jsw`, any target it exports by the names `workspace run` takes, with what each
/// declares granted the way `workspace run` grants it.
#[test]
fn a_packed_workspace_runs_without_its_workspace() {
    let showcase = synced("jsw-runs");
    let workspace = showcase.workspace();
    let packed = showcase.root.join("showcase.jsw");
    let report = pack(&workspace, &packed);
    assert_eq!(report["default"], "org.example.showcase:showcase");
    assert_eq!(
        report["targets"],
        serde_json::json!(["org.example.greeter:greet", "org.example.showcase:showcase"])
    );
    let again = showcase.root.join("again.jsw");
    pack(&workspace, &again);
    assert_eq!(fs::read(&packed).unwrap(), fs::read(&again).unwrap());

    let listed = json(
        &jess()
            .args(["--format", "json", "workspace", "--workspace"])
            .arg(&packed)
            .arg("targets")
            .output()
            .unwrap(),
    );
    assert_eq!(listed[0]["target"], "org.example.greeter:greet");
    assert_eq!(listed[0]["capabilities"], serde_json::json!(["process.environment"]));
    assert_eq!(listed[1]["default"], true);

    fs::remove_dir_all(&workspace).unwrap();
    let elsewhere = showcase.root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let default = jess().current_dir(&elsewhere).arg(&packed).output().unwrap();
    assert!(default.status.success(), "{}", stderr(&default));
    assert_eq!(stdout(&default), "hello, showcase\n");

    let greet = |grant: &[&str]| {
        jess()
            .current_dir(&elsewhere)
            .args(["workspace", "--workspace"])
            .arg(&packed)
            .args(["run", "greet"])
            .args(grant)
            .args(["--", "one", "two"])
            .output()
            .unwrap()
    };
    let greeted = greet(&["--trusted"]);
    assert_eq!(greeted.status.code(), Some(2), "{}", stderr(&greeted));
    assert_eq!(stdout(&greeted), "HELLO, ONE! (hello)\nHELLO, TWO! (hello)\n");
    let denied = greet(&[]);
    assert!(!denied.status.success());
    assert_eq!(stdout(&denied), "");
    assert!(
        stderr(&denied).contains("required capability denied:\n    process.environment"),
        "{}",
        stderr(&denied)
    );
    let sealed = jess()
        .args(["workspace", "--workspace"])
        .arg(&packed)
        .arg("sync")
        .output()
        .unwrap();
    assert!(
        stderr(&sealed).contains("is a packed workspace, which is sealed"),
        "{}",
        stderr(&sealed)
    );
}

/// `[pack] default` picks what `jess app.jsw` runs, and running it as a program grants the target
/// what it declares, as `jess script.jsl` runs a script as a program.
#[test]
fn a_packed_workspace_run_as_a_program_holds_what_its_target_declares() {
    let showcase = synced("jsw-program");
    let workspace = showcase.workspace();
    let file = workspace.join("jess-workspace.toml");
    let listed = fs::read_to_string(&file).unwrap();
    fs::write(&file, format!("{listed}\n[pack]\ndefault = \"greet\"\n")).unwrap();
    let packed = showcase.root.join("greeter.jsw");
    pack(&workspace, &packed);
    let greeted = jess().arg(&packed).arg("you").output().unwrap();
    assert_eq!(greeted.status.code(), Some(1), "{}", stderr(&greeted));
    assert_eq!(stdout(&greeted), "HELLO, YOU! (hello)\n");
}
