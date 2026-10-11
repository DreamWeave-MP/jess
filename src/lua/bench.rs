//! `jess.bench`: what a measurement needs from the host and nothing else gives it.
//!
//! A number means something only on the build releases ship, so a harness has to be able to tell
//! which build it is running on; and one batch's garbage must not be collected inside the next
//! one's timing, so it has to be able to collect between them.

use l3i::Runtime;
use l3i::bind::Call;
use l3i::stack::Scope;
use l3i::value::Table;

use crate::lua::plan::Functions;

/// Whether this jess was built with l3i's verified toolchain: cross-language LTO, as releases are.
///
/// `tools/dev/no-lto.sh` and the MSRV check set `L3I_UNVERIFIED_TOOLCHAIN`, and what they build
/// runs Luau slower than a release does.
pub const VERIFIED: bool = option_env!("L3I_UNVERIFIED_TOOLCHAIN").is_none();

/// The module: `verified` and `collect`.
pub(crate) fn module(runtime: &Runtime) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "bench")?;
    module.set("verified", &VERIFIED)?;
    module.function("collect", collect)?;
    module.finish()
}

/// A full collection cycle, run from inside the script's call.
#[allow(ffi_unwind_calls)] // A collection doesn't raise.
fn collect(call: &Call<'_>) {
    // SAFETY: the call's state is live, and a full collection is what Luau's own `collectgarbage`
    // runs from inside a call; nothing here holds a pointer into the heap.
    unsafe { l3i::ffi::lua_gc(call.state(), l3i::ffi::LUA_GCCOLLECT, 0) };
}
