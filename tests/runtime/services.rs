//! Services: native state a domain carries by type, reached from bindings through the running
//! code's authority, shared with a worker only when its spec says so.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use jess::runtime::{ContextSpec, DiagnosticKind, Message, Phase, RuntimeError, ScriptDomain, WorkerSpec};

use super::support::{self, Counter};

const WAIT: Option<Duration> = Some(Duration::from_secs(20));

const MAIN: &str = r"
local context = require('@jess/context')
local service = require('@test/service')
local journal = require('@test/journal')

journal.write(`load {tostring(service.bump())}`)

context.bind('main', {
  update = function()
    journal.write(`update {tostring(service.bump())}`)
  end,
})
";

const WORKER: &str = r"
local service = require('@test/service')

return function()
  return service.bump()
end
";

fn domain_with_counter(name: &str) -> (support::Harness, Arc<Counter>) {
    let counter = Arc::new(Counter::default());
    let (spec, diagnostics, profile) = support::spec(name, &[]);
    let domain = ScriptDomain::create(support::factory(), spec.service(Arc::clone(&counter))).unwrap();
    (
        support::Harness {
            domain,
            diagnostics,
            profile,
        },
        counter,
    )
}

#[test]
fn a_domain_carries_its_services_for_load_handlers_and_restarts_alike() {
    let (harness, counter) = domain_with_counter("served");
    let domain = &harness.domain;
    assert_eq!(domain.services().names(), [std::any::type_name::<Counter>()]);
    assert!(Arc::ptr_eq(&domain.service::<Counter>().unwrap(), &counter));
    assert!(domain.service::<String>().is_none());
    assert!(
        jess::runtime::service::<Counter>().is_none(),
        "host code holds no authority, so it sees no service through the thread"
    );

    domain
        .register_context(ContextSpec::new("main").schedule([Phase::Update]))
        .unwrap();
    domain.load_source("=main", MAIN).unwrap();
    domain.start().unwrap();
    domain.run_phase(Phase::Update, &Message::Nil).unwrap();
    assert_eq!(support::journal(), ["load 1", "update 2"]);

    // Host-level evaluation runs under no authority: the binding answers nil.
    assert!(support::eval::<Option<f64>>(domain, "return require('@test/service').bump()").is_none());

    // The service is the host's, not the generation's: it survives a restart with its state.
    domain
        .restart(|runtime| jess::runtime::chunk::load(runtime, "=main", MAIN)?.invoke::<(), ()>(&runtime.stack(), ()))
        .unwrap();
    domain.run_phase(Phase::Update, &Message::Nil).unwrap();
    assert_eq!(support::journal(), ["load 3", "update 4"]);
    assert_eq!(counter.0.load(Ordering::Relaxed), 4);
    assert!(Arc::ptr_eq(&domain.service::<Counter>().unwrap(), &counter));
}

#[test]
fn a_worker_sees_only_the_services_its_spec_shares() {
    let (harness, counter) = domain_with_counter("sharing");
    let domain = &harness.domain;
    let apart = domain.spawn_worker(WorkerSpec::new("apart", WORKER)).unwrap();
    let shared = domain
        .spawn_worker(WorkerSpec::new("shared", WORKER).share::<Counter>())
        .unwrap();
    for worker in [apart, shared] {
        domain.send_to_worker(worker, Message::Nil).unwrap();
    }
    assert_eq!(
        domain.receive_from_worker(apart, WAIT).unwrap(),
        None,
        "a nil reply is no reply: the worker found no service"
    );
    assert_eq!(domain.receive_from_worker(shared, WAIT).unwrap(), Some(Message::Number(1.0)));
    assert_eq!(counter.0.load(Ordering::Relaxed), 1);

    // Sharing a type the domain carries no service of is refused at spawn, and diagnosed.
    let refused = domain
        .spawn_worker(WorkerSpec::new("wants-a-string", WORKER).share::<String>())
        .unwrap_err();
    match &refused {
        RuntimeError::UnknownService { domain: id, service } => {
            assert_eq!(*id, domain.id());
            assert_eq!(*service, std::any::type_name::<String>());
        }
        other => panic!("expected UnknownService, got {other:?}"),
    }
    assert_eq!(
        refused.to_string(),
        format!(
            "{} carries no service of type alloc::string::String for a worker to share",
            domain.id()
        )
    );
    let refusals = harness.diagnostics.of_kind(DiagnosticKind::SpawnRefused);
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].message, refused.to_string());
    assert_eq!(domain.workers().len(), 2);
    assert_eq!(domain.stop().unwrap().workers_stopped, 2);
}
