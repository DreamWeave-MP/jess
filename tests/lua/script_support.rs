//! TOML through `@jess/data/toml`, and running files: what load-order scripts use besides
//! records.

use jess::lua::l3i::Runtime;

mod common;

use common::run;

fn lua() -> Runtime {
    common::runtime()
}

#[test]
fn toml_decodes_with_its_layout_through_the_one_codec() {
    run(
        &lua(),
        r#"
        local toml = require("@jess/data/toml")
        local value, layout = toml.decode([[
            zeta = 1
            alpha = 2.5
            flag = true
            list = [1, 2.0, "three"]

            [table.inner]
            name = "x"
            when = 1979-05-27
        ]])
        assert(value.zeta == 1 and value.alpha == 2.5 and value.flag == true)
        local keys = layout[value].keys
        assert(keys[1] == "zeta" and keys[2] == "alpha" and keys[3] == "flag" and keys[4] == "list")
        assert(layout[value].floats.alpha and not layout[value].floats.zeta)
        assert(layout[value.list].floats[2] and not layout[value.list].floats[1])
        assert(value.table.inner.name == "x" and value.table.inner.when == "1979-05-27")
        assert(layout[value.table.inner].datetimes.when and not layout[value.table.inner].datetimes.name)
        assert(not pcall(toml.decode, "key = "))
        assert(jess.toml == nil and not pcall(require, "@jess/toml"))
        "#,
    );
}

/// A runtime that runs a file twice runs what the file holds each time: bytecode is cached by
/// the source it came from, not just by the chunk name.
#[test]
fn a_file_run_again_after_an_edit_runs_the_edit() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("script-support-edited");
    drop(std::fs::remove_dir_all(&root));
    std::fs::create_dir_all(&root).unwrap();
    let script = root.join("edited.jsl");
    let runtime = lua();
    let run_text = |text: &str| {
        std::fs::write(&script, text).unwrap();
        let values = jess::lua::scripts::run_file(&runtime, &script, &[]).unwrap();
        runtime
            .stack()
            .with_frame(|frame| values[0].push_to(frame)?.read::<f64>())
            .unwrap()
    };
    let one = run_text("return 1");
    assert!((one - 1.0).abs() < f64::EPSILON, "{one}");
    let two = run_text("return 2");
    assert!((two - 2.0).abs() < f64::EPSILON, "{two}");
    std::fs::remove_dir_all(root).unwrap();
}
