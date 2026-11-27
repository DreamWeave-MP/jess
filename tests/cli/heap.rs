//! `--max-heap`: the Luau heap ceiling of the runtimes `jess run`, `jess test`, and the package and
//! workspace commands make for the code they run. Past it an allocation fails with Luau's "not
//! enough memory", which the code can catch, and the process reports it and exits as it would for
//! any error. Every VM has a ceiling of its own, a worker's included.

use super::common::{Path, PathBuf, fs, jess, temp_dir_path};

/// Holds 64 MiB of buffers, one at a time, and says how many it got.
const HOG: &str =
    "local held = {}\nfor index = 1, 64 do\n  held[index] = buffer.create(1024 * 1024)\nend\nprint(`held {#held} MiB`)\n";

/// What a worker that tries for 64 MiB under a pcall answers.
const LANE: &str = "local lane = require('@jess/worker').spawn {\n  source = [[\n    return function(mebibytes)\n      local held = {}\n      local ok, problem = pcall(function()\n        for index = 1, mebibytes do\n          held[index] = buffer.create(1024 * 1024)\n        end\n      end)\n      return if ok then `held {#held} MiB` else `worker: {problem}`\n    end\n  ]],\n}\nlane:send(64)\nprint(lane:receive(20))\nrequire('@jess/application').exit(0)\n";

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A scratch folder for one test, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root = temp_dir_path(name);
        drop(fs::remove_dir_all(&root));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, path: &str, contents: &str) -> PathBuf {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

/// A workspace at `root` whose one member, `app`, has a command target that holds 64 MiB, an
/// application target whose worker tries to, and a test that does.
fn workspace(scratch: &Scratch) -> PathBuf {
    scratch.write(
        "jess-workspace.toml",
        "manifest_version = 1\n[workspace]\nmembers = [\"app\"]\n",
    );
    scratch.write(
        "app/jess.toml",
        "manifest_version = 1\n[package]\nid = \"org.example.heap\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
         default_target = \"hog\"\ninclude = [\"src\", \"tests\"]\ntests = [\"tests/hog.jsl\"]\n\
         [targets.hog]\nkind = \"command\"\nentry = \"src/hog.jsl\"\n\
         [targets.lane]\nkind = \"application\"\nentry = \"src/lane.jsl\"\ncapabilities = [\"worker.spawn\"]\n",
    );
    scratch.write("app/src/hog.jsl", HOG);
    scratch.write("app/src/lane.jsl", LANE);
    scratch.write(
        "app/tests/hog.jsl",
        "local test = require '@jess/test'\nlocal suite = test.new()\n\
         suite.it('holds 64 MiB', function()\n  local held = {}\n  for index = 1, 64 do\n    held[index] = buffer.create(1024 * 1024)\n  end\n  test.expect(#held).toBe(64)\nend)\nreturn suite\n",
    );
    scratch.0.join("app")
}

fn run(arguments: &[&str], at: &Path) -> std::process::Output {
    jess().current_dir(at).args(arguments).output().unwrap()
}

#[test]
fn a_script_past_its_ceiling_fails_and_one_without_a_ceiling_does_not() {
    let scratch = Scratch::new("heap-run");
    let hog = scratch.write("hog.jsl", HOG);
    let hog = hog.to_str().unwrap();

    let free = run(&["run", hog], &scratch.0);
    assert!(free.status.success(), "{}", text(&free.stderr));
    assert_eq!(text(&free.stdout), "held 64 MiB\n");

    let capped = run(&["run", "--max-heap", "16M", hog], &scratch.0);
    assert_eq!(capped.status.code(), Some(1), "{}", text(&capped.stderr));
    assert_eq!(text(&capped.stdout), "");
    assert!(
        text(&capped.stderr).contains("memory limit exceeded"),
        "{}",
        text(&capped.stderr)
    );

    let roomy = run(&["run", "--max-heap", "1GiB", hog], &scratch.0);
    assert!(roomy.status.success(), "{}", text(&roomy.stderr));
}

/// The failed allocation raises an error like any other, so the script that caught it goes on.
#[test]
fn a_script_catches_the_memory_error_and_carries_on() {
    let scratch = Scratch::new("heap-pcall");
    let script = scratch.write(
        "catch.jsl",
        "local held = {}\nlocal ok, problem = pcall(function()\n  for index = 1, 64 do\n    held[index] = buffer.create(1024 * 1024)\n  end\nend)\nheld = {}\nprint(ok, problem)\nprint(buffer.len(buffer.create(1024)))\n",
    );
    let caught = run(&["run", "--max-heap", "16M", script.to_str().unwrap()], &scratch.0);
    assert!(caught.status.success(), "{}", text(&caught.stderr));
    assert_eq!(text(&caught.stdout), "false\tnot enough memory\n1024\n");
}

#[test]
fn a_size_is_bytes_or_a_binary_multiple_and_never_zero() {
    let scratch = Scratch::new("heap-sizes");
    let script = scratch.write("empty.jsl", "print('ran')\n");
    let script = script.to_str().unwrap();
    for size in ["67108864", "65536K", "64M", "64MB", "64mib", "1G", "1T"] {
        let output = run(&["run", "--max-heap", size, script], &scratch.0);
        assert!(output.status.success(), "{size}: {}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "ran\n");
    }
    for (size, problem) in [
        ("0", "no heap at all"),
        ("12Q", "no heap size"),
        ("1.5G", "no heap size"),
        ("99999999999999999999T", "no heap size"),
    ] {
        let output = run(&["run", "--max-heap", size, script], &scratch.0);
        assert_eq!(output.status.code(), Some(2), "{size}");
        assert!(text(&output.stderr).contains(problem), "{size}: {}", text(&output.stderr));
    }
}

/// `jess test`, `jess package test`, and `jess workspace test` hold every test module under the
/// ceiling, so a test past it fails as a test, and the run says so.
#[test]
fn tests_past_the_ceiling_fail_as_tests() {
    let scratch = Scratch::new("heap-tests");
    let app = workspace(&scratch);
    let app_text = app.to_str().unwrap();
    let module = app.join("tests/hog.jsl");
    let module = module.to_str().unwrap();

    for (free, capped) in [
        (vec!["test", module], vec!["test", "--max-heap", "16M", module]),
        (
            vec!["package", "test", app_text],
            vec!["package", "test", "--max-heap", "16M", app_text],
        ),
        (vec!["workspace", "test"], vec!["workspace", "test", "--max-heap", "16M"]),
    ] {
        let passed = run(&free, &scratch.0);
        assert!(passed.status.success(), "{free:?}: {}", text(&passed.stderr));
        let failed = run(&capped, &scratch.0);
        assert!(!failed.status.success(), "{capped:?}: {}", text(&failed.stdout));
        assert!(
            text(&failed.stdout).contains("not enough memory"),
            "{capped:?}: {}{}",
            text(&failed.stdout),
            text(&failed.stderr)
        );
    }
}

/// A package or workspace target runs under the ceiling, and an application's worker has a
/// ceiling of its own the size of the domain's: its 64 MiB fail in the worker, which reports it.
#[test]
fn targets_and_their_workers_run_under_the_ceiling() {
    let scratch = Scratch::new("heap-targets");
    let app = workspace(&scratch);
    let app = app.to_str().unwrap();

    let free = run(&["package", "run", app], &scratch.0);
    assert!(free.status.success(), "{}", text(&free.stderr));
    assert_eq!(text(&free.stdout), "held 64 MiB\n");
    for capped in [
        vec!["package", "run", "--max-heap", "16M", app],
        vec!["workspace", "run", "--max-heap", "16M", "org.example.heap:hog"],
    ] {
        let output = run(&capped, &scratch.0);
        assert_eq!(output.status.code(), Some(1), "{capped:?}: {}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "", "{capped:?}");
        assert!(
            text(&output.stderr).contains("memory limit exceeded"),
            "{capped:?}: {}",
            text(&output.stderr)
        );
    }

    let lane = run(&["package", "run", "--trusted", "--target", "lane", app], &scratch.0);
    assert!(lane.status.success(), "{}", text(&lane.stderr));
    assert_eq!(text(&lane.stdout), "held 64 MiB\n");
    let capped = run(
        &["package", "run", "--trusted", "--target", "lane", "--max-heap", "16M", app],
        &scratch.0,
    );
    assert!(capped.status.success(), "{}", text(&capped.stderr));
    assert_eq!(text(&capped.stdout), "worker: not enough memory\n");
}
