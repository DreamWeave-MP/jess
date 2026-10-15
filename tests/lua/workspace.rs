//! Workspaces: which one a folder is in, what its members are, and what a workspace file may say.

use std::path::{Path, PathBuf};

use jess::lua::workspace::{self, Locate, Workspace};

use super::{common, text, write};

/// A package `org.example.<name>` at `root`, version `version`, with `extra` after its `[package]`
/// table: one layer, `example.<name>`, exporting `@<name>`.
fn member(root: &Path, name: &str, version: &str, extra: &str) {
    write(
        root,
        &[
            (
                "jess.toml",
                &format!(
                    "manifest_version = 1\n[package]\nid = \"org.example.{name}\"\nversion = \"{version}\"\n\
                     jess_api = \"1\"\ninclude = [\"src\"]\n{extra}\n[modules]\n\"@{name}\" = \"src/init.jsl\"\n\
                     [[layer]]\nid = \"example.{name}\"\nexports = [\"@{name}\"]\n"
                ),
            ),
            ("src/init.jsl", &format!("return {{ name = '{name}' }}\n")),
        ],
    );
}

/// A `jess-workspace.toml` in `root` listing `members`.
fn workspace_file(root: &Path, members: &[&str]) {
    let listed = members.iter().map(|path| format!("{path:?}")).collect::<Vec<_>>().join(", ");
    write(
        root,
        &[(
            "jess-workspace.toml",
            &format!("manifest_version = 1\n\n[workspace]\nmembers = [{listed}]\n"),
        )],
    );
}

fn open(at: &Locate) -> Workspace {
    workspace::open(&common::runtime(), at).unwrap_or_else(|error| panic!("{error}"))
}

fn from(path: &Path) -> Workspace {
    open(&Locate::From(path.to_path_buf()))
}

fn failure(at: &Locate) -> String {
    match workspace::open(&common::runtime(), at) {
        Ok(opened) => panic!("expected a failure, got {opened:#?}"),
        Err(error) => error.to_string(),
    }
}

/// Whether `reported`, a folder jess reported, is the folder `path`: the same place once links are
/// resolved, however each platform spells a resolved path.
fn same(reported: &str, path: &Path) -> bool {
    std::fs::canonicalize(reported).unwrap() == std::fs::canonicalize(path).unwrap()
}

fn paths(opened: &Workspace) -> Vec<&str> {
    opened.members.iter().map(|member| member.path.as_str()).collect()
}

/// A temporary folder with no workspace or package above it.
fn scratch() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    (temp, root)
}

/// A virtual workspace, a root package with members, and a package no workspace lists, each found
/// from its root, from inside a member, from a folder inside it that holds no package, and by
/// name.
#[test]
fn a_workspace_is_found_from_anywhere_inside_it() {
    let (_temp, root) = scratch();
    workspace_file(&root, &["tools/tool", "lib"]);
    member(&root.join("lib"), "lib", "1.0.0", "");
    member(
        &root.join("tools/tool"),
        "tool",
        "0.1.0",
        "[dependencies]\n\"org.example.lib\" = \"1\"\n",
    );
    std::fs::create_dir_all(root.join("docs")).unwrap();
    member(&root.join("examples/loose"), "loose", "1.0.0", "");

    for start in [
        root.clone(),
        root.join("lib"),
        root.join("tools/tool/src"),
        root.join("docs"),
        root.join("tools"),
    ] {
        let opened = from(&start);
        assert!(same(&opened.root, &root), "from {}: {}", start.display(), opened.root);
        assert_eq!(opened.kind, "virtual");
        // Listed tools/tool first; members are by path, whatever order the file lists them in.
        assert_eq!(paths(&opened), ["lib", "tools/tool"]);
    }
    let opened = open(&Locate::Root(root.clone()));
    assert_eq!(opened.members[1].id, "org.example.tool");
    assert!(same(&opened.members[1].location, &root.join("tools/tool")));
    assert_eq!(opened.members[1].dependencies[0].id, "org.example.lib");

    // A package under a workspace that doesn't list it is a workspace of its own, and says why.
    let loose = from(&root.join("examples/loose/src"));
    assert_eq!(loose.kind, "implicit");
    assert!(same(&loose.root, &root.join("examples/loose")), "{}", loose.root);
    assert_eq!(paths(&loose), ["."]);
    assert!(
        loose.note.as_deref().unwrap().contains("doesn't list this package"),
        "{loose:#?}"
    );

    // The root's own package is a member when the file lists `.`.
    let (_temp, rooted) = scratch();
    member(&rooted, "app", "1.0.0", "");
    member(&rooted.join("tools/editor"), "editor", "1.0.0", "");
    workspace_file(&rooted, &[".", "tools/editor"]);
    for start in [rooted.clone(), rooted.join("src"), rooted.join("tools/editor/src")] {
        let opened = from(&start);
        assert_eq!(opened.kind, "package");
        assert_eq!(paths(&opened), [".", "tools/editor"]);
    }

    // The root's package isn't a member unless it is listed, but the root is still the workspace's.
    workspace_file(&rooted, &["tools/editor"]);
    let opened = from(&rooted.join("src"));
    assert_eq!((opened.kind.as_str(), paths(&opened)), ("virtual", vec!["tools/editor"]));

    // No workspace above it: a package is a workspace of one, with nothing to say about it.
    let (_temp, alone) = scratch();
    member(&alone, "alone", "1.0.0", "");
    let opened = from(&alone.join("src"));
    assert_eq!((opened.kind.as_str(), opened.note.as_deref()), ("implicit", None));

    let (_temp, empty) = scratch();
    let nothing = failure(&Locate::From(empty.clone()));
    assert!(nothing.contains("no jess-workspace.toml or jess.toml in"), "{nothing}");
    let neither = failure(&Locate::Root(empty));
    assert!(
        neither.contains("holds neither a jess-workspace.toml nor a jess.toml"),
        "{neither}"
    );
}

/// Opening a workspace reads its file and its members' manifests, and runs nothing.
#[test]
fn opening_a_workspace_runs_nothing() {
    let (_temp, root) = scratch();
    workspace_file(&root, &["tool"]);
    write(
        &root.join("tool"),
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.tool\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
                 include = [\"src\"]\n[modules]\n\"@tool/start\" = \"src/start.jsl\"\n\
                 [[layer]]\nid = \"example.tool\"\n[[layer.entrypoint]]\nmodule = \"@tool/start\"\n",
            ),
            ("src/start.jsl", "error('a workspace ran package code')\n"),
        ],
    );
    assert_eq!(paths(&from(&root)), ["tool"]);
}

/// Every way a workspace file can name a member it shouldn't fails, naming the file and the member.
#[test]
fn a_workspace_file_names_its_members_plainly() {
    let (_temp, root) = scratch();
    member(&root.join("tools/tool"), "tool", "1.0.0", "");
    member(&root.join("tools/twin"), "tool", "2.0.0", "");
    member(&root.join("nested"), "nested", "1.0.0", "");
    workspace_file(&root.join("nested"), &["."]);
    write(
        &root,
        &[("bad/jess.toml", "manifest_version = 1\n[package]\nid = \"Not An Id\"\n")],
    );
    std::fs::create_dir_all(root.join("empty")).unwrap();
    write(&root, &[("file", "")]);

    for (members, expected) in [
        (&["../outside"][..], "member '../outside' is not a path of folder names"),
        (&["/absolute"], "member '/absolute' is not a path of folder names"),
        (&["tools\\tool"], "member 'tools\\tool' is not a path of folder names"),
        (&["C:/tool"], "member 'C:/tool' is not a path of folder names"),
        (&["./tools/tool"], "member './tools/tool' is not a path of folder names"),
        (&["tools/tool/"], "member 'tools/tool/' is not a path of folder names"),
        (&[""], "member '' is not a path of folder names"),
        (&["tools/tool", "tools/tool"], "lists 'tools/tool' twice"),
        // Refused by spelling, before either is looked for: Tools/ needn't exist, and on a
        // filesystem that ignores case it would be tools/.
        (
            &["tools/tool", "Tools/tool"],
            "members 'tools/tool' and 'Tools/tool' differ only in case",
        ),
        (&["missing"], "member 'missing' is not a folder"),
        (&["file"], "member 'file' is not a folder"),
        (&["empty"], "member 'empty' has no jess.toml"),
        (&["bad"], "member 'bad': package id 'Not An Id'"),
        (
            &["tools/tool", "tools/twin"],
            "members 'tools/tool' and 'tools/twin' are both org.example.tool",
        ),
        (
            &["nested"],
            "member 'nested' is a workspace of its own; workspaces don't nest",
        ),
        (&["."], "member '.' has no jess.toml"),
    ] {
        workspace_file(&root, members);
        let problem = failure(&Locate::Root(root.clone()));
        assert!(problem.contains("jess-workspace.toml"), "{problem}");
        assert!(problem.contains(expected), "{members:?}: {problem}");
    }

    for (contents, expected) in [
        (
            "manifest_version = 2\n[workspace]\nmembers = []\n",
            "manifest_version must be 1",
        ),
        ("manifest_version = 1\n", "has no [workspace] table"),
        ("manifest_version = 1\n[workspace]\n", "needs members, a list of folders"),
        (
            "manifest_version = 1\n[workspace]\nmembers = [1]\n",
            "members are folder paths, as strings",
        ),
        (
            "manifest_version = 1\n[workspace]\nmembers = []\nexclude = []\n",
            "has an unknown key 'exclude'",
        ),
        (
            "manifest_version = 1\n[package]\n[workspace]\nmembers = []\n",
            "has an unknown key 'package'",
        ),
        ("manifest_version = 1\n[workspace\n", "jess-workspace.toml'"),
    ] {
        write(&root, &[("jess-workspace.toml", contents)]);
        let problem = failure(&Locate::Root(root.clone()));
        assert!(problem.contains(expected), "{contents}: {problem}");
    }

    // A package two workspaces list belongs to neither until one lets it go.
    workspace_file(&root, &["nested"]);
    let both = failure(&Locate::From(root.join("nested/src")));
    assert!(both.contains("belongs to both"), "{both}");
    assert!(both.contains("workspaces don't nest"), "{both}");

    // A workspace with no members is a workspace, and a folder in it finds it.
    workspace_file(&root, &[]);
    std::fs::remove_file(root.join("nested/jess-workspace.toml")).unwrap();
    assert_eq!(from(&root.join("empty")).members.len(), 0);
}

/// A member reached through a link is the folder the link points to: it must be inside the
/// workspace, and not another member under a second name.
#[cfg(unix)]
#[test]
fn a_member_through_a_link_is_where_the_link_points() {
    let (temp, root) = scratch();
    member(&root.join("tool"), "tool", "1.0.0", "");
    member(&temp.path().join("outside"), "outside", "1.0.0", "");
    std::os::unix::fs::symlink(temp.path().join("outside"), root.join("escape")).unwrap();
    std::os::unix::fs::symlink(root.join("tool"), root.join("alias")).unwrap();

    workspace_file(&root, &["escape"]);
    let escape = failure(&Locate::Root(root.clone()));
    assert!(escape.contains("member 'escape' is '"), "{escape}");
    assert!(escape.contains("outside the workspace"), "{escape}");

    workspace_file(&root, &["tool", "alias"]);
    let alias = failure(&Locate::Root(root.clone()));
    assert!(alias.contains("members 'tool' and 'alias' are the same folder"), "{alias}");

    // A link inside the workspace to a folder inside it is that folder, under the name it's listed by.
    workspace_file(&root, &["alias"]);
    let opened = open(&Locate::Root(root.clone()));
    assert_eq!(opened.members[0].path, "alias");
    assert!(same(&opened.members[0].location, &root.join("tool")));
}

/// A workspace of four members: `app` and `tool`, each with targets; `lib`, which `app` depends
/// on; and `fixtures`, which only `app`'s tests need. `tool` declares `lib` too, to show a member's
/// dependency is still its own declaration.
fn targets(root: &Path) {
    workspace_file(root, &["app", "tool", "lib", "fixtures"]);
    member(&root.join("lib"), "lib", "1.2.0", "");
    member(&root.join("fixtures"), "fixtures", "1.0.0", "");
    member(
        &root.join("app"),
        "app",
        "0.1.0",
        "default_target = \"main\"\ntests = [\"tests/run.jsl\"]\n\
         [dependencies]\n\"org.example.lib\" = \"1\"\n[dev-dependencies]\n\"org.example.fixtures\" = \"1\"\n\
         [targets.main]\nkind = \"command\"\nentry = \"src/main.jsl\"\ncommand = \"app\"\n\
         capabilities = [\"process.environment\"]\n\
         [targets.check]\nkind = \"command\"\nentry = \"src/check.jsl\"\n\
         [targets.peek]\nkind = \"command\"\nentry = \"src/peek.jsl\"\n",
    );
    write(
        &root.join("app"),
        &[
            (
                "src/main.jsl",
                "local first, second = ...\nassert(first == arg[1] and #arg == 2)\n\
                 print(require('@lib').name, first, second)\nreturn 5\n",
            ),
            ("src/check.jsl", "return 0\n"),
            // A member's exports are its dependents' only: app doesn't declare tool.
            ("src/peek.jsl", "return require '@tool'\n"),
            (
                "tests/run.jsl",
                "local test = require '@jess/test'\nlocal suite = test.new()\n\
                 suite.it('sees its fixtures', function() test.expect(require('@fixtures').name).toBe 'fixtures' end)\n\
                 return suite\n",
            ),
        ],
    );
    member(
        &root.join("tool"),
        "tool",
        "0.1.0",
        "[dependencies]\n\"org.example.lib\" = \"^1.2\"\n\
         [targets.check]\nkind = \"command\"\nentry = \"src/check.jsl\"\ncommand = \"lint\"\n",
    );
    write(&root.join("tool"), &[("src/check.jsl", "return 0\n")]);
}

/// A target is named by package and target, by a target id or command name no other target has, or
/// by its member's package id; without a name, it is the default target of the member the command
/// runs in.
#[test]
fn a_workspace_target_is_named_or_found_from_where_it_runs() {
    let (_temp, root) = scratch();
    targets(&root);
    let runtime = common::runtime();
    let at = Locate::Root(root.clone());
    let chosen = |name: Option<&str>, cwd: &Path| -> (String, String) {
        let chosen = workspace::choose(&runtime, &at, name, cwd).unwrap_or_else(|error| panic!("{error}"));
        (chosen.package, chosen.target)
    };
    let picked = |package: &str, target: &str| (format!("org.example.{package}"), target.to_owned());
    for (name, expected) in [
        ("org.example.tool:check", picked("tool", "check")),
        ("org.example.app:check", picked("app", "check")),
        ("main", picked("app", "main")),
        ("app", picked("app", "main")),
        ("lint", picked("tool", "check")),
        ("org.example.app", picked("app", "main")),
    ] {
        assert_eq!(chosen(Some(name), &root), expected, "{name}");
    }
    assert_eq!(chosen(None, &root.join("app/src")), picked("app", "main"));

    let chosen = workspace::choose(&runtime, &at, Some("main"), &root).unwrap();
    assert!(same(&chosen.location, &root.join("app")), "{}", chosen.location);
    // The other members are its candidates, opened only if it needs them.
    assert_eq!(chosen.candidates.len(), 3);
    assert!(!chosen.candidates.contains(&chosen.location));

    for (name, cwd, expected) in [
        (
            Some("check"),
            &root,
            "'check' could be any of org.example.app:check, org.example.tool:check",
        ),
        (Some("nothing"), &root, "nothing in the workspace is named 'nothing'"),
        (Some("org.example.tool"), &root, "org.example.tool has no default_target"),
        (Some("org.example.tool:main"), &root, "org.example.tool has no target 'main'"),
        (
            Some("org.example.other:main"),
            &root,
            "no member of the workspace is org.example.other",
        ),
        (None, &root, "name a target to run"),
        (None, &root.join("tool/src"), "org.example.tool has no default_target"),
    ] {
        let problem = workspace::choose(&runtime, &at, name, cwd).unwrap_err().to_string();
        assert!(problem.contains(expected), "{name:?}: {problem}");
        // Every failure but an ambiguity, which lists its own choices, lists the targets there are.
        assert!(
            problem.contains("could be any of")
                || problem.contains("the workspace has: org.example.app:check, org.example.app:main"),
            "{problem}"
        );
    }
}

/// A workspace target runs as its package's target does, resolving over the other members: with
/// its arguments, its exit value, exactly the capabilities it declares once granted, and only the
/// dependencies it declares.
#[test]
fn a_workspace_target_runs_over_its_members() {
    let (_temp, root) = scratch();
    targets(&root);
    let options = jess::lua::Options::default();
    let request = |target: &str, grant: Option<&[&str]>| workspace::RunRequest {
        at: Locate::From(root.join("tool")),
        target: Some(target.to_owned()),
        cwd: root.clone(),
        grant: grant.map(|grant| grant.iter().map(ToString::to_string).collect()),
        args: vec!["one".to_owned(), "two".to_owned()],
    };
    let ran = workspace::run_target(&options, &request("app", None)).unwrap_or_else(|error| panic!("{error}"));
    let status = ran
        .runtime
        .stack()
        .with_frame(|frame| ran.values[0].push_to(frame)?.read::<f64>())
        .unwrap();
    assert_eq!(status, 5.0);

    let denied = workspace::run_target(&options, &request("app", Some(&[])))
        .unwrap_err()
        .to_string();
    assert!(
        denied.contains("required capability denied:\n    process.environment"),
        "{denied}"
    );
    workspace::run_target(&options, &request("app", Some(&["process.environment"])))
        .unwrap_or_else(|error| panic!("{error}"));

    let undeclared = workspace::run_target(&options, &request("peek", Some(&[])))
        .unwrap_err()
        .to_string();
    assert!(undeclared.contains("module not found: @tool"), "{undeclared}");

    // What app's tests need is no part of what its target runs with.
    let resolved = workspace::resolve_target(&common::runtime(), &request("app", None).at, Some("main"), &root)
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        resolved
            .plan
            .packages
            .iter()
            .map(|package| package.id.as_str())
            .collect::<Vec<_>>(),
        ["org.example.app", "org.example.lib"]
    );
    assert_eq!(resolved.target.capabilities, ["process.environment"]);
}

/// Members' tests run over the workspace, with their dev-dependencies: every member that has
/// tests, or the ones named.
#[test]
fn workspace_tests_resolve_over_the_members() {
    let (temp, root) = scratch();
    targets(&root);
    let options = jess::lua::Options::default();
    let at = Locate::Root(root);
    for names in [vec![], vec!["org.example.app".to_owned()], vec!["app".to_owned()]] {
        let reports =
            workspace::test(&options, &at, &names, &temp.path().join("scratch")).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(reports.len(), 1, "{names:?}");
        assert_eq!((reports[0].passed, reports[0].failed), (2, 0), "{reports:#?}");
    }
    let unknown = workspace::test(&options, &at, &["nothing".to_owned()], &temp.path().join("scratch"))
        .unwrap_err()
        .to_string();
    assert!(unknown.contains("no member of the workspace is 'nothing'"), "{unknown}");
}

/// Syncs the workspace at `root` from `artifacts` and the package paths `paths`.
fn sync(root: &Path, artifacts: &[&Path], paths: &[&Path], locked: bool) -> Result<workspace::Synced, String> {
    let request = workspace::SyncRequest {
        at: Locate::Root(root.to_path_buf()),
        artifacts: artifacts.iter().map(|path| text(path)).collect(),
        paths: paths.iter().map(|path| text(path)).collect(),
        locked,
    };
    workspace::sync(&common::runtime(), &request).map_err(|error| error.to_string())
}

/// Packs the Package Source at `source` into `output`, returning its digest.
fn pack(source: &Path, output: &Path) -> String {
    jess::lua::package::pack(&common::runtime(), &text(source), &text(output))
        .unwrap_or_else(|error| panic!("{error}"))
        .digest
}

/// What running `target` of the workspace at `root` prints, or why it couldn't run.
fn run(root: &Path, target: &str) -> Result<(), String> {
    let request = workspace::RunRequest {
        at: Locate::Root(root.to_path_buf()),
        target: Some(target.to_owned()),
        cwd: root.to_path_buf(),
        grant: None,
        args: Vec::new(),
    };
    workspace::run_target(&jess::lua::Options::default(), &request)
        .map(drop)
        .map_err(|error| error.to_string())
}

/// A workspace like the showcase: `app` uses `words` and, for its tests, `fixtures`; `tool` uses
/// `style`, which uses `words` itself. `vendor` gets the Package Sources of those three at the
/// versions given, `words` at each.
fn dependents(root: &Path, vendor: &Path, words: &[&str]) {
    workspace_file(root, &["app", "tool"]);
    member(
        &root.join("app"),
        "app",
        "0.1.0",
        "tests = [\"tests/run.jsl\"]\n[dependencies]\n\"org.example.words\" = \"1\"\n\
         [dev-dependencies]\n\"org.example.fixtures\" = \"1\"\n\
         [targets.main]\nkind = \"command\"\nentry = \"src/main.jsl\"\n",
    );
    write(
        &root.join("app"),
        &[
            ("src/main.jsl", "assert(require('@words').name == 'words')\n"),
            (
                "tests/run.jsl",
                "local test = require '@jess/test'\nlocal suite = test.new()\n\
                 suite.it('has its fixtures', function() test.expect(require('@fixtures').name).toBe 'fixtures' end)\n\
                 return suite\n",
            ),
        ],
    );
    member(
        &root.join("tool"),
        "tool",
        "0.1.0",
        "[dependencies]\n\"org.example.style\" = \"1\"\n\
         [targets.main]\nkind = \"command\"\nentry = \"src/main.jsl\"\n",
    );
    write(
        &root.join("tool"),
        &[("src/main.jsl", "assert(require('@style').name == 'style')\n")],
    );
    member(
        &vendor.join("style"),
        "style",
        "1.0.0",
        "[dependencies]\n\"org.example.words\" = \"^1.1\"\n",
    );
    member(&vendor.join("fixtures"), "fixtures", "1.0.0", "");
    for version in words {
        member(&vendor.join(format!("words-{version}")), "words", version, "");
    }
}

/// Packs every Package Source in `vendor` into `jpks`, as `<folder>.jpk`.
fn pack_all(vendor: &Path, jpks: &Path) {
    std::fs::create_dir_all(jpks).unwrap();
    for entry in std::fs::read_dir(vendor).unwrap() {
        let source = entry.unwrap().path();
        let name = format!("{}.jpk", source.file_name().unwrap().to_string_lossy());
        pack(&source, &jpks.join(name));
    }
}

/// `sync` pins one package for each id the members need beyond each other, dev-dependencies and
/// what dependencies depend on included, keeps them as JPKs in `.jess/packages`, and writes the
/// same `jess.lock` every time. From then on the workspace runs from those alone, wherever it is.
#[test]
fn sync_pins_what_the_members_need_and_runs_without_it() {
    let (temp, root) = scratch();
    let (vendor, jpks) = (temp.path().join("vendor"), temp.path().join("jpks"));
    dependents(&root, &vendor, &["1.2.0"]);
    pack_all(&vendor, &jpks);

    let before = from(&root);
    assert_eq!((before.lock.file, before.lock.state.as_str()), (false, "stale"));
    let stale = run(&root, "org.example.app:main").unwrap_err();
    assert!(stale.contains("jess.lock is out of date:"), "{stale}");
    assert!(
        stale.contains("org.example.app 0.1.0 requires org.example.words 1, which jess.lock doesn't pin"),
        "{stale}"
    );

    let synced = sync(&root, &[], &[&jpks], false).unwrap();
    assert_eq!(synced.lock, "written");
    assert_eq!(
        synced.packages.iter().map(|package| package.id.as_str()).collect::<Vec<_>>(),
        ["org.example.fixtures", "org.example.style", "org.example.words"]
    );
    assert!(synced.packages.iter().all(|package| package.installed));
    let lock = std::fs::read_to_string(root.join("jess.lock")).unwrap();
    assert!(
        lock.contains("[[package]]\nid = \"org.example.style\"\nversion = \"1.0.0\"\n"),
        "{lock}"
    );
    assert!(
        lock.contains("source = \"local\"\ndependencies = [\"org.example.words\"]\n"),
        "{lock}"
    );
    // Nothing in it depends on where the workspace or the packages it came from are.
    assert!(!lock.contains(&text(temp.path())), "{lock}");
    for package in &synced.packages {
        let kept =
            root.join(".jess/packages")
                .join(format!("{}-{}-{}.jpk", package.id, package.version, &package.digest[..16]));
        assert!(kept.is_file(), "{}", kept.display());
    }

    // Again, it changes nothing.
    let again = sync(&root, &[], &[&jpks], false).unwrap();
    assert_eq!(again.lock, "unchanged");
    assert!(again.packages.iter().all(|package| !package.installed));
    assert_eq!(std::fs::read_to_string(root.join("jess.lock")).unwrap(), lock);
    assert_eq!(sync(&root, &[], &[], true).unwrap().lock, "unchanged");

    std::fs::remove_dir_all(&vendor).unwrap();
    std::fs::remove_dir_all(&jpks).unwrap();
    let opened = from(&root.join("app"));
    assert_eq!(opened.lock.state, "fresh", "{:#?}", opened.lock);
    run(&root, "org.example.app:main").unwrap();
    run(&root, "org.example.tool:main").unwrap();

    // Its tests have their dev-dependency; its target doesn't.
    let reports = workspace::test(
        &jess::lua::Options::default(),
        &Locate::Root(root.clone()),
        &[],
        &temp.path().join("scratch"),
    )
    .unwrap();
    assert_eq!((reports[0].passed, reports[0].failed), (2, 0), "{reports:#?}");
    let resolved = workspace::resolve_target(
        &common::runtime(),
        &Locate::Root(root.clone()),
        Some("org.example.app:main"),
        &root,
    )
    .unwrap();
    assert_eq!(
        resolved
            .plan
            .packages
            .iter()
            .map(|package| package.id.as_str())
            .collect::<Vec<_>>(),
        ["org.example.app", "org.example.words"]
    );
    assert_eq!(resolved.pins.len(), 3);

    // Moved somewhere else, it is the same workspace.
    let moved = temp.path().join("moved");
    std::fs::rename(&root, &moved).unwrap();
    assert_eq!(from(&moved).lock.state, "fresh");
    run(&moved, "org.example.app:main").unwrap();
}

/// A Package Source offered is packed into `.jess/packages`, with the digest its source has; a
/// package named outright is the one for its id; and what is named must be something the
/// workspace needs, and not one of its members.
#[test]
fn sync_packs_sources_and_takes_what_is_named() {
    let (temp, root) = scratch();
    let vendor = temp.path().join("vendor");
    dependents(&root, &vendor, &["1.2.0", "1.3.0"]);

    let ambiguous = sync(&root, &[], &[&vendor], false).unwrap_err();
    assert!(
        ambiguous.contains("org.example.words is offered more than once: 1.2.0 (source "),
        "{ambiguous}"
    );
    assert!(
        ambiguous.contains("name the one to pin with jess workspace sync <file>"),
        "{ambiguous}"
    );
    assert!(!root.join("jess.lock").exists());

    let older = vendor.join("words-1.2.0");
    let synced = sync(&root, &[&older], &[&vendor], false).unwrap();
    let words = synced
        .packages
        .iter()
        .find(|package| package.id == "org.example.words")
        .unwrap();
    assert_eq!(words.version, "1.2.0");
    let digest = jess::lua::package::inspect(&common::runtime(), &text(&older)).unwrap().digest;
    assert_eq!(words.digest, digest);

    // A newer version offered later doesn't move a pin that still does.
    std::fs::remove_dir_all(&older).unwrap();
    let kept = sync(&root, &[], &[&vendor], false).unwrap();
    assert_eq!(kept.lock, "unchanged");
    // Named outright, the newer one is the one.
    let newer = sync(&root, &[&vendor.join("words-1.3.0")], &[&vendor], false).unwrap();
    assert_eq!(newer.lock, "written");
    assert!(
        newer
            .packages
            .iter()
            .any(|package| package.id == "org.example.words" && package.version == "1.3.0")
    );

    let member = sync(&root, &[&root.join("tool")], &[&vendor], false).unwrap_err();
    assert!(
        member.contains("is org.example.tool, which is the workspace's member 'tool'"),
        "{member}"
    );
    member_alone(&temp.path().join("unused"), "unused");
    let unused = sync(&root, &[&temp.path().join("unused")], &[&vendor], false).unwrap_err();
    assert!(
        unused.contains("is org.example.unused, which nothing in the workspace requires"),
        "{unused}"
    );
}

fn member_alone(root: &Path, name: &str) {
    member(root, name, "1.0.0", "");
}

/// Every requirement on an id is met by the one version pinned for it, or sync says which
/// requirements nothing offered meets, and who made them.
#[test]
fn sync_meets_every_requirement_or_says_whose_it_cannot() {
    let (temp, root) = scratch();
    let vendor = temp.path().join("vendor");
    dependents(&root, &vendor, &["1.2.0", "1.3.0"]);
    // app wants exactly 1.2.0; style, which tool uses, wants at least 1.3.
    let manifest = root.join("app/jess.toml");
    let text_of = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text_of.replace("\"org.example.words\" = \"1\"", "\"org.example.words\" = \"=1.2.0\""),
    )
    .unwrap();
    member(
        &vendor.join("style"),
        "style",
        "1.0.0",
        "[dependencies]\n\"org.example.words\" = \"^1.3\"\n",
    );
    let conflict = sync(&root, &[], &[&vendor], false).unwrap_err();
    assert!(
        conflict.contains("nothing offered for org.example.words is one the workspace can use"),
        "{conflict}"
    );
    assert!(
        conflict.contains("offered, but not meeting that: 1.2.0 (source"),
        "{conflict}"
    );
    assert!(
        conflict.contains("org.example.app 0.1.0 requires org.example.words =1.2.0"),
        "{conflict}"
    );
    assert!(
        conflict.contains("org.example.style 1.0.0 requires org.example.words ^1.3"),
        "{conflict}"
    );

    // A requirement found only after its id was settled is checked against what was settled.
    std::fs::write(
        &manifest,
        text_of.replace(
            "\"org.example.words\" = \"1\"",
            "\"org.example.base\" = \"1\"\n\"org.example.zed\" = \"1\"",
        ),
    )
    .unwrap();
    member(&vendor.join("base"), "base", "1.0.0", "");
    member(
        &vendor.join("zed"),
        "zed",
        "1.0.0",
        "[dependencies]\n\"org.example.base\" = \"^1.5\"\n",
    );
    member(
        &vendor.join("style"),
        "style",
        "1.0.0",
        "[dependencies]\n\"org.example.words\" = \"^1.3\"\n",
    );
    let late = sync(&root, &[&vendor.join("words-1.3.0")], &[&vendor], false).unwrap_err();
    assert!(
        late.contains("no one org.example.base meets every requirement on it; 1.0.0 is the one selected"),
        "{late}"
    );
    assert!(
        late.contains("org.example.zed 1.0.0 requires org.example.base ^1.5"),
        "{late}"
    );

    // A member a requirement names is the member, whatever is offered.
    let (_temp, both) = scratch();
    workspace_file(&both, &["lib", "user"]);
    member(&both.join("lib"), "lib", "1.0.0", "");
    member(
        &both.join("user"),
        "user",
        "1.0.0",
        "[dependencies]\n\"org.example.lib\" = \"2\"\n",
    );
    let expected =
        "org.example.user 1.0.0 requires org.example.lib 2, and the workspace's member 'lib' is org.example.lib 1.0.0";
    let outgrown = sync(&both, &[], &[], false).unwrap_err();
    assert!(outgrown.contains(expected), "{outgrown}");
    // Opening the workspace says the same, though nothing outside it is needed.
    let opened = from(&both);
    assert_eq!(opened.lock.state, "stale");
    assert_eq!(opened.lock.problems, [expected]);
}

/// The `dependents` workspace in a temporary folder, synced from its packages packed as JPKs:
/// the folder, the workspace's root, the folder of JPKs, and the lock the sync wrote.
fn synced() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let (temp, root) = scratch();
    let (vendor, jpks) = (temp.path().join("vendor"), temp.path().join("jpks"));
    dependents(&root, &vendor, &["1.2.0"]);
    pack_all(&vendor, &jpks);
    sync(&root, &[], &[&jpks], false).unwrap();
    let lock = std::fs::read_to_string(root.join("jess.lock")).unwrap();
    (temp, root, jpks, lock)
}

/// A package `jess.lock` pins that `.jess/packages` lacks stops a run until a sync puts it back,
/// and a kept package that isn't the one pinned never runs.
#[test]
fn a_lock_runs_only_the_packages_it_pins() {
    let (temp, root, jpks, _lock) = synced();
    let words = from(&root)
        .lock
        .packages
        .into_iter()
        .find(|package| package.id == "org.example.words")
        .unwrap();
    let kept = PathBuf::from(&words.location);

    // Missing from .jess/packages: a run names the file, sync without a source can't help, and a
    // sync that can puts it back without touching the lock, even --locked.
    std::fs::rename(&kept, temp.path().join("aside.jpk")).unwrap();
    assert_eq!(from(&root).lock.state, "missing");
    let missing = run(&root, "org.example.app:main").unwrap_err();
    assert!(
        missing.contains("jess.lock pins packages .jess/packages doesn't hold"),
        "{missing}"
    );
    assert!(
        missing.contains(&format!(
            "'.jess/packages/{}' isn't there",
            kept.file_name().unwrap().to_string_lossy()
        )),
        "{missing}"
    );
    let nowhere = sync(&root, &[], &[], false).unwrap_err();
    assert!(
        nowhere.contains("jess.lock pins 1.2.0, which isn't in .jess/packages"),
        "{nowhere}"
    );
    let restored = sync(&root, &[], &[&jpks], true).unwrap();
    assert_eq!(restored.lock, "unchanged");
    assert!(kept.is_file());

    // Not what was pinned: a different words 1.2.0 under the pinned name.
    std::fs::rename(&kept, temp.path().join("aside.jpk")).unwrap();
    member(&temp.path().join("forged"), "words", "1.2.0", "");
    write(
        &temp.path().join("forged"),
        &[("src/init.jsl", "error('a forged package ran')\n")],
    );
    pack(&temp.path().join("forged"), &kept);
    assert_eq!(from(&root).lock.state, "fresh");
    let forged = run(&root, "org.example.app:main").unwrap_err();
    assert!(
        forged.contains(&format!("not the {} it is pinned to", words.digest)),
        "{forged}"
    );
    std::fs::write(&kept, b"not a jpk").unwrap();
    assert_eq!(from(&root).lock.state, "missing");
    let fixed = sync(&root, &[], &[&jpks], false).unwrap();
    assert!(
        fixed
            .packages
            .iter()
            .any(|package| package.id == "org.example.words" && package.installed)
    );
    run(&root, "org.example.app:main").unwrap();
}

/// A change to what the members require makes `jess.lock` stale, and a lock this jess can't read,
/// or that names a file outside `.jess/packages`, opens nothing.
#[test]
fn a_lock_is_held_to_what_the_members_need() {
    let (_temp, root, jpks, lock) = synced();
    let manifest = root.join("tool/jess.toml");
    let original = std::fs::read_to_string(&manifest).unwrap();
    // A sync that could fix it does nothing with --locked; one that couldn't says why.
    for (changed, expected, refused) in [
        (
            "\"org.example.style\" = \"2\"",
            "org.example.tool 0.1.0 requires org.example.style 2, and jess.lock pins 1.0.0",
            "nothing offered for org.example.style is one the workspace can use",
        ),
        (
            "\"org.example.style\" = \"1\"\n\"org.example.other\" = \"1\"",
            "requires org.example.other 1, which jess.lock doesn't pin",
            "nothing offered for org.example.other is one the workspace can use",
        ),
        (
            "",
            "jess.lock pins org.example.style 1.0.0, which nothing in the workspace requires",
            "isn't what syncing would write, and --locked leaves it as it is",
        ),
    ] {
        std::fs::write(&manifest, original.replace("\"org.example.style\" = \"1\"", changed)).unwrap();
        let opened = from(&root);
        assert_eq!(opened.lock.state, "stale");
        assert!(
            opened.lock.problems.iter().any(|problem| problem.contains(expected)),
            "{:#?}",
            opened.lock.problems
        );
        let locked = sync(&root, &[], &[&jpks], true).unwrap_err();
        assert!(locked.contains(refused), "{locked}");
        assert_eq!(std::fs::read_to_string(root.join("jess.lock")).unwrap(), lock);
    }
    std::fs::write(&manifest, &original).unwrap();

    // A lock this jess can't read, or that names a file outside the store, opens nothing.
    for (from_text, to_text, expected) in [
        (
            "lock_version = 1",
            "lock_version = 2",
            "is lock_version 2, and this jess reads lock_version 1",
        ),
        (
            "id = \"org.example.words\"",
            "id = \"../../escape\"",
            "id must be a package id",
        ),
        (
            "source = \"local\"",
            "source = \"registry\"",
            "source 'registry' is not one this jess knows",
        ),
    ] {
        std::fs::write(root.join("jess.lock"), lock.replacen(from_text, to_text, 1)).unwrap();
        let problem = failure(&Locate::Root(root.clone()));
        assert!(problem.contains(expected), "{problem}");
    }
}

/// Two versions of one package kept side by side are no problem for the workspace, whose lock
/// says which, and still are for a package path, which never chooses between them.
#[test]
fn the_lock_chooses_between_kept_versions() {
    let (temp, root) = scratch();
    let vendor = temp.path().join("vendor");
    dependents(&root, &vendor, &["1.2.0", "1.3.0"]);
    write(
        &vendor.join("words-1.3.0"),
        &[("src/init.jsl", "return { name = 'newer words' }\n")],
    );
    sync(&root, &[&vendor.join("words-1.3.0")], &[&vendor], false).unwrap();
    sync(&root, &[&vendor.join("words-1.2.0")], &[&vendor], false).unwrap();
    let kept = std::fs::read_dir(root.join(".jess/packages"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("org.example.words-")
        })
        .count();
    assert_eq!(kept, 2);
    // The pinned 1.2.0 is the one that runs: app asserts its name is 'words'.
    run(&root, "org.example.app:main").unwrap();

    let search = jess::lua::package::Search {
        paths: vec![text(&root.join(".jess/packages"))],
        roots: vec!["org.example.words".to_owned()],
        ..Default::default()
    };
    let ambiguous = jess::lua::package::graph_with(&common::runtime(), &[], &[], &search)
        .unwrap_err()
        .to_string();
    assert!(ambiguous.contains("has more than one candidate"), "{ambiguous}");
}
