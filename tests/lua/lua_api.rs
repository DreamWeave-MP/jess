//! Generic require behavior without application configuration or VFS lookup.

mod common;

#[test]
fn relative_modules_are_cached_by_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("value.luau"), "return {}").unwrap();
    let main = dir.path().join("main.luau");
    std::fs::write(&main, "assert(require('./value') == require('./value'))").unwrap();
    jess::lua::scripts::run_file(&common::runtime(), &main, &[]).unwrap();
}

#[test]
fn bare_names_do_not_guess_application_data_paths() {
    let runtime = common::runtime();
    let error = runtime.exec("require('scripts.myMod.util')").unwrap_err();
    assert!(error.to_string().contains("module not found"), "{error}");
    common::run(&runtime, "assert(require('math') == math and require('jess') == jess)");
}
