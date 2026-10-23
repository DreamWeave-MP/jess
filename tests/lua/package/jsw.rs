//! Packed workspaces: what `pack` freezes, that the frozen plan is the workspace's, what a run
//! needs (the one file), what opening one refuses, and where an executable carries one.

use std::path::{Path, PathBuf};

use jess::lua::image::Image;
use jess::lua::package::TargetRun;
use jess::lua::workspace::{self, Locate, RunRequest};

use super::{common, library, text, write};

/// A workspace of two Package Sources, `org.example.app` and `org.example.lib`, with
/// `org.example.base` pinned as a JPK: what a run of the app's `hello` target resolves over.
fn workspace(root: &Path) -> PathBuf {
    let folder = root.join("workspace");
    library(&root.join("vendor/base"), "base", "", "return { name = 'base' }\n");
    library(
        &folder.join("lib"),
        "lib",
        "[dependencies]\n\"org.example.base\" = \"1\"\n",
        "return { name = 'lib on ' .. require('@base').name }\n",
    );
    write(
        &folder,
        &[
            (
                "jess-workspace.toml",
                "manifest_version = 1\n\n[workspace]\nmembers = [\"app\", \"lib\"]\n",
            ),
            (
                "app/jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.app\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
                 default_target = \"hello\"\ninclude = [\"src\"]\n[dependencies]\n\"org.example.lib\" = \"1\"\n\
                 [targets.hello]\nkind = \"command\"\ncommand = \"hello\"\nentry = \"src/main.jsl\"\ncapabilities = []\n",
            ),
            ("app/src/main.jsl", "return 'hello from ' .. require('@lib').name, ...\n"),
        ],
    );
    let jpk = root.join("base.jpk");
    jess::lua::package::pack(&common::runtime(), &text(&root.join("vendor/base")), &text(&jpk)).unwrap();
    let request = workspace::SyncRequest {
        at: Locate::Root(folder.clone()),
        artifacts: vec![text(&jpk)],
        paths: Vec::new(),
        locked: false,
        offline: true,
        indexes: Vec::new(),
        publishers: Vec::new(),
    };
    workspace::sync(&common::runtime(), &request).unwrap_or_else(|error| panic!("{error}"));
    folder
}

fn pack(folder: &Path, output: &Path) -> workspace::Packed {
    workspace::pack(&common::runtime(), &Locate::Root(folder.to_path_buf()), output)
        .unwrap_or_else(|error| panic!("{error}"))
}

fn packed(path: &Path) -> Locate {
    Locate::Packed(Image::open(path).unwrap())
}

fn run(at: Locate, target: Option<&str>, args: &[&str]) -> Result<TargetRun, String> {
    let request = RunRequest {
        at,
        target: target.map(ToOwned::to_owned),
        cwd: std::env::temp_dir(),
        grant: None,
        args: args.iter().map(ToString::to_string).collect(),
    };
    workspace::run_target(&jess::lua::Options::default(), &request).map_err(|error| error.to_string())
}

fn strings(run: &TargetRun) -> Vec<String> {
    run.values
        .iter()
        .map(|value| {
            run.runtime
                .stack()
                .with_frame(|frame| value.push_to(frame)?.read::<String>())
                .unwrap()
        })
        .collect()
}

/// A packed workspace holds each exported target's whole plan and nothing else, the same bytes
/// every time, and runs from the one file with the workspace gone, resolving exactly as the
/// workspace did.
#[test]
fn a_packed_workspace_is_the_workspace_s_plan_in_one_file() {
    let temp = tempfile::tempdir().unwrap();
    let folder = workspace(temp.path());
    let output = temp.path().join("app.jsw");
    let report = pack(&folder, &output);
    assert_eq!(report.default.as_deref(), Some("org.example.app:hello"));
    assert_eq!(report.targets, ["org.example.app:hello"]);
    assert_eq!(
        report.packages.iter().map(|package| package.id.as_str()).collect::<Vec<_>>(),
        ["org.example.app", "org.example.base", "org.example.lib"]
    );
    let again = temp.path().join("again.jsw");
    pack(&folder, &again);
    assert_eq!(std::fs::read(&output).unwrap(), std::fs::read(&again).unwrap());

    let runtime = common::runtime();
    let at = packed(&output);
    let cwd = std::env::temp_dir();
    let from_workspace = workspace::resolve_target(&runtime, &Locate::Root(folder.clone()), Some("hello"), &cwd).unwrap();
    let from_image = workspace::resolve_target(&runtime, &at, None, &cwd).unwrap();
    let identity = |resolved: &workspace::ResolvedTarget| {
        let packages = resolved
            .plan
            .packages
            .iter()
            .map(|package| {
                (
                    package.id.clone(),
                    package.version.clone(),
                    package.digest.clone(),
                    package.dependencies.clone(),
                )
            })
            .collect::<Vec<_>>();
        let modules = resolved
            .plan
            .modules
            .iter()
            .map(|module| (module.name.clone(), module.package.clone(), module.path.clone()))
            .collect::<Vec<_>>();
        (packages, modules, resolved.target.capabilities.clone())
    };
    assert_eq!(identity(&from_workspace), identity(&from_image));
    assert!(
        from_image
            .plan
            .packages
            .iter()
            .all(|package| package.kind == "jpk" && package.location.starts_with(&format!("{}!/packages/", text(&output)))),
        "{from_image:#?}"
    );

    std::fs::remove_dir_all(&folder).unwrap();
    std::fs::remove_dir_all(temp.path().join("vendor")).unwrap();
    std::fs::remove_file(temp.path().join("base.jpk")).unwrap();
    let ran = run(at, None, &["and more"]).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(strings(&ran), ["hello from lib on base", "and more"]);

    let opened = workspace::open(&runtime, &packed(&output)).unwrap();
    assert_eq!(opened.kind, "packed");
    assert_eq!(opened.members.len(), 1);
    assert_eq!(opened.members[0].default_target.as_deref(), Some("hello"));
    assert_eq!(opened.lock.packages.len(), 3);
    for doing in [
        workspace::test(&jess::lua::Options::default(), &packed(&output), &[], temp.path()).map(|_| ()),
        workspace::pack(&runtime, &packed(&output), &again).map(|_| ()),
    ] {
        let refused = doing.unwrap_err().to_string();
        assert!(refused.contains("is a packed workspace, which is sealed"), "{refused}");
    }
}

/// `[pack]` names the targets a packed workspace exports and its default, by the names `run`
/// takes; a lock that isn't fresh is never packed.
#[test]
fn a_workspace_file_says_what_is_packed() {
    let temp = tempfile::tempdir().unwrap();
    let folder = workspace(temp.path());
    let file = folder.join("jess-workspace.toml");
    let listed = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        format!("{listed}\n[pack]\ntargets = [\"hello\"]\ndefault = \"org.example.app\"\n"),
    )
    .unwrap();
    let report = pack(&folder, &temp.path().join("app.jsw"));
    assert_eq!(report.default.as_deref(), Some("org.example.app:hello"));

    std::fs::write(&file, format!("{listed}\n[pack]\ntargets = [\"nothing\"]\n")).unwrap();
    let unnamed = workspace::pack(&common::runtime(), &Locate::Root(folder.clone()), &temp.path().join("x.jsw"))
        .unwrap_err()
        .to_string();
    assert!(unnamed.contains("nothing in the workspace is named 'nothing'"), "{unnamed}");

    std::fs::write(&file, &listed).unwrap();
    std::fs::remove_file(folder.join("jess.lock")).unwrap();
    let stale = workspace::pack(&common::runtime(), &Locate::Root(folder), &temp.path().join("x.jsw"))
        .unwrap_err()
        .to_string();
    assert!(stale.contains("jess.lock is out of date"), "{stale}");
    assert!(!temp.path().join("x.jsw").exists());
}

/// Rewrites the packed workspace at `from` into `to` with `change`, a Luau chunk over `files`
/// (each `{ path, data, store }`, everything stored) and `index`, `jsw.toml`'s text, which is
/// written back. `extra` is a file the chunk can read, as the global `extra`.
fn rewritten(from: &Path, to: &Path, change: &str, extra: Option<&Path>) {
    let runtime = common::filesystem_runtime();
    common::set_global(&runtime, "from", text(from).as_str());
    common::set_global(&runtime, "to", text(to).as_str());
    if let Some(extra) = extra {
        common::set_global(&runtime, "extra", text(extra).as_str());
    }
    common::run(
        &runtime,
        &format!(
            "local jpk, fs = require '@jess/jpk', require '@dream/fs'\n\
             local archive = jpk.open(assert(fs.readFile(from)))\n\
             local files = {{}}\n\
             for _, entry in archive.entries do\n  \
               if entry.path ~= 'jsw.toml' then\n    \
                 table.insert(files, {{ path = entry.path, data = archive.read(entry), store = true }})\n  \
               end\n\
             end\n\
             local index = buffer.tostring(archive.read 'jsw.toml')\n\
             {change}\n\
             table.insert(files, {{ path = 'jsw.toml', data = buffer.fromstring(index), store = true }})\n\
             assert(fs.writeFile(to, jpk.write(files), {{ atomic = true }}))"
        ),
    );
}

/// A packed workspace is opened as untrusted input: an index that disagrees with what is held, a
/// package that isn't the one pinned, anything held that isn't listed, and a jess it wasn't packed
/// for are each refused, before anything a package holds runs.
#[test]
fn a_packed_workspace_that_does_not_add_up_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let folder = workspace(temp.path());
    let good = temp.path().join("app.jsw");
    let report = pack(&folder, &good);
    let base = &report
        .packages
        .iter()
        .find(|package| package.id == "org.example.base")
        .unwrap()
        .digest;
    let bad = temp.path().join("bad.jsw");
    let refusal = |change: &str| -> String {
        rewritten(&good, &bad, change, None);
        let opened = Image::open(&bad).map_err(|error| error.to_string());
        match opened.and_then(|image| run(Locate::Packed(image), None, &[]).map(|_| ())) {
            Ok(()) => panic!("expected a refusal for {change}"),
            Err(error) => error,
        }
    };

    let repinned = refusal(&format!("index = string.gsub(index, '{base}', string.rep('a', 64))"));
    assert!(
        repinned.contains(&format!("has digest {base}, not the {} it is pinned to", "a".repeat(64))),
        "{repinned}"
    );
    let stray = refusal("table.insert(files, { path = 'notes.txt', data = buffer.fromstring 'hi' })");
    assert!(stray.contains("holds 'notes.txt', which its index doesn't list"), "{stray}");
    let missing = refusal(
        "for at, file in files do\n  \
           if file.path == 'packages/org.example.base-1.0.0.jpk' then table.remove(files, at) break end\n\
         end",
    );
    assert!(
        missing.contains("lists org.example.base 1.0.0, which it doesn't hold"),
        "{missing}"
    );
    let swapped = refusal(
        "local lib\n\
         for _, file in files do if file.path == 'packages/org.example.lib-1.0.0.jpk' then lib = file.data end end\n\
         for _, file in files do if file.path == 'packages/org.example.base-1.0.0.jpk' then file.data = lib end end",
    );
    assert!(
        swapped.contains("is org.example.lib 1.0.0, not the org.example.base 1.0.0 its index lists"),
        "{swapped}"
    );
    let future = refusal("index = string.gsub(index, 'format_version = 1', 'format_version = 2')");
    assert!(
        future.contains("is format_version 2, and this jess reads format_version 1"),
        "{future}"
    );
    let elsewhere = refusal("index = string.gsub(index, 'builtin = \"[^\"]*\"', 'builtin = \"9.0.0\"')");
    assert!(
        elsewhere.contains("was packed for jess's built-in package 9.0.0"),
        "{elsewhere}"
    );
    let undeclared = refusal("index = string.gsub(index, 'id = \"hello\"', 'id = \"goodbye\"')");
    assert!(
        undeclared.contains("exports org.example.app:goodbye, which none of its packages declares"),
        "{undeclared}"
    );

    // A package claiming the built-in package's id is refused when opened, as one on disk is.
    let forged = temp.path().join("forged");
    write(
        &forged,
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.dreamweave.jess\"\nversion = \"0.1.0\"\njess_api = \"1\"\n\
                 include = [\"src\"]\n[targets.evil]\nkind = \"command\"\nentry = \"src/evil.jsl\"\n",
            ),
            ("src/evil.jsl", "error('this must never run')\n"),
        ],
    );
    let forged_jpk = temp.path().join("forged.jpk");
    jess::lua::package::pack(&common::runtime(), &text(&forged), &text(&forged_jpk)).unwrap();
    // Refused before its digest is ever compared, so any digest will do.
    let digest = "b".repeat(64);
    rewritten(
        &good,
        &bad,
        &format!(
            "table.insert(files, {{ path = 'packages/org.dreamweave.jess-0.1.0.jpk', data = assert(fs.readFile(extra)), store = true }})\n\
             index ..= '\\n[[target]]\\npackage = \"org.dreamweave.jess\"\\nid = \"evil\"\\n\\n[[package]]\\nid = \"org.dreamweave.jess\"\\nversion = \"0.1.0\"\\ndigest = \"{digest}\"\\n'"
        ),
        Some(&forged_jpk),
    );
    let impostor = run(packed(&bad), Some("org.dreamweave.jess:evil"), &[]).unwrap_err();
    assert!(impostor.contains("says it is jess's built-in package"), "{impostor}");
}
