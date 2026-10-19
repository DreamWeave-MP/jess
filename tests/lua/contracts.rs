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
        for _, name in { 'shader', 'context', 'worker', 'events', 'interfaces', 'application', 'bench' } do
            assert(require('@jess/' .. name) == jess[name], name)
        end
        -- @jess/input is the built-in package's Luau, over jess.input.
        assert(require('@jess/input') ~= jess.input and require('@jess/input').runtime == jess.input.runtime)
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

/// `jess.granted` answers from the runtime's grant, and a name no jess runtime can be granted is
/// an error, not a quiet `false`.
#[test]
fn granted_answers_from_the_runtime_s_grant() {
    let runtime = common::runtime();
    common::run(
        &runtime,
        r"
        assert(jess.granted('process.environment') == false)
        local ok, problem = pcall(jess.granted, 'proces.environment')
        assert(not ok and string.find(problem, 'unknown capability', 1, true), problem)
    ",
    );
    let granted =
        jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &["process.environment".to_owned()]).unwrap();
    common::run(
        &granted,
        "assert(jess.granted('process.environment') and not jess.granted('process.spawn'))",
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
    for fixture in ["input/acceptance", "shader/program", "shader/errors"] {
        // The input fixture configures a headless runtime, as a host script may.
        let runtime =
            jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &jess::lua::host_script_grant(&[])).unwrap();
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
