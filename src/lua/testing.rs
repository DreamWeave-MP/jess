//! The host's side of `@jess/test`: loading test modules, running their suites, and reporting.
//!
//! Cases, hooks, expectations, and per-case log capture are the module's; the host only catches
//! what a module raises while loading and counts what the suites report.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use l3i::Runtime;
use l3i::value::{Function, Table};
use serde::{Deserialize, Serialize};

use crate::lua::convert;
use crate::lua::stdlib::LogRecord;

/// One case's outcome, as `@jess/test` reports it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestResult {
    /// The case's full name: its `describe` names and its own, joined with ` / `.
    pub name: String,
    /// Whether setup, body, and cleanup all passed.
    pub passed: bool,
    /// The tracebacks of what failed, one per line group.
    pub error: Option<String>,
    /// What the case logged through `@jess/log`.
    pub logs: Vec<CapturedLog>,
}

/// A record `@jess/log` captured during a case, which the sink never saw.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CapturedLog {
    /// `debug`, `info`, `warn`, or `error`.
    pub level: String,
    /// The logger's target.
    pub target: String,
    /// The message.
    pub message: String,
    /// The merged fields.
    pub fields: std::collections::BTreeMap<String, serde_json::Value>,
}

/// A suite's report, as its `run` returned it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestReport {
    /// Cases that passed.
    pub passed: usize,
    /// Cases that failed.
    pub failed: usize,
    /// Every case, in registration order.
    pub tests: Vec<TestResult>,
}

/// One test module: its suite's report, or why it didn't run.
#[derive(Clone, Debug, Serialize)]
pub struct ModuleReport {
    /// The module's path, as given.
    pub input: PathBuf,
    /// The report, when the module returned a suite and it ran.
    pub report: Option<TestReport>,
    /// The error that stopped it: loading, not returning a suite, or a report that doesn't add up.
    pub error: Option<String>,
    /// What the module logged outside its cases, while loading.
    pub logs: Vec<LogRecord>,
}

/// A test run over several modules. A module that didn't run counts as one failure.
#[derive(Clone, Debug, Serialize)]
pub struct TestRun {
    /// Cases that passed.
    pub passed: usize,
    /// Cases that failed, plus modules that didn't run.
    pub failed: usize,
    /// The modules, in the order given.
    pub modules: Vec<ModuleReport>,
}

/// Runs the test modules at `paths`, each a Luau file returning a suite made with
/// `@jess/test`, and reports every case.
///
/// With `isolated`, each module runs in a fresh VM; otherwise they share one, and with it the
/// module cache. Either way a module's error is reported, and the next module still runs.
///
/// # Errors
///
/// Returns an error if a VM can't be made.
pub fn run(options: &crate::lua::Options, paths: &[PathBuf], isolated: bool) -> l3i::Result<TestRun> {
    run_with(options, &[], paths, isolated)
}

/// [`run`], in runtimes granted `capabilities`: what a suite that reads or writes files needs.
///
/// # Errors
///
/// Returns an error if a VM can't be made.
pub fn run_with(
    options: &crate::lua::Options,
    capabilities: &[String],
    paths: &[PathBuf],
    isolated: bool,
) -> l3i::Result<TestRun> {
    let mut shared = None;
    let mut run = TestRun {
        passed: 0,
        failed: 0,
        modules: Vec::new(),
    };
    for path in paths {
        if shared.is_none() || isolated {
            shared = Some(crate::lua::runtime_with_capabilities(options.clone(), capabilities)?);
        }
        let runtime = shared.as_ref().ok_or_else(|| crate::lua::error("no runtime"))?;
        run_module(runtime, path, &mut run)?;
    }
    Ok(run)
}

/// [`run`] in one runtime the caller made, such as one with a package plan activated.
///
/// # Errors
///
/// Returns an error if the runtime has no jess built-in modules.
pub fn run_in(runtime: &Runtime, paths: &[PathBuf]) -> l3i::Result<TestRun> {
    let mut run = TestRun {
        passed: 0,
        failed: 0,
        modules: Vec::new(),
    };
    for path in paths {
        run_module(runtime, path, &mut run)?;
    }
    Ok(run)
}

/// Runs package tests with their host-owned scratch directory exposed as `scratchRoot`.
pub(crate) fn run_in_with_scratch(runtime: &Runtime, paths: &[PathBuf], scratch: &Path) -> l3i::Result<TestRun> {
    let globals = Table::from_value(runtime.global("_G")?)?;
    let scratch = scratch.display().to_string();
    l3i::readonly::set_read_only_field(runtime, &globals, "scratchRoot", &scratch)?;
    run_in(runtime, paths)
}

/// Runs one module's suite in `runtime`, adding its report to `run`.
fn run_module(runtime: &Runtime, path: &Path, run: &mut TestRun) -> l3i::Result<()> {
    let logs = Rc::new(RefCell::new(Vec::new()));
    let captured = logs.clone();
    crate::lua::stdlib::set_log_sink(runtime, move |record| captured.borrow_mut().push(record.clone()))?;
    let (report, error) = match run_suite(runtime, path) {
        Ok(report) => {
            run.passed += report.passed;
            run.failed += report.failed;
            (Some(report), None)
        }
        Err(error) => {
            run.failed += 1;
            (None, Some(crate::lua::error_message(&error)))
        }
    };
    run.modules.push(ModuleReport {
        input: path.to_owned(),
        report,
        error,
        logs: logs.take(),
    });
    Ok(())
}

/// Loads one module, which must return a suite, and runs it.
fn run_suite(runtime: &Runtime, path: &Path) -> l3i::Result<TestReport> {
    let returned = crate::lua::scripts::run_file(runtime, path, &[])?;
    let suite = returned
        .first()
        .and_then(|value| Table::from_value(value.clone()).ok())
        .ok_or_else(|| crate::lua::error("a test module must return a suite from require('@jess/test').new()"))?;
    let stack = runtime.stack();
    let function: Function = suite
        .get(&stack, "run")
        .map_err(|_| crate::lua::error("a test module must return a suite from require('@jess/test').new()"))?;
    let jess = crate::lua::state(runtime)?;
    let shape = serde_json::json!({ "passed": 0, "failed": 0, "tests": [{
        "name": "", "passed": true, "error": null, "logs": [{
            "level": "", "target": "", "message": "", "fields": {}
        }]
    }] });
    let report: TestReport = function.invoke_with(&stack, (), |frame, value| {
        convert::from_lua(frame, &jess, value, Some(&shape))
    })?;
    let passed = report.tests.iter().filter(|case| case.passed).count();
    if report.passed != passed || report.failed != report.tests.len() - passed {
        return Err(crate::lua::error(
            "the suite's report doesn't add up: its counts disagree with its cases",
        ));
    }
    Ok(report)
}
