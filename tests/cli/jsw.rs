//! Packed workspaces from the command line, over the showcase: `jess workspace pack`, `jess
//! app.jsw`, a packed workspace named with `--workspace`, and a standalone executable.

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

fn pack(workspace: &Path, output: &Path, runtime: Option<&Path>) -> serde_json::Value {
    let mut command = jess();
    command
        .current_dir(workspace)
        .args(["--format", "json", "workspace", "pack", "--output"])
        .arg(output);
    if let Some(runtime) = runtime {
        command.arg("--runtime").arg(runtime);
    }
    json(&command.output().unwrap())
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
    let report = pack(&workspace, &packed, None);
    assert_eq!(report["default"], "org.example.showcase:showcase");
    assert_eq!(
        report["targets"],
        serde_json::json!(["org.example.greeter:greet", "org.example.showcase:showcase"])
    );
    let again = showcase.root.join("again.jsw");
    pack(&workspace, &again, None);
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
    pack(&workspace, &packed, None);
    let greeted = jess().arg(&packed).arg("you").output().unwrap();
    assert_eq!(greeted.status.code(), Some(1), "{}", stderr(&greeted));
    assert_eq!(stdout(&greeted), "HELLO, YOU! (hello)\n");
}

/// A standalone executable is a jess carrying a packed workspace: run from anywhere, it is that
/// application, every argument its default target's, and under the name of another target's
/// command it runs that target. What it carries is the packed workspace, byte for byte: after the
/// runtime on Linux and Windows, in the section the runtime reserves on macOS, where the result is
/// signed ad hoc when the runtime was, passes `codesign`'s strict check, and can be signed again
/// with the hardened runtime and still run.
#[test]
fn a_standalone_executable_is_the_application_it_carries() {
    let showcase = synced("jsw-standalone");
    let workspace = showcase.workspace();
    let packed = showcase.root.join("showcase.jsw");
    pack(&workspace, &packed, None);
    let name = if cfg!(windows) { "showcase.exe" } else { "showcase" };
    let standalone = showcase.root.join(name);
    let runtime = std::path::PathBuf::from(env!("CARGO_BIN_EXE_jess"));
    let report = pack(&workspace, &standalone, Some(&runtime));
    assert_eq!(report["sha256"], pack(&workspace, &packed, None)["sha256"]);
    if !cfg!(target_os = "macos") {
        let (runtime_bytes, image, executable) = (
            fs::read(&runtime).unwrap(),
            fs::read(&packed).unwrap(),
            fs::read(&standalone).unwrap(),
        );
        assert_eq!(&executable[..runtime_bytes.len()], &runtime_bytes[..]);
        assert_eq!(
            &executable[runtime_bytes.len()..runtime_bytes.len() + image.len()],
            &image[..]
        );
    }
    let carried = json(
        &jess()
            .args(["--format", "json", "workspace", "--workspace"])
            .arg(&standalone)
            .arg("info")
            .output()
            .unwrap(),
    );
    assert_eq!(carried["kind"], "packed");

    fs::remove_dir_all(&workspace).unwrap();
    let elsewhere = showcase.root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let moved = elsewhere.join(name);
    fs::rename(&standalone, &moved).unwrap();
    let ran = std::process::Command::new(&moved)
        .current_dir(&elsewhere)
        .arg("--help")
        .output()
        .unwrap();
    assert!(ran.status.success(), "{}", stderr(&ran));
    assert_eq!(stdout(&ran), "hello, showcase\n");

    let alias = elsewhere.join(if cfg!(windows) { "greet.exe" } else { "greet" });
    fs::copy(&moved, &alias).unwrap();
    let greeted = std::process::Command::new(&alias)
        .current_dir(&elsewhere)
        .args(["one", "two", "three"])
        .output()
        .unwrap();
    assert_eq!(greeted.status.code(), Some(3), "{}", stderr(&greeted));
    assert_eq!(
        stdout(&greeted),
        "HELLO, ONE! (hello)\nHELLO, TWO! (hello)\nHELLO, THREE! (hello)\n"
    );

    // A link keeps the name it was run by, though the executable it reaches has another.
    #[cfg(unix)]
    {
        let linked = elsewhere.join("linked");
        fs::create_dir_all(&linked).unwrap();
        std::os::unix::fs::symlink(&moved, linked.join("greet")).unwrap();
        let greeted = std::process::Command::new(linked.join("greet"))
            .current_dir(&elsewhere)
            .arg("link")
            .output()
            .unwrap();
        assert_eq!(greeted.status.code(), Some(1), "{}", stderr(&greeted));
        assert_eq!(stdout(&greeted), "HELLO, LINK! (hello)\n");
    }

    if cfg!(target_os = "macos") {
        signs_on_macos(&runtime, &moved, &elsewhere);
    }
}

/// A folder of runtimes packs every platform at once: the packed workspace, and beside it one
/// standalone executable per runtime, named for the output and the runtime, each carrying the
/// same packed workspace. A file in the folder that isn't a runtime is refused.
#[test]
fn a_folder_of_runtimes_packs_every_platform() {
    let showcase = synced("jsw-runtimes");
    let workspace = showcase.workspace();
    let runtimes = showcase.root.join("runtimes");
    fs::create_dir_all(&runtimes).unwrap();
    let suffix = std::env::consts::EXE_SUFFIX;
    for name in ["jess-Here-X64", "jess-There-ARM64"] {
        fs::copy(env!("CARGO_BIN_EXE_jess"), runtimes.join(format!("{name}{suffix}"))).unwrap();
    }
    let output = showcase.root.join("dist/showcase.jsw");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    let report = json(
        &jess()
            .current_dir(&workspace)
            .args(["--format", "json", "workspace", "pack", "--output"])
            .arg(&output)
            .arg("--runtimes")
            .arg(&runtimes)
            .output()
            .unwrap(),
    );
    assert_eq!(
        report["sha256"],
        pack(&workspace, &showcase.root.join("again.jsw"), None)["sha256"]
    );
    let executables = report["executables"].as_array().unwrap();
    assert_eq!(executables.len(), 2);
    let here = output.with_file_name(format!("showcase-Here-X64{suffix}"));
    assert!(output.with_file_name(format!("showcase-There-ARM64{suffix}")).is_file());
    assert_eq!(executables[0]["path"].as_str().map(Path::new), Some(here.as_path()));
    let ran = std::process::Command::new(&here).output().unwrap();
    assert_eq!(stdout(&ran), "hello, showcase\n", "{}", stderr(&ran));

    fs::write(runtimes.join("README"), "not a runtime").unwrap();
    let refused = jess()
        .current_dir(&workspace)
        .args(["workspace", "pack", "--output"])
        .arg(&output)
        .arg("--runtimes")
        .arg(&runtimes)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("is not a runtime"), "{}", stderr(&refused));
}

/// `moved`, a standalone executable written from `runtime`, verifies as strictly as its runtime
/// does, and still runs signed again with the hardened runtime and `allow-jit`.
fn signs_on_macos(runtime: &Path, moved: &Path, elsewhere: &Path) {
    let codesign = |args: &[&str]| {
        let output = std::process::Command::new("codesign").args(args).arg(moved).output().unwrap();
        assert!(output.status.success(), "codesign {args:?}: {}", stderr(&output));
    };
    // arm64's linker signs ad hoc and so does packing; x86-64's signs nothing, and neither
    // does packing. Either way the result signs.
    let signed = std::process::Command::new("codesign")
        .arg("--verify")
        .arg(runtime)
        .output()
        .unwrap()
        .status
        .success();
    if signed {
        codesign(&["--verify", "--strict", "--verbose=2"]);
    }
    let entitlements = elsewhere.join("jit.plist");
    fs::write(
        &entitlements,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>com.apple.security.cs.allow-jit</key><true/></dict></plist>\n",
    )
    .unwrap();
    let entitlements = entitlements.display().to_string();
    codesign(&[
        "--force",
        "--sign",
        "-",
        "--options",
        "runtime",
        "--entitlements",
        &entitlements,
    ]);
    codesign(&["--verify", "--strict", "--verbose=2"]);
    let resigned = std::process::Command::new(moved).current_dir(elsewhere).output().unwrap();
    assert_eq!(stdout(&resigned), "hello, showcase\n", "{}", stderr(&resigned));
}
