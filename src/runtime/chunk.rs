//! Loading a chunk the way jess loads every script.
//!
//! The chunk is compiled with the runtime's own options and loaded through l3i's `LoadScope`,
//! which compiles it natively when the plan has a code generator, so a domain's or a worker's
//! code is never quietly slower than the same code under `jess run`.

use l3i::Runtime;
use l3i::source::LoadScope;
use l3i::value::Function;

/// Compiles `source` and loads it as the chunk `chunk_name` (`@path` for a file, `=name` for
/// generated code), returning the chunk's function pinned. Nothing runs yet.
///
/// # Errors
///
/// A compile error, with Luau's message and the chunk name, or a chunk name holding NUL.
pub fn load(runtime: &Runtime, chunk_name: &str, source: &str) -> l3i::Result<Function> {
    runtime.stack().load_source(chunk_name, source, &runtime.compile_options())
}
