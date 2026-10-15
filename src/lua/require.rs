//! Generic module loading: native modules, package exports, and caller-relative files.

use std::path::PathBuf;

use l3i::Runtime;
use l3i::bind::{Call, StackResults};
use l3i::debug::DebugScope;
use l3i::stack::Scope;
use l3i::value::{Function, Value};

use crate::lua::{Jess, error};

/// Standard libraries available by name.
const PRELOADED: [&str; 6] = ["coroutine", "math", "string", "table", "utf8", "os"];

/// The members the built-in package wraps in a Luau module of the same `@jess/` name: `jess.input`
/// is the native half of `@jess/input`.
const WRAPPED: [&str; 1] = ["input"];

/// Fills `require`'s cache with what it hands out by name: the preloaded standard libraries, and
/// the installed modules by their global names, with and without a leading `@`. Luau's own
/// `require`, which the plan installed, is kept for the `@` module paths it knows.
pub fn seed_require(runtime: &Runtime, jess: &Jess) -> l3i::Result<()> {
    let stack = runtime.stack();
    stack.with_frame(|frame| {
        let loaded = jess.loaded.push_to(frame)?;
        for name in PRELOADED {
            let library = Value::get_global(frame, name)?;
            if library.is_table() {
                library.push_to(frame)?;
                loaded.raw_set(frame, name)?;
            }
        }
        let module = Value::get_global(frame, "jess")?;
        for name in ["jess", "@jess"] {
            module.push_to(frame)?;
            loaded.raw_set(frame, name)?;
        }
        // Each of jess's own modules is `@jess/<name>` too: `require("@jess/shader")`, except the
        // ones the built-in package wraps, whose `@jess/` name is its Luau module.
        let jess_module = jess.module.push_to(frame)?;
        for name in crate::lua::plan::MEMBERS.into_iter().filter(|name| !WRAPPED.contains(name)) {
            frame.with_frame(|frame| {
                let member = jess_module.raw_get(frame, name)?;
                if member.is_table() {
                    loaded.raw_set(frame, &format!("@jess/{name}"))?;
                }
                Ok(())
            })?;
        }
        Ok(())
    })?;
    drop(stack);
    *jess.luau_require.borrow_mut() = Some(Function::from_value(runtime.global("require")?)?);
    Ok(())
}

/// Resolves installed modules, package exports, and caller-relative files.
pub fn require(call: &Call<'_>, jess: &Jess, name: &str) -> l3i::Result<StackResults> {
    let relative = name.starts_with("./") || name.starts_with("../") || name.starts_with("@self/");
    // Before the cache: a module one package loaded is no more another's to require for that.
    if !relative {
        check_declared(call, jess, name)?;
    }
    let cached = call.with_frame(|frame| {
        let loaded = jess.loaded.push_to(frame)?;
        let cached = loaded.raw_get(frame, name)?;
        if cached.is_nil() {
            return Ok(None);
        }
        Value::store(cached).map(Some)
    })?;
    if let Some(cached) = cached {
        cached.push_to_scope(call)?;
        return Ok(StackResults);
    }
    // A package export comes before Luau's own `@` modules; resolution already refused any
    // export a host module has.
    let mut export = if relative { None } else { package_export(call, jess, name)? };
    // The built-in package's exports resolve in every runtime; a copy of it activated from disk
    // is byte for byte the same package, and was found first.
    if export.is_none() && !relative && name.starts_with("@jess/") {
        let manifest = crate::lua::builtin::manifest(call, jess)?;
        if let Some(path) = manifest.export(name) {
            export = manifest.module(path).map(Module::from);
        }
    }
    // `@jess/` is jess's own namespace: its built-in modules, its members, and the exports of its
    // built-in package, never a file.
    if export.is_none() && name.starts_with("@jess/") && !crate::lua::stdlib::contains(name) {
        return Err(error(format!(
            "module not found: {name}: @jess/ is jess's own namespace (its members, such as @jess/shader, {}, and the built-in package's exports, such as @jess/style), never a file",
            crate::lua::stdlib::MODULES.join(", ")
        )));
    }
    if export.is_none() && !relative && name.starts_with('@') {
        let luau_require = jess
            .luau_require
            .borrow()
            .clone()
            .ok_or_else(|| error("this runtime wasn't made by jess::lua::runtime"))?;
        // Luau's own require says "could not reset to requiring context" for a name nothing
        // registered, which says nothing of what is wrong: no package in this run exports it.
        return match call.forward_to(&luau_require, 1) {
            Ok(_) => Ok(StackResults),
            // The failed call left its error on top of the stack.
            Err(l3i::Error::LuaErrorOnStack)
                if call
                    .at(-1)
                    .read::<&str>()
                    .is_ok_and(|message| message.contains("could not reset to requiring context")) =>
            {
                Err(error(format!(
                    "module not found: {name}: no package activated in this run exports it, and it isn't one of jess's own modules"
                )))
            }
            Err(problem) => Err(problem),
        };
    }
    let module = if let Some(module) = export {
        module
    } else if relative {
        // The caller is the function that called `require`, one level up.
        let caller = caller_chunk(call);
        match package_relative(call, jess, name, caller.as_deref())? {
            Some(module) => module,
            None => find_relative(name, caller.as_deref())?,
        }
    } else {
        return Err(error(format!("module not found: {name}")));
    };
    let value = call.with_frame(|frame| {
        let loaded = jess.loaded.push_to(frame)?;
        let cached = loaded.raw_get(frame, &module.key)?;
        let value = if cached.is_nil() {
            let chunk = jess.load_as(frame, &module.key, &module.chunk, &module.source)?;
            let value = chunk
                .invoke_with(frame, (name,), |_, value| Value::store(value))
                .map_err(crate::lua::callback_error)?;
            freeze_export(&module.key, value.push_to(frame)?)?;
            loaded.raw_set(frame, &module.key)?;
            value
        } else {
            Value::store(cached)?
        };
        if !relative {
            value.push_to(frame)?;
            loaded.raw_set(frame, name)?;
        }
        Ok(value)
    })?;
    value.push_to_scope(call)?;
    Ok(StackResults)
}

/// Freezes `value`, what the module cached under `key` returned, when it is a package module's
/// table. A package module's value is shared by every module that requires it, whether `require`
/// or a layer's entrypoint loaded it first, so it is frozen once returned: one package can't
/// patch another's API for the whole plan. A loose script's own files (keyed by their chunk name,
/// not `\0...`) stay its business.
pub(crate) fn freeze_export(key: &str, value: l3i::stack::ValueView<'_>) -> l3i::Result<()> {
    if key.starts_with('\0') && value.is_table() {
        let table = value.as_table()?;
        if !table.is_read_only() {
            table.set_read_only(true)?;
        }
    }
    Ok(())
}

/// Fails when a mounted package's module requires another mounted package's export without
/// declaring that package as a dependency. A script that belongs to no package holds the whole
/// plan's namespace: it is the host's, and the host chose the plan.
fn check_declared(call: &Call<'_>, jess: &Jess, name: &str) -> l3i::Result<()> {
    let mounted = jess.packages.borrow();
    let Some(mounted) = mounted.as_ref() else {
        return Ok(());
    };
    let Some((owner, _)) = mounted.export(name) else {
        return Ok(());
    };
    // The nearest Luau function is the one requiring, whatever it called `require` through.
    let caller = (1..)
        .map_while(|level| call.call_site(level))
        .find(|site| site.source != "[C]")
        .map(|site| format!("@{}", site.source));
    let Some(caller) = caller else {
        return Ok(());
    };
    let Some((index, _)) = mounted.owner(&caller) else {
        return Ok(());
    };
    if mounted.may_require(index, owner) {
        return Ok(());
    }
    Err(error(format!(
        "module not found: {name} (required by {caller}): {} exports it, and {} doesn't declare {} as a dependency",
        mounted.id(owner),
        mounted.id(index),
        mounted.id(owner)
    )))
}

/// The chunk name of the function that called `require`, with its `@`: level 1 above the
/// `require` C function.
fn caller_chunk(call: &Call<'_>) -> Option<String> {
    let site = call.call_site(1)?;
    (site.source != "[C]").then(|| format!("@{}", site.source))
}

/// A module's source, the chunk name it loads with (`@` and its path on disk or in
/// a package), and the key it is cached by: the chunk name, or for a package module its package's
/// identity and digest with its path.
pub struct Module {
    pub chunk: String,
    pub key: String,
    pub source: Vec<u8>,
}

impl Module {
    fn new(chunk: String, source: Vec<u8>) -> Self {
        Self {
            key: chunk.clone(),
            chunk,
            source,
        }
    }
}

impl From<crate::lua::package::PackageModule> for Module {
    fn from(module: crate::lua::package::PackageModule) -> Self {
        Self {
            chunk: module.chunk,
            key: module.key,
            source: module.source,
        }
    }
}

/// The module a mounted package exports as `name`, if one does.
fn package_export(call: &Call<'_>, jess: &Jess, name: &str) -> l3i::Result<Option<Module>> {
    let mounted = jess.packages.borrow();
    let Some(mounted) = mounted.as_ref() else {
        return Ok(None);
    };
    let Some((index, path)) = mounted.export(name) else {
        return Ok(None);
    };
    let module = mounted.load(call, index, path)?.ok_or_else(|| {
        error(format!(
            "module not found: {name}: {} exports it as '{path}', which it doesn't hold",
            mounted.id(index)
        ))
    })?;
    Ok(Some(module.into()))
}

/// A `./`, `../`, or `@self/` module required from a mounted package's module, resolved the way
/// [`find_relative`] resolves files, inside that package: a path that leaves its root is an error,
/// never a lookup somewhere else. `None` when the caller isn't a package module.
fn package_relative(call: &Call<'_>, jess: &Jess, name: &str, caller: Option<&str>) -> l3i::Result<Option<Module>> {
    let Some(caller) = caller else {
        return Ok(None);
    };
    let mounted = jess.packages.borrow();
    if let Some(mounted) = mounted.as_ref()
        && let Some((index, path)) = mounted.owner(caller)
    {
        for candidate in relative_candidates(name, caller, path, mounted.id(index))? {
            if let Some(module) = mounted.load(call, index, &candidate)? {
                return Ok(Some(module.into()));
            }
        }
        return Err(error(format!("module not found: {name} (required by {caller})")));
    }
    // The modules the bootstrap loaded to read TOML, before any manifest was read.
    if let Some(path) = caller.strip_prefix(crate::lua::builtin::BOOTSTRAP_PREFIX) {
        for candidate in relative_candidates(name, caller, path, crate::lua::builtin::ID)? {
            if let Some(module) = crate::lua::builtin::bootstrap_module(&candidate) {
                return Ok(Some(module.into()));
            }
        }
        return Err(error(format!("module not found: {name} (required by {caller})")));
    }
    // The built-in package, when it wasn't activated from disk.
    if caller.starts_with(&format!("@{}@", crate::lua::builtin::ID)) {
        let manifest = crate::lua::builtin::manifest(call, jess)?;
        if let Some(path) = caller.strip_prefix(manifest.prefix()) {
            for candidate in relative_candidates(name, caller, path, crate::lua::builtin::ID)? {
                if let Some(module) = manifest.module(&candidate) {
                    return Ok(Some(module.into()));
                }
            }
            return Err(error(format!("module not found: {name} (required by {caller})")));
        }
    }
    Ok(None)
}

/// The package paths a relative `name` required from the module at `path` may be, in the order
/// [`find_relative`] tries files: a path that leaves the package's root is an error.
fn relative_candidates(name: &str, caller: &str, path: &str, package: &str) -> l3i::Result<Vec<String>> {
    let init = std::path::Path::new(path)
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("init"));
    let (relative, levels) = match name.strip_prefix("@self/") {
        Some(_) if !init => {
            return Err(error(format!("module not found: {name}: @self is only for init scripts")));
        }
        Some(rest) => (format!("./{rest}"), 1),
        None => (name.to_owned(), if init { 2 } else { 1 }),
    };
    let escapes = || {
        error(format!(
            "module not found: {name} (required by {caller}): it leaves the package root of {package}"
        ))
    };
    let mut parts = path.split('/').collect::<Vec<_>>();
    for _ in 0..levels {
        parts.pop().ok_or_else(escapes)?;
    }
    for part in relative.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop().ok_or_else(escapes)?;
            }
            part => parts.push(part),
        }
    }
    let base = parts.join("/");
    if base.is_empty() {
        return Ok(Vec::new());
    }
    Ok([".jsl", "/init.jsl"].iter().map(|suffix| format!("{base}{suffix}")).collect())
}

/// Resolves a file import relative to its calling script.
///
/// It reads the host's files without `filesystem.read`, on purpose: a loose script's sibling
/// modules are its own code, which the host chose to run when it ran the script, and reading them
/// is loading code, not reading data. Only a loose script gets here: a package module's relative
/// imports resolve inside its payload (`package_relative`), and a chunk can't pass for a loose
/// script's (`scripts::chunk_name`).
fn find_relative(name: &str, caller: Option<&str>) -> l3i::Result<Module> {
    let not_found = || error(format!("module not found: {name}"));
    let Some(caller) = caller.and_then(|caller| caller.strip_prefix('@')) else {
        return Err(error(format!(
            "module not found: {name}: a relative require needs a script file to be relative to"
        )));
    };
    let suffixes = [".jsl", "/init.jsl"];
    // An init script is its folder's module: `./` starts beside the folder, and `@self/` in it.
    let init = std::path::Path::new(caller)
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("init"));
    let (name, levels) = match name.strip_prefix("@self/") {
        Some(_) if !init => {
            return Err(error(format!("module not found: {name}: @self is only for init scripts")));
        }
        Some(rest) => (format!("./{rest}"), 1),
        None => (name.to_owned(), if init { 2 } else { 1 }),
    };
    let script = std::path::Path::new(caller);
    if script.is_file() {
        let mut base = script;
        for _ in 0..levels {
            base = base.parent().unwrap_or_else(|| std::path::Path::new(""));
        }
        let base = base.join(&name);
        for suffix in suffixes {
            let candidate = PathBuf::from(format!("{}{suffix}", base.display()));
            if candidate.is_file() {
                let path = std::fs::canonicalize(&candidate).unwrap_or(candidate);
                let source = std::fs::read(&path)
                    .map_err(|source| error(format!("failed to read '{}': {source}", path.display())))?;
                return Ok(Module::new(format!("@{}", path.display()), source));
            }
        }
        return Err(not_found());
    }
    Err(not_found())
}
