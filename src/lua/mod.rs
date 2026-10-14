//! Generic native services and package execution for Luau applications.

// Counts, lengths, and indices here come from Luau tables and Rust collections far below 2^52
// entries, pushed as the numbers scripts see; the wrap lint has nothing to add.
#![allow(clippy::cast_possible_wrap)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Display;
use std::rc::Rc;

use l3i::Runtime;
use l3i::bind::Call;
use l3i::extension::RuntimePlan;
use l3i::source::LoadScope;
use l3i::stack::{Frame, Scope, ValueView};
use l3i::value::{Function, Table, Value};

pub use l3i;

pub mod bench;
pub mod builtin;
mod convert;
mod files;
mod fs;
pub mod init;
mod input;
mod l10n;
#[expect(
    clippy::module_inception,
    reason = "the binding shapes keep the name they had as the jess-lua crate's `lua` module"
)]
mod lua;
#[doc(hidden)]
pub mod namespaces;
pub mod package;
pub mod plan;
mod regexp;
mod require;
pub mod scripts;
pub mod shader;
pub mod stdlib;
pub mod testing;
mod toml;
pub mod types;

pub(crate) use lua::{Bytes, Opts, Text, kind, push_table};

/// Luau type definitions for the generic runtime services and installed modules.
///
/// # Panics
/// Panics if the extension plan cannot be finalized.
#[must_use]
pub fn definitions() -> String {
    format!(
        "-- jess {} Luau types, from `jess luau-types`, for Luau {} on l3i.\n\n{}\n{}\n",
        env!("CARGO_PKG_VERSION"),
        l3i::LUAU_VERSION,
        plan::downstream_definitions().expect("the downstream plan finalizes"),
        include_str!("definitions.jsl"),
    )
}

/// Host options for a generic runtime; no application configuration is loaded implicitly.
#[derive(Clone, Debug, Default)]
pub struct Options {}

/// The generic runtime plan, cached per thread on first use.
///
/// # Errors
///
/// Returns an error if the plan doesn't finalize.
pub fn plan() -> l3i::Result<Rc<RuntimePlan>> {
    plan::plan()
}

/// A sandboxed Luau VM with generic Jess services, package-aware `require`, and `print`.
/// Relative file imports resolve beside their calling script. Application VFS policy belongs
/// to packages, not the host.
///
/// # Errors
///
/// Returns an error if the plan doesn't finalize or the VM can't be set up.
pub fn runtime(options: Options) -> l3i::Result<Runtime> {
    runtime_with_capabilities(options, &[])
}

/// Makes a runtime granting only the optional capabilities explicitly approved by its host.
/// The ordinary [`runtime`] constructor grants none; a plan's grants cannot change after code starts.
///
/// # Errors
///
/// Returns an error if the runtime plan doesn't finalize or the VM can't be set up.
pub fn runtime_with_capabilities(options: Options, capabilities: &[String]) -> l3i::Result<Runtime> {
    runtime_with_profile(options, capabilities, false)
}

/// Makes a runtime with optional capabilities and l3i call/native-code profiling enabled.
/// Sampling is attributed to the context `1` used by Jess's script runner.
///
/// # Errors
///
/// Returns an error if the runtime plan doesn't finalize or the VM can't be set up.
pub fn runtime_with_profile(_options: Options, capabilities: &[String], profiling: bool) -> l3i::Result<Runtime> {
    let runtime_plan = if profiling {
        plan::plan_with_profiling(capabilities)?
    } else {
        plan::plan_with_capabilities(capabilities)?
    };
    let runtime = Runtime::from_plan(&runtime_plan)?;
    let jess = state(&runtime)?;
    *jess.compile_options.borrow_mut() = Some(runtime.compile_options());
    runtime.set_global("jess", &jess.module)?;
    require::seed_require(&runtime, &jess)?;
    let require = runtime.bind_function("jess.require", move |call: &Call<'_>, name: Text| {
        require::require(call, &jess, &name.0)
    })?;
    runtime.set_global("require", &require)?;
    runtime.set_global("print", &print_function(&runtime)?)?;
    // `vector:writef32x3(buffer, offset)`: three f32 stores, which native code lowers to stores
    // behind one bounds check; this is the interpreter half a failed guard lands in.
    runtime.install_vector_buffer_writer()?;
    // Luau only resolves builtins like `buffer.readi32` and `math.floor` once, and only calls them
    // on its fast path, in an environment marked safe. Without it every call is a table lookup
    // and a full call, which is most of the time a numeric script spends. What jess replaces
    // (`print`, `require`) is replaced above, before any script loads.
    runtime.sandbox_globals();
    if profiling {
        runtime.set_sampled_context(Some(1));
    }
    Ok(runtime)
}

/// Per-VM generic service and module-loading state.
pub(crate) struct Jess {
    /// The `jess` module table, the global of the same name.
    pub(crate) module: Table,
    /// The metatable every list `to_lua` writes gets, so a script can tell an empty list from an
    /// empty struct (`getmetatable(value) ~= nil`), and jess can when it reads one back.
    pub(crate) array_marker: Table,
    /// The plan's compile options, for the chunks jess loads; set with the runtime.
    compile_options: RefCell<Option<l3i::source::CompileOptions>>,
    /// Each chunk's bytecode by chunk name, compiled once per runtime. Chunks load on the thread that runs them (`LoadScope`), so Luau
    /// resolves their builtin imports against the safe globals up front.
    bytecode: RefCell<HashMap<String, Rc<Vec<u8>>>>,
    /// `require`'s cache: the standard libraries and installed modules by name, and every module
    /// loaded by its file.
    pub(crate) loaded: Table,
    /// Luau's own `require`, for the `@` module paths the plan registers.
    pub(crate) luau_require: RefCell<Option<Function>>,
    /// The packages an activated plan mounted ([`package::activate`]); `None` until one is.
    pub(crate) packages: RefCell<Option<package::Mounted>>,
    /// `tostring`, for `print`.
    pub(crate) tostring: Function,
}

impl Jess {
    /// Loads the chunk `source`, named `name` (an `@path` for a file or package path), as a function to
    /// call on `scope`'s thread: compiled the first time a runtime sees the name, loaded on the
    /// scope's own thread so Luau resolves its builtin imports against the safe globals, and
    /// compiled to machine code under the plan's native-code policy, whether the scope is the
    /// runtime's stack or a bound function's `Call`.
    pub(crate) fn load(&self, scope: &impl Scope, name: &str, source: &[u8]) -> l3i::Result<Function> {
        self.load_as(scope, name, name, source)
    }

    /// [`Jess::load`], with the bytecode cached under `key` rather than the chunk name: a package
    /// module's chunk name is the same in every generation of its package, its key is not.
    pub(crate) fn load_as(&self, scope: &impl Scope, key: &str, name: &str, source: &[u8]) -> l3i::Result<Function> {
        let cached = self.bytecode.borrow().get(key).cloned();
        let bytecode = if let Some(bytecode) = cached {
            bytecode
        } else {
            let source = std::str::from_utf8(source).map_err(|_| error(format!("{name}: the script isn't UTF-8")))?;
            let options = self.compile_options.borrow();
            let options = options
                .as_ref()
                .ok_or_else(|| error("this runtime wasn't made by jess::lua::runtime"))?;
            let bytecode = Rc::new(l3i::source::compile(source, options)?);
            self.bytecode.borrow_mut().insert(key.to_owned(), bytecode.clone());
            bytecode
        };
        scope.load_bytecode(name, &bytecode)
    }
}

/// The jess state of a runtime made by [`runtime`].
pub(crate) fn state(runtime: &Runtime) -> l3i::Result<Rc<Jess>> {
    runtime
        .state_of::<Jess>(plan::EXTENSION_ID)
        .ok_or_else(|| error("jess isn't installed in this Luau runtime"))
}

/// Luau's `print`, written through Rust's stdout.
///
/// Luau's own writes with C's `stdout`, which Windows opens in text mode, turning every `\n` into
/// `\r\n`, and which buffers apart from Rust's stdout, so the two could interleave out of order.
/// Like Luau's, it ignores write errors.
fn print_function(runtime: &Runtime) -> l3i::Result<Function> {
    let jess = state(runtime)?;
    runtime.bind_function("jess.print", move |call: &Call<'_>, values: l3i::bind::ArgView<'_>| {
        use std::io::Write;

        let mut line = Vec::new();
        for index in 0..values.len() {
            if index > 0 {
                line.push(b'\t');
            }
            let text = jess
                .tostring
                .invoke_with(call, (values.get(index),), |_, text| Ok(text.read::<&[u8]>()?.to_vec()))?;
            line.extend_from_slice(&text);
        }
        line.push(b'\n');
        let mut stdout = std::io::stdout().lock();
        drop(stdout.write_all(&line).and_then(|()| stdout.flush()));
        Ok::<(), l3i::Error>(())
    })
}

/// An error's message as a script or the host should see it: what jess or the script said, without
/// the `Lua error: ` a call from Rust into Lua puts in front.
pub(crate) fn error_message(problem: &l3i::Error) -> String {
    let message = problem.to_string();
    if let Some(rest) = message.strip_prefix("Lua error: ") {
        return rest.to_owned();
    }
    if let Some(rest) = message.strip_prefix("Lua error at stack index ") {
        return rest.split_once(": ").map_or(message.as_str(), |(_, rest)| rest).to_owned();
    }
    message
}

/// An error a call from Rust into a script's function raised, as the script sees it.
#[allow(clippy::needless_pass_by_value)] // Used as `map_err(callback_error)`.
pub(crate) fn callback_error(problem: l3i::Error) -> l3i::Error {
    error(error_message(&problem))
}

/// Checks an option table's keys against the ones `function` takes, so a misspelled option is an
/// error naming it instead of silently doing nothing.
pub(crate) fn check_options(frame: &Frame<'_>, options: ValueView<'_>, function: &str, keys: &[&str]) -> l3i::Result<()> {
    if options.is_nil() {
        return Ok(());
    }
    let table = options.as_table()?;
    let mut unknown = None;
    table.for_each(frame, |_, key, _| {
        if unknown.is_some() {
            return Ok(());
        }
        let known = key
            .read::<&[u8]>()
            .is_ok_and(|name| key.is_string() && keys.iter().any(|known| known.as_bytes() == name));
        if !known {
            unknown = Some(unknown_option(function, key, keys));
        }
        Ok(())
    })?;
    unknown.map_or(Ok(()), Err)
}

/// The error for an option `function` doesn't take, listing the ones it does.
pub(crate) fn unknown_option(function: &str, key: ValueView<'_>, keys: &[&str]) -> l3i::Error {
    let key = if key.is_string() {
        format!("'{}'", String::from_utf8_lossy(key.read::<&[u8]>().unwrap_or_default()))
    } else if key.is_number() || key.is_integer() {
        lua::number_text(key.read::<f64>().unwrap_or_default())
    } else {
        kind(key).to_owned()
    };
    let expected = match keys {
        [] => "none".to_owned(),
        [only] => (*only).to_owned(),
        [first, second] => format!("{first} or {second}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    };
    error(format!("unknown {function} option {key}: expected {expected}"))
}

/// A Luau error carrying `message`.
pub(crate) fn error(message: impl Display) -> l3i::Error {
    l3i::Error::runtime(message.to_string())
}

/// Checks that the calling script holds `capability`, when it runs under a `ScriptDomain`.
///
/// Outside any domain nothing holds authority: a plain `jess run`, a CLI command, or a test
/// evaluating a runtime directly is the process's own user acting, and every call succeeds.
/// Under a domain the check is the broker's, and a denial is its permission error, attributed
/// to the domain, generation, and context or worker that asked.
pub(crate) fn guard(capability: &str) -> l3i::Result<()> {
    if crate::runtime::current_authority().is_none() {
        return Ok(());
    }
    crate::runtime::require_capability(capability).map_err(l3i::Error::from)
}

/// The `Value` on top of `scope` after `build` pushed it, pinned.
pub(crate) fn stored(scope: &impl Scope, build: impl FnOnce(&Frame<'_>) -> l3i::Result<()>) -> l3i::Result<Value> {
    scope.with_frame(|frame| {
        build(frame)?;
        Value::store(frame.top_value())
    })
}
