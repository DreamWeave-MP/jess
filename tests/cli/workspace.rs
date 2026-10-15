//! `jess workspace`, over the showcase in `fixtures/workspace`: a root package and a tool that
//! uses it, with the libraries they depend on in `fixtures/workspace-vendor`.

use crate::common::{Path, PathBuf, fs, jess, temp_dir_path};

/// Copies the folder `from` to `to`.
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cli/fixtures").join(name)
}

/// A copy of the showcase workspace in a folder of its own, removed when dropped.
struct Showcase {
    root: PathBuf,
}

impl Showcase {
    fn new(name: &str) -> Self {
        let root = temp_dir_path(name);
        drop(fs::remove_dir_all(&root));
        copy(&fixture("workspace"), &root.join("workspace"));
        Self { root }
    }

    fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }
}

impl Drop for Showcase {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.root));
    }
}

fn json(output: &std::process::Output) -> serde_json::Value {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// From a folder deep inside a member, `info` and `targets` show the workspace that member is in.
#[test]
fn a_workspace_shows_its_members_and_their_targets() {
    let showcase = Showcase::new("workspace-info");
    let inside = showcase.workspace().join("tools/greeter/src");
    let info = json(
        &jess()
            .current_dir(&inside)
            .args(["--format", "json", "workspace", "info"])
            .output()
            .unwrap(),
    );
    assert_eq!(info["kind"], "package");
    assert_eq!(info["members"][0]["path"], ".");
    assert_eq!(info["members"][1]["id"], "org.example.greeter");

    let targets = json(
        &jess()
            .args(["--format", "json", "workspace", "targets", "--workspace"])
            .arg(showcase.workspace())
            .output()
            .unwrap(),
    );
    // Members are by path, and `.` comes first.
    assert_eq!(targets[0]["target"], "org.example.showcase:showcase");
    assert_eq!(targets[0]["default"], true);
    assert_eq!(targets[1]["target"], "org.example.greeter:greet");
    assert_eq!(targets[1]["command"], "greet");
    assert_eq!(targets[1]["capabilities"][0], "process.environment");

    let nowhere = jess()
        .args(["workspace", "info", "--workspace"])
        .arg(showcase.root.join("missing"))
        .output()
        .unwrap();
    assert!(!nowhere.status.success());
    assert!(String::from_utf8_lossy(&nowhere.stderr).contains("is not there"));
}

/// A workspace of a tool and the library it uses, both loose sources, under `root`.
fn tool_and_library(root: &Path) {
    let write = |path: &str, contents: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    };
    write(
        "jess-workspace.toml",
        "manifest_version = 1\n[workspace]\nmembers = [\"tool\", \"lib\"]\n",
    );
    write(
        "lib/jess.toml",
        "manifest_version = 1\n[package]\nid = \"org.example.lib\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
         include = [\"src\"]\n[modules]\n\"@lib\" = \"src/init.jsl\"\n[[layer]]\nid = \"example.lib\"\nexports = [\"@lib\"]\n",
    );
    write("lib/src/init.jsl", "return { name = 'lib' }\n");
    write(
        "tool/jess.toml",
        "manifest_version = 1\n[package]\nid = \"org.example.tool\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
         default_target = \"greet\"\ninclude = [\"src\", \"tests\"]\ntests = [\"tests/run.jsl\"]\n\
         [dependencies]\n\"org.example.lib\" = \"1\"\n\
         [targets.greet]\nkind = \"command\"\nentry = \"src/greet.jsl\"\ncapabilities = [\"process.environment\"]\n",
    );
    write(
        "tool/src/greet.jsl",
        "print(require('@lib').name, ...)\nreturn select('#', ...)\n",
    );
    write(
        "tool/tests/run.jsl",
        "local test = require '@jess/test'\nlocal suite = test.new()\n\
         suite.it('sees lib', function() test.expect(require('@lib').name).toBe 'lib' end)\nreturn suite\n",
    );
}

/// From inside a member, `run` with no target runs that member's default target over the
/// workspace, with the arguments after `--`, exiting with what it returns, and with only what the
/// command line grants. `graph` resolves it without running it, and `test` runs the members' tests.
#[test]
fn a_workspace_target_runs_from_inside_its_member() {
    let root = temp_dir_path("workspace-run");
    drop(fs::remove_dir_all(&root));
    tool_and_library(&root);
    let inside = root.join("tool/src");
    let run = |extra: &[&str]| {
        jess()
            .current_dir(&inside)
            .args(["workspace", "run"])
            .args(extra)
            .output()
            .unwrap()
    };
    let denied = run(&["--", "one"]);
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("required capability denied:\n    process.environment"),
        "{}",
        String::from_utf8_lossy(&denied.stderr)
    );
    let trusted = run(&["--trusted", "--", "one", "--two"]);
    assert_eq!(trusted.status.code(), Some(2), "{}", String::from_utf8_lossy(&trusted.stderr));
    assert_eq!(String::from_utf8(trusted.stdout).unwrap(), "lib\tone\t--two\n");
    let granted = run(&["org.example.tool:greet", "--capability", "process.environment"]);
    assert_eq!(granted.status.code(), Some(0), "{}", String::from_utf8_lossy(&granted.stderr));

    let graph = json(
        &jess()
            .current_dir(&inside)
            .args(["--format", "json", "workspace", "graph"])
            .output()
            .unwrap(),
    );
    assert_eq!(graph["target"]["id"], "greet");
    assert_eq!(graph["plan"]["packages"][0]["id"], "org.example.lib");

    let tested = json(
        &jess()
            .current_dir(&root)
            .args(["--format", "json", "workspace", "test"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        (tested[0]["passed"].as_u64(), tested[0]["failed"].as_u64()),
        (Some(2), Some(0))
    );
    fs::remove_dir_all(root).unwrap();
}

/// The showcase, end to end: the libraries it needs are JPKs handed to `sync`, which pins them in
/// jess.lock and keeps them in .jess/packages. With the JPKs gone, the tool's target runs from
/// deep inside its member, offline, with its arguments and exit status, and only what is granted.
#[test]
fn the_showcase_syncs_its_jpks_and_runs_without_them() {
    let showcase = Showcase::new("workspace-showcase");
    let (workspace, jpks) = (showcase.workspace(), showcase.root.join("jpks"));
    fs::create_dir_all(&jpks).unwrap();
    for name in ["words", "style"] {
        let packed = jess()
            .arg("pack")
            .arg(fixture("workspace-vendor").join(name))
            .arg("--output")
            .arg(jpks.join(format!("{name}.jpk")))
            .output()
            .unwrap();
        assert!(packed.status.success(), "{}", String::from_utf8_lossy(&packed.stderr));
    }

    let unsynced = jess()
        .current_dir(&workspace)
        .args(["workspace", "run", "org.example.greeter:greet", "--trusted"])
        .output()
        .unwrap();
    assert!(!unsynced.status.success());
    assert!(
        String::from_utf8_lossy(&unsynced.stderr).contains("jess.lock is out of date"),
        "{}",
        String::from_utf8_lossy(&unsynced.stderr)
    );

    let synced = json(
        &jess()
            .current_dir(&workspace)
            .args(["--format", "json", "workspace", "sync"])
            .arg(jpks.join("words.jpk"))
            .arg(jpks.join("style.jpk"))
            .output()
            .unwrap(),
    );
    assert_eq!(synced["lock"], "written");
    assert_eq!(synced["packages"][0]["id"], "org.example.style");
    assert_eq!(synced["packages"][1]["id"], "org.example.words");
    fs::remove_dir_all(&jpks).unwrap();
    let locked = jess()
        .current_dir(&workspace)
        .args(["workspace", "sync", "--locked"])
        .output()
        .unwrap();
    assert!(locked.status.success(), "{}", String::from_utf8_lossy(&locked.stderr));

    let inside = workspace.join("tools/greeter/src");
    let run = |extra: &[&str]| {
        jess()
            .current_dir(&inside)
            .args(["workspace", "run", "org.example.greeter:greet"])
            .args(extra)
            .output()
            .unwrap()
    };
    let greeted = run(&["--trusted", "--", "one", "two"]);
    assert_eq!(greeted.status.code(), Some(2), "{}", String::from_utf8_lossy(&greeted.stderr));
    assert_eq!(
        String::from_utf8(greeted.stdout).unwrap(),
        "HELLO, ONE! (hello)\nHELLO, TWO! (hello)\n"
    );
    let denied = run(&["--", "one"]);
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("required capability denied:\n    process.environment"),
        "{}",
        String::from_utf8_lossy(&denied.stderr)
    );
    let root = jess()
        .current_dir(&workspace)
        .args(["workspace", "run", "showcase"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(root.stdout).unwrap(), "hello, showcase\n");
}
