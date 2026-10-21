//! `jess init`: a script folder with types, an example, and editor settings.

use crate::common;

use common::*;
use serde_json::Value;

fn init(dir: &Path, args: &[&str]) -> Value {
    let output = jess()
        .args(["--format", "json", "init"])
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn written(report: &Value) -> Vec<String> {
    report["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|written| written["path"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn init_writes_types_an_example_and_each_editors_settings() {
    let root = temp_dir_path("init-editors");
    drop(fs::remove_dir_all(&root));

    for editor in ["vscode", "neovim", "zed", "helix", "none"] {
        let dir = root.join(editor).join("scripts");
        let report = init(&dir, &["--editor", editor]);
        assert_eq!(report["editor"], editor);
        let definitions = fs::read_to_string(dir.join("types/jess.d.jsl")).unwrap();
        assert!(definitions.contains("declare jess: {"));
        assert!(
            fs::read_to_string(dir.join("types/jess/log.jsl"))
                .unwrap()
                .contains("JessLogger")
        );
        assert_eq!(
            report["types"]["definitions"],
            dir.join("types/jess.d.jsl").display().to_string()
        );
        let jessrc: Value = serde_json::from_str(&fs::read_to_string(dir.join(".jessrc")).unwrap()).unwrap();
        assert_eq!(jessrc["languageMode"], "strict");
        assert_eq!(jessrc["aliases"]["jess"], "types/jess");
        assert_eq!(jessrc["aliases"]["dream"], "types/dream");
        assert!(fs::read_to_string(dir.join("main.jsl")).unwrap().starts_with("--!strict"));
        assert_ne!(report["next_steps"].as_array().unwrap().as_slice(), Vec::<Value>::new());
        match editor {
            "vscode" => {
                let settings: Value =
                    serde_json::from_str(&fs::read_to_string(dir.join(".vscode/settings.json")).unwrap()).unwrap();
                assert_eq!(settings["luau-lsp.types.definitionFiles"]["@jess"], "types/jess.d.jsl");
                assert_eq!(settings["luau-lsp.platform.type"], "standard");
                assert_eq!(settings["luau-lsp.fflags.enableNewSolver"], true);
            }
            "zed" => {
                let settings: Value =
                    serde_json::from_str(&fs::read_to_string(dir.join(".zed/settings.json")).unwrap()).unwrap();
                let luau = &settings["lsp"]["luau-lsp"]["settings"];
                assert_eq!(luau["definitions"][0], "types/jess.d.jsl");
                assert_eq!(luau["luau-lsp"]["platform"]["type"], "standard");
                assert_eq!(luau["fflags"]["enable_new_solver"], true);
            }
            "helix" => {
                let text = fs::read_to_string(dir.join(".helix/languages.toml")).unwrap();
                let languages: toml::Value = toml::from_str(&text).unwrap();
                let server = &languages["language-server"]["luau-lsp"];
                assert_eq!(server["command"].as_str(), Some("luau-lsp"));
                assert_eq!(server["args"][1].as_str(), Some("--definitions:@jess=types/jess.d.jsl"));
                assert_eq!(server["args"][2].as_str(), Some("--flag:LuauSolverV2=true"));
                assert_eq!(languages["language"][0]["name"].as_str(), Some("luau"));
            }
            "neovim" => {
                let config = fs::read_to_string(dir.join(".nvim.lua")).unwrap();
                assert!(config.contains("vim.lsp.config('luau_lsp'"), "{config}");
                assert!(config.contains("--definitions:@jess="), "{config}");
                assert!(config.contains("'--flag:LuauSolverV2=true'"), "{config}");
                assert!(report["next_steps"].to_string().contains("exrc"), "{report}");
            }
            _ => assert_eq!(written(&report).len(), 2, "{report}"),
        }
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rerunning_init_refreshes_the_types_and_keeps_your_files() {
    let dir = temp_dir_path("init-rerun");
    drop(fs::remove_dir_all(&dir));
    init(&dir, &[]);
    fs::write(dir.join("main.jsl"), "print('mine')\n").unwrap();
    fs::write(dir.join(".vscode/settings.json"), "{}").unwrap();
    fs::write(dir.join("types/jess.d.jsl"), "stale").unwrap();

    let again = init(&dir, &[]);
    assert!(written(&again).is_empty(), "{again}");
    assert_eq!(again["skipped"].as_array().unwrap().len(), 3, "{again}");
    assert_eq!(fs::read_to_string(dir.join("main.jsl")).unwrap(), "print('mine')\n");
    assert_eq!(fs::read_to_string(dir.join(".vscode/settings.json")).unwrap(), "{}");
    assert_ne!(fs::read_to_string(dir.join("types/jess.d.jsl")).unwrap(), "stale");

    let forced = init(&dir, &["--force"]);
    assert_eq!(written(&forced).len(), 3, "{forced}");
    assert_eq!(fs::read_to_string(dir.join("main.jsl.bak")).unwrap(), "print('mine')\n");
    assert!(fs::read_to_string(dir.join("main.jsl")).unwrap().starts_with("--!strict"));

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn init_package_writes_a_skeleton_that_tests_checks_and_runs() {
    let dir = temp_dir_path("init-package");
    drop(fs::remove_dir_all(&dir));
    let report = init(&dir, &["--editor", "none", "--package", "org.example.greeter"]);
    let mut paths = written(&report);
    paths.sort();
    let expected = [".jessrc", "jess.toml", "src/init.jsl", "tests/run.jsl", "tools/greeter.jsl"]
        .iter()
        .map(|name| dir.join(name).display().to_string())
        .collect::<Vec<_>>();
    assert_eq!(paths, expected, "{report}");
    assert!(!dir.join("main.jsl").exists());
    assert!(report["next_steps"].to_string().contains("jess package test"), "{report}");
    let manifest = fs::read_to_string(dir.join("jess.toml")).unwrap();
    assert!(manifest.contains("id = \"org.example.greeter\""), "{manifest}");
    assert!(manifest.contains("\"@greeter\" = \"src/init.jsl\""), "{manifest}");
    assert!(manifest.contains("id = \"example.greeter\""), "{manifest}");

    for command in [["package", "test"], ["package", "check"]] {
        let output = jess().args(["--format", "json"]).args(command).arg(&dir).output().unwrap();
        assert!(
            output.status.success(),
            "{:?}: {}",
            command,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let run = jess()
        .args(["package", "run", "--capability", "process.environment"])
        .arg(&dir)
        .env("USER", "ada")
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "Hello, ada.");

    let bad = jess()
        .args(["init", "--editor", "none", "--package", "Not.An.Id"])
        .arg(dir.join("bad"))
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("is not lowercase ASCII words joined by dots"),
        "{}",
        String::from_utf8_lossy(&bad.stderr)
    );

    fs::remove_dir_all(dir).unwrap();
}
