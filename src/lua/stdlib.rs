//! jess's built-in Luau modules: `@jess/log`, `@jess/test`, `@jess/jpk`, and `@jess/package`.
//!
//! A built-in module is one jess owns and resolves: its source is embedded in jess, it installs
//! through the runtime plan like a native module, its exports are frozen, and its types print
//! with `jess luau-types`. "Built-in" is about ownership and resolution, not language: these
//! are written in Luau over the smallest native primitive each needs. For `@jess/log` that is
//! one structured-log sink the host selects and scripts can't reach: the domain's log stream
//! ([`DomainSpec::logs`](crate::runtime::DomainSpec::logs)) under a domain that has one, else the runtime's own
//! ([`set_log_sink`]);
//! `@jess/test` and `@jess/jpk` need nothing native at all (the codecs are `@dream/bytes`'s), and
//! `@jess/package` reads and writes files through host functions only it is handed
//! ([`crate::lua::package`]).
//!
//! The `@jess/` prefix is reserved: `require` resolves it here or fails, so neither a VFS file
//! nor a package module can stand in for one of these, and a name jess doesn't ship is an error
//! (`unknown jess built-in module`), never a lookup.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use l3i::Runtime;
use l3i::bind::Call;
use l3i::debug::DebugScope;
use l3i::extension::{Extension, ExtensionDescriptor, InstallContext};
use l3i::source::LoadScope;
use l3i::stack::Scope;
use l3i::value::{Function, Table, Value};

/// The extension's id, and the debug root of the sink.
pub const EXTENSION_ID: &str = "jess.stdlib";

/// One record a script logged through `@jess/log`, as the sink receives it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct LogRecord {
    /// `debug`, `info`, `warn`, or `error`.
    pub level: String,
    /// The logger's target: the name given to `log.new`, or `jess` for the module's own functions.
    pub target: String,
    /// The message.
    pub message: String,
    /// The logger's context merged with the call's fields; strings, finite numbers, and booleans.
    pub fields: BTreeMap<String, serde_json::Value>,
    /// The chunk that logged, as the VM attributes it (a script's path, or a module's), not
    /// anything the script said.
    pub source: Option<String>,
}

type Sink = Rc<dyn Fn(&LogRecord)>;

/// The runtime's sink, replaced by [`set_log_sink`].
struct Logging(RefCell<Sink>);

/// Replaces where `@jess/log` records go in `runtime`. The default writes every record but
/// `debug` ones to stderr as one JSON object per line.
///
/// Code running under a domain with a log stream ([`DomainSpec::logs`](crate::runtime::DomainSpec::logs)) logs there
/// instead, attributed.
///
/// The sink runs synchronously inside the logging call; it must not call back into the VM.
///
/// # Errors
///
/// Returns an error if `runtime` wasn't made from a plan with jess's built-in modules.
pub fn set_log_sink(runtime: &Runtime, sink: impl Fn(&LogRecord) + 'static) -> l3i::Result<()> {
    let state = runtime
        .state_of::<Logging>(EXTENSION_ID)
        .ok_or_else(|| l3i::Error::logic("this runtime has no jess built-in modules"))?;
    *state.0.borrow_mut() = Rc::new(sink);
    Ok(())
}

fn default_sink(record: &LogRecord) {
    if record.level == "debug" {
        return;
    }
    eprintln!(
        "{}",
        serde_json::json!({
            "level": record.level,
            "target": record.target,
            "message": record.message,
            "fields": record.fields,
            "source": record.source,
        })
    );
}

/// A member of a built-in module: its name, Luau type, and one-line doc.
type Member = (&'static str, &'static str, &'static str);

/// A built-in module: its path, embedded source, and the members it exports.
struct BuiltIn {
    path: &'static str,
    doc: &'static str,
    source: &'static str,
    members: &'static [Member],
}

const FIELDS: &str = "JessLogFields";
const LOG: BuiltIn = BuiltIn {
    path: "@jess/log",
    doc: "Structured logging: loggers with context, and capture for tests.",
    source: include_str!("stdlib/log.jsl"),
    members: &[
        (
            "new",
            "(target: string, fields: JessLogFields?) -> JessLogger",
            "A logger for `target`, with `fields` in every record it writes.",
        ),
        (
            "debug",
            "(message: string, fields: JessLogFields?) -> ()",
            "Logs at `debug` with the target `jess`.",
        ),
        (
            "info",
            "(message: string, fields: JessLogFields?) -> ()",
            "Logs at `info` with the target `jess`.",
        ),
        (
            "warn",
            "(message: string, fields: JessLogFields?) -> ()",
            "Logs at `warn` with the target `jess`.",
        ),
        (
            "error",
            "(message: string, fields: JessLogFields?) -> ()",
            "Logs at `error` with the target `jess`.",
        ),
        (
            "capture",
            "(body: () -> ()) -> { JessLogRecord }",
            "Runs `body` and returns what it logged, which the sink never sees; an error in `body` is raised again.",
        ),
    ],
};
const TEST: BuiltIn = BuiltIn {
    path: "@jess/test",
    doc: "Test suites: describe, it, hooks, expectations, and a report.",
    source: include_str!("stdlib/test.jsl"),
    members: &[
        ("new", "() -> JessTestSuite", "An empty suite to register cases on and run."),
        (
            "expect",
            "(actual: unknown) -> JessTestExpectation",
            "An assertion on `actual`: `toBe`, `toEqual`, `toBeTruthy`, `toBeFalsy`, `toBeNil`, `toContain`, `toThrow`.",
        ),
    ],
};

const JPK: BuiltIn = BuiltIn {
    path: "@jess/jpk",
    doc: "The JPK container: a narrow ZIP subset, read with budgets and written canonically.",
    source: include_str!("stdlib/jpk.jsl"),
    members: &[
        (
            "open",
            "(source: buffer | JessJpkSource, budgets: JessJpkBudgets?) -> JessJpkArchive",
            "Reads an archive's central directory, checked against `budgets`; entries are read when asked for.",
        ),
        (
            "write",
            "(files: { JessJpkFile }) -> buffer",
            "The canonical archive of `files`: sorted by path, each zstd when smaller and stored otherwise, every header field fixed.",
        ),
        (
            "checkPath",
            "(path: string) -> string?",
            "Why `path` can't name an entry, or nil when it is a canonical relative path.",
        ),
        ("budgets", "JessJpkBudgets", "The reader's default budgets."),
    ],
};
const PACKAGE: BuiltIn = BuiltIn {
    path: "@jess/package",
    doc: "Packages from a Package Source or a JPK: manifests, content digests, packing, and resolution.",
    source: include_str!("stdlib/package.jsl"),
    members: &[
        (
            "open",
            "(location: string) -> JessPackage",
            "Opens a Package Source folder or a `.jpk` file; reads it, runs nothing.",
        ),
        (
            "peek",
            "(location: string) -> JessPackageOffer",
            "Reads the manifest of the package at `location` and nothing else: what a resolution needs to know whether to open it.",
        ),
        (
            "candidates",
            "(folder: string) -> { JessPackageOffer }",
            "Every package directly inside `folder`, a package path: each Package Source folder and `.jpk` file, in name order.",
        ),
        (
            "pack",
            "(source: string, output: string) -> JessPackResult",
            "Packs the Package Source at `source` into the canonical JPK at `output`, written atomically.",
        ),
        (
            "resolve",
            "(packages: { JessPackage }, request: JessPackageRequest) -> (JessActivationPlan, { JessPackage })",
            "Resolves requested layers over `packages` into an activation plan, without running anything; the second result is the packages it selected.",
        ),
        (
            "manifest",
            "(text: string) -> JessPackageManifest",
            "Parses and checks a `jess.toml`.",
        ),
        (
            "satisfies",
            "(version: string, requirement: string) -> boolean",
            "Whether `version` meets `requirement` (`^1.2`, `=1.2.3`).",
        ),
    ],
};

/// Every built-in module, in installation order: `@jess/test` is built on `@jess/log`, and
/// `@jess/package` on `@jess/jpk`.
const BUILT_INS: [&BuiltIn; 4] = [&LOG, &TEST, &JPK, &PACKAGE];

/// The paths of the built-in modules, the whole `@jess/` namespace `require` resolves.
pub const MODULES: [&str; 4] = [LOG.path, TEST.path, JPK.path, PACKAGE.path];

/// Whether `name` is a built-in module's path.
#[must_use]
pub fn contains(name: &str) -> bool {
    MODULES.contains(&name)
}

/// The types `@jess/jpk`'s and `@jess/package`'s signatures name. Nothing in them is formatted in.
const PACKAGE_TYPES: &str = "\
         --- One entry of a JPK: its path, how it is stored, its sizes, CRC-32, and local header offset.\n\
         export type JessJpkEntry = { path: string, method: \"stored\" | \"deflate\" | \"zstd\", compressedSize: number, size: number, crc32: number, offset: number }\n\n\
         --- Where a JPK's bytes come from: its size, and a read of `length` bytes at `offset`.\n\
         export type JessJpkSource = { size: number, read: (offset: number, length: number) -> buffer }\n\n\
         --- The reader's limits, each optional against `budgets`' defaults.\n\
         export type JessJpkBudgets = { maxEntries: number?, maxPathLength: number?, maxCentralDirectory: number?, maxEntrySize: number?, maxTotalSize: number? }\n\n\
         --- An opened JPK: its entries, a lookup by path, and a read that unpacks and CRC-checks one entry.\n\
         export type JessJpkArchive = { entries: { JessJpkEntry }, find: (path: string) -> JessJpkEntry?, read: (entry: JessJpkEntry | string) -> buffer }\n\n\
         --- A file to write into a JPK.\n\
         export type JessJpkFile = { path: string, data: buffer }\n\n\
         --- Code a layer runs when it is activated, and the capabilities it needs.\n\
         export type JessPackageEntrypoint = { module: string, capabilities: { string } }\n\n\
         --- A layer as a manifest declares it: what it requires, the modules it exports as inert API, and\n\
         --- the entrypoints that run when it is activated, which alone hold capabilities.\n\
         export type JessPackageLayer = { id: string, requires: { string }, exports: { string }, entrypoints: { JessPackageEntrypoint } }\n\n\
         --- A managed implementation a target supervisor may load into its application topology.\n\
         export type JessPackageManaged = { id: string, module: string, interface: string? }\n\n\
         --- One runnable role in a package.\n\
         export type JessPackageTarget = { id: string, kind: \"application\" | \"command\", entry: string, capabilities: { string }, command: string? }\n\n\
         --- A checked `jess.toml`.\n\
         export type JessPackageManifest = {\n  \
           manifestVersion: number,\n  \
           id: string,\n  \
           version: string,\n  \
           jessApi: string,\n  \
           defaultTarget: string?,\n  \
           include: { string },\n  \
           tests: { string },\n  \
           dependencies: { [string]: string },\n  \
           devDependencies: { [string]: string },\n  \
           modules: { [string]: string },\n  \
           layers: { JessPackageLayer },\n  \
           managed: { [string]: JessPackageManaged },\n  \
           targets: { [string]: JessPackageTarget },\n\
         }\n\n\
         --- A package opened from a Package Source (`source`) or a JPK (`jpk`): its manifest, identity,\n\
         --- content digest, payload in canonical order, and a read of one payload file.\n\
         export type JessPackage = {\n  \
           manifest: JessPackageManifest,\n  \
           id: string,\n  \
           version: string,\n  \
           digest: string,\n  \
           kind: \"source\" | \"jpk\",\n  \
           location: string,\n  \
           files: { string },\n  \
           read: (path: string) -> buffer?,\n\
         }\n\n\
         --- A package a resolution may select: its manifest and where it is, with nothing else read.\n\
         export type JessPackageOffer = { manifest: JessPackageManifest, id: string, version: string, kind: \"source\" | \"jpk\", location: string }\n\n\
         --- What `pack` wrote.\n\
         export type JessPackResult = { path: string, digest: string, files: number, size: number }\n\n\
         --- What to resolve: the layers, the module names the host already provides, the capabilities it\n\
         --- grants, candidates opened only when a layer or a dependency needs one, package ids selected\n\
         --- whatever the layers (`roots`), and the ids whose `[dev-dependencies]` come with them (`dev`).\n\
         export type JessPackageRequest = { layers: { string }, hostModules: { string }?, grant: { string }?, candidates: { JessPackageOffer }?, roots: { string }?, dev: { string }? }\n\n\
         --- An activation plan: requested layers resolved over packages, with nothing run.\n\
         export type JessActivationPlan = {\n  \
           requested: { string },\n  \
           layers: { { id: string, package: string, version: string, requires: { string }, exports: { string }, entrypoints: { JessPackageEntrypoint } } },\n  \
           packages: { { id: string, version: string, digest: string, kind: \"source\" | \"jpk\", location: string, dependencies: { string } } },\n  \
           modules: { { name: string, package: string, path: string } },\n  \
           entrypoints: { { module: string, package: string, path: string, layer: string } },\n  \
           capabilities: { { capability: string, package: string, version: string, layer: string, entrypoint: string } },\n\
         }\n";

/// The types the members' signatures name, for the definitions.
pub(crate) fn definitions() -> String {
    format!(
        "--- The context a logger carries and the fields one call adds: strings, finite numbers, and booleans.\n\
         export type {FIELDS} = {{ [string]: string | number | boolean }}\n\n\
         --- A logger from `@jess/log`: one function per level, and `child` for a logger with more context.\n\
         export type JessLogger = {{\n  \
           debug: (message: string, fields: {FIELDS}?) -> (),\n  \
           info: (message: string, fields: {FIELDS}?) -> (),\n  \
           warn: (message: string, fields: {FIELDS}?) -> (),\n  \
           error: (message: string, fields: {FIELDS}?) -> (),\n  \
           child: (fields: {FIELDS}) -> JessLogger,\n\
         }}\n\n\
         --- One record, as `@jess/log`'s `capture` and a test result's `logs` hand it back.\n\
         export type JessLogRecord = {{ level: string, target: string, message: string, fields: {FIELDS} }}\n\n\
         --- An assertion from `@jess/test`'s `expect`; each check raises on failure.\n\
         export type JessTestExpectation = {{\n  \
           --- `rawequal`.\n  \
           toBe: (expected: unknown) -> (),\n  \
           --- Deep equality over tables, cycles included.\n  \
           toEqual: (expected: unknown) -> (),\n  \
           toBeTruthy: () -> (),\n  \
           toBeFalsy: () -> (),\n  \
           toBeNil: () -> (),\n  \
           --- A substring of a string, or an element of a list (by `toEqual`).\n  \
           toContain: (item: unknown) -> (),\n  \
           --- The function raises, and its message contains `contains` when given.\n  \
           toThrow: (contains: string?) -> (),\n\
         }}\n\n\
         --- One case's outcome: its full name (`describe / it`), whether setup, body, and cleanup all\n\
         --- passed, the tracebacks otherwise, and what it logged.\n\
         export type JessTestResult = {{ name: string, passed: boolean, error: string?, logs: {{ JessLogRecord }} }}\n\n\
         --- A suite's report from `run`, cases in registration order.\n\
         export type JessTestReport = {{ passed: number, failed: number, tests: {{ JessTestResult }} }}\n\n\
         --- A suite from `@jess/test`'s `new`: `describe` groups cases, `it` adds one, `beforeEach` and\n\
         --- `afterEach` add hooks to the current group, and `run` runs every case.\n\
         export type JessTestSuite = {{\n  \
           describe: (name: string, body: () -> ()) -> (),\n  \
           it: (name: string, body: () -> ()) -> (),\n  \
           beforeEach: (body: () -> ()) -> (),\n  \
           afterEach: (body: () -> ()) -> (),\n  \
           run: () -> JessTestReport,\n\
         }}\n\n"
    ) + PACKAGE_TYPES
}

thread_local! {
    /// Each module's bytecode, compiled once per thread: the source never changes.
    static BYTECODE: RefCell<HashMap<&'static str, Rc<Vec<u8>>>> = RefCell::new(HashMap::new());
}

/// The built-in modules as an l3i extension: declared, installed, and frozen like native ones.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdlibExtension;

impl Extension for StdlibExtension {
    fn id(&self) -> &'static str {
        EXTENSION_ID
    }

    fn describe(&self, d: &mut ExtensionDescriptor) -> l3i::Result<()> {
        for built_in in BUILT_INS {
            let module = d.module(built_in.path);
            module.doc(built_in.doc);
            for (name, signature, doc) in built_in.members {
                module.installed(name).signature(*signature).doc(*doc);
            }
        }
        // `@jess/package` opens and packs only what the runtime's policy lets a script read and
        // write.
        d.optional_capability(l3i::fs::READ_CAPABILITY);
        d.optional_capability(l3i::fs::WRITE_CAPABILITY);
        Ok(())
    }

    fn install(&self, cx: &mut InstallContext<'_>) -> l3i::Result<()> {
        let read = cx.has_capability(l3i::fs::READ_CAPABILITY)?;
        let write = cx.has_capability(l3i::fs::WRITE_CAPABILITY)?;
        let runtime = cx.runtime();
        let logging = cx.insert_state(Logging(RefCell::new(Rc::new(default_sink))));
        let emit = runtime.bind_function(
            "jess.stdlib.emit",
            move |call: &Call<'_>, level: String, target: String, message: String, fields: Table| -> l3i::Result<()> {
                let fields = call.with_frame(|frame| read_fields(frame, &fields))?;
                // The sink's caller is the logger's level function, called from the script.
                let source = call.call_site(3).map(|site| site.source);
                let record = LogRecord {
                    level,
                    target,
                    message,
                    fields,
                    source,
                };
                if let Some(authority) = crate::runtime::current_authority()
                    && let Some(logs) = authority.logs()
                {
                    logs.record(domain_record(authority.attribution(), record)?);
                    return Ok(());
                }
                let sink = logging.0.borrow().clone();
                sink(&record);
                Ok(())
            },
        )?;
        let log = install(cx, &LOG, (&emit,))?;
        install(cx, &TEST, (&log,))?;
        let jpk = install(cx, &JPK, ())?;
        let dependencies = package_dependencies(runtime, &jpk, read, write)?;
        install(cx, &PACKAGE, (&dependencies,))?;
        cx.insert_state(HostPackages {
            jpk,
            module: RefCell::new(None),
        });
        Ok(())
    }
}

/// What `@jess/package`'s factory is built on: the container, and host file access that reads
/// and writes or doesn't.
fn package_dependencies(runtime: &Runtime, jpk: &Table, read: bool, write: bool) -> l3i::Result<Table> {
    let host = crate::lua::package::host_table(runtime, read, write)?;
    let stack = runtime.stack();
    let dependencies = Table::new(&stack, 0, 2)?;
    dependencies.set(&stack, "host", &host)?;
    dependencies.set(&stack, "jpk", jpk)?;
    Ok(dependencies)
}

/// The host's own `@jess/package`, made when it is first needed.
struct HostPackages {
    jpk: Table,
    module: RefCell<Option<Table>>,
}

/// `@jess/package` as the host uses it for its own package operations: the same module over file
/// access the runtime's policy doesn't narrow. Nothing a script can require hands it out.
pub(crate) fn host_packages(runtime: &Runtime) -> l3i::Result<Table> {
    let state = runtime
        .state_of::<HostPackages>(EXTENSION_ID)
        .ok_or_else(|| l3i::Error::logic("jess.stdlib isn't installed in this Luau runtime"))?;
    if let Some(module) = state.module.borrow().clone() {
        return Ok(module);
    }
    let dependencies = package_dependencies(runtime, &state.jpk, true, true)?;
    let module = exports(runtime, &PACKAGE, (&dependencies,))?;
    l3i::readonly::make_read_only(runtime, &module)?;
    *state.module.borrow_mut() = Some(module.clone());
    Ok(module)
}

/// A record for the domain's log stream, attributed to the code that logged it.
fn domain_record(attribution: crate::runtime::Attribution, record: LogRecord) -> l3i::Result<crate::runtime::LogRecord> {
    let level = crate::runtime::LogLevel::parse(&record.level)
        .ok_or_else(|| l3i::Error::logic(format!("unknown log level '{}'", record.level)))?;
    let fields = record
        .fields
        .into_iter()
        .map(|(key, value)| {
            let value = match value {
                serde_json::Value::String(text) => crate::runtime::Message::text(&text),
                serde_json::Value::Bool(on) => crate::runtime::Message::Boolean(on),
                number => crate::runtime::Message::Number(number.as_f64().unwrap_or_default()),
            };
            (key, value)
        })
        .collect();
    Ok(crate::runtime::LogRecord {
        attribution,
        level,
        target: record.target,
        message: record.message,
        fields,
        source: record.source,
    })
}

/// The scalar fields of a record, as JSON values.
fn read_fields(frame: &l3i::stack::Frame<'_>, fields: &Table) -> l3i::Result<BTreeMap<String, serde_json::Value>> {
    let mut values = BTreeMap::new();
    fields.push_to(frame)?.for_each(frame, |_, key, value| {
        let key = key.read::<String>()?;
        let value = if value.is_string() {
            serde_json::Value::String(value.read()?)
        } else if value.is_boolean() {
            serde_json::Value::Bool(value.read()?)
        } else {
            let number: f64 = value.read()?;
            serde_json::Number::from_f64(number)
                .map(serde_json::Value::Number)
                .ok_or_else(|| l3i::Error::runtime("log fields must be finite"))?
        };
        values.insert(key, value);
        Ok(())
    })?;
    Ok(values)
}

/// Runs a built-in module's source, a factory taking the values it's built on, and installs what
/// it exports as the plan's module: frozen there, and frozen in the table it returned.
fn install(cx: &mut InstallContext<'_>, built_in: &BuiltIn, dependencies: impl l3i::call::PushArgs) -> l3i::Result<Table> {
    let runtime = cx.runtime();
    // One root stack at a time: the exports are read before the module installer opens its own.
    let (exports, values) = {
        let exports = exports(runtime, built_in, dependencies)?;
        let stack = runtime.stack();
        let values = built_in
            .members
            .iter()
            .map(|(name, _, _)| exports.get::<Value>(&stack, name).map(|value| (*name, value)))
            .collect::<l3i::Result<Vec<_>>>()?;
        (exports, values)
    };
    l3i::readonly::make_read_only(runtime, &exports)?;
    let module = cx.module(built_in.path)?;
    for (name, value) in values {
        module.set(name, &value)?;
    }
    Ok(module.table().clone())
}

/// What a built-in module's factory returns when it is handed `dependencies`.
fn exports(runtime: &Runtime, built_in: &BuiltIn, dependencies: impl l3i::call::PushArgs) -> l3i::Result<Table> {
    let bytecode = BYTECODE.with_borrow_mut(|cache| -> l3i::Result<Rc<Vec<u8>>> {
        if let Some(bytecode) = cache.get(built_in.path) {
            return Ok(bytecode.clone());
        }
        let bytecode = Rc::new(l3i::source::compile(built_in.source, &runtime.compile_options())?);
        cache.insert(built_in.path, bytecode.clone());
        Ok(bytecode)
    })?;
    let stack = runtime.stack();
    let chunk = stack.load_bytecode(built_in.path, &bytecode)?;
    let factory: Function = chunk.invoke(&stack, ())?;
    factory.invoke(&stack, dependencies)
}
