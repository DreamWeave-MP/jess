//! File and module execution through the runtime's ordinary loading and require paths.

use std::path::Path;

use l3i::Runtime;
use l3i::runtime::{CallContext, CallKind, MemoryCategory};
use l3i::userdata::Userdata;
use l3i::value::{Function, Table, Value};

use crate::lua::{Jess, error, state};

/// A normalized virtual script path.
#[must_use]
pub fn script_name(entry: &str) -> String {
    entry.replace('\\', "/").to_lowercase()
}

fn script_call(runtime: &Runtime) -> l3i::runtime::CallScope<'_> {
    runtime.call_scope(
        CallContext {
            id: 1,
            category: MemoryCategory(0),
        },
        CallKind::ScriptCall,
    )
}

/// Loads a named source chunk as a function.
///
/// # Errors
/// Returns a compilation error or an error if the runtime has no Jess host state.
pub fn load(runtime: &Runtime, name: &str, source: &[u8]) -> l3i::Result<Function> {
    let jess = state(runtime)?;
    jess.load(&runtime.stack(), &format!("@{name}"), source)
}

/// Runs a file, passing its arguments through `...` and the global `arg` table.
///
/// # Errors
/// Returns a file-read, compilation, or script error.
pub fn run_file(runtime: &Runtime, path: &Path, args: &[String]) -> l3i::Result<Vec<Value>> {
    let source = std::fs::read(path).map_err(|source| error(format!("failed to read '{}': {source}", path.display())))?;
    let name = path.display().to_string();
    let function = load(runtime, &name, &source)?;
    run_loaded(runtime, &name, &function, args)
}

/// Runs a loaded chunk as a script: `args` through `...` and the global `arg` table, whose index
/// 0 is `name`.
pub(crate) fn run_loaded(runtime: &Runtime, name: &str, function: &Function, args: &[String]) -> l3i::Result<Vec<Value>> {
    let jess = state(runtime)?;
    let (arg, call_args) = {
        let stack = runtime.stack();
        let arg = Table::new(&stack, args.len(), 1)?;
        stack.with_frame(|frame| {
            let table = arg.push_to(frame)?;
            frame.push_string(name);
            table.raw_set_index(frame, 0)?;
            for (index, value) in args.iter().enumerate() {
                frame.push_string(value);
                table.raw_set_index(frame, index as i64 + 1)?;
            }
            Ok(())
        })?;
        let call_args = args
            .iter()
            .map(|value| {
                crate::lua::stored(&stack, |frame| {
                    frame.push_string(value);
                    Ok(())
                })
            })
            .collect::<l3i::Result<Vec<_>>>()?;
        (arg, call_args)
    };
    let globals = Table::from_value(runtime.global("_G")?)?;
    l3i::readonly::set_read_only_field(runtime, &globals, "arg", &arg)?;
    let _scope = script_call(runtime);
    call_variadic(runtime, &jess, function, &call_args).map_err(crate::lua::callback_error)
}

/// Runs a file or requires a module and checks its first returned userdata value.
///
/// # Errors
/// Returns a loading/execution error, or an error when the result is not a `T`.
pub fn load_module<T: Userdata + Clone>(runtime: &Runtime, module: &Path) -> l3i::Result<T> {
    let jess = state(runtime)?;
    let name = module.display().to_string();
    let function = if module.is_file() {
        let source = std::fs::read(module).map_err(|source| error(format!("failed to read '{name}': {source}")))?;
        jess.load(&runtime.stack(), &format!("@{name}"), &source)?
    } else {
        Function::from_value(runtime.global("require")?)?
    };
    let stack = runtime.stack();
    let _scope = script_call(runtime);
    function
        .invoke_with(&stack, (name.as_str(),), |_, returned| {
            if let Some(value) = l3i::userdata::receiver::<T>(returned) {
                return Ok(value.clone());
            }
            let came_back = if returned.is_nil() {
                "nil".to_owned()
            } else if returned.is_userdata() {
                format!("a {}", l3i::diagnostics::object_type_name(returned))
            } else {
                format!("a {}", crate::lua::kind(returned))
            };
            Err(error(format!(
                "{name}: expected the module to return a {}, but it returned {came_back}",
                T::NAME
            )))
        })
        .map_err(crate::lua::callback_error)
}

fn call_variadic(runtime: &Runtime, jess: &Jess, function: &Function, args: &[Value]) -> l3i::Result<Vec<Value>> {
    let stack = runtime.stack();
    let list = Table::new(&stack, args.len(), 0)?;
    stack.with_frame(|frame| {
        let table = list.push_to(frame)?;
        for (index, value) in args.iter().enumerate() {
            value.push_to(frame)?;
            table.raw_set_index(frame, index as i64 + 1)?;
        }
        Ok(())
    })?;
    let unpack = jess.load(
        &stack,
        "=jess",
        b"local chunk, args, count = ...\nreturn chunk(table.unpack(args, 1, count))",
    )?;
    #[allow(clippy::cast_precision_loss)]
    unpack.invoke_multi(&stack, (function, &list, args.len() as f64))
}
