//! Generic runtime surface contracts; application APIs are package-owned.

mod common;

#[test]
fn only_generic_members_are_installed() {
    let runtime = common::runtime();
    common::run(
        &runtime,
        r"
        for _, name in { 'plugin', 'nif', 'land', 'save', 'openmw', 'luadata', 'compare', 'select', 'meta' } do
            assert(jess[name] == nil, name)
            assert(not pcall(require, '@jess/' .. name), name)
        end
        assert(vfstool == nil and openmwConfig == nil and dreamPath == nil)
        for _, name in { 'fs', 'regex', 'shader', 'context', 'worker', 'input', 'bench' } do
            assert(require('@jess/' .. name) == jess[name], name)
        end
    ",
    );
}

/// `jess.bench` says which build this is, and collects on request: a weak table's unreachable
/// entry survives until then and not after.
#[test]
fn bench_names_the_build_and_collects() {
    let runtime = common::runtime();
    common::set_global(&runtime, "verified", &jess::lua::bench::VERIFIED);
    common::run(
        &runtime,
        r"
        assert(jess.bench.verified == verified and table.isfrozen(jess.bench))
        local held = setmetatable({}, { __mode = 'v' })
        held[1] = { 'unreachable once this statement ends' }
        jess.bench.collect()
        assert(held[1] == nil, 'a full collection drops what only a weak table holds')
        assert(select('#', jess.bench.collect()) == 0)
    ",
    );
}

#[test]
fn definitions_and_stubs_have_no_native_format_surface() {
    let types = jess::lua::types::types().unwrap();
    for name in [
        "@jess/plugin",
        "@jess/nif",
        "@jess/openmw",
        "@dream/vfs",
        "@dream/openmw-config",
        "@dream/path",
    ] {
        assert!(types.module_file(name).is_none(), "{name}");
    }
    for name in ["TES3Object", "NifRecord", "OpenMwContext", "PluginValidation"] {
        assert!(!types.definitions().contains(name), "{name}");
    }
}

#[test]
fn generic_contract_fixtures_keep_their_answers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lua/contracts");
    for fixture in ["regex/regex", "input/acceptance", "shader/program", "shader/errors"] {
        let runtime = common::runtime();
        let oracle: l3i::value::Table = runtime.eval(include_str!("contracts/oracle.jsl")).unwrap();
        let source = std::fs::read_to_string(root.join(format!("{fixture}.jsl"))).unwrap();
        let name = format!("$REPO/tests/lua/contracts/{fixture}.jsl");
        let function = jess::lua::scripts::load(&runtime, &name, source.as_bytes()).unwrap();
        let values = function.invoke_multi(&runtime.stack(), (&oracle,)).unwrap();
        let finish: l3i::value::Function = oracle.get(&runtime.stack(), "finish").unwrap();
        let actual: String = finish.invoke_with_values(&runtime.stack(), &values).unwrap();
        let expected = std::fs::read_to_string(root.join(format!("{fixture}.expected.json"))).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            serde_json::from_str::<serde_json::Value>(&expected).unwrap(),
            "{fixture}"
        );
    }
}
