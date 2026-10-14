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

fn canonical(path: &Path) -> String {
    text(&std::fs::canonicalize(path).unwrap())
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
        assert_eq!(opened.root, canonical(&root), "from {}", start.display());
        assert_eq!(opened.kind, "virtual");
        // Listed tools/tool first; members are by path, whatever order the file lists them in.
        assert_eq!(paths(&opened), ["lib", "tools/tool"]);
    }
    let opened = open(&Locate::Root(root.clone()));
    assert_eq!(opened.members[1].id, "org.example.tool");
    assert_eq!(opened.members[1].location, canonical(&root.join("tools/tool")));
    assert_eq!(opened.members[1].dependencies[0].id, "org.example.lib");

    // A package under a workspace that doesn't list it is a workspace of its own, and says why.
    let loose = from(&root.join("examples/loose/src"));
    assert_eq!(loose.kind, "implicit");
    assert_eq!(loose.root, canonical(&root.join("examples/loose")));
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
    member(&root.join("Tools/tool"), "upper", "1.0.0", "");
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
    assert_eq!(opened.members[0].location, canonical(&root.join("tool")));
}
