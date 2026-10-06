//! The integration fixture: two domains in one process from one plan, walked through everything
//! the runtime promises. Headless; this is the definition of success for the crate.
//!
//! ```text
//! process
//! ├── Domain A: test.echo, worker.spawn
//! │   ├── context main: startup / update / event / onDemand / shutdown
//! │   └── worker a1
//! └── Domain B: nothing
//!     └── context main: onDemand
//! ```

use std::time::Duration;

use jess::runtime::{
    ContextSpec, DiagnosticKind, DomainState, EventTarget, Generation, Message, Phase, ProfileEvent, RuntimeError,
    WORKER_SPAWN, WorkerState,
};

use crate::support::{self, TEST_ECHO};

const DOMAIN_A: &str = r"
local context = require('@jess/context')
local worker = require('@jess/worker')
local echo = require('@test/echo')
local journal = require('@test/journal')
local scratch = require('@test/scratch')

marker = 'A'
hostOnly = true
scratch.tag = 'A'

local decoder

context.bind('main', {
  startup = function()
    journal.write('a startup')
  end,

  update = function(deltaSeconds)
    journal.write(`a update {deltaSeconds}`)
  end,

  event = function(payload)
    journal.write(`a event {payload.kind}`)
  end,

  onDemand = function(payload)
    if payload.kind == 'echo' then
      journal.write(`a echo {echo.echo(payload.value)}`)
    elseif payload.kind == 'spawn' then
      decoder = worker.spawn {
        name = 'a1',
        capabilities = {},
        source = payload.source,
      }
      journal.write(`a spawned {tostring(decoder)}`)
    elseif payload.kind == 'ask' then
      decoder:send { kind = 'sum', values = { 1, 2, 3 }, bytes = buffer.fromstring('abc') }
      local reply = decoder:receive(20)
      journal.write(`a sum {reply.sum} bytes {buffer.tostring(reply.bytes)} marker {reply.marker} sawHost {reply.sawHost}`)
    elseif payload.kind == 'bad' then
      -- A function can't cross a worker boundary: send throws, naming where it was.
      local ok, problem = pcall(function()
        decoder:send { items = { 1, { callback = function() end } } }
      end)
      journal.write(`a bad {ok} {problem}`)
    end
  end,

  shutdown = function()
    journal.write('a shutdown')
  end,
})
";

const WORKER_A1: &str = r"
local total = 0
marker = 'worker'

return function(message)
  for _, value in message.values do
    total += value
  end

  return { sum = total, bytes = message.bytes, marker = marker, sawHost = hostOnly ~= nil }
end
";

const DOMAIN_A_GENERATION_2: &str = r"
local context = require('@jess/context')
local journal = require('@test/journal')

context.bind('main', {
  event = function(payload)
    journal.write(`a2 event {payload.kind}`)
  end,

  shutdown = function()
    journal.write('a2 shutdown')
  end,
})
";

const DOMAIN_B: &str = r"
local context = require('@jess/context')
local worker = require('@jess/worker')
local echo = require('@test/echo')
local journal = require('@test/journal')
local scratch = require('@test/scratch')

journal.write(`b sees marker {tostring(marker)} scratch {tostring(scratch.tag)}`)

context.bind('main', {
  onDemand = function(payload)
    if payload.kind == 'echo' then
      journal.write(`b echo {echo.echo(payload.value)}`)
    elseif payload.kind == 'spawn' then
      worker.spawn { source = 'return function() end' }
    elseif payload.kind == 'alive' then
      journal.write('b alive')
    end
  end,
})
";

fn kind(kind: &str) -> Message {
    Message::map([("kind", Message::text(kind))])
}

#[test]
#[allow(clippy::too_many_lines)] // One walk through the whole contract, in the order it is promised.
fn two_domains_one_plan() {
    // 1. Create A and B from the same plan.
    let a = support::domain("a", &[TEST_ECHO, WORKER_SPAWN]);
    let b = support::domain("b", &[]);
    assert_ne!(a.domain.id(), b.domain.id());
    assert_eq!(
        (a.domain.state(), b.domain.state()),
        (DomainState::Created, DomainState::Created)
    );
    assert_eq!(a.domain.generation(), Generation::FIRST);
    let shared_plan = a.domain.with_runtime(|left| {
        b.domain
            .with_runtime(|right| std::rc::Rc::ptr_eq(&left.plan().unwrap(), &right.plan().unwrap()))
            .unwrap()
    });
    assert_eq!(shared_plan, Some(true), "both runtimes come from one plan");

    // 3–5. A holds test.echo; B doesn't. Both declare a context; A's runs the generic phases.
    let phases = [Phase::Startup, Phase::Update, Phase::Event, Phase::OnDemand, Phase::Shutdown];
    let main_a = a.domain.register_context(ContextSpec::new("main").schedule(phases)).unwrap();
    let main_b = b
        .domain
        .register_context(ContextSpec::new("main").schedule([Phase::OnDemand]))
        .unwrap();
    assert_ne!(main_a, main_b);
    a.domain.load_source("=domain-a", DOMAIN_A).unwrap();
    b.domain.load_source("=domain-b", DOMAIN_B).unwrap();
    assert_eq!((a.domain.state(), b.domain.state()), (DomainState::Ready, DomainState::Ready));

    // 2. Globals and module state are isolated: B saw neither A's global nor A's module write.
    assert_eq!(support::journal(), ["b sees marker nil scratch nil"]);
    assert_eq!(support::eval::<String>(&a.domain, "return marker"), "A");
    assert_eq!(support::eval::<String>(&a.domain, "return require('@test/scratch').tag"), "A");

    // 6. Deterministic scheduler ticks.
    assert_eq!(a.domain.start().unwrap().ran, 1);
    b.domain.start().unwrap();
    for _ in 0..2 {
        let tick = a.domain.advance(Duration::from_millis(250)).unwrap();
        assert!(tick.failures().is_empty(), "{:?}", tick.failures());
    }
    assert_eq!(support::journal(), ["a startup", "a update 0.25", "a update 0.25"]);
    assert_eq!(a.domain.stats().updates, 2);

    // 3–4. The same native service from both: A succeeds, B is refused, structurally.
    let ask = Message::map([("kind", Message::text("echo")), ("value", Message::Number(7.0))]);
    assert_eq!(a.domain.run_phase(Phase::OnDemand, &ask).unwrap().failures, []);
    let refused = b.domain.run_phase(Phase::OnDemand, &ask).unwrap();
    assert_eq!(support::journal(), ["a echo 7"]);
    let [
        RuntimeError::Script {
            attribution,
            phase,
            message,
        },
    ] = refused.failures.as_slice()
    else {
        panic!("B's echo fails as one script error, got {:?}", refused.failures);
    };
    assert_eq!(
        (attribution.domain, attribution.context, *phase),
        (b.domain.id(), Some(main_b), Some(Phase::OnDemand))
    );
    assert!(message.contains("capability 'test.echo' denied"), "{message}");
    let denials = b.diagnostics.of_kind(DiagnosticKind::CapabilityDenied);
    assert_eq!(denials.len(), 1);
    assert_eq!(denials[0].attribution, *attribution);
    assert_eq!(denials[0].capability.map(jess::runtime::CapabilityId::name), Some(TEST_ECHO));
    assert_eq!(
        (a.domain.stats().capability_denials, b.domain.stats().capability_denials),
        (0, 1)
    );
    assert!(a.diagnostics.entries().is_empty(), "{:?}", a.diagnostics.entries());

    // B can't spawn a worker either: it holds no worker.spawn.
    let refused = b.domain.run_phase(Phase::OnDemand, &kind("spawn")).unwrap();
    assert!(
        refused.failures[0].to_string().contains("capability 'worker.spawn' denied"),
        "{:?}",
        refused.failures
    );
    assert_eq!(b.domain.workers(), []);

    // 7–9. A spawns worker a1, sends a transferable message, and gets the result back.
    let spawn = Message::map([("kind", Message::text("spawn")), ("source", Message::text(WORKER_A1))]);
    assert_eq!(a.domain.run_phase(Phase::OnDemand, &spawn).unwrap().failures, []);
    let [worker] = a.domain.workers().try_into().expect("A owns one worker");
    assert_eq!(worker.name, "a1");
    assert_eq!(a.domain.run_phase(Phase::OnDemand, &kind("ask")).unwrap().failures, []);
    assert_eq!(
        support::journal(),
        [
            format!("a spawned Worker({})", worker.id),
            "a sum 6 bytes abc marker worker sawHost false".to_owned()
        ],
        "only the message crossed: the worker has its own globals and none of A's"
    );
    assert_eq!(
        support::eval::<String>(&a.domain, "return marker"),
        "A",
        "the worker's global write stayed in the worker"
    );

    // 10. An unsupported transfer is an error naming the path, and a diagnostic.
    assert_eq!(a.domain.run_phase(Phase::OnDemand, &kind("bad")).unwrap().failures, []);
    let [bad] = support::journal().try_into().expect("one line");
    assert!(bad.starts_with("a bad false "), "{bad}");
    assert!(
        bad.contains("message.items[2].callback: a function can't cross a worker or event boundary"),
        "{bad}"
    );
    let unsupported = a.diagnostics.of_kind(DiagnosticKind::TransferUnsupported);
    assert_eq!(unsupported.len(), 1);
    assert_eq!(unsupported[0].attribution.context, Some(main_a));
    assert!(
        unsupported[0]
            .source
            .as_deref()
            .is_some_and(|source| source.starts_with("domain-a:")),
        "{unsupported:?}"
    );

    // 11. Queue an event for generation 1, and own a native resource in it.
    let mailbox = a.domain.mailbox().unwrap();
    mailbox.post(EventTarget::Context(main_a), kind("stale")).unwrap();
    let released = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = std::rc::Rc::clone(&released);
    a.domain.own("test-resource", move || counter.set(counter.get() + 1));
    assert_eq!(a.domain.owned_resources().len(), 1);

    // 12. Restart A as generation 2.
    let generation = a
        .domain
        .restart(|runtime| {
            jess::runtime::chunk::load(runtime, "=domain-a2", DOMAIN_A_GENERATION_2)?.invoke::<(), ()>(&runtime.stack(), ())
        })
        .unwrap();
    assert_eq!(generation, Generation::FIRST.next());
    assert_eq!(a.domain.state(), DomainState::Running);
    assert_eq!(
        support::journal(),
        ["a shutdown"],
        "the old generation shut down; the new one has no startup handler"
    );
    assert!(a.domain.workers().is_empty(), "the old generation's worker is gone");
    assert_eq!(released.get(), 1, "and so is its resource");
    #[cfg(target_os = "linux")]
    assert!(support::lane_gone(worker.id));
    assert!(support::eval::<bool>(&a.domain, "return marker == nil"), "a fresh runtime");

    // 13. The stale event never reaches the new code; a fresh one does.
    let drained = a.domain.drain_events().unwrap();
    assert_eq!((drained.delivered, drained.stale), (0, 1));
    assert_eq!(support::journal(), Vec::<String>::new());
    assert_eq!(a.domain.stats().stale_discarded, 1);
    let stale = a.diagnostics.of_kind(DiagnosticKind::StaleGenerationDiscarded);
    assert_eq!(stale.len(), 1);
    assert_eq!(
        (stale[0].attribution.generation, stale[0].attribution.context),
        (generation, Some(main_a))
    );
    mailbox.post(EventTarget::Context(main_a), kind("fresh")).unwrap();
    assert_eq!(a.domain.drain_events().unwrap().delivered, 1);
    assert_eq!(support::journal(), ["a2 event fresh"]);

    // 14. Stop A cleanly.
    mailbox.post(EventTarget::All, kind("never")).unwrap();
    let teardown = a.domain.stop().unwrap();
    assert_eq!(a.domain.state(), DomainState::Stopped);
    assert_eq!(support::journal(), ["a2 shutdown"]);
    assert_eq!((teardown.events_discarded, teardown.contexts_removed), (1, 1));
    assert_eq!(teardown.shutdown_failures, []);
    assert!(
        mailbox.post(EventTarget::All, kind("late")).is_err(),
        "a stopped domain's mailbox is closed"
    );

    // 15. B is still alive.
    assert_eq!(b.domain.state(), DomainState::Running);
    assert_eq!(b.domain.run_phase(Phase::OnDemand, &kind("alive")).unwrap().failures, []);
    assert_eq!(support::journal(), ["b alive"]);

    // 16. Stop B.
    b.domain.stop().unwrap();

    // 17. Nothing either domain owned remains.
    for harness in [&a, &b] {
        assert_eq!(harness.domain.state(), DomainState::Stopped);
        assert_eq!(harness.domain.workers(), []);
        assert!(harness.domain.contexts().is_empty());
        assert_eq!(harness.domain.owned_resources(), []);
        assert!(harness.domain.with_runtime(|_| ()).is_none(), "the runtime is dropped");
    }

    // The profile attributes what ran to the domain, the context, and the worker.
    let events = a.profile.events();
    assert!(events.iter().all(|event| event.attribution().domain == a.domain.id()));
    assert!(matches!(events.first(), Some(ProfileEvent::DomainCreated { .. })));
    assert!(matches!(events.last(), Some(ProfileEvent::Teardown { .. })));
    let context_calls = events
        .iter()
        .filter(
            |event| matches!(event, ProfileEvent::ContextCall { attribution, .. } if attribution.context == Some(main_a)),
        )
        .count();
    assert_eq!(context_calls as u64, a.domain.stats().context_calls);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProfileEvent::WorkerSpawned { .. }) && event.worker() == Some(worker.id))
    );
    assert!(
        events.iter().any(
            |event| matches!(event, ProfileEvent::WorkerCall { failed: false, .. }) && event.worker() == Some(worker.id)
        )
    );
    assert_eq!(
        worker.state,
        WorkerState::Starting,
        "the snapshot was taken before the first reply"
    );
    let stats = a.domain.stats();
    assert_eq!(
        (stats.workers_spawned, stats.messages_to_workers, stats.messages_from_workers),
        (1, 1, 1)
    );
    assert!(stats.bytes_to_workers >= 3 && stats.bytes_from_workers >= 3);
}
