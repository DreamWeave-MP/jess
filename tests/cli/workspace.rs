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
    assert_eq!(info["dev_dependencies"], serde_json::json!([]));
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

/// A JPK packed from a vendor fixture, with the facts a manifest states about it.
struct Published {
    id: String,
    version: String,
    digest: String,
    sha256: String,
    size: u64,
    path: PathBuf,
    /// Its declared `[dependencies]`, as the manifest must state them.
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

/// A publisher: a `DreamWeave` site whose projects are Jess packages, one project per package,
/// each with one release that is the package's JPK (protocol `schema_version` 3).
struct PublisherSite {
    /// The site's `dreamweave.json`.
    index: String,
    /// Each package's project manifest, by package id.
    manifests: std::collections::BTreeMap<String, String>,
}

impl PublisherSite {
    fn manifest(&self, id: &str) -> &str {
        &self.manifests[id]
    }
}

/// A project id for a fixture package: a UUID spelled from the package id's bytes.
fn project_id(id: &str) -> String {
    let hex: String = id
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .chain(std::iter::repeat_n("0".to_owned(), 32))
        .take(32)
        .collect();
    format!(
        "{}-{}-4{}-8{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

/// A publisher site on `server` under `base`: a project manifest per package, each listing the
/// package's one release and its archive, downloadable there, and an index naming which project
/// publishes which package.
fn publisher(server: &crate::common::http::Server, base: &str, name: &str, packages: &[&Published]) -> PublisherSite {
    let site = server.url(base);
    let repository = format!("https://github.com/fixture/{}", name.replace(' ', "-").to_lowercase());
    let mut entries = Vec::new();
    let mut manifests = std::collections::BTreeMap::new();
    for package in packages {
        let archive = format!("{base}packages/{}-{}.jpk", package.id, package.version);
        server.serve(&archive, "application/vnd.jess.jpk", fs::read(&package.path).unwrap());
        let project = project_id(&package.id);
        let manifest_path = format!("{base}dreamweave/projects/{project}.json");
        let manifest_url = server.url(&manifest_path);
        let page = format!("{site}{}/", package.id);
        let filename = format!("{}-{}.jpk", package.id, package.version);
        let manifest = serde_json::json!({
            "schema_version": "3",
            "document": "project",
            "generator": "fixture",
            "project": {
                "id": project, "name": package.id, "type": "library", "status": "active", "versioning": "numeric",
                "game": "morrowind", "tags": [], "maintainers": [],
                "links": { "page": page, "source": repository, "issues": format!("{repository}/issues") },
                "integrations": {}, "media": [], "credits": []
            },
            "channels": { "stable": { "version": package.version } },
            "releases": [{
                "version": package.version, "channel": "stable", "date": "2026-10-19", "status": "available",
                "source": { "repository": repository, "tag": format!("{}-{}", package.id, package.version) },
                "runtimes": {}, "platforms": [], "provides": [], "relationships": [], "components": [], "groups": [],
                "extensions": {}, "critical_extensions": [],
                "artifacts": [{
                    "id": "jpk", "format": "jpk", "filename": filename, "media_type": "application/vnd.jess.jpk",
                    "size": package.size, "digests": { "sha256": package.sha256 },
                    "jess": { "package": package.id, "jess_api": "1", "content_digest": package.digest, "dependencies": package.dependencies },
                    "sources": [{ "url": server.url(&archive), "kind": "publisher" }],
                    "signatures": []
                }]
            }]
        });
        server.serve(
            &manifest_path,
            "application/json",
            serde_json::to_vec_pretty(&manifest).unwrap(),
        );
        // jess reads an entry as a pointer and nothing more, so the digest here is not computed.
        entries.push(serde_json::json!({
            "id": project, "name": package.id, "type": "library", "jess_package": package.id, "status": "active",
            "page": page, "manifest": manifest_url, "manifest_sha256": "0".repeat(64), "updated": "2026-10-19",
            "channels": { "stable": package.version }
        }));
        manifests.insert(package.id.clone(), manifest_url);
    }
    let index = serde_json::json!({
        "schema_version": "3",
        "document": "index",
        "generator": "fixture",
        "site": { "name": name, "url": site },
        "projects": entries,
    });
    let path = format!("{base}dreamweave.json");
    server.serve(&path, "application/json", serde_json::to_vec_pretty(&index).unwrap());
    PublisherSite {
        index: server.url(&path),
        manifests,
    }
}

/// An index on `server` that says which manifest publishes which id, as the `AsHyAmS` file does.
fn index(server: &crate::common::http::Server, claims: &[(&str, &str)]) -> String {
    let packages: Vec<serde_json::Value> = claims
        .iter()
        .map(|(id, manifest)| serde_json::json!({ "id": id, "origin": "fixture", "manifest_url": manifest }))
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

/// A workspace with nothing of its own but content, the way a `DreamWeave` site is: it requires a
/// tool from a publisher, the tool requires a library the same site publishes, and nothing is
/// offered locally. Choosing reaches along the publisher one dependency at a time, as far as it goes, and the
/// tool's target then runs over the workspace.
#[test]
fn a_site_like_workspace_reaches_a_publisher_s_dependencies_of_dependencies() {
    let showcase = Showcase::new("workspace-publisher-chain");
    let (site, jpks) = (showcase.root.join("site"), showcase.root.join("jpks"));
    fs::create_dir_all(site.join("content")).unwrap();
    fs::create_dir_all(&jpks).unwrap();
    fs::write(site.join("content/index.md"), "# A site\n").unwrap();
    fs::write(
        site.join("jess.toml"),
        concat!(
            "manifest_version = 1\n\n[package]\nid = \"org.example.site\"\nversion = \"0.0.0\"\n",
            "jess_api = \"1\"\ninclude = [\"content\"]\n\n[dependencies]\n\"org.example.tool\" = \"1\"\n",
        ),
    )
    .unwrap();
    let words = publish("words", &jpks);
    let tool = publish("tool", &jpks);
    let server = crate::common::http::Server::start();
    let published = publisher(&server, "/publisher/", "Fixture publisher", &[&words, &tool]);

    let synced = json(&sync(&site, &["--publisher", &published.index]));
    assert_eq!(synced["lock"], "written");
    let mut ids: Vec<&str> = synced["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|package| package["id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, ["org.example.tool", "org.example.words"]);

    let ran = jess()
        .current_dir(&site)
        .args(["workspace", "run", "--trusted", "hello", "--", "world"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(0), "{}", stderr(&ran));
    assert_eq!(String::from_utf8(ran.stdout).unwrap(), "hello world from the tool\n");

    // A clean checkout restores both from the lock, asking the publisher for nothing it doesn't pin.
    fs::remove_dir_all(site.join(".jess")).unwrap();
    let restored = json(&sync(&site, &["--locked", "--publisher", &published.index]));
    assert_eq!(restored["lock"], "unchanged");
    assert_eq!(restored["packages"].as_array().unwrap().len(), 2);
}

/// What the showcase needs comes from a publisher's manifests found through an index: downloaded,
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
    let published = publisher(&server, "/publisher/", "Fixture publisher", &[&words, &style]);
    let index_url = index(
        &server,
        &[
            ("org.example.words", published.manifest("org.example.words")),
            ("org.example.style", published.manifest("org.example.style")),
        ],
    );

    let synced = json(&sync(&workspace, &["--index", &index_url]));
    assert_eq!(synced["lock"], "written");
    for package in synced["packages"].as_array().unwrap() {
        assert_eq!(package["source"], "publisher");
        assert_eq!(package["manifest"], published.manifest(package["id"].as_str().unwrap()));
        assert_eq!(package["installed"], true);
    }
    let lock = fs::read_to_string(workspace.join("jess.lock")).unwrap();
    assert!(lock.contains("lock_version = 2"), "{lock}");
    assert!(lock.contains("source = \"publisher\""), "{lock}");
    assert!(
        lock.contains(&format!("manifest = \"{}\"", published.manifest("org.example.words"))),
        "{lock}"
    );
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
    assert_eq!(listing.index, published.index);
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
        assert_eq!(
            package["source"], "publisher",
            "the same content is still the publisher's package"
        );
    }
}

/// A publisher whose archive is not what its manifest says, a manifest that changed its story about
/// a pinned version, and two sites claiming one id each stop a sync with a diagnostic naming the
/// cause, and nothing is kept or pinned.
#[test]
fn a_sync_refuses_bytes_manifests_and_publishers_that_do_not_add_up() {
    let showcase = Showcase::new("workspace-refusals");
    let (workspace, jpks) = (showcase.workspace(), showcase.root.join("jpks"));
    fs::create_dir_all(&jpks).unwrap();
    let words = publish("words", &jpks);
    let style = publish("style", &jpks);
    let server = crate::common::http::Server::start();
    let published = publisher(&server, "/publisher/", "Fixture publisher", &[&words, &style]);

    // Tampered bytes: the archive served is not the one the manifest describes.
    server.serve(
        "/publisher/packages/org.example.words-1.2.0.jpk",
        "application/vnd.jess.jpk",
        fs::read(&style.path).unwrap(),
    );
    let tampered = sync(&workspace, &["--publisher", &published.index]);
    assert!(!tampered.status.success());
    let text = stderr(&tampered);
    assert!(text.contains("could not be downloaded"), "{text}");
    assert!(text.contains("the manifest says") || text.contains("more than"), "{text}");
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

    // Honest bytes again, from a manifest that lies about the content digest.
    publisher(&server, "/publisher/", "Fixture publisher", &[&words, &style]);
    let lying = Published {
        digest: "0".repeat(64),
        ..publish("words", &jpks)
    };
    let liar = publisher(&server, "/liar/", "A liar", &[&lying, &style]);
    let misdescribed = sync(&workspace, &["--publisher", &liar.index]);
    assert!(!misdescribed.status.success());
    assert!(stderr(&misdescribed).contains("content digest"), "{}", stderr(&misdescribed));
    assert!(!workspace.join("jess.lock").exists());

    // Two sites claim org.example.words: no publisher is chosen for you.
    let other = publisher(&server, "/other/", "Somebody else", &[&words]);
    let index_url = index(
        &server,
        &[
            ("org.example.words", published.manifest("org.example.words")),
            ("org.example.words", other.manifest("org.example.words")),
            ("org.example.style", published.manifest("org.example.style")),
        ],
    );
    let ambiguous = sync(&workspace, &["--index", &index_url]);
    assert!(!ambiguous.status.success());
    let text = stderr(&ambiguous);
    assert!(
        text.contains("more than one site")
            && text.contains(other.manifest("org.example.words"))
            && text.contains("--publisher"),
        "{text}"
    );

    // Naming the publisher settles it.
    let settled = json(&sync(&workspace, &["--index", &index_url, "--publisher", &published.index]));
    assert_eq!(settled["lock"], "written");

    // A pinned publisher whose manifest now describes other bytes for the pinned version is refused.
    fs::remove_dir_all(workspace.join(".jess/packages")).unwrap();
    publisher(&server, "/publisher/", "Fixture publisher", &[&lying, &style]);
    let changed = sync(&workspace, &["--locked", "--publisher", &published.index]);
    assert!(!changed.status.success());
    assert!(
        stderr(&changed).contains("no longer describes the bytes"),
        "{}",
        stderr(&changed)
    );
}

/// A tool the workspace depends on runs by its command name, resolved over the workspace and the
/// packages the lock pins, without a wrapper in the workspace: what a site does with the
/// `DreamWeave` template.
#[test]
fn a_pinned_package_s_target_runs_over_the_workspace() {
    let showcase = Showcase::new("workspace-pinned-target");
    let (workspace, jpks) = (showcase.workspace(), showcase.root.join("jpks"));
    fs::create_dir_all(&jpks).unwrap();
    let words = publish("words", &jpks);
    let style = publish("style", &jpks);
    let tool = publish("tool", &jpks);
    // The showcase's root package takes the tool as a dependency, so the lock pins it.
    let manifest = workspace.join("jess.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replace("[dependencies]\n", "[dependencies]\n\"org.example.tool\" = \"1\"\n"),
    )
    .unwrap();
    let synced = sync(
        &workspace,
        &[
            "--offline",
            words.path.to_str().unwrap(),
            style.path.to_str().unwrap(),
            tool.path.to_str().unwrap(),
        ],
    );
    assert!(synced.status.success(), "{}", stderr(&synced));

    let listed = json(
        &jess()
            .args(["--format", "json", "workspace", "targets", "--workspace"])
            .arg(&workspace)
            .output()
            .unwrap(),
    );
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .all(|target| target["target"] != "org.example.tool:hello"),
        "targets lists the members' targets, not the pinned packages'"
    );

    let ran = jess()
        .current_dir(&workspace)
        .args(["workspace", "run", "--trusted", "hello", "--", "you", "me"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(0), "{}", stderr(&ran));
    assert_eq!(
        String::from_utf8(ran.stdout).unwrap(),
        "hello you from the tool\nhello me from the tool\n"
    );

    let by_id = jess()
        .current_dir(&workspace)
        .args(["workspace", "run", "--trusted", "org.example.tool:hello", "--", "x"])
        .output()
        .unwrap();
    assert_eq!(by_id.status.code(), Some(0), "{}", stderr(&by_id));

    let graph = json(
        &jess()
            .current_dir(&workspace)
            .args(["--format", "json", "workspace", "graph", "hello"])
            .output()
            .unwrap(),
    );
    assert_eq!(graph["package"], "org.example.tool");
    assert!(
        graph["location"]
            .as_str()
            .unwrap()
            .contains(".jess/packages/org.example.tool-1.0.0-")
    );

    let nowhere = jess()
        .current_dir(&workspace)
        .args(["workspace", "run", "--trusted", "nothing-here"])
        .output()
        .unwrap();
    assert!(!nowhere.status.success());
    assert!(
        stderr(&nowhere).contains("org.example.tool:hello"),
        "the error lists pinned targets too: {}",
        stderr(&nowhere)
    );
}
