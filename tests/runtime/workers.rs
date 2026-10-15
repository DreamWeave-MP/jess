//! Workers: isolation, messages, buffers, failure, and the threads under them.

use std::sync::Arc;
use std::time::Duration;

use jess::runtime::l3i::Runtime;
use jess::runtime::{
    Budget, DiagnosticKind, Direction, Message, ProfileEvent, ResourceHandle, RuntimeError, WorkerFailurePolicy, WorkerSpec,
    WorkerStage, WorkerState,
};

use crate::support::{self, Rendezvous};

const WAIT: Option<Duration> = Some(Duration::from_secs(20));

/// A worker that answers `get` with its global `value` and sets it on anything else.
const REGISTER: &str = r"
local scratch = require('@test/scratch')

return function(message)
  if message == 'get' then
    return { value = value, scratch = scratch.tag }
  end

  value = message
  scratch.tag = message
  return 'set'
end
";

#[test]
fn workers_share_nothing_with_each_other_or_their_domain() {
    let harness = support::domain("isolated", &[]);
    let domain = &harness.domain;
    domain
        .with_runtime(|runtime| runtime.exec("value = 'domain' require('@test/scratch').tag = 'domain'"))
        .unwrap()
        .unwrap();
    let one = domain.spawn_worker(WorkerSpec::new("one", REGISTER)).unwrap();
    let two = domain.spawn_worker(WorkerSpec::new("two", REGISTER)).unwrap();
    assert_ne!(one, two);

    domain.send_to_worker(one, Message::text("from one")).unwrap();
    assert_eq!(domain.receive_from_worker(one, WAIT).unwrap(), Some(Message::text("set")));
    for (worker, expected) in [(one, Message::text("from one")), (two, Message::Nil)] {
        domain.send_to_worker(worker, Message::text("get")).unwrap();
        let reply = domain.receive_from_worker(worker, WAIT).unwrap().expect("a reply");
        assert_eq!(
            reply.get("value"),
            (expected != Message::Nil).then_some(&expected),
            "globals are per worker"
        );
        assert_eq!(
            reply.get("scratch"),
            (expected != Message::Nil).then_some(&expected),
            "module state is per worker"
        );
    }
    assert_eq!(support::eval::<String>(domain, "return value"), "domain");
    assert_eq!(
        support::eval::<String>(domain, "return require('@test/scratch').tag"),
        "domain"
    );

    // Closing one leaves the other running; joining reclaims the lane.
    assert_eq!(domain.join_worker(one, WAIT).unwrap(), WorkerState::Stopped);
    #[cfg(target_os = "linux")]
    assert!(support::lane_gone(one));
    let error = domain.send_to_worker(one, Message::Nil).unwrap_err();
    assert!(
        error.to_string().contains("the worker is stopped; it takes no more messages"),
        "{error}"
    );
    domain.send_to_worker(two, Message::text("get")).unwrap();
    assert!(domain.receive_from_worker(two, WAIT).unwrap().is_some());
    let states: Vec<WorkerState> = domain.workers().iter().map(|worker| worker.state).collect();
    assert_eq!(states, [WorkerState::Stopped, WorkerState::Running]);
    assert_eq!(domain.stop().unwrap().workers_stopped, 2);
}

#[test]
fn every_transferable_value_crosses_and_nothing_else_does() {
    let harness = support::domain("transfer", &[]);
    let domain = &harness.domain;
    let echo = domain
        .spawn_worker(WorkerSpec::new("echo", "return function(message) return { message } end"))
        .unwrap();
    let megabyte: Arc<[u8]> = (0..1_048_576u32)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<u8>>()
        .into();
    let values = [
        Message::Boolean(false),
        Message::Number(-0.5),
        Message::Integer(i64::MIN),
        Message::String(vec![0, 159, 146, 150, 255]),
        Message::Vector([1.0, -2.0, 3.5]),
        Message::Bytes(Arc::clone(&megabyte)),
        Message::List(vec![
            Message::Number(1.0),
            Message::text("two"),
            Message::List(vec![Message::Boolean(true)]),
        ]),
        Message::map([("name", Message::text("Fargoth")), ("level", Message::Number(3.0))]),
        Message::Handle(ResourceHandle {
            kind: "texture".to_owned(),
            id: u64::MAX,
        }),
    ];
    for value in &values {
        domain.send_to_worker(echo, value.clone()).unwrap();
        assert_eq!(
            domain.receive_from_worker(echo, WAIT).unwrap(),
            Some(Message::List(vec![value.clone()]))
        );
    }

    // The megabyte went as bytes, not as a table of numbers: the counters saw it once each way.
    let megabyte_events: Vec<Direction> = harness
        .profile
        .events()
        .iter()
        .filter_map(|event| match event {
            ProfileEvent::WorkerMessage { direction, bytes, .. } if *bytes == megabyte.len() => Some(*direction),
            _ => None,
        })
        .collect();
    assert_eq!(megabyte_events, [Direction::ToWorker, Direction::FromWorker]);
    let stats = domain.stats();
    assert_eq!((stats.messages_to_workers, stats.messages_from_workers), (9, 9));
    assert!(stats.bytes_to_workers >= megabyte.len() as u64 && stats.bytes_to_workers < 2 * megabyte.len() as u64);

    // A reply that can't cross is the worker's failure, named by path.
    let bad = domain
        .spawn_worker(WorkerSpec::new("bad", "return function() return { f = print } end"))
        .unwrap();
    domain.send_to_worker(bad, Message::Nil).unwrap();
    let RuntimeError::Worker(error) = domain.receive_from_worker(bad, WAIT).unwrap_err() else {
        panic!("a worker error")
    };
    assert_eq!(error.stage, WorkerStage::Handle);
    assert_eq!(error.message, "reply.f: a function can't cross a worker or event boundary");
}

#[test]
fn a_worker_may_send_more_than_it_returns() {
    let harness = support::domain("chatty", &[]);
    let domain = &harness.domain;
    let source = r"
local worker = require('@jess/worker')

return function(count)
  for index = 1, count do
    worker.send({ progress = index })
  end

  return 'done'
end
";
    let chatty = domain.spawn_worker(WorkerSpec::new("chatty", source)).unwrap();
    domain.send_to_worker(chatty, Message::Number(3.0)).unwrap();
    let mut replies = Vec::new();
    while let Some(reply) = domain.receive_from_worker(chatty, WAIT).unwrap() {
        let done = reply == Message::text("done");
        replies.push(reply);
        if done {
            break;
        }
    }
    let progress = |index: f64| Message::map([("progress", Message::Number(index))]);
    assert_eq!(replies, [progress(1.0), progress(2.0), progress(3.0), Message::text("done")]);
    assert_eq!(
        domain.receive_from_worker(chatty, None).unwrap(),
        None,
        "nothing waiting, and no wait"
    );

    // Replies sent before a join are still there after it.
    domain.send_to_worker(chatty, Message::Number(1.0)).unwrap();
    assert_eq!(domain.join_worker(chatty, WAIT).unwrap(), WorkerState::Stopped);
    assert_eq!(domain.receive_from_worker(chatty, None).unwrap(), Some(progress(1.0)));
    assert_eq!(domain.receive_from_worker(chatty, None).unwrap(), Some(Message::text("done")));
    assert_eq!(domain.receive_from_worker(chatty, None).unwrap(), None);

    // A worker can't spawn, and the domain's own code can't use the worker's `send`.
    let nested = domain
        .spawn_worker(WorkerSpec::new(
            "nested",
            "return function() require('@jess/worker').spawn { source = '' } end",
        ))
        .unwrap();
    domain.send_to_worker(nested, Message::Nil).unwrap();
    let error = domain.receive_from_worker(nested, WAIT).unwrap_err().to_string();
    assert!(error.contains("worker.spawn is for a domain's own code"), "{error}");
    let error = domain
        .with_runtime(|runtime| runtime.exec("require('@jess/worker').send(1)").unwrap_err())
        .unwrap();
    assert!(
        error.to_string().contains("worker.send is for a worker's own code"),
        "{error}"
    );
}

#[test]
fn a_worker_failure_is_structured_attributed_and_survivable() {
    let harness = support::domain("failing", &[]);
    let domain = &harness.domain;
    let source = "return function(message) if message == 'boom' then error('worker broke') end return message end";

    // Stop, the default: the failure is reported once and the worker ends.
    let fragile = domain
        .spawn_worker(WorkerSpec::new("fragile", source).chunk_name("=fragile"))
        .unwrap();
    domain.send_to_worker(fragile, Message::text("boom")).unwrap();
    let RuntimeError::Worker(error) = domain.receive_from_worker(fragile, WAIT).unwrap_err() else {
        panic!("a worker error")
    };
    assert_eq!(
        (error.attribution.domain, error.attribution.worker),
        (domain.id(), Some(fragile))
    );
    assert_eq!(error.attribution.generation, domain.generation());
    assert_eq!(
        (error.module.as_str(), error.stage, error.message.as_str()),
        ("=fragile", WorkerStage::Handle, "fragile:1: worker broke")
    );
    assert_eq!(domain.join_worker(fragile, WAIT).unwrap(), WorkerState::Failed);
    assert!(domain.send_to_worker(fragile, Message::Nil).is_err());

    // Continue: the failure is reported and the worker handles the next message.
    let sturdy = WorkerSpec::new("sturdy", source).on_failure(WorkerFailurePolicy::Continue);
    let sturdy = domain.spawn_worker(sturdy).unwrap();
    domain.send_to_worker(sturdy, Message::text("boom")).unwrap();
    domain.send_to_worker(sturdy, Message::text("fine")).unwrap();
    assert!(matches!(
        domain.receive_from_worker(sturdy, WAIT),
        Err(RuntimeError::Worker(_))
    ));
    assert_eq!(domain.receive_from_worker(sturdy, WAIT).unwrap(), Some(Message::text("fine")));
    assert_eq!(domain.workers()[1].state, WorkerState::Running);

    // A module that doesn't load, and one that doesn't return a handler.
    let broken = domain
        .spawn_worker(WorkerSpec::new("broken", "local = 1").chunk_name("=broken"))
        .unwrap();
    let RuntimeError::Worker(error) = domain.receive_from_worker(broken, WAIT).unwrap_err() else {
        panic!("a worker error")
    };
    assert_eq!(error.stage, WorkerStage::Load);
    assert!(error.message.contains("Expected identifier"), "{}", error.message);
    let idle = domain.spawn_worker(WorkerSpec::new("idle", "return 42")).unwrap();
    let RuntimeError::Worker(error) = domain.receive_from_worker(idle, WAIT).unwrap_err() else {
        panic!("a worker error")
    };
    assert_eq!(
        error.message,
        "a worker module returns its message handler, a function, not number"
    );
    assert_eq!(domain.join_worker(idle, WAIT).unwrap(), WorkerState::Failed);

    // A worker that overruns its budget fails like any other; the domain is untouched.
    let spinner = WorkerSpec::new("spinner", "return function() while true do end end")
        .budget(Budget::time(Duration::from_millis(30)));
    let spinner = domain.spawn_worker(spinner).unwrap();
    domain.send_to_worker(spinner, Message::Nil).unwrap();
    let error = domain.receive_from_worker(spinner, WAIT).unwrap_err().to_string();
    assert!(error.contains("Lua execution time limit exceeded"), "{error}");

    let failures = harness.diagnostics.of_kind(DiagnosticKind::WorkerFailed);
    assert_eq!(failures.len(), 5);
    assert!(
        failures
            .iter()
            .all(|diagnostic| diagnostic.attribution.domain == domain.id() && diagnostic.attribution.worker.is_some())
    );
    assert_eq!(failures[0].source.as_deref(), Some("=fragile"));
    assert_eq!(domain.stats().worker_failures, 5);
    assert_eq!(
        domain.state(),
        jess::runtime::DomainState::Created,
        "a worker failing never fails its domain"
    );
    assert_eq!(domain.stop().unwrap().workers_stopped, 5);
}

#[test]
fn a_worker_holds_only_the_capabilities_it_was_given() {
    let harness = support::domain("granting", &[support::TEST_ECHO, "vfs.read"]);
    let domain = &harness.domain;
    let source = "local echo = require('@test/echo') return function(value) return echo.echo(value) end";
    let trusted = jess::runtime::CapabilitySet::from_names([support::TEST_ECHO]).unwrap();
    let trusted = domain
        .spawn_worker(WorkerSpec::new("trusted", source).capabilities(trusted))
        .unwrap();
    let untrusted = jess::runtime::CapabilitySet::from_names(["vfs.read"]).unwrap();
    let untrusted = domain
        .spawn_worker(WorkerSpec::new("untrusted", source).capabilities(untrusted))
        .unwrap();

    domain.send_to_worker(trusted, Message::Number(5.0)).unwrap();
    assert_eq!(domain.receive_from_worker(trusted, WAIT).unwrap(), Some(Message::Number(5.0)));
    domain.send_to_worker(untrusted, Message::Number(5.0)).unwrap();
    let error = domain.receive_from_worker(untrusted, WAIT).unwrap_err().to_string();
    assert!(error.contains("capability 'test.echo' denied"), "{error}");
    let [denial] = harness
        .diagnostics
        .of_kind(DiagnosticKind::CapabilityDenied)
        .try_into()
        .expect("one denial");
    assert_eq!(
        denial.attribution.worker,
        Some(untrusted),
        "the denial is the worker's, not the domain's"
    );
    assert_eq!(domain.stats().capability_denials, 1);
}

#[test]
fn two_workers_make_progress_at_the_same_time() {
    // Each handler blocks in native code until the other has arrived. If the two states could
    // not run at once, the first would wait out its timeout alone and both would answer false.
    let rendezvous = Rendezvous::new(2);
    let harness = support::domain_from(support::factory_with(rendezvous), "concurrent", &[]);
    let domain = &harness.domain;
    let source = "local rendezvous = require('@test/rendezvous') return function() return rendezvous.meet() end";
    let left = domain.spawn_worker(WorkerSpec::new("left", source)).unwrap();
    let right = domain.spawn_worker(WorkerSpec::new("right", source)).unwrap();
    domain.send_to_worker(left, Message::Nil).unwrap();
    domain.send_to_worker(right, Message::Nil).unwrap();
    assert_eq!(domain.receive_from_worker(left, WAIT).unwrap(), Some(Message::Boolean(true)));
    assert_eq!(domain.receive_from_worker(right, WAIT).unwrap(), Some(Message::Boolean(true)));
}

#[test]
fn a_runtime_lives_and_dies_on_one_thread_and_each_thread_can_have_its_own() {
    // l3i's `Runtime` is `!Send` (the crate docs hold the compile-fail proof), so the supported
    // shape is a runtime per thread, each from a plan made on that thread. Two threads, each
    // with a domain of its own, run while the main thread runs a third.
    let rendezvous = Rendezvous::new(3);
    let run = |name: &'static str, rendezvous: Arc<Rendezvous>| {
        move || {
            let harness = support::domain_from(support::factory_with(rendezvous), name, &[]);
            let met = support::eval::<bool>(&harness.domain, "return require('@test/rendezvous').meet()");
            harness.domain.stop().unwrap();
            (harness.domain.id(), met)
        }
    };
    let first = std::thread::spawn(run("first", Arc::clone(&rendezvous)));
    let second = std::thread::spawn(run("second", Arc::clone(&rendezvous)));
    let (main_id, main_met) = run("main", rendezvous)();
    let (first_id, first_met) = first.join().unwrap();
    let (second_id, second_met) = second.join().unwrap();
    assert!(
        main_met && first_met && second_met,
        "three runtimes on three threads ran at once"
    );
    assert!(
        main_id != first_id && first_id != second_id && main_id != second_id,
        "ids are unique across threads"
    );

    // A plan made on one thread builds any number of runtimes there.
    let plan = support::plan().unwrap();
    let (one, two) = (Runtime::from_plan(&plan).unwrap(), Runtime::from_plan(&plan).unwrap());
    one.exec("shared = true").unwrap();
    assert!(two.eval::<bool>("return shared == nil").unwrap());
}

#[test]
fn domains_and_workers_come_and_go_without_leaking() {
    const ROUNDS: usize = 6;
    const WORKERS: usize = 4;
    const MESSAGES: usize = 5;
    let source = "local count = 0 return function(message) count += 1 return { count = count, echo = message } end";
    let mut spawned = Vec::new();
    for round in 0..ROUNDS {
        let harness = support::domain("stress", &[]);
        let domain = &harness.domain;
        let workers: Vec<_> = (0..WORKERS)
            .map(|index| domain.spawn_worker(WorkerSpec::new(format!("w{index}"), source)).unwrap())
            .collect();
        for message in 0..MESSAGES {
            for worker in &workers {
                #[allow(clippy::cast_precision_loss)]
                domain
                    .send_to_worker(*worker, Message::Number((round * 100 + message) as f64))
                    .unwrap();
            }
        }
        for worker in &workers {
            for message in 0..MESSAGES {
                let reply = domain
                    .receive_from_worker(*worker, WAIT)
                    .unwrap()
                    .expect("a reply per message");
                #[allow(clippy::cast_precision_loss)]
                let expected = (
                    Message::Number((message + 1) as f64),
                    Message::Number((round * 100 + message) as f64),
                );
                assert_eq!(
                    (reply.get("count"), reply.get("echo")),
                    (Some(&expected.0), Some(&expected.1)),
                    "in order, per worker"
                );
            }
        }
        domain.own("round", || {});
        let teardown = domain.stop().unwrap();
        assert_eq!((teardown.workers_stopped, teardown.resources_released), (WORKERS, 1));
        assert!(domain.workers().is_empty() && domain.owned_resources().is_empty());
        let stats = domain.stats();
        let total = (WORKERS * MESSAGES) as u64;
        assert_eq!(
            (stats.workers_spawned, stats.messages_to_workers, stats.messages_from_workers),
            (WORKERS as u64, total, total)
        );
        assert!(
            harness.diagnostics.entries().is_empty(),
            "{:?}",
            harness.diagnostics.entries()
        );
        spawned.extend(workers);
    }
    assert_eq!(spawned.len(), ROUNDS * WORKERS);
    #[cfg(target_os = "linux")]
    for worker in spawned {
        assert!(support::lane_gone(worker), "{worker}'s lane outlived its domain");
    }
}

#[test]
fn a_worker_queue_is_bounded_and_says_when_it_is_full() {
    let rendezvous = Rendezvous::new(2);
    let harness = support::domain_from(support::factory_with(Arc::clone(&rendezvous)), "bounded", &[]);
    let domain = &harness.domain;
    let source = "local rendezvous = require('@test/rendezvous')\n\
                  return function(message) if message == 'block' then rendezvous.meet() end end";
    let worker = domain.spawn_worker(WorkerSpec::new("slow", source)).unwrap();
    domain.send_to_worker(worker, Message::text("block")).unwrap();

    // The handler is held on `block`, so the queue fills, and the next send says so.
    let mut accepted = 0;
    let full = loop {
        match domain.send_to_worker(worker, Message::Nil) {
            Ok(()) => accepted += 1,
            Err(error) => break error,
        }
        assert!(accepted <= jess::runtime::WORKER_QUEUE, "the queue grew past its bound");
    };
    assert!(full.to_string().contains("the worker's queue is full"), "{full}");
    assert!(accepted >= jess::runtime::WORKER_QUEUE - 1, "{accepted}");

    // Closing a full queue doesn't block the domain: the lane stops once it has caught up.
    domain.close_worker(worker).unwrap();
    assert!(rendezvous.meet());
    assert_eq!(domain.join_worker(worker, WAIT).unwrap(), WorkerState::Stopped);
}

#[test]
fn a_wait_on_a_worker_is_bounded_by_the_execution_time_limit() {
    let harness = support::domain("waiting", &[jess::runtime::WORKER_SPAWN]);
    let domain = &harness.domain;
    domain
        .register_context(
            jess::runtime::ContextSpec::new("main")
                .schedule([jess::runtime::Phase::OnDemand])
                .budget(Budget::time(Duration::from_millis(100))),
        )
        .unwrap();
    domain
        .load_source(
            "=waiting",
            r"
            local context = require('@jess/context')
            local worker = require('@jess/worker')
            local silent = worker.spawn { source = 'return function() end' }

            context.bind('main', {
              onDemand = function()
                silent:receive(1e300)
              end,
            })
            ",
        )
        .unwrap();
    domain.start().unwrap();
    let began = std::time::Instant::now();
    let report = domain.run_phase(jess::runtime::Phase::OnDemand, &Message::Nil).unwrap();
    assert!(began.elapsed() < Duration::from_secs(10), "the wait outlived the limit");
    let message = report.failures[0].to_string();
    assert!(message.contains("execution time limit"), "{message}");
    domain.stop().unwrap();
}
