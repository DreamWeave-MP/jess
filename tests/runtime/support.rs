//! What the runtime tests share: a plan with the runtime's Luau modules and a few test services,
//! and the factory that builds runtimes from it on whatever thread asks.
//!
//! The test services are what a real host's would be, in miniature:
//! - `@test/echo`: a native service behind the `test.echo` capability.
//! - `@test/scratch`: a mutable module table, to show module state is per runtime.
//! - `@test/journal`: appends a line to the calling thread's journal, the way a test reads what
//!   handlers did after their runtime is gone.
//! - `@test/rendezvous`: blocks until every party has arrived, to prove two states run at once.
//! - `@test/service`: bumps the `Counter` service the running code's domain carries, the way a
//!   real binding finds its domain's input runtime or localization registry.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use jess::runtime::l3i;
use jess::runtime::l3i::extension::{Extension, ExtensionDescriptor, RuntimePlan, RuntimePolicy};
use jess::runtime::luau::RuntimeExtension;
use std::sync::atomic::{AtomicU64, Ordering};

use jess::runtime::{
    CapabilitySet, CollectedDiagnostics, CollectedProfile, DomainSpec, FromPlan, RootAuthority, RuntimeFactory,
    ScriptDomain, require_capability,
};

/// The capability `@test/echo` checks.
pub const TEST_ECHO: &str = "test.echo";

/// A service a host attaches to a domain: a counter every binding under that domain shares.
#[derive(Debug, Default)]
pub struct Counter(pub AtomicU64);

/// A meeting point for `parties` callers: each blocks until all have arrived, or gives up.
pub struct Rendezvous {
    parties: usize,
    arrived: Mutex<usize>,
    everyone: Condvar,
}

impl Rendezvous {
    pub fn new(parties: usize) -> Arc<Rendezvous> {
        Arc::new(Rendezvous {
            parties,
            arrived: Mutex::new(0),
            everyone: Condvar::new(),
        })
    }

    /// True when every party arrived while this caller was still waiting.
    // The guard goes into the condvar wait and comes back; there is nothing to drop earlier.
    #[allow(clippy::significant_drop_tightening)]
    pub fn meet(&self) -> bool {
        let mut arrived = self.arrived.lock().unwrap();
        *arrived += 1;
        self.everyone.notify_all();
        let (guard, waited) = self
            .everyone
            .wait_timeout_while(arrived, Duration::from_secs(20), |arrived| *arrived < self.parties)
            .unwrap();
        drop(guard);
        !waited.timed_out()
    }
}

thread_local! {
    static JOURNAL: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PLAN: RefCell<Option<Rc<RuntimePlan>>> = const { RefCell::new(None) };
}

/// Takes what `@test/journal` wrote on this thread so far.
pub fn journal() -> Vec<String> {
    JOURNAL.with_borrow_mut(std::mem::take)
}

struct TestExtension {
    rendezvous: Arc<Rendezvous>,
}

impl Extension for TestExtension {
    fn id(&self) -> &'static str {
        "test"
    }

    fn describe(&self, d: &mut ExtensionDescriptor) -> l3i::Result<()> {
        d.module("@test/echo")
            .function("echo", |value: f64| -> l3i::Result<f64> {
                require_capability(TEST_ECHO)?;
                Ok(value)
            })
            .signature("(value: number) -> number");
        d.module("@test/scratch").mutable();
        d.module("@test/journal")
            .function("write", |line: String| JOURNAL.with_borrow_mut(|journal| journal.push(line)))
            .signature("(line: string) -> ()");
        let rendezvous = Arc::clone(&self.rendezvous);
        d.module("@test/rendezvous")
            .function("meet", move || rendezvous.meet())
            .signature("() -> boolean");
        d.module("@test/service")
            .function("bump", || -> Option<f64> {
                let counter = jess::runtime::service::<Counter>()?;
                #[allow(clippy::cast_precision_loss)] // A test counter.
                Some((counter.0.fetch_add(1, Ordering::Relaxed) + 1) as f64)
            })
            .signature("() -> number?");
        Ok(())
    }
}

/// The test plan: the runtime's own extension and the test services.
pub fn plan_with(rendezvous: &Arc<Rendezvous>) -> l3i::Result<Rc<RuntimePlan>> {
    RuntimePlan::builder()
        .policy(RuntimePolicy::new().debug_root("test"))
        .extension(RuntimeExtension)
        .extension(TestExtension {
            rendezvous: Arc::clone(rendezvous),
        })
        .finalize()
}

/// The test plan, made once per thread: what `jess::lua::plan` does for jess's.
pub fn plan() -> l3i::Result<Rc<RuntimePlan>> {
    if let Some(plan) = PLAN.with_borrow(Clone::clone) {
        return Ok(plan);
    }
    let plan = plan_with(&Rendezvous::new(1))?;
    PLAN.with_borrow_mut(|cached| *cached = Some(Rc::clone(&plan)));
    Ok(plan)
}

/// A factory over the per-thread test plan.
pub fn factory() -> Arc<dyn RuntimeFactory> {
    Arc::new(FromPlan(plan))
}

/// A factory whose runtimes share `rendezvous`.
pub fn factory_with(rendezvous: Arc<Rendezvous>) -> Arc<dyn RuntimeFactory> {
    Arc::new(FromPlan(move || plan_with(&rendezvous)))
}

/// A domain and the sinks its diagnostics and profile events collect in.
pub struct Harness {
    pub domain: ScriptDomain,
    pub diagnostics: Arc<CollectedDiagnostics>,
    pub profile: Arc<CollectedProfile>,
}

/// A spec named `name` holding `capabilities` from the root, with collecting sinks.
pub fn spec(name: &str, capabilities: &[&str]) -> (DomainSpec, Arc<CollectedDiagnostics>, Arc<CollectedProfile>) {
    let diagnostics = Arc::new(CollectedDiagnostics::new());
    let profile = Arc::new(CollectedProfile::new());
    let grant = RootAuthority::new().grant(CapabilitySet::from_names(capabilities).unwrap());
    let spec = DomainSpec::new(name, grant)
        .diagnostics(diagnostics.clone())
        .profiler(profile.clone());
    (spec, diagnostics, profile)
}

/// A created domain named `name` holding `capabilities`, from the shared test factory.
pub fn domain(name: &str, capabilities: &[&str]) -> Harness {
    domain_from(factory(), name, capabilities)
}

pub fn domain_from(factory: Arc<dyn RuntimeFactory>, name: &str, capabilities: &[&str]) -> Harness {
    let (spec, diagnostics, profile) = spec(name, capabilities);
    let domain = ScriptDomain::create(factory, spec).unwrap();
    Harness {
        domain,
        diagnostics,
        profile,
    }
}

/// Evaluates `source` in the domain's primary runtime at host level.
pub fn eval<R: l3i::call::CallResults>(domain: &ScriptDomain, source: &str) -> R {
    domain
        .with_runtime(|runtime| runtime.eval::<R>(source))
        .expect("the domain has a runtime")
        .unwrap_or_else(|error| panic!("{error}"))
}

/// True once no thread named for `worker` is left in this process. A joined thread can linger
/// in `/proc` for a moment after `join` returns (the kernel wakes the joiner before it unlinks
/// the task), so this waits a little for it to go rather than reading once.
#[cfg(target_os = "linux")]
pub fn lane_gone(worker: jess::runtime::WorkerId) -> bool {
    // A thread's `comm` holds the first 15 bytes of its name.
    let name = format!("jess-{worker}");
    let name = &name[..name.len().min(15)];
    let alive = || {
        std::fs::read_dir("/proc/self/task")
            .unwrap()
            .flatten()
            .any(|task| std::fs::read_to_string(task.path().join("comm")).is_ok_and(|comm| comm.trim_end() == name))
    };
    for _ in 0..500 {
        if !alive() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}
