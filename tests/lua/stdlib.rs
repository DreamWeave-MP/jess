//! The built-in modules: `@jess/log` and `@jess/test` resolve through `require` like native
//! modules, are frozen, keep their sink private, and can't be shadowed by a file.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;

use jess::lua::l3i::Runtime;
use jess::lua::l3i::value::Table;

mod common;

#[test]
fn modules_are_frozen_and_the_sink_is_private() {
    let runtime = common::runtime();
    common::run(&runtime, include_str!("stdlib/contracts.jsl"));
}

#[test]
fn records_reach_the_selected_sink_with_the_logging_chunk_as_their_source() {
    let runtime = common::runtime();
    let records = Rc::new(RefCell::new(Vec::new()));
    let captured = records.clone();
    jess::lua::stdlib::set_log_sink(&runtime, move |record| captured.borrow_mut().push(record.clone())).unwrap();
    let script = tempfile::tempdir().unwrap();
    let path = script.path().join("importer.jsl");
    std::fs::write(
        &path,
        "local log = require('@jess/log').new('importer', { package = 'example' })\nlog.info('resolved', { count = 2 })\nlog.debug('skipped')\nlog.child({ step = 'archives' }).warn('missing')\n",
    )
    .unwrap();
    jess::lua::scripts::run_file(&runtime, &path, &[]).unwrap();
    let records = records.borrow();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].level, "info");
    assert_eq!(records[0].target, "importer");
    assert_eq!(records[0].message, "resolved");
    assert_eq!(records[0].fields["count"].as_f64(), Some(2.0));
    assert_eq!(records[0].fields["package"], "example");
    assert_eq!(records[0].source.as_deref(), Some(path.display().to_string().as_str()));
    assert_eq!(records[1].level, "debug");
    assert_eq!(records[2].fields.keys().collect::<Vec<_>>(), ["package", "step"]);
}

#[test]
fn a_plan_only_runtime_has_the_modules() {
    let runtime = Runtime::from_plan(&jess::lua::plan().unwrap()).unwrap();
    common::run(
        &runtime,
        "local test = require('@jess/test') test.expect(type(require('@jess/log').new)).toBe('function')",
    );
}

/// A data folder with a `jess/log.lua` module and a package whose `init.jsl` requires
/// `@jess/log`, beside a `jess/log.jsl` of its own: neither stands in for the built-in module.
#[test]
fn no_file_or_package_module_shadows_the_namespace() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(data.join("jess")).unwrap();
    std::fs::write(data.join("jess/log.lua"), "return { impostor = true }").unwrap();
    std::fs::write(data.join("jess/test.jsl"), "return { impostor = true }").unwrap();
    let package = temp.path().join("package");
    std::fs::create_dir_all(package.join("jess")).unwrap();
    std::fs::write(package.join("jess/log.jsl"), "return { impostor = true }").unwrap();
    std::fs::write(
        package.join("init.jsl"),
        "local log = require('@jess/log')\nlocal own = require('@self/jess/log')\nreturn { log = log, own = own }",
    )
    .unwrap();
    let runtime = common::runtime();
    let returned = jess::lua::scripts::run_file(&runtime, &package.join("init.jsl"), &[]).unwrap();
    common::set_global(&runtime, "package", &Table::from_value(returned[0].clone()).unwrap());
    common::run(
        &runtime,
        r"
        local expect = require('@jess/test').expect
        expect(package.log).toBe(require('@jess/log'))
        expect(package.log.impostor).toBeNil()
        expect(package.own.impostor).toBe(true)
        expect(function() require('jess.log') end).toThrow('module not found')
        expect(require('@jess/log').impostor).toBeNil()
        expect(require('@jess/test').impostor).toBeNil()
        expect(function() require('@jess/impostor') end).toThrow('never a file')
        ",
    );
}

#[test]
fn the_definitions_type_the_modules() {
    let definitions = jess::lua::definitions();
    for needle in [
        "export type Module__jess_log",
        "export type Module__jess_test",
        "export type JessLogger",
        "export type JessTestSuite",
    ] {
        assert!(definitions.contains(needle), "{needle} is missing from the definitions");
    }
    // `require` reaches each module's type through its stub in the types folder.
    let types = jess::lua::types::types().unwrap();
    for (module, needle) in [
        ("@jess/log", "-> JessLogger"),
        ("@jess/test", "-> JessTestSuite"),
        ("@jess/jpk", "-> JessJpkArchive"),
        ("@jess/package", "-> JessPackageOffer"),
        ("@jess/shader", "return jess.shader"),
        ("@jess", "return jess"),
    ] {
        let stub = types.module_file(module).unwrap_or_else(|| panic!("no stub for {module}"));
        assert!(stub.source.contains(needle), "{module}: {}", stub.source);
    }
}

#[test]
fn a_test_run_reports_every_module() {
    let temp = tempfile::tempdir().unwrap();
    let passing = temp.path().join("passing.jsl");
    std::fs::write(
        &passing,
        "local test = require('@jess/test')\nlocal suite = test.new()\nsuite.it('adds', function() test.expect(1 + 1).toBe(2) end)\nsuite.it('fails', function() test.expect(1).toBe(2) end)\nreturn suite",
    )
    .unwrap();
    let broken = temp.path().join("broken.jsl");
    std::fs::write(&broken, "require('@jess/log').warn('loading')\nreturn 42").unwrap();
    let run = jess::lua::testing::run(&jess::lua::Options::default(), &[passing, broken], true).unwrap();
    assert_eq!((run.passed, run.failed), (1, 2));
    let report = run.modules[0].report.as_ref().unwrap();
    assert_eq!(report.tests[0].name, "adds");
    assert!(report.tests[0].passed);
    assert!(report.tests[1].error.as_deref().unwrap().contains("expected 1 to be 2"));
    assert!(run.modules[1].report.is_none());
    assert!(run.modules[1].error.as_deref().unwrap().contains("must return a suite"));
    assert_eq!(run.modules[1].logs[0].message, "loading");
}

/// Every module `require` resolves natively is typed through its stub, however many there are, all
/// at once in one script, and a mistake through one is caught.
#[test]
fn every_module_is_typed_through_require() {
    let types = jess::lua::types::types().unwrap();
    assert!(types.modules.len() > 8, "{:?}", types.modules);
    let mut script = String::new();
    for (index, module) in types.modules.iter().enumerate() {
        writeln!(script, "local _m{index} = require('{module}')").unwrap();
    }
    let report = common::type_check(&script);
    assert!(report.is_empty(), "{report}");
    let wrong = common::type_check("local jpk = require '@jess/jpk'\nlocal size: number = jpk.write {}\n");
    assert!(wrong.contains("buffer"), "{wrong}");
}

/// The built-in package's exports are typed through their own source, copied into the types
/// folder: an export named after its path resolves to the copy, one named otherwise through a
/// stub beside it, and a mistake through either is caught.
#[test]
fn the_built_in_packages_exports_are_typed_through_their_source() {
    let types = jess::lua::types::types().unwrap();
    for (module, path) in [
        ("@jess/json", "jess/json.jsl"),
        ("@jess/data/toml", "jess/data/toml.jsl"),
        ("@jess/style", "jess/style/init.jsl"),
    ] {
        let file = types.module_file(module).unwrap_or_else(|| panic!("no file for {module}"));
        assert_eq!(file.path, path, "{module}");
    }
    let stub = types.module_file("@jess/json").unwrap();
    assert!(stub.source.contains("return require('./data/json')"), "{}", stub.source);
    assert!(types.files.iter().any(|file| file.path == "jess/data/json.jsl"));
    let right = common::type_check(
        "local json = require '@jess/json'\nlocal style = require '@jess/style'\nlocal text: string = json.encodeTable({})\nlocal findings = style.check(text)\nprint(#findings)\n",
    );
    assert!(right.is_empty(), "{right}");
    let wrong = common::type_check("local json = require '@jess/json'\nlocal text: number = json.encodeTable({})\n");
    assert!(wrong.contains("number"), "{wrong}");
}

#[test]
fn numbers_are_compared_within_a_tolerance() {
    common::run(
        &common::runtime(),
        r"
        local test = require('@jess/test')
        test.expect(0.1 + 0.2).toBeCloseTo(0.3)
        test.expect(1.004).toBeCloseTo(1, 0.01)
        -- The expectation raises on purpose; its message is what's compared.
        local ok, problem = pcall(function() test.expect(1).toBeCloseTo(1.1, 0.01) end)
        assert(not ok and string.find(problem, 'to be within 0.01 of 1.1', 1, true), problem)
        -- Not a number at all.
        ok = pcall(function() test.expect('1').toBeCloseTo(1) end)
        assert(not ok)
        ",
    );
}

/// The built-in writer the CLI prints through spells a float as `serde_json` does, Ryū's digits and
/// layout, including where two shortest spellings both read back as the value.
#[test]
fn floats_are_written_as_serde_json_writes_them() {
    let values = [
        0.1,
        2.0 / 3.0,
        1e21,
        1e-7,
        123.456,
        -42.5,
        802_532_279_307_174.2,
        5e-324,
        f64::MAX,
        1e15,
        1e16,
    ];
    // And a spread of values across the whole range, from a fixed seed.
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    let mut values = values.to_vec();
    for _ in 0..20_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let value = f64::from_bits(state);
        if value.is_finite() {
            values.push(value);
        }
    }
    let written = jess::lua::document::encode(&serde_json::json!(values), "json").unwrap();
    assert_eq!(written, serde_json::to_string(&values).unwrap());
}
