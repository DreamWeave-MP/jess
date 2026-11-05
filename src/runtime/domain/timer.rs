//! Domain timer ownership and dispatch through the same call path as every other phase.
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use l3i::value::Function;

use super::{ContextSpec, Dispatch, DomainCore, DomainState, Phase, PhaseReport, RuntimeError, ScriptDomain};
use crate::runtime::capability::current_authority;

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
        let target = Dispatch {
            context,
            handler,
            source: Some(Rc::from(source)),
            event: None,
            stats: Rc::clone(&record.stats),
            capabilities: Arc::clone(authority.capabilities()),
            budget: record.budget,
        };
        self.timers
            .borrow_mut()
            .insert(delay, repeating, target)
            .map_err(l3i::Error::runtime)
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
        for id in batch {
            let target = self.core.timers.borrow().get(id).cloned();
            let Some(target) = target else { continue };
            if !self.core.contexts.borrow().iter().any(|record| record.id == target.context) {
                self.core.cancel_timer(id);
                continue;
            }
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
        }
        Ok(report)
    }
}
