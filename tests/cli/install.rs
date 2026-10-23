//! `jess install` and `jess uninstall`: the showcase workspace, its packed workspace, a standalone
//! executable, and a package from a publisher, each put in an install root of its own and run
//! from there by name.

use super::common::{Path, PathBuf, fs, jess};
use super::workspace::{Published, Showcase, fixture, index, json, project_id, publish, publisher};

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

    // A command that has changed since it was installed is someone's now: putting it back takes
    // --force. Nothing grows, so it needs no yes.
    let refused = install(&root, &["--path", path]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("has changed since it was installed"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(fs::read(command(&root, "showcase")).unwrap(), b"something else");
    let repaired = json(&install(&root, &["--path", path, "--force"]));
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

/// A directory where a command goes is someone's: installing refuses it, `--force` or not, and
/// everything in it is still there afterwards.
#[test]
fn a_directory_where_a_command_goes_is_never_replaced() {
    let showcase = synced("install-directory");
    let (workspace, root) = (showcase.workspace(), showcase.root.join("root"));
    let path = workspace.to_str().unwrap();
    let kept = command(&root, "greet").join("important/data.txt");
    fs::create_dir_all(kept.parent().unwrap()).unwrap();
    fs::write(&kept, "keep me").unwrap();

    for force in [false, true] {
        let mut args = vec!["--path", path, "--yes"];
        if force {
            args.push("--force");
        }
        let refused = install(&root, &args);
        assert!(!refused.status.success());
        assert!(
            stderr(&refused).contains("never replaces or removes one"),
            "{}",
            stderr(&refused)
        );
        assert_eq!(fs::read_to_string(&kept).unwrap(), "keep me");
        assert!(!command(&root, "showcase").exists(), "nothing was installed beside it");
    }
}

/// Uninstalling removes only what the install wrote: a command that has changed since is left
/// where it is, and the report says so.
#[test]
fn uninstalling_keeps_a_command_that_has_changed_since() {
    let showcase = synced("install-uninstall-changed");
    let (workspace, root) = (showcase.workspace(), showcase.root.join("root"));
    json(&install(&root, &["--path", workspace.to_str().unwrap(), "--yes"]));
    // Its commands are links to one file: replacing one is a new file, not writing into the shared one.
    fs::remove_file(command(&root, "showcase")).unwrap();
    fs::write(command(&root, "showcase"), b"mine now").unwrap();

    let removed = json(&uninstall(&root, &["org.example.showcase"]));
    assert_eq!(removed[0]["commands"], serde_json::json!(["greet"]));
    assert!(
        removed[0]["kept"][0]
            .as_str()
            .unwrap()
            .contains("has changed since it was installed"),
        "{removed}"
    );
    assert!(!command(&root, "greet").exists());
    assert_eq!(fs::read(command(&root, "showcase")).unwrap(), b"mine now");
    assert!(!root.join("installs/org.example.showcase").exists());
}

/// An install that fails partway changes nothing: here its folder can't be swapped in after its
/// executables were put in place, so they are taken out again and the earlier install's put
/// back, executables, record and all.
#[cfg(unix)]
#[test]
fn an_install_that_fails_partway_puts_back_what_was_there() {
    use std::os::unix::fs::PermissionsExt;

    let showcase = synced("install-rollback");
    let (workspace, root) = (showcase.workspace(), showcase.root.join("root"));
    let path = workspace.to_str().unwrap();
    json(&install(&root, &["--path", path, "--yes"]));
    let ledger = root.join("installs/org.example.showcase/install.toml");
    let (before, executable) = (fs::read(&ledger).unwrap(), fs::read(command(&root, "showcase")).unwrap());

    fs::write(
        workspace.join("src/main.jsl"),
        fs::read_to_string(workspace.join("src/main.jsl"))
            .unwrap()
            .replace("'showcase'", "'changed'"),
    )
    .unwrap();
    let installs = root.join("installs");
    fs::set_permissions(&installs, fs::Permissions::from_mode(0o555)).unwrap();
    let failed = install(&root, &["--path", path, "--yes"]);
    fs::set_permissions(&installs, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(!failed.status.success());
    assert!(
        stderr(&failed).contains("everything before it was put back"),
        "{}",
        stderr(&failed)
    );
    assert_eq!(fs::read(&ledger).unwrap(), before);
    assert_eq!(fs::read(command(&root, "showcase")).unwrap(), executable);
    let ran = std::process::Command::new(command(&root, "showcase")).output().unwrap();
    assert_eq!(stdout(&ran), "hello, showcase\n", "{}", stderr(&ran));
    let listed = json(&install(&root, &["--list"]));
    assert_eq!(listed[0]["commands"][0]["state"], "installed");
    assert_eq!(listed[0]["commands"][1]["state"], "installed");
    for leftovers in ["staging", "trash"] {
        let folder = root.join(leftovers);
        let count = fs::read_dir(&folder).map_or(0, Iterator::count);
        assert_eq!(count, 0, "{leftovers} keeps nothing");
    }

    let upgraded = json(&install(&root, &["--path", path, "--yes"]));
    assert_eq!(upgraded["package"], "org.example.showcase");
    let ran = std::process::Command::new(command(&root, "showcase")).output().unwrap();
    assert_eq!(stdout(&ran), "hello, changed\n", "{}", stderr(&ran));
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

// Applications ------------------------------------------------------------------------------

/// The tool packed as an application: a workspace of nothing but the tool, pinned from `jpks`,
/// exporting its one target. Answers the `.jsw` and what packing it reported.
fn tool_application(showcase: &Showcase, tool: &Published, words: &Published) -> (PathBuf, serde_json::Value) {
    let workspace = showcase.root.join("application");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(
        workspace.join("jess-workspace.toml"),
        concat!(
            "manifest_version = 1\n\n[workspace]\nmembers = []\n\n",
            "[dev-dependencies]\n\"org.example.tool\" = \"=1.0.0\"\n\n",
            "[pack]\ntargets = [\"org.example.tool:hello\"]\ndefault = \"org.example.tool:hello\"\n",
        ),
    )
    .unwrap();
    let synced = jess()
        .current_dir(&workspace)
        .args(["workspace", "sync", "--offline"])
        .arg(&tool.path)
        .arg(&words.path)
        .output()
        .unwrap();
    assert!(synced.status.success(), "{}", stderr(&synced));
    let packed = workspace.join("tool.jsw");
    let report = json(
        &jess()
            .current_dir(&workspace)
            .args(["--format", "json", "workspace", "pack", "--output"])
            .arg(&packed)
            .output()
            .unwrap(),
    );
    (packed, report)
}

/// A protocol `schema_version` 4 site publishing the tool as one project whose release carries
/// its JPK and the application, and words as another, with `edit` applied to the application's
/// `jess_workspace` block. Answers the site's `dreamweave.json`.
fn application_site(
    server: &super::common::http::Server,
    base: &str,
    packages: (&Published, &Published),
    application: (&Path, &serde_json::Value),
    edit: impl FnOnce(&mut serde_json::Value),
) -> String {
    let (tool, words) = packages;
    let (packed, report) = application;
    let builtin = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("jpk/builtin/jess.toml"))
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("version = \"")
                .map(|rest| rest.trim_end_matches('"').to_owned())
        })
        .unwrap();
    let source = |file: &str| {
        let path = format!("{base}files/{file}");
        server.url(&path)
    };
    let serve = |file: &str, media: &str, path: &Path| {
        server.serve(&format!("{base}files/{file}"), media, fs::read(path).unwrap());
    };
    let jsw_bytes = fs::read(packed).unwrap();
    let mut block = serde_json::json!({
        "package": "org.example.tool",
        "builtin": builtin,
        "default": report["default"],
        "targets": report["targets"],
        "commands": { "hello": "org.example.tool:hello" },
        "packages": report["packages"].as_array().unwrap().iter().map(|pinned| (
            pinned["id"].as_str().unwrap().to_owned(),
            serde_json::json!({ "version": pinned["version"], "content_digest": pinned["digest"] }),
        )).collect::<serde_json::Map<_, _>>(),
    });
    edit(&mut block);
    let jpk = |package: &Published| {
        serde_json::json!({
            "id": "jpk", "format": "jpk", "filename": format!("{}.jpk", package.id), "media_type": "application/vnd.jess.jpk",
            "size": package.size, "digests": { "sha256": package.sha256 },
            "jess": { "package": package.id, "jess_api": "1", "content_digest": package.digest, "dependencies": package.dependencies },
            "sources": [{ "url": source(&format!("{}.jpk", package.id)), "kind": "publisher" }], "signatures": []
        })
    };
    serve("org.example.tool.jpk", "application/vnd.jess.jpk", &tool.path);
    serve("org.example.words.jpk", "application/vnd.jess.jpk", &words.path);
    serve("tool.jsw", "application/vnd.jess.jsw", packed);
    let mut entries = Vec::new();
    for (package, artifacts) in [
        (
            tool,
            vec![
                jpk(tool),
                serde_json::json!({
                    "id": "jsw", "format": "jsw", "filename": "tool.jsw", "media_type": "application/vnd.jess.jsw",
                    "size": jsw_bytes.len(), "digests": { "sha256": report["sha256"] },
                    "jess_workspace": block, "sources": [{ "url": source("tool.jsw"), "kind": "publisher" }], "signatures": []
                }),
            ],
        ),
        (words, vec![jpk(words)]),
    ] {
        let project = project_id(&package.id);
        let manifest_path = format!("{base}dreamweave/projects/{project}.json");
        let manifest = serde_json::json!({
            "schema_version": "4", "document": "project", "generator": "fixture",
            "project": {
                "id": project, "name": package.id, "type": "tool", "status": "active", "versioning": "numeric",
                "game": "example", "tags": [], "maintainers": [],
                "links": { "page": server.url(base), "source": "https://github.com/fixture/tool", "issues": "https://github.com/fixture/tool/issues" },
                "integrations": {}, "media": [], "credits": []
            },
            "channels": { "stable": { "version": package.version } },
            "releases": [{
                "version": package.version, "channel": "stable", "date": "2026-10-23", "status": "available",
                "source": { "repository": "https://github.com/fixture/tool" },
                "runtimes": {}, "platforms": [], "provides": [], "relationships": [], "components": [], "groups": [],
                "extensions": {}, "critical_extensions": [], "artifacts": artifacts
            }]
        });
        server.serve(&manifest_path, "application/json", serde_json::to_vec(&manifest).unwrap());
        entries.push(serde_json::json!({
            "id": project, "name": package.id, "type": "tool", "jess_package": package.id, "status": "active",
            "page": server.url(base), "manifest": server.url(&manifest_path), "manifest_sha256": "0".repeat(64),
            "updated": "2026-10-23", "channels": { "stable": package.version }
        }));
    }
    let index = serde_json::json!({
        "schema_version": "4", "document": "index", "generator": "fixture",
        "site": { "name": "Applications", "url": server.url(base) }, "projects": entries,
    });
    let path = format!("{base}dreamweave.json");
    server.serve(&path, "application/json", serde_json::to_vec(&index).unwrap());
    server.url(&path)
}

/// A package whose release carries it as an application installs the publisher's own packed
/// workspace: the frozen set it shipped, checked against the manifest, never resolved here. The
/// same release again downloads nothing; `--resolve` builds one from the packages instead; and a
/// packed workspace that isn't what the manifest says, or one this jess can't run, is refused.
#[test]
fn a_package_installs_its_publisher_s_packed_workspace() {
    let showcase = Showcase::new("install-application");
    let (jpks, root) = (showcase.root.join("jpks"), showcase.root.join("root"));
    fs::create_dir_all(&jpks).unwrap();
    let words = publish("words", &jpks);
    let tool = publish("tool", &jpks);
    let (packed, report) = tool_application(&showcase, &tool, &words);
    let server = super::common::http::Server::start();
    let empty = index(&server, &[]);
    let site = application_site(&server, "/apps/", (&tool, &words), (&packed, &report), |_| {});

    let installed = json(&install(
        &root,
        &["org.example.tool", "--publisher", &site, "--index", &empty, "--yes"],
    ));
    assert_eq!(installed["version"], "1.0.0");
    let ran = std::process::Command::new(command(&root, "hello"))
        .arg("world")
        .output()
        .unwrap();
    assert_eq!(stdout(&ran), "hello world from the tool\n", "{}", stderr(&ran));
    assert_eq!(
        fs::read(root.join("installs/org.example.tool/app.jsw")).unwrap(),
        fs::read(&packed).unwrap()
    );
    let ledger = fs::read_to_string(root.join("installs/org.example.tool/install.toml")).unwrap();
    assert!(ledger.contains("source = \"publisher\""), "{ledger}");
    assert!(
        ledger.contains(&format!("artifact_sha256 = \"{}\"", report["sha256"].as_str().unwrap())),
        "{ledger}"
    );

    let downloads = || server.requests().iter().filter(|path| path.ends_with("tool.jsw")).count();
    assert_eq!(downloads(), 1);
    let again = json(&install(
        &root,
        &["org.example.tool", "--publisher", &site, "--index", &empty],
    ));
    assert_eq!(again["state"], "already installed");
    assert_eq!(downloads(), 1, "the same release isn't downloaded again");

    let resolved = json(&install(
        &root,
        &[
            "org.example.tool",
            "--resolve",
            "--publisher",
            &site,
            "--index",
            &empty,
            "--force",
        ],
    ));
    assert_eq!(resolved["version"], "1.0.0");
    let ledger = fs::read_to_string(root.join("installs/org.example.tool/install.toml")).unwrap();
    assert!(ledger.contains("source = \"resolved\""), "{ledger}");

    let lying = application_site(&server, "/lying/", (&tool, &words), (&packed, &report), |block| {
        block["packages"]["org.example.words"]["content_digest"] = "0".repeat(64).into();
    });
    let refused = install(
        &root,
        &[
            "org.example.tool",
            "--publisher",
            &lying,
            "--index",
            &empty,
            "--yes",
            "--force",
        ],
    );
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("is not what the manifest describes"),
        "{}",
        stderr(&refused)
    );

    let future = application_site(&server, "/future/", (&tool, &words), (&packed, &report), |block| {
        block["builtin"] = "99.0.0".into();
    });
    let refused = install(
        &root,
        &[
            "org.example.tool",
            "--publisher",
            &future,
            "--index",
            &empty,
            "--yes",
            "--force",
        ],
    );
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("runs on this jess; upgrade jess"),
        "{}",
        stderr(&refused)
    );
}
