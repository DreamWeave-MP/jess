//! Generic runtime surface contracts; application APIs are package-owned.

use super::common;

/// The `jess` global holds exactly the kernel's members, each table among them is also its
/// `@jess/` module, and the runtime adds no global of its own beside `jess`.
#[test]
fn only_generic_members_are_installed() {
    let runtime = common::runtime();
    common::run(
        &runtime,
        r"
        local expected = {
            'application', 'audio', 'bench', 'context', 'events', 'gpu', 'granted', 'input',
            'interfaces', 'null', 'payload', 'shader', 'version', 'worker',
        }
        local members = {}
        for name in jess do table.insert(members, name) end
        table.sort(members)
        assert(table.concat(members, ' ') == table.concat(expected, ' '), table.concat(members, ' '))
        for _, name in { 'shader', 'gpu', 'context', 'worker', 'events', 'interfaces', 'application', 'bench' } do
            assert(require('@jess/' .. name) == jess[name], name)
        end
        -- @jess/input is the built-in package's Luau, over jess.input.
        assert(require('@jess/input') ~= jess.input and require('@jess/input').runtime == jess.input.runtime)
        -- So is @jess/audio, over jess.audio.
        assert(require('@jess/audio') ~= jess.audio and require('@jess/audio').offline == jess.audio.offline)
        local luau = {
            _G = true, _VERSION = true, assert = true, bit32 = true, buffer = true, coroutine = true,
            error = true, gcinfo = true, getmetatable = true, ipairs = true, math = true, newproxy = true,
            next = true, os = true, pairs = true, pcall = true, print = true, rawequal = true, rawget = true,
            rawlen = true, rawset = true, require = true, select = true, setmetatable = true, string = true,
            table = true, tonumber = true, tostring = true, type = true, typeof = true, unpack = true,
            utf8 = true, vector = true, xpcall = true, debug = true, getfenv = true, setfenv = true, integer = true,
            __l3i_data = true,
            jess = true, arg = true,
        }
        for name in _G do
            assert(luau[name], 'unexpected global ' .. tostring(name))
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

/// The types folder covers exactly the kernel's `@jess/` surface: the global, its module members,
/// the built-in modules, and the built-in package's exports, and nothing under any other scope
/// but L3i's `@dream/`.
#[test]
fn the_types_cover_exactly_the_kernel_s_modules() {
    let types = jess::lua::types::types().unwrap();
    let mut jess_modules = types
        .modules
        .iter()
        .filter(|name| name.starts_with("@jess"))
        .map(String::as_str)
        .collect::<Vec<_>>();
    jess_modules.sort_unstable();
    assert_eq!(
        jess_modules,
        [
            "@jess",
            "@jess/analysis",
            "@jess/application",
            "@jess/audio",
            "@jess/backup",
            "@jess/bench",
            "@jess/context",
            "@jess/data/toml",
            "@jess/data/value",
            "@jess/events",
            "@jess/gpu",
            "@jess/http",
            "@jess/input",
            "@jess/interfaces",
            "@jess/jpk",
            "@jess/json",
            "@jess/l10n",
            "@jess/log",
            "@jess/package",
            "@jess/render",
            "@jess/render/camera",
            "@jess/shader",
            "@jess/style",
            "@jess/test",
            "@jess/worker",
            "@jess/yaml",
        ]
    );
    assert!(
        types
            .modules
            .iter()
            .all(|name| name.starts_with("@jess") || name.starts_with("@dream/")),
        "{:?}",
        types.modules
    );
    assert_eq!(types.scopes, ["dream", "jess"]);
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
        let path = root.join(format!("{fixture}.expected.json"));
        // Re-blessing is deliberate, as the WGSL goldens' is: the reason goes in the commit.
        if std::env::var_os("JESS_BLESS").is_some() {
            let newline = if actual.ends_with('\n') { "" } else { "\n" };
            std::fs::write(&path, actual + newline).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(path).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            serde_json::from_str::<serde_json::Value>(&expected).unwrap(),
            "{fixture}"
        );
    }
}
