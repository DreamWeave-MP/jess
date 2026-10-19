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

// Publishers --------------------------------------------------------------------------------

/// A JPK packed from a vendor fixture, with the facts a feed states about it.
struct Published {
    id: String,
    version: String,
    digest: String,
    sha256: String,
    size: u64,
    path: PathBuf,
    /// Its declared `[dependencies]`, as the feed must state them.
    dependencies: serde_json::Value,
}

fn publish(name: &str, into: &Path) -> Published {
    let path = into.join(format!("{name}.jpk"));
    let packed = jess()
        .arg("pack")
        .arg(fixture("workspace-vendor").join(name))
        .arg("--output")
        .arg(&path)
        .output()
        .unwrap();
    assert!(packed.status.success(), "{}", String::from_utf8_lossy(&packed.stderr));
    let inspected = json(
        &jess()
            .args(["--format", "json", "package", "inspect"])
            .arg(&path)
            .output()
            .unwrap(),
    );
    let hashed = jess()
        .args(["run", "--capability", "filesystem.read"])
        .arg(fixture("sha256.jsl"))
        .arg("--")
        .arg(&path)
        .output()
        .unwrap();
    assert!(hashed.status.success(), "{}", String::from_utf8_lossy(&hashed.stderr));
    Published {
        id: inspected["id"].as_str().unwrap().to_owned(),
        version: inspected["version"].as_str().unwrap().to_owned(),
        digest: inspected["digest"].as_str().unwrap().to_owned(),
        sha256: String::from_utf8(hashed.stdout).unwrap().trim().to_owned(),
        size: fs::metadata(&path).unwrap().len(),
        dependencies: inspected["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|dependency| {
                (
                    dependency["id"].as_str().unwrap().to_owned(),
                    dependency["requirement"].clone(),
                )
            })
            .collect::<serde_json::Map<String, serde_json::Value>>()
            .into(),
        path,
    }
}

/// A publisher site on `server` under `base`: its feed lists `packages`, each downloadable there.
fn publisher(server: &crate::common::http::Server, base: &str, name: &str, packages: &[&Published]) -> String {
    let entries: Vec<serde_json::Value> = packages
        .iter()
        .map(|package| {
            let archive = format!("{base}packages/{}-{}.jpk", package.id, package.version);
            server.serve(&archive, "application/vnd.jess.jpk", fs::read(&package.path).unwrap());
            serde_json::json!({
                "id": package.id,
                "versions": [{
                    "version": package.version,
                    "jess_api": "1",
                    "status": "available",
                    "content_digest": package.digest,
                    "dependencies": package.dependencies,
                    "artifact": {
                        "filename": format!("{}-{}.jpk", package.id, package.version),
                        "media_type": "application/vnd.jess.jpk",
                        "size": package.size,
                        "digests": { "sha256": package.sha256 },
                        "sources": [{ "url": server.url(&archive), "kind": "publisher" }],
                        "signatures": []
                    }
                }]
            })
        })
        .collect();
    let feed = serde_json::json!({
        "format": "jess-packages",
        "format_version": 1,
        "publisher": { "name": name, "url": server.url(base) },
        "packages": entries,
    });
    let path = format!("{base}jess-packages.json");
    server.serve(&path, "application/json", serde_json::to_vec_pretty(&feed).unwrap());
    server.url(&path)
}

/// An index on `server` that says which feed publishes which id, as the `AsHyAmS` file does.
fn index(server: &crate::common::http::Server, claims: &[(&str, &str)]) -> String {
    let packages: Vec<serde_json::Value> = claims
        .iter()
        .map(|(id, feed)| serde_json::json!({ "id": id, "origin": "fixture", "feed_url": feed }))
        .collect();
    let document = serde_json::json!({
        "format": "dreamweave-network-jess-packages",
        "format_version": 1,
        "notice": "a fixture",
        "packages": packages,
        "conflicts": [],
    });
    server.serve("/index.json", "application/json", serde_json::to_vec(&document).unwrap());
    server.url("/index.json")
}

fn sync(workspace: &Path, args: &[&str]) -> std::process::Output {
    jess()
        .current_dir(workspace)
        .env_remove("JESS_OFFLINE")
        .env_remove("JESS_REGISTRY_URL")
        .args(["--format", "json", "workspace", "sync"])
        .args(args)
        .output()
        .unwrap()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// What the showcase needs comes from a publisher's feed found through an index: downloaded,
/// checked, pinned with the publisher in jess.lock, and run offline from then on. A clean
/// checkout restores the exact pins from that publisher with `--locked`, never rewriting the
/// lock; offline it restores from a vendor folder with the same bytes, or says plainly why not.
#[test]
fn the_showcase_syncs_from_a_publisher_and_restores_its_pins() {
    let showcase = Showcase::new("workspace-publisher");
    let (workspace, jpks) = (showcase.workspace(), showcase.root.join("jpks"));
    fs::create_dir_all(&jpks).unwrap();
    let words = publish("words", &jpks);
    let style = publish("style", &jpks);
    let server = crate::common::http::Server::start();
    let feed = publisher(&server, "/publisher/", "Fixture publisher", &[&words, &style]);
    let index_url = index(&server, &[("org.example.words", &feed), ("org.example.style", &feed)]);

    let synced = json(&sync(&workspace, &["--index", &index_url]));
    assert_eq!(synced["lock"], "written");
    for package in synced["packages"].as_array().unwrap() {
        assert_eq!(package["source"], "feed");
        assert_eq!(package["feed"], feed);
        assert_eq!(package["installed"], true);
    }
    let lock = fs::read_to_string(workspace.join("jess.lock")).unwrap();
    assert!(lock.contains("lock_version = 2"), "{lock}");
    assert!(lock.contains("source = \"feed\""), "{lock}");
    assert!(lock.contains(&format!("feed = \"{feed}\"")), "{lock}");
    assert!(lock.contains(&format!("artifact_sha256 = \"{}\"", words.sha256)), "{lock}");
    assert!(lock.contains(&format!("digest = \"{}\"", words.digest)), "{lock}");
    assert!(!lock.contains("127.0.0.1:0"), "{lock}");
    let kept = workspace.join(".jess/packages");
    assert!(
        kept.join(format!("{}-{}-{}.jpk", words.id, words.version, words.digest))
            .is_file()
    );
    assert!(
        fs::read_dir(&kept)
            .unwrap()
            .all(|entry| { entry.unwrap().path().extension().is_some_and(|extension| extension == "jpk") }),
        "no partial download is left behind"
    );

    // Runs offline from what it kept: the publisher is not asked again.
    server.remove("/publisher/jess-packages.json");
    let ran = jess()
        .current_dir(workspace.join("tools/greeter/src"))
        .args(["workspace", "run", "--trusted", "--", "you"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{}", stderr(&ran));

    // A clean checkout: `--locked` restores the exact pins from the pinned publisher.
    fs::remove_dir_all(&kept).unwrap();
    let before = server.requests().len();
    let offline = sync(&workspace, &["--locked", "--offline"]);
    assert!(!offline.status.success());
    assert!(stderr(&offline).contains("offline"), "{}", stderr(&offline));
    assert_eq!(server.requests().len(), before, "offline asks nobody");

    let listing = publisher(&server, "/publisher/", "Fixture publisher", &[&words, &style]);
    assert_eq!(listing, feed);
    let restored = json(&sync(&workspace, &["--locked", "--index", &index_url]));
    assert_eq!(restored["lock"], "unchanged");
    assert_eq!(
        fs::read_to_string(workspace.join("jess.lock")).unwrap(),
        lock,
        "byte for byte"
    );
    assert!(
        server
            .requests()
            .iter()
            .skip(before)
            .any(|path| Path::new(path).extension().is_some_and(|extension| extension == "jpk")),
        "{:?}",
        server.requests()
    );
    assert!(
        !server.requests().iter().skip(before).any(|path| path == "/index.json"),
        "a locked restore never needs the index"
    );

    // A vendor folder with the same bytes restores offline, and the lock keeps its publisher.
    fs::remove_dir_all(&kept).unwrap();
    let vendored = json(&sync(
        &workspace,
        &["--locked", "--offline", "--package-path", jpks.to_str().unwrap()],
    ));
    assert_eq!(vendored["lock"], "unchanged");
    assert_eq!(fs::read_to_string(workspace.join("jess.lock")).unwrap(), lock);
    for package in vendored["packages"].as_array().unwrap() {
        assert_eq!(package["source"], "feed", "the same content is still the publisher's package");
    }
}

/// A publisher whose archive is not what its feed says, a feed that changed its story about a
/// pinned version, and two sites claiming one id each stop a sync with a diagnostic naming the
/// cause, and nothing is kept or pinned.
#[test]
fn a_sync_refuses_bytes_feeds_and_publishers_that_do_not_add_up() {
    let showcase = Showcase::new("workspace-refusals");
    let (workspace, jpks) = (showcase.workspace(), showcase.root.join("jpks"));
    fs::create_dir_all(&jpks).unwrap();
    let words = publish("words", &jpks);
    let style = publish("style", &jpks);
    let server = crate::common::http::Server::start();
    let feed = publisher(&server, "/publisher/", "Fixture publisher", &[&words, &style]);

    // Tampered bytes: the archive served is not the one the feed describes.
    server.serve(
        "/publisher/packages/org.example.words-1.2.0.jpk",
        "application/vnd.jess.jpk",
        fs::read(&style.path).unwrap(),
    );
    let tampered = sync(&workspace, &["--feed", &feed]);
    assert!(!tampered.status.success());
    let text = stderr(&tampered);
    assert!(text.contains("could not be downloaded"), "{text}");
    assert!(text.contains("the feed says") || text.contains("more than"), "{text}");
    assert!(!workspace.join("jess.lock").exists(), "nothing is pinned");
    let kept: Vec<String> = fs::read_dir(workspace.join(".jess/packages"))
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        kept.iter().all(|name| {
            Path::new(name).extension().is_some_and(|extension| extension == "jpk") && !name.starts_with("org.example.words")
        }),
        "the refused archive and no partial file is kept: {kept:?}"
    );

    // Honest bytes again, from a feed that lies about the content digest.
    publisher(&server, "/publisher/", "Fixture publisher", &[&words, &style]);
    let lying = Published {
        digest: "0".repeat(64),
        ..publish("words", &jpks)
    };
    let liar = publisher(&server, "/liar/", "A liar", &[&lying, &style]);
    let misdescribed = sync(&workspace, &["--feed", &liar]);
    assert!(!misdescribed.status.success());
    assert!(stderr(&misdescribed).contains("content digest"), "{}", stderr(&misdescribed));
    assert!(!workspace.join("jess.lock").exists());

    // Two sites claim org.example.words: no publisher is chosen for you.
    let other = publisher(&server, "/other/", "Somebody else", &[&words]);
    let index_url = index(
        &server,
        &[
            ("org.example.words", &feed),
            ("org.example.words", &other),
            ("org.example.style", &feed),
        ],
    );
    let ambiguous = sync(&workspace, &["--index", &index_url]);
    assert!(!ambiguous.status.success());
    let text = stderr(&ambiguous);
    assert!(
        text.contains("more than one site") && text.contains(&other) && text.contains("--feed"),
        "{text}"
    );

    // Naming the publisher settles it.
    let settled = json(&sync(&workspace, &["--index", &index_url, "--feed", &feed]));
    assert_eq!(settled["lock"], "written");

    // A pinned publisher whose feed now describes other bytes for the pinned version is refused.
    fs::remove_dir_all(workspace.join(".jess/packages")).unwrap();
    publisher(&server, "/publisher/", "Fixture publisher", &[&lying, &style]);
    let changed = sync(&workspace, &["--locked", "--feed", &feed]);
    assert!(!changed.status.success());
    assert!(
        stderr(&changed).contains("no longer describes the bytes"),
        "{}",
        stderr(&changed)
    );
}
