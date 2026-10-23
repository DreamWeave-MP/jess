//! `jess install` and `jess uninstall`: the showcase workspace, its packed workspace, a standalone
//! executable, and a package from a publisher, each put in an install root of its own and run
//! from there by name.

use super::common::{Path, PathBuf, fs, jess};
use super::workspace::{Showcase, fixture, index, json, publish, publisher};

/// The showcase with its JPKs synced into the lock, ready to install or pack.
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
        assert!(packed.status.success(), "{}", stderr(&packed));
        sync.arg(jpk);
    }
    let synced = sync.output().unwrap();
    assert!(synced.status.success(), "{}", stderr(&synced));
    fs::remove_dir_all(&jpks).unwrap();
    showcase
}

/// `jess install` into `root`, with no terminal to ask in and nothing from the environment.
fn install(root: &Path, args: &[&str]) -> std::process::Output {
    jess()
        .env_remove("JESS_OFFLINE")
        .env_remove("JESS_REGISTRY_URL")
        .env_remove("JESS_INSTALL_ROOT")
        .stdin(std::process::Stdio::null())
        .args(["--format", "json", "install", "--root"])
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}

fn uninstall(root: &Path, names: &[&str]) -> std::process::Output {
    jess()
        .env_remove("JESS_INSTALL_ROOT")
        .args(["--format", "json", "uninstall", "--root"])
        .arg(root)
        .args(names)
        .output()
        .unwrap()
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The command `name` in `root`'s bin, as this platform names an executable.
fn command(root: &Path, name: &str) -> PathBuf {
    root.join("bin").join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// A folder to run installed commands from, far from anything they were installed from.
fn elsewhere(showcase: &Showcase) -> PathBuf {
    let folder = showcase.root.join("elsewhere");
    fs::create_dir_all(&folder).unwrap();
    folder
}

/// A workspace installs as one executable per command it exports, each running its own target
/// from anywhere with what that target declares. Nothing is installed without a yes, which with no
/// terminal is `--yes`; the same install again changes nothing; a list says what is there; and
/// uninstalling one command's name removes the whole install.
#[test]
fn a_workspace_installs_as_commands_that_run_by_name() {
    let showcase = synced("install-workspace");
    let (workspace, root) = (showcase.workspace(), showcase.root.join("root"));
    let path = workspace.to_str().unwrap();

    let unasked = install(&root, &["--path", path]);
    assert!(!unasked.status.success());
    assert!(
        stderr(&unasked).contains("greet     runs org.example.greeter:greet, holding process.environment"),
        "{}",
        stderr(&unasked)
    );
    assert!(stderr(&unasked).contains("pass --yes"), "{}", stderr(&unasked));
    assert!(!command(&root, "greet").exists());
    assert_eq!(fs::read_dir(root.join("staging")).unwrap().count(), 0);

    let installed = json(&install(&root, &["--path", path, "--yes"]));
    assert_eq!(installed["package"], "org.example.showcase");
    assert_eq!(installed["version"], "0.1.0");
    let names: Vec<&str> = installed["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|written| written["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["greet", "showcase"]);

    let away = elsewhere(&showcase);
    let showcased = std::process::Command::new(command(&root, "showcase"))
        .current_dir(&away)
        .output()
        .unwrap();
    assert_eq!(stdout(&showcased), "hello, showcase\n", "{}", stderr(&showcased));
    let greeted = std::process::Command::new(command(&root, "greet"))
        .current_dir(&away)
        .args(["one", "two"])
        .output()
        .unwrap();
    assert_eq!(greeted.status.code(), Some(2), "{}", stderr(&greeted));
    assert_eq!(stdout(&greeted), "HELLO, ONE! (hello)\nHELLO, TWO! (hello)\n");

    let again = json(&install(&root, &["--path", path]));
    assert_eq!(again["state"], "already installed");

    let listed = json(&install(&root, &["--list"]));
    assert_eq!(listed[0]["package"], "org.example.showcase");
    assert_eq!(listed[0]["source"], "workspace");
    assert_eq!(listed[0]["commands"][0]["state"], "installed");
    fs::write(command(&root, "showcase"), b"something else").unwrap();
    let changed = json(&install(&root, &["--list"]));
    assert_eq!(changed[0]["commands"][1]["state"], "changed");

    // Nothing grows by installing it again over the changed file, so it needs no yes.
    let repaired = json(&install(&root, &["--path", path]));
    assert_eq!(repaired["package"], "org.example.showcase");
    let showcased = std::process::Command::new(command(&root, "showcase")).output().unwrap();
    assert_eq!(stdout(&showcased), "hello, showcase\n", "{}", stderr(&showcased));

    let removed = json(&uninstall(&root, &["greet"]));
    assert_eq!(removed[0]["package"], "org.example.showcase");
    assert!(!command(&root, "greet").exists());
    assert!(!command(&root, "showcase").exists());
    assert!(!root.join("installs/org.example.showcase").exists());
    let unknown = uninstall(&root, &["greet"]);
    assert!(!unknown.status.success());
    assert!(stderr(&unknown).contains("is called 'greet'"), "{}", stderr(&unknown));
}

/// A packed workspace installs as its bytes are, and so does the one a standalone executable
/// carries: either way the commands carry this jess and the same packed workspace, which is what
/// the install keeps.
#[test]
fn a_packed_workspace_or_an_executable_carrying_one_installs_as_it_is() {
    let showcase = synced("install-file");
    let (workspace, root) = (showcase.workspace(), showcase.root.join("root"));
    let packed = showcase.root.join("showcase.jsw");
    let report = json(
        &jess()
            .current_dir(&workspace)
            .args(["--format", "json", "workspace", "pack", "--output"])
            .arg(&packed)
            .output()
            .unwrap(),
    );

    let installed = json(&install(&root, &[packed.to_str().unwrap(), "--yes"]));
    assert_eq!(installed["package"], "org.example.showcase");
    assert_eq!(
        fs::read(root.join("installs/org.example.showcase/app.jsw")).unwrap(),
        fs::read(&packed).unwrap()
    );
    let ledger = fs::read_to_string(root.join("installs/org.example.showcase/install.toml")).unwrap();
    assert!(
        ledger.contains(&format!("jsw_sha256 = \"{}\"", report["sha256"].as_str().unwrap())),
        "{ledger}"
    );
    assert!(ledger.contains("source = \"file\""), "{ledger}");
    let showcased = std::process::Command::new(command(&root, "showcase")).output().unwrap();
    assert_eq!(stdout(&showcased), "hello, showcase\n", "{}", stderr(&showcased));

    let standalone = showcase.root.join(format!("showcase{}", std::env::consts::EXE_SUFFIX));
    json(
        &jess()
            .current_dir(&workspace)
            .args(["--format", "json", "workspace", "pack", "--output"])
            .arg(&standalone)
            .arg("--runtime")
            .arg(env!("CARGO_BIN_EXE_jess"))
            .output()
            .unwrap(),
    );
    let again = json(&install(&root, &[standalone.to_str().unwrap()]));
    assert_eq!(again["state"], "already installed");
}

/// Two installs can't both have a command name, and a file in bin no install wrote isn't
/// overwritten, unless `--force` takes the name over: then the other install no longer has it.
#[test]
fn a_command_name_is_one_install_s_unless_forced() {
    let showcase = synced("install-collisions");
    let (workspace, root) = (showcase.workspace(), showcase.root.join("root"));
    let path = workspace.to_str().unwrap();
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(command(&root, "showcase"), b"mine").unwrap();

    let stray = install(&root, &["--path", path, "--yes"]);
    assert!(!stray.status.success());
    assert!(
        stderr(&stray).contains("is there, and no install owns it"),
        "{}",
        stderr(&stray)
    );
    assert_eq!(fs::read(command(&root, "showcase")).unwrap(), b"mine");
    json(&install(&root, &["--path", path, "--yes", "--force"]));

    // The greeter alone, packed, is another install claiming `greet`.
    let greeter = showcase.root.join("greeter.jsw");
    fs::write(
        workspace.join("jess-workspace.toml"),
        fs::read_to_string(workspace.join("jess-workspace.toml")).unwrap() + "\n[pack]\ntargets = [\"greet\"]\n",
    )
    .unwrap();
    json(
        &jess()
            .current_dir(&workspace)
            .args(["--format", "json", "workspace", "pack", "--output"])
            .arg(&greeter)
            .output()
            .unwrap(),
    );
    let taken = install(&root, &[greeter.to_str().unwrap(), "--yes"]);
    assert!(!taken.status.success());
    assert!(
        stderr(&taken).contains("'greet' is org.example.showcase's"),
        "{}",
        stderr(&taken)
    );

    let forced = json(&install(&root, &[greeter.to_str().unwrap(), "--yes", "--force"]));
    assert_eq!(forced["package"], "org.example.greeter");
    let listed = json(&install(&root, &["--list"]));
    assert_eq!(listed[0]["package"], "org.example.greeter");
    assert_eq!(listed[1]["package"], "org.example.showcase");
    assert_eq!(listed[1]["commands"].as_array().unwrap().len(), 1);
    assert_eq!(listed[1]["commands"][0]["name"], "showcase");
}

/// A package id installs from its publisher: the newest release, or the one `--version` names,
/// resolved with what it depends on in the install's own workspace and packed there.
#[test]
fn a_package_installs_from_its_publisher() {
    let showcase = Showcase::new("install-publisher");
    let (jpks, root) = (showcase.root.join("jpks"), showcase.root.join("root"));
    fs::create_dir_all(&jpks).unwrap();
    let words = publish("words", &jpks);
    let tool = publish("tool", &jpks);
    let server = super::common::http::Server::start();
    let site = publisher(&server, "/publisher/", "Fixture publisher", &[&words, &tool]);
    let empty = index(&server, &[]);

    let missing = install(
        &root,
        &[
            "org.example.tool",
            "--version",
            "2",
            "--publisher",
            &site.index,
            "--index",
            &empty,
            "--yes",
        ],
    );
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("no release of org.example.tool meeting 2 to install"),
        "{}",
        stderr(&missing)
    );

    let offline = install(&root, &["org.example.tool", "--offline", "--yes"]);
    assert!(!offline.status.success());
    assert!(stderr(&offline).contains("offline"), "{}", stderr(&offline));

    let installed = json(&install(
        &root,
        &[
            "org.example.tool",
            "--version",
            "1.0.0",
            "--publisher",
            &site.index,
            "--index",
            &empty,
            "--yes",
        ],
    ));
    assert_eq!(installed["package"], "org.example.tool");
    assert_eq!(installed["version"], "1.0.0");
    let ran = std::process::Command::new(command(&root, "hello"))
        .arg("world")
        .output()
        .unwrap();
    assert_eq!(stdout(&ran), "hello world from the tool\n", "{}", stderr(&ran));
    let listed = json(&install(&root, &["--list"]));
    assert_eq!(listed[0]["source"], "resolved");
    assert!(
        listed[0]["manifest"].as_str().unwrap().starts_with("http://127.0.0.1:"),
        "{listed}"
    );
    assert!(root.join("installs/org.example.tool/jess.lock").is_file());

    // Found through an index alone, with no version named: the newest release.
    json(&uninstall(&root, &["org.example.tool"]));
    let claims = index(
        &server,
        &[
            ("org.example.tool", site.manifest("org.example.tool")),
            ("org.example.words", site.manifest("org.example.words")),
        ],
    );
    let indexed = json(&install(&root, &["org.example.tool", "--index", &claims, "--yes"]));
    assert_eq!(indexed["version"], "1.0.0");
    let ran = std::process::Command::new(command(&root, "hello"))
        .arg("again")
        .output()
        .unwrap();
    assert_eq!(stdout(&ran), "hello again from the tool\n", "{}", stderr(&ran));
}
