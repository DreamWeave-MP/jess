//! Application targets from the command line: `jess package run` and `jess workspace run` over the
//! example in `tests/lua/application`, from its Package Sources and from JPKs, with the ways an
//! application stops: it asks to, `--frames` runs out, or it is interrupted.

use crate::common::{Path, PathBuf, fs, jess, temp_dir_path};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lua/application")
}

/// What the example prints, start to end, when it runs until it asks to stop.
const EXAMPLE: &str = "armor replaces light armor, keeping its weights
entry: heavy helm, weighs 5
director load
inventory startup
director startup: heavy cuirass, weighs 30
director update 1 (number)
director update 2 (number)
inventory drops Caius's loot
armor dents for Caius
scoreboard: death 1, Caius on frame 2
director update 3 (number)
director game over: Caius died
director update 4 (number)
director shutdown
";

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A scratch folder for one test, empty, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root = temp_dir_path(name);
        drop(fs::remove_dir_all(&root));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

/// A one-package application at `root/app` whose one managed module is `module` and whose entry
/// is `entry`.
fn app(root: &Path, module: &str, entry: &str) -> PathBuf {
    let app = root.join("app");
    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(
        app.join("jess.toml"),
        "manifest_version = 1\n\n[package]\nid = \"org.example.app\"\nversion = \"1.0.0\"\njess_api = \"1\"\ndefault_target = \"app\"\ninclude = [\"src\"]\n\n[managed.main]\nmodule = \"src/main.jsl\"\n\n[targets.app]\nkind = \"application\"\nentry = \"src/entry.jsl\"\n\n[targets.tool]\nkind = \"command\"\nentry = \"src/entry.jsl\"\n",
    )
    .unwrap();
    fs::write(app.join("src/main.jsl"), module).unwrap();
    fs::write(app.join("src/entry.jsl"), entry).unwrap();
    app
}

#[test]
fn package_run_launches_an_application_until_it_asks_to_stop() {
    let output = jess()
        .args(["package", "run"])
        .arg(example().join("game"))
        .arg("--package-path")
        .arg(example())
        .arg("print")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), EXAMPLE);
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn packed_jpks_run_the_same_application() {
    let scratch = Scratch::new("application-jpks");
    for name in ["journal", "inventory", "armor", "game"] {
        let packed = jess()
            .arg("pack")
            .arg(example().join(name))
            .arg("-o")
            .arg(scratch.0.join(format!("{name}.jpk")))
            .output()
            .unwrap();
        assert!(packed.status.success(), "{}", text(&packed.stderr));
    }
    let output = jess()
        .args(["package", "run"])
        .arg(scratch.0.join("game.jpk"))
        .arg("--package-path")
        .arg(&scratch.0)
        .arg("print")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), EXAMPLE);
}

#[test]
fn workspace_run_launches_a_member_s_application() {
    let output = jess()
        .args(["workspace", "--workspace"])
        .arg(example())
        .args(["run", "org.example.game:game", "--", "print"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), EXAMPLE);
}

#[test]
fn frames_stop_an_application_and_its_shutdown_still_runs() {
    let output = jess()
        .args(["package", "run"])
        .arg(example().join("game"))
        .args(["--package-path"])
        .arg(example())
        .args(["--frames", "2", "print"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    let printed = text(&output.stdout);
    let lines = printed.lines().collect::<Vec<_>>();
    assert_eq!(lines[lines.len() - 2], "scoreboard: death 1, Caius on frame 2");
    assert_eq!(lines[lines.len() - 1], "director shutdown");
    // The game over the scoreboard emitted on the last frame never got a drain.
    assert_eq!(
        text(&output.stderr),
        "jess: info in org.example.game:game: 1 queued events were dropped at teardown\n"
    );

    let scratch = Scratch::new("application-frames-command");
    let tool = app(&scratch.0, "return {}\n", "return nil\n");
    let refused = jess()
        .args(["package", "run"])
        .arg(&tool)
        .args(["--target", "tool", "--frames", "2"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        text(&refused.stderr)
            .contains("--frames is for application targets; target 'tool' of org.example.app is a command target"),
        "{}",
        text(&refused.stderr)
    );
}

#[test]
fn an_application_s_status_is_what_it_asks_to_stop_with_and_failures_go_to_stderr() {
    let scratch = Scratch::new("application-status");
    let root = app(
        &scratch.0,
        r"local application = require('@jess/application')
local frames = 0

return {
  on = {
    host = {
      update = function()
        frames += 1
        if frames == 2 then error('frame two broke') end
        if frames == 3 then application.exit(3) end
      end,
    },
  },
}
",
        "return 0\n",
    );
    let output = jess().args(["package", "run"]).arg(&root).output().unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", text(&output.stderr));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with("jess: error in org.example.app/main during update (org.example.app@1.0.0/src/main.jsl:7): ")
            && stderr.contains("frame two broke"),
        "{stderr}"
    );
}

#[test]
fn a_malformed_managed_module_stops_the_launch() {
    let scratch = Scratch::new("application-malformed");
    let root = app(
        &scratch.0,
        "return { on = { host = { updat = function() end } } }\n",
        "return nil\n",
    );
    let output = jess().args(["package", "run"]).arg(&root).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains(
            "org.example.app target app failed to load:\nerror in org.example.app:app during load: managed 'main' of org.example.app (org.example.app@1.0.0/src/main.jsl): on.host.updat is not a host callback"
        ),
        "{stderr}"
    );
    assert_eq!(text(&output.stdout), "");
}

/// A child process that is killed and reaped when dropped: a test that fails before it stops an
/// application started without `--frames` must not leave it running forever.
#[cfg(unix)]
struct Reaped(std::process::Child);

#[cfg(unix)]
impl Drop for Reaped {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            drop(self.0.kill());
            drop(self.0.wait());
        }
    }
}

/// Ctrl+C asks the application to stop: its shutdown handlers run, and the status is 130.
#[cfg(unix)]
#[test]
fn an_interrupt_stops_an_application_gracefully() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let scratch = Scratch::new("application-interrupt");
    let root = app(
        &scratch.0,
        "return { on = { host = {\n  startup = function() print('running') end,\n  shutdown = function() print('shut down') end,\n} } }\n",
        "return nil\n",
    );
    let mut child = Reaped(
        jess()
            .args(["package", "run"])
            .arg(&root)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut stdout = BufReader::new(child.0.stdout.take().unwrap());
    let mut line = String::new();
    // Not a race: the signal goes once the application says it is running.
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "running\n");
    let pid = libc::pid_t::try_from(child.0.id()).unwrap();
    // SAFETY: kill only sends a signal to the child this test spawned and hasn't reaped.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGINT) }, 0);
    let status = child.0.wait().unwrap();
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut stdout, &mut rest).unwrap();
    assert_eq!(rest, "shut down\n");
    assert_eq!(status.code(), Some(130));
}
