//! Domain timer ownership and dispatch through the same call path as every other phase.
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use l3i::value::Function;

use super::{ContextSpec, Dispatch, DomainCore, DomainState, Phase, PhaseReport, RuntimeError, ScriptDomain};
use crate::runtime::capability::current_authority;
use crate::runtime::ids::ContextId;

/// What a scheduled timer remembers: whose it is and what to call. Its authority and budget are
/// its context's, read when it fires, so reconfiguring the context reaches timers already queued.
#[derive(Clone)]
pub(super) struct Scheduled {
    context: ContextId,
    handler: Function,
    source: Rc<str>,
}

impl DomainCore {
    pub(crate) fn schedule_timer(
        &self,
        delay: Duration,
        repeating: bool,
        handler: Function,
        source: String,
    ) -> l3i::Result<u64> {
        self.require_state(
            &[DomainState::Loading, DomainState::Ready, DomainState::Running],
            "schedule a timer in",
        )
        .map_err(l3i::Error::from)?;
        let authority =
            current_authority().ok_or_else(|| l3i::Error::runtime("timer registration needs domain authority"))?;
        if authority.attribution().domain != self.id || authority.attribution().generation != self.generation.get() {
            return Err(l3i::Error::permission(
                "timer registration belongs to a different domain generation",
            ));
        }
        let context = if let Some(context) = authority.attribution().context {
            context
        } else {
            let existing = self
                .contexts
                .borrow()
                .iter()
                .find(|record| record.name == "@jess/timer")
                .map(|record| record.id);
            match existing {
                Some(id) => id,
                None => self
                    .register_context(ContextSpec::new("@jess/timer").schedule([Phase::Timer]))
                    .map_err(l3i::Error::from)?,
            }
        };
        let contexts = self.contexts.borrow();
        let record = contexts
            .iter()
            .find(|record| record.id == context)
            .ok_or_else(|| l3i::Error::runtime("the timer's context no longer exists"))?;
        if !record.schedule.contains(&Phase::Timer) {
            return Err(l3i::Error::permission(format!(
                "context '{}' does not schedule the timer phase",
                record.name
            )));
        }
        let scheduled = Scheduled {
            context,
            handler,
            source: Rc::from(source),
        };
        self.timers
            .borrow_mut()
            .insert(delay, repeating, scheduled)
            .map_err(l3i::Error::runtime)
    }

    /// The call `scheduled` makes now: its context's current authority and budget, or `None` when
    /// the context is gone or no longer schedules the timer phase.
    fn timer_dispatch(&self, scheduled: Scheduled) -> Option<Dispatch> {
        let contexts = self.contexts.borrow();
        let record = contexts
            .iter()
            .find(|record| record.id == scheduled.context && record.schedule.contains(&Phase::Timer))?;
        Some(Dispatch {
            context: scheduled.context,
            handler: scheduled.handler,
            source: Some(scheduled.source),
            event: None,
            stats: Rc::clone(&record.stats),
            capabilities: Arc::clone(&record.capabilities),
            budget: record.budget,
        })
    }

    pub(crate) fn cancel_timer(&self, id: u64) -> bool {
        self.timers.borrow_mut().cancel(id)
    }
    pub(crate) fn timer_active(&self, id: u64) -> bool {
        self.timers.borrow().get(id).is_some()
    }
    pub(crate) fn timer_now(&self) -> Duration {
        self.timers.borrow().now()
    }
}

impl ScriptDomain {
    pub(super) fn dispatch_timers(&self) -> Result<PhaseReport, RuntimeError> {
        let _dispatch = self.core.enter_dispatch(Phase::Timer, "dispatch timers in")?;
        let runtime = self.runtime.borrow();
        let runtime = runtime.as_ref().ok_or_else(|| self.core.lifecycle("dispatch timers in"))?;
        let batch = self.core.timers.borrow_mut().due();
        let mut report = PhaseReport {
            phase: Phase::Timer,
            ran: 0,
            failures: Vec::new(),
        };
        let started = Instant::now();
        let mut batch = batch.into_iter();
        while let Some(id) = batch.next() {
            let scheduled = self.core.timers.borrow().get(id).cloned();
            let Some(scheduled) = scheduled else { continue };
            let Some(target) = self.core.timer_dispatch(scheduled) else {
                self.core.cancel_timer(id);
                continue;
            };
            report.ran += 1;
            if let Err(error) = self
                .core
                .call(runtime, &target, Phase::Timer, Duration::ZERO, true, |handler| {
                    handler.invoke::<(), ()>(&runtime.stack(), ())
                })
            {
                report.failures.push(error);
            }
            self.core.timers.borrow_mut().finish(id);
            if self
                .core
                .timer_dispatch_budget
                .is_some_and(|budget| started.elapsed() >= budget)
            {
                self.core.timers.borrow_mut().restore(batch);
                break;
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use l3i::Runtime;

    use crate::lua::{Options, runtime_with_capabilities};
    use crate::runtime::{
        Budget, CapabilitySet, ContextSpec, DomainSpec, Phase, RootAuthority, RuntimeFactory, ScriptDomain,
    };

    const GRANT: &str = "shared-buffer.create";

    struct Factory;

    impl RuntimeFactory for Factory {
        fn create(&self) -> l3i::Result<Runtime> {
            runtime_with_capabilities(Options::default(), &[GRANT.to_owned()])
        }
    }

    fn timer_failures(domain: &ScriptDomain) -> Option<(usize, usize)> {
        let report = domain.advance(Duration::from_millis(1)).unwrap();
        let phase = report.phases.iter().find(|phase| phase.phase == Phase::Timer)?;
        Some((phase.ran, phase.failures.len()))
    }

    /// Only the crate can redeclare a context mid-generation, so this is a unit test.
    #[test]
    fn a_queued_timer_runs_under_what_its_context_is_now() {
        let grant = RootAuthority::new().grant(CapabilitySet::from_names(&[GRANT.to_owned()]).unwrap());
        let domain = ScriptDomain::create(Arc::new(Factory), DomainSpec::new("reconfigured", grant)).unwrap();
        let schedule = [Phase::Startup, Phase::Timer];
        domain
            .register_context(ContextSpec::new("module").schedule(schedule))
            .unwrap();
        domain
            .load_source(
                "=reconfigured",
                r"
            local shared = require('@jess/sharedBuffer')
            local timer = require('@jess/timer')
            local firings = 0
            require('@jess/context').bind('module', { startup = function()
                timer.every(1, function()
                    firings += 1
                    shared.create(8)
                    -- The third firing has the budget. The watchdog polls every 64 safepoints, so a
                    -- short spin that one scheduler stall carries past its end can finish unpolled:
                    -- this one only ends by being stopped. The limit stops a hang if it never is.
                    local spin = if firings == 3 then 60 else 0.04
                    local started = os.clock()
                    repeat until os.clock() - started > spin
                end)
            end })
            ",
            )
            .unwrap();
        domain.start().unwrap();
        assert_eq!(timer_failures(&domain), Some((1, 0)));

        domain
            .core
            .declare_context(
                ContextSpec::new("module")
                    .schedule(schedule)
                    .capabilities(CapabilitySet::empty()),
            )
            .unwrap();
        assert_eq!(
            timer_failures(&domain),
            Some((1, 1)),
            "the narrowed grant reaches the queued timer"
        );

        domain
            .core
            .declare_context(
                ContextSpec::new("module")
                    .schedule(schedule)
                    .budget(Budget::time(Duration::from_millis(5))),
            )
            .unwrap();
        assert_eq!(
            timer_failures(&domain),
            Some((1, 1)),
            "the new budget reaches the queued timer"
        );

        domain
            .core
            .declare_context(ContextSpec::new("module").schedule(schedule))
            .unwrap();
        assert_eq!(timer_failures(&domain), Some((1, 0)), "and so does lifting both");

        domain
            .core
            .declare_context(ContextSpec::new("module").schedule([Phase::Startup]))
            .unwrap();
        assert_eq!(
            timer_failures(&domain),
            Some((0, 0)),
            "without the timer phase it is dropped unrun"
        );
        assert_eq!(timer_failures(&domain), None, "and stays gone");
    }
}
