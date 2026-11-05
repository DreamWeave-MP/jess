//! Timer callbacks register with their owning domain; no standalone event loop is hidden here.
use std::rc::Rc;
use std::time::Duration;

use l3i::bind::Call;
use l3i::extension::{ExtensionDescriptor, InstallContext, TagPolicy};
use l3i::userdata::{Owned, Userdata};
use l3i::value::{Function, Table};

use super::{Bindings, DomainCore, DomainLink, Owner};

const MODULE: &str = "@jess/timer";

struct Timer {
    domain: DomainLink,
    id: u64,
}

// SAFETY: only a weak Rust reference and integer; dropping this handle never calls Luau.
unsafe impl Userdata for Timer {
    const NAME: &'static str = "jess.runtime.Timer";
}

pub(super) fn describe(d: &mut ExtensionDescriptor) {
    let mut handle = d.userdata::<Timer>(Timer::NAME);
    handle
        .tag(TagPolicy::Preferred)
        .doc("One domain-owned timer. Dropping the handle does not cancel it.");
    handle
        .method("cancel", |timer: &Timer| {
            timer.domain.upgrade().is_some_and(|domain| domain.cancel_timer(timer.id))
        })
        .signature("(self): boolean")
        .doc("Cancels once; false after cancellation, completion or teardown. Safe inside the callback.");
    handle
        .getter("active", |timer: &Timer| {
            timer.domain.upgrade().is_some_and(|domain| domain.timer_active(timer.id))
        })
        .signature("boolean");
    d.module(MODULE)
        .doc("Domain timers driven by advance(delta), in milliseconds. Standalone scripts use @jess/net's cooperative task runner.")
        .installed("after").signature("(delayMs: number, callback: () -> ()) -> jess_runtime_Timer")
        .doc("Schedules one callback under the current context's authority and budget. Zero waits until the next timer dispatch. Contexts must schedule the timer phase; application managed contexts do so automatically.")
        .installed("every").signature("(intervalMs: number, callback: () -> ()) -> jess_runtime_Timer")
        .doc("Schedules a positive repeating interval. Fires at most once per advance, then schedules from that advance's time; missed ticks are skipped. Timers created during dispatch wait for a later advance.")
        .installed("now").signature("() -> number")
        .doc("Elapsed simulated domain time in milliseconds, advanced by the host's delta and reset on restart.");
}

/// The domain a timer belongs to. A script with no frame loop is told what it can do instead,
/// because a callback scheduled there would never run.
fn owner(bindings: &Bindings, name: &str) -> l3i::Result<Rc<DomainCore>> {
    if matches!(&*bindings.owner.borrow(), Owner::Detached) {
        return Err(l3i::Error::runtime(format!(
            "@jess/timer.{name}: needs a running application, and nothing would ever fire here; a script with no frames waits with @jess/net's run and sleep"
        )));
    }
    bindings.domain(&format!("@jess/timer.{name}"))
}

pub(super) fn install(cx: &mut InstallContext<'_>, bindings: &Rc<Bindings>) -> l3i::Result<Table> {
    let module = cx.module(MODULE)?;
    for (name, repeating) in [("after", false), ("every", true)] {
        let bindings = Rc::clone(bindings);
        module.function(name, move |call: &Call<'_>, milliseconds: f64, callback: Function| -> l3i::Result<Owned<Timer>> {
            // A year is an explicit API bound, not a float-to-duration conversion accident.
            if !(0.0..=31_536_000_000.0).contains(&milliseconds) || (repeating && milliseconds == 0.0) {
                return Err(l3i::Error::runtime("timer delay must be finite, nonnegative and at most 31536000000 milliseconds; repeating intervals must be positive"));
            }
            let delay = Duration::try_from_secs_f64(milliseconds / 1000.0)
                .map_err(|_| l3i::Error::runtime("timer delay cannot be represented"))?;
            if repeating && delay.is_zero() { return Err(l3i::Error::runtime("timer interval is below one nanosecond")); }
            let domain = owner(&bindings, name)?;
            let id = domain.schedule_timer(delay, repeating, callback, l3i::runtime::caller_location(call))?;
            Ok(Owned(Timer { domain: Rc::downgrade(&domain), id }))
        })?;
    }
    let bindings = Rc::clone(bindings);
    module.function("now", move || -> l3i::Result<f64> {
        Ok(owner(&bindings, "now")?.timer_now().as_secs_f64() * 1000.0)
    })?;
    Ok(module.table().clone())
}
