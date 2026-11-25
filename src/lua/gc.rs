//! `jess.gc`: the frame's share of Luau's garbage collection, for code that drives its own frames
//! and for the application whose host drives them.
//!
//! An application host collects after every frame ([`crate::host::Host::collect`]); `step` is the
//! same collection, so a script running its own frame loop (a benchmark simulating frames, say)
//! can do and measure exactly what the host does. Each runtime paces its own heap: the domain's
//! code shares its host's [`Collector`], and a worker or a script without a host has one of its
//! own.

use std::sync::Arc;
use std::time::Duration;

use l3i::Runtime;
use l3i::bind::Call;
use l3i::stack::Scope;
use l3i::value::Table;

use crate::host::collector::{self, Collector, Heap};
use crate::lua::error;
use crate::lua::plan::Functions;

/// The calling script's heap, stepped from inside its call: what a collection from Luau steps.
struct Script(*mut l3i::ffi::lua_State);

// Neither a count nor a collector step raises.
#[allow(ffi_unwind_calls)]
impl Heap for Script {
    fn kilobytes(&self) -> u64 {
        // SAFETY: the call's state is live; a count neither allocates nor collects.
        u64::try_from(unsafe { l3i::ffi::lua_gc(self.0, l3i::ffi::LUA_GCCOUNT, 0) }).unwrap_or(0)
    }

    fn paused(&self) -> bool {
        // SAFETY: as for the count.
        unsafe { l3i::ffi::lua_gc(self.0, l3i::ffi::LUA_GCISPAUSED, 0) == 1 }
    }

    fn step(&self, kilobytes: i32) -> bool {
        // SAFETY: the call's state is live, and an incremental step is as permitted from inside a
        // call as the full collection Luau's own `collectgarbage` runs there: the call's arguments
        // are on its stack, and nothing here holds a pointer into the heap.
        unsafe { l3i::ffi::lua_gc(self.0, l3i::ffi::LUA_GCSTEP, kilobytes) == 1 }
    }
}

/// The collector the calling code paces its heap with: its host's for the domain's own code, the
/// runtime's own for a worker and for code with no host.
fn collector(own: &Arc<Collector>) -> Arc<Collector> {
    crate::runtime::current_authority()
        .filter(|authority| authority.attribution().worker.is_none())
        .and_then(|authority| authority.service::<Collector>())
        .unwrap_or_else(|| Arc::clone(own))
}

/// `milliseconds` as a budget, or why it isn't one.
fn budget(function: &str, milliseconds: f64) -> l3i::Result<Duration> {
    let most = collector::MAX_BUDGET.as_secs_f64() * 1000.0;
    if !(0.0..=most).contains(&milliseconds) {
        return Err(error(format!(
            "jess.gc.{function}: the budget is from 0 to {most} milliseconds, not {milliseconds}"
        )));
    }
    Ok(Duration::from_secs_f64(milliseconds / 1000.0))
}

/// The module: `step`, `budget`, and `setBudget`.
pub fn module(runtime: &Runtime) -> l3i::Result<Table> {
    let own = Arc::new(Collector::default());
    let module = Functions::module(runtime, "gc")?;
    let stepping = Arc::clone(&own);
    module.function(
        "step",
        move |call: &Call<'_>, milliseconds: Option<f64>| -> l3i::Result<(f64, bool)> {
            let collector = collector(&stepping);
            let budget = match milliseconds {
                Some(milliseconds) => budget("step", milliseconds)?,
                None => collector.budget(),
            };
            let collection = collector.collect(&Script(call.state()), budget);
            Ok((collection.spent.as_secs_f64() * 1000.0, collection.finished))
        },
    )?;
    let reading = Arc::clone(&own);
    module.function("budget", move || collector(&reading).budget().as_secs_f64() * 1000.0)?;
    module.function("setBudget", move |milliseconds: f64| -> l3i::Result<()> {
        let budget = budget("setBudget", milliseconds)?;
        collector(&own).set_budget(budget);
        Ok(())
    })?;
    module.finish()
}
