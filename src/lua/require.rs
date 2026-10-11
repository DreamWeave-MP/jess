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
        // Each of jess's own modules is `@jess/<name>` too: `require("@jess/shader")`.
        let jess_module = jess.module.push_to(frame)?;
        for name in crate::lua::plan::MEMBERS {
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
    if export.is_none()
        && !relative
        && let Some(path) = crate::lua::builtin::export(name)
    {
        export = crate::lua::builtin::module(path).map(Module::from);
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
        call.forward_to(&luau_require, 1)?;
        return Ok(StackResults);
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
            value.push_to(frame)?;
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
    // The built-in package, when it wasn't activated from disk.
    if let Some(path) = caller.strip_prefix(crate::lua::builtin::prefix()) {
        for candidate in relative_candidates(name, caller, path, crate::lua::builtin::ID)? {
            if let Some(module) = crate::lua::builtin::module(&candidate) {
                return Ok(Some(module.into()));
            }
        }
        return Err(error(format!("module not found: {name} (required by {caller})")));
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
    Ok([".luau", ".lua", "/init.luau", "/init.lua"]
        .iter()
        .map(|suffix| format!("{base}{suffix}"))
        .collect())
}

/// Resolves a file import relative to its calling script.
fn find_relative(name: &str, caller: Option<&str>) -> l3i::Result<Module> {
    let not_found = || error(format!("module not found: {name}"));
    let Some(caller) = caller.and_then(|caller| caller.strip_prefix('@')) else {
        return Err(error(format!(
            "module not found: {name}: a relative require needs a script file to be relative to"
        )));
    };
    let suffixes = [".luau", ".lua", "/init.luau", "/init.lua"];
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
