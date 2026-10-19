//! Named events: what `@jess/events` queues, the scheduler delivers, and a host routes to another
//! domain. They share the domain's one queue with raw events, so emission order is delivery order,
//! but the two kinds never reach each other's handlers.

use std::time::Duration;

use jess::runtime::l3i::Runtime;
use jess::runtime::l3i::value::{Function, Table};
use jess::runtime::{ContextSpec, DiagnosticKind, EventLimits, EventTarget, Message, Phase, PostRefused, ScriptDomain};

use crate::support;

/// Loads `source`, whose chunk returns a table of functions, and binds each `name = function` as
/// the handler of the named event `name` in `context`, in the order `bindings` lists them.
fn load_handlers(domain: &ScriptDomain, source: &str, bindings: &[(&str, &str, &str)]) {
    let mut functions = Vec::new();
    domain
        .load(|runtime| {
            let chunk = jess::runtime::chunk::load(runtime, "=handlers", source)?;
            let table = chunk.invoke::<Table, ()>(&runtime.stack(), ())?;
            for (_, _, key) in bindings {
                functions.push(table.get::<Function>(&runtime.stack(), key)?);
            }
            Ok(())
        })
        .unwrap();
    for ((context, event, _), function) in bindings.iter().zip(functions) {
        domain.bind_event_handler(context, event, function).unwrap();
    }
}

const HANDLERS: &str = r"
local events = require('@jess/events')
local context = require('@jess/context')
local journal = require('@test/journal')

-- The phase handlers share names with events below; emitting those names must never run them.
context.bind('first', {
  update = function()
    journal.write('first update (phase)')
  end,
  startup = function()
    journal.write('first startup (phase)')
  end,
  event = function(payload)
    journal.write(`first raw {payload}`)
  end,
})

return {
  firstDied = function(payload)
    journal.write(`first died {payload.player}`)
    -- Deferred: delivered at the next drain, not inside this one.
    events.emit('later', { from = 'first' })
  end,
  secondDied = function(payload)
    journal.write(`second died {payload.player}`)
  end,
  secondFails = function()
    error('second fails on purpose')
  end,
  thirdDied = function(payload)
    journal.write(`third died {payload.player}`)
  end,
  later = function(payload)
    journal.write(`later from {payload.from}`)
  end,
  update = function()
    journal.write('named update')
  end,
  emitStartup = function()
    events.emit('startup')
    events.emit('update')
    events.emit('interfaceOverride', {})
  end,
}
";

#[test]
fn named_events_are_queued_and_delivered_in_order_apart_from_phases() {
    let harness = support::domain("named", &[]);
    let domain = &harness.domain;
    for (name, priority) in [("first", 0), ("second", -1), ("third", 0)] {
        domain
            .register_context(
                ContextSpec::new(name)
                    .schedule([Phase::Startup, Phase::Update, Phase::Event])
                    .priority(priority),
            )
            .unwrap();
    }
    load_handlers(
        domain,
        HANDLERS,
        &[
            ("first", "playerDied", "firstDied"),
            ("second", "playerDied", "secondDied"),
            ("third", "playerDied", "thirdDied"),
            ("second", "boom", "secondFails"),
            ("third", "boom", "thirdDied"),
            ("first", "later", "later"),
            ("third", "go", "emitStartup"),
            ("third", "update", "update"),
        ],
    );
    domain.start().unwrap();
    assert_eq!(support::journal(), ["first startup (phase)"]);
    assert_eq!(
        domain.context("third").unwrap().events,
        ["boom", "go", "playerDied", "update"]
    );

    // Emitting runs nothing; the drain runs every subscriber, by priority then registration.
    support::eval::<()>(domain, "require('@jess/events').emit('playerDied', { player = 'Caius' })");
    assert_eq!(support::journal(), Vec::<String>::new());
    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.unhandled), (3, 0));
    assert_eq!(
        support::journal(),
        ["second died Caius", "first died Caius", "third died Caius"]
    );

    // What a handler emitted while the queue drained waits for the next drain.
    let report = domain.drain_events().unwrap();
    assert_eq!(report.delivered, 1);
    assert_eq!(support::journal(), ["later from first"]);

    // A failing handler is attributed with its event and doesn't stop the next one.
    support::eval::<()>(domain, "require('@jess/events').emit('boom', { player = 'Fargoth' })");
    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.failures.len()), (2, 1));
    assert_eq!(support::journal(), ["third died Fargoth"]);
    let failed = harness.diagnostics.of_kind(DiagnosticKind::HandlerFailed);
    let failed = failed.last().unwrap();
    assert_eq!(failed.phase, Some(Phase::Event));
    assert_eq!(failed.event.as_deref(), Some("boom"));
    assert!(failed.message.contains("second fails on purpose"), "{failed}");
    assert!(failed.to_string().contains("during event 'boom'"), "{failed}");

    // An event nothing listens for is counted and dropped, once.
    support::eval::<()>(domain, "require('@jess/events').emit('nobody')");
    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.unhandled), (0, 1));
    assert_eq!(domain.drain_events().unwrap(), jess::runtime::EventReport::default());

    // Names that are also host phases reach only the handlers bound to the name: `startup` and
    // `interfaceOverride` have none, and `update` runs the named handler, not the phase's.
    support::eval::<()>(domain, "require('@jess/events').emit('go')");
    domain.drain_events().unwrap();
    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.unhandled), (1, 2));
    assert_eq!(support::journal(), ["named update"]);

    // A raw event goes to raw handlers only, and a named one never to them.
    domain
        .mailbox()
        .unwrap()
        .post(EventTarget::All, Message::text("raw"))
        .unwrap();
    domain.mailbox().unwrap().post_named("event", Message::text("named")).unwrap();
    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.unhandled), (1, 1));
    assert_eq!(support::journal(), ["first raw raw"]);

    // Each call counted itself to its context: second ran playerDied and boom, and boom raised.
    let second = domain.context("second").unwrap();
    assert_eq!((second.calls, second.failures), (2, 1));
    let third = domain.context("third").unwrap();
    assert_eq!((third.calls, third.failures), (4, 0));
    assert!(!third.time.is_zero());
}

#[test]
fn raw_and_named_events_share_one_fifo_queue() {
    let harness = support::domain("fifo", &[]);
    let domain = &harness.domain;
    domain
        .register_context(ContextSpec::new("first").schedule([Phase::Event]))
        .unwrap();
    domain
        .register_context(ContextSpec::new("second").schedule([Phase::Event]))
        .unwrap();
    load_handlers(
        domain,
        r"
        local context = require('@jess/context')
        local journal = require('@test/journal')
        context.bind('first', { event = function(payload) journal.write(`raw {payload}`) end })
        return { named = function(payload) journal.write(`named {payload}`) end }
        ",
        &[("second", "tick", "named")],
    );
    domain.start().unwrap();
    let mailbox = domain.mailbox().unwrap();
    mailbox.post(EventTarget::All, Message::text("1")).unwrap();
    support::eval::<()>(domain, "require('@jess/events').emit('tick', '2')");
    mailbox.post_named("tick", Message::text("3")).unwrap();
    mailbox.post(EventTarget::All, Message::text("4")).unwrap();
    domain.drain_events().unwrap();
    assert_eq!(support::journal(), ["raw 1", "named 2", "named 3", "raw 4"]);
}

#[test]
fn named_events_need_a_domain_a_name_and_a_transferable_payload() {
    let harness = support::domain("payloads", &["worker.spawn"]);
    let domain = &harness.domain;
    domain
        .register_context(ContextSpec::new("main").schedule([Phase::Event]))
        .unwrap();
    domain.load_source("=main", "return nil").unwrap();
    domain.start().unwrap();
    let refused = |source: &str| {
        domain
            .with_runtime(|runtime| runtime.eval::<()>(source))
            .unwrap()
            .unwrap_err()
            .to_string()
    };
    let function = refused("require('@jess/events').emit('hit', { player = { onDeath = print } })");
    assert!(
        function.contains("payload.player.onDeath: a function can't cross a worker or event boundary"),
        "{function}"
    );
    let cyclic = refused("local loop = {}; loop.self = loop; require('@jess/events').emit('hit', loop)");
    assert!(cyclic.contains("payload.self.self"), "{cyclic}");
    assert!(cyclic.contains("nested more than 32 deep"), "{cyclic}");
    assert_eq!(harness.diagnostics.of_kind(DiagnosticKind::TransferUnsupported).len(), 2);
    let unnamed = refused("require('@jess/events').emit('')");
    assert!(unnamed.contains("events.emit: an event needs a name"), "{unnamed}");
    assert_eq!(domain.drain_events().unwrap().unhandled, 0, "nothing refused was queued");

    // A runtime nobody owns, and a worker, have no domain to emit into.
    let detached = Runtime::from_plan(&support::plan().unwrap()).unwrap();
    let outside = detached
        .eval::<()>("require('@jess/events').emit('hit')")
        .unwrap_err()
        .to_string();
    assert!(outside.contains("events.emit: needs a ScriptDomain"), "{outside}");
    let worker = domain
        .spawn_worker(jess::runtime::WorkerSpec::new(
            "emitter",
            "return function() require('@jess/events').emit('hit') end",
        ))
        .unwrap();
    domain.send_to_worker(worker, Message::Nil).unwrap();
    let failed = domain
        .receive_from_worker(worker, Some(Duration::from_secs(20)))
        .unwrap_err()
        .to_string();
    assert!(failed.contains("events.emit is for a domain's own code"), "{failed}");
}

#[test]
fn a_named_event_queued_for_an_old_generation_is_discarded() {
    let harness = support::domain("stale", &[]);
    let domain = &harness.domain;
    domain
        .register_context(ContextSpec::new("main").schedule([Phase::Event]))
        .unwrap();
    load_handlers(
        domain,
        "return { hit = function() require('@test/journal').write('old hit') end }",
        &[("main", "hit", "hit")],
    );
    domain.start().unwrap();
    support::eval::<()>(domain, "require('@jess/events').emit('hit')");
    domain.restart(|_| Ok(())).unwrap();
    assert!(
        domain.context("main").unwrap().events.is_empty(),
        "handlers go with their generation"
    );
    let report = domain.drain_events().unwrap();
    assert_eq!((report.stale, report.delivered), (1, 0));
    let stale = harness.diagnostics.of_kind(DiagnosticKind::StaleGenerationDiscarded);
    assert!(
        stale[0].message.starts_with("the event 'hit' queued for gen#1"),
        "{}",
        stale[0]
    );
    assert_eq!(support::journal(), Vec::<String>::new());
}

#[test]
fn a_host_routes_named_events_to_another_domain_explicitly() {
    let sender = support::domain("sender", &[]);
    let receiver = support::domain("receiver", &[]);
    receiver
        .domain
        .register_context(ContextSpec::new("main").schedule([Phase::Event]))
        .unwrap();
    load_handlers(
        &receiver.domain,
        r"
        local journal = require('@test/journal')
        return {
          died = function(payload) journal.write(`receiver died {payload.player}`) end,
          other = function() journal.write('receiver other') end,
        }
        ",
        &[("main", "playerDied", "died"), ("main", "unrouted", "other")],
    );
    sender.domain.load_source("=sender", "return nil").unwrap();
    sender.domain.start().unwrap();
    receiver.domain.start().unwrap();

    // Nothing crosses until the host says so, and then only the named event.
    support::eval::<()>(
        &sender.domain,
        "require('@jess/events').emit('playerDied', { player = 'one' })",
    );
    assert_eq!(receiver.domain.drain_events().unwrap().delivered, 0);
    sender.domain.route("playerDied", receiver.domain.mailbox().unwrap()).unwrap();
    // Routing it again is the same route, not a second one.
    sender.domain.route("playerDied", receiver.domain.mailbox().unwrap()).unwrap();
    support::eval::<()>(
        &sender.domain,
        r"
        local events = require('@jess/events')
        events.emit('playerDied', { player = 'two' })
        events.emit('unrouted')
        ",
    );
    let report = receiver.domain.drain_events().unwrap();
    assert_eq!(report.delivered, 1);
    assert_eq!(support::journal(), ["receiver died two"]);
    // The sender's own queue still has both, for its own handlers (it has none).
    assert_eq!(sender.domain.drain_events().unwrap().unhandled, 3);

    // A host that sets its routes up again after a restart still delivers each event once.
    sender.domain.restart(|_| Ok(())).unwrap();
    sender.domain.route("playerDied", receiver.domain.mailbox().unwrap()).unwrap();
    support::eval::<()>(
        &sender.domain,
        "require('@jess/events').emit('playerDied', { player = 'three' })",
    );
    assert_eq!(receiver.domain.drain_events().unwrap().delivered, 1);
    assert_eq!(support::journal(), ["receiver died three"]);

    let to_itself = sender.domain.route("x", sender.domain.mailbox().unwrap()).unwrap_err();
    assert!(to_itself.to_string().contains("can't route 'x' to itself"), "{to_itself}");

    // A recipient that stopped is the host's to hear about, not the emitter's error.
    receiver.domain.stop().unwrap();
    support::eval::<()>(&sender.domain, "require('@jess/events').emit('playerDied', {})");
    let dropped = sender.diagnostics.of_kind(DiagnosticKind::TeardownDiscarded);
    assert!(dropped[0].message.contains("'playerDied' routed to"), "{}", dropped[0]);
}

#[test]
fn named_handlers_need_a_context_that_schedules_events() {
    let harness = support::domain("unscheduled", &[]);
    let domain = &harness.domain;
    domain
        .register_context(ContextSpec::new("main").schedule([Phase::Update]))
        .unwrap();
    let mut function = None;
    domain
        .load(|runtime| {
            function = Some(runtime.eval::<Function>("return function() end")?);
            Ok(())
        })
        .unwrap();
    let refused = domain.bind_event_handler("main", "hit", function.unwrap()).unwrap_err();
    assert!(refused.to_string().contains("does not schedule the event phase"), "{refused}");
}

/// A domain with `limits`, one context `main` scheduling events, and `source`'s chunk's
/// functions bound as named handlers per `bindings` (event, key).
fn limited(limits: EventLimits, source: &str, bindings: &[(&str, &str)]) -> support::Harness {
    let (spec, diagnostics, profile) = support::spec("limited", &[]);
    let domain = ScriptDomain::create(support::factory(), spec.event_limits(limits)).unwrap();
    domain
        .register_context(ContextSpec::new("main").schedule([Phase::Event]))
        .unwrap();
    let bindings = bindings.iter().map(|(event, key)| ("main", *event, *key)).collect::<Vec<_>>();
    load_handlers(&domain, source, &bindings);
    domain.start().unwrap();
    support::Harness {
        domain,
        diagnostics,
        profile,
    }
}

fn emit_error(domain: &ScriptDomain, source: &str) -> String {
    domain
        .with_runtime(|runtime| runtime.eval::<()>(source))
        .unwrap()
        .unwrap_err()
        .to_string()
}

#[test]
fn a_full_queue_refuses_named_events_observably_and_counts_raw_ones() {
    let limits = EventLimits {
        pending: 3,
        bytes: 1 << 20,
        per_drain: 2,
        ..EventLimits::default()
    };
    let harness = limited(
        limits,
        "return { hit = function(payload) require('@test/journal').write(`hit {payload}`) end }",
        &[("hit", "hit")],
    );
    let domain = &harness.domain;
    support::eval::<()>(domain, "for n = 1, 3 do require('@jess/events').emit('hit', n) end");
    let full = emit_error(domain, "require('@jess/events').emit('hit', 4)");
    assert!(
        full.contains("events.emit: 'hit' wasn't queued: ") && full.contains("event queue is full: 3 events are waiting"),
        "{full}"
    );
    let reported = harness.diagnostics.of_kind(DiagnosticKind::QueueFull);
    assert_eq!(reported.len(), 1);
    assert!(
        reported[0].message.starts_with("the event 'hit' wasn't queued"),
        "{}",
        reported[0]
    );
    let mailbox = domain.mailbox().unwrap();
    assert!(matches!(
        mailbox.post_named("hit", Message::Number(5.0)),
        Err(PostRefused::Full(_))
    ));
    // A host's raw post is counted, and never refused.
    mailbox.post(EventTarget::All, Message::Nil).unwrap();

    // A drain delivers two and leaves the rest, in order, for the next.
    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.pending), (2, 2));
    assert_eq!(support::journal(), ["hit 1", "hit 2"]);
    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.unhandled, report.pending), (1, 1, 0));
    assert_eq!(support::journal(), ["hit 3"]);
    support::eval::<()>(domain, "require('@jess/events').emit('hit', 6)");
    domain.drain_events().unwrap();
    assert_eq!(support::journal(), ["hit 6"], "room again once drained");
}

#[test]
fn a_payload_larger_than_the_queue_holds_is_refused() {
    // What one 'small' event named 'hit' holds: its payload and its name.
    let small = Message::text("small").queue_weight() + "hit".len();
    let limits = EventLimits {
        pending: 10,
        bytes: small + small / 2,
        per_drain: 10,
        ..EventLimits::default()
    };
    let harness = limited(limits, "return { hit = function() end }", &[("hit", "hit")]);
    let domain = &harness.domain;
    // A script's payload is read no further than the queue holds.
    let refused = emit_error(domain, "require('@jess/events').emit('hit', string.rep('x', 1000))");
    assert!(
        refused.contains(&format!(
            "payload: a value past the {} bytes a queue holds of one payload",
            limits.bytes
        )),
        "{refused}"
    );
    support::eval::<()>(domain, "require('@jess/events').emit('hit', 'small')");
    let crowded = emit_error(domain, "require('@jess/events').emit('hit', 'small')");
    assert!(
        crowded.contains(&format!(
            "{small} bytes of payload are waiting, and {small} more don't fit under {}",
            limits.bytes
        )),
        "{crowded}"
    );
    // A host's named post of a payload larger than the whole queue is refused for that.
    let Err(PostRefused::Full(full)) = domain.mailbox().unwrap().post_named("hit", Message::text(&"x".repeat(1000))) else {
        panic!("a payload larger than the queue is refused");
    };
    assert!(
        full.to_string()
            .contains(&format!("holds at most {} bytes of payload", limits.bytes)),
        "{full}"
    );
}

/// A handler that answers every event with two more would double the queue each drain; the
/// limits hold it to its size and each drain to its share, and the refusals are reported.
#[test]
fn an_event_storm_is_bounded_in_memory_and_in_drain_work() {
    let limits = EventLimits {
        pending: 50,
        bytes: 1 << 20,
        per_drain: 10,
        ..EventLimits::default()
    };
    let harness = limited(
        limits,
        r"
        local events = require('@jess/events')
        return {
          storm = function()
            events.emit('storm')
            events.emit('storm')
          end,
        }
        ",
        &[("storm", "storm")],
    );
    let domain = &harness.domain;
    support::eval::<()>(domain, "require('@jess/events').emit('storm')");
    let mut refused = 0;
    for _ in 0..40 {
        let report = domain.drain_events().unwrap();
        assert!(report.delivered <= 10, "{report:?}");
        assert!(report.pending <= 50, "{report:?}");
        refused += report.failures.len();
    }
    assert!(refused > 0, "the storm hit the limit");
    assert_eq!(harness.diagnostics.of_kind(DiagnosticKind::QueueFull).len(), refused);
    assert!(
        harness.diagnostics.of_kind(DiagnosticKind::HandlerFailed)[0]
            .message
            .contains("events.emit: 'storm' wasn't queued"),
    );
}

/// A named event's handlers share one payload, built once and frozen through: none of them can
/// change what the next one reads.
#[test]
fn a_named_event_s_handlers_share_one_frozen_payload() {
    let harness = support::domain("shared", &[]);
    let domain = &harness.domain;
    for name in ["first", "second"] {
        domain
            .register_context(ContextSpec::new(name).schedule([Phase::Event]))
            .unwrap();
    }
    load_handlers(
        domain,
        r"
        local journal = require('@test/journal')
        local seen
        return {
          first = function(payload)
            seen = payload
            payload.nested.n = 2
          end,
          second = function(payload)
            journal.write(`same {tostring(rawequal(payload, seen))} n {payload.nested.n}`)
          end,
        }
        ",
        &[("first", "hit", "first"), ("second", "hit", "second")],
    );
    domain.start().unwrap();
    support::eval::<()>(domain, "require('@jess/events').emit('hit', { nested = { n = 1 } })");
    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.failures.len()), (2, 1));
    assert!(report.failures[0].to_string().contains("readonly"), "{:?}", report.failures);
    assert_eq!(support::journal(), ["same true n 1"]);
}

/// Payloads of many tiny values weigh what they hold: two lists of 600 empty tables don't fit
/// where one does, and a table of a hundred thousand is refused part way through the read.
#[test]
fn many_tiny_values_count_against_the_queue() {
    let six_hundred = Message::List(vec![Message::Map(Vec::new()); 600]);
    let weight = six_hundred.queue_weight();
    let limits = EventLimits {
        pending: 100,
        bytes: weight + weight / 2,
        per_drain: 100,
        ..EventLimits::default()
    };
    let harness = limited(limits, "return { hit = function() end }", &[("hit", "hit")]);
    let domain = &harness.domain;
    let emit = "local t = {} for n = 1, COUNT do t[n] = {} end require('@jess/events').emit('hit', t)";
    support::eval::<()>(domain, &emit.replace("COUNT", "600"));
    let full = emit_error(domain, &emit.replace("COUNT", "600"));
    assert!(full.contains("bytes of payload are waiting"), "{full}");
    let vast = emit_error(domain, &emit.replace("COUNT", "100000"));
    assert!(
        vast.contains(&format!(
            "a value past the {} bytes a queue holds of one payload",
            limits.bytes
        )),
        "{vast}"
    );
    assert!(vast.contains("payload["), "{vast}");
    assert_eq!(
        domain.drain_events().unwrap().unhandled,
        0,
        "one hit, delivered to its handler"
    );
}

/// Freezing doesn't reach into a buffer, so a payload that holds one is built for each handler:
/// what the first writes into its buffer, the second never reads, and the tables around it are
/// frozen all the same.
#[test]
fn a_named_event_s_buffers_are_each_handler_s_own() {
    let harness = support::domain("buffers", &[]);
    let domain = &harness.domain;
    for name in ["first", "second"] {
        domain
            .register_context(ContextSpec::new(name).schedule([Phase::Event]))
            .unwrap();
    }
    load_handlers(
        domain,
        r"
        local journal = require('@test/journal')
        return {
          first = function(payload)
            buffer.writeu8(payload.data, 0, 42)
            journal.write(`first wrote {buffer.readu8(payload.data, 0)}`)
          end,
          second = function(payload)
            journal.write(`second reads {buffer.readu8(payload.data, 0)}, frozen {tostring(table.isfrozen(payload))}`)
          end,
        }
        ",
        &[("first", "hit", "first"), ("second", "hit", "second")],
    );
    domain.start().unwrap();
    support::eval::<()>(domain, "require('@jess/events').emit('hit', { data = buffer.create(1) })");
    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.failures.len()), (2, 0), "{:?}", report.failures);
    assert_eq!(support::journal(), ["first wrote 42", "second reads 0, frozen true"]);
}

/// An event's name is part of what it holds: a name past the limit is refused before anything
/// copies it, and names count toward the queue's bytes, so a flood of long names with no
/// payload still fills it.
#[test]
fn event_names_are_limited_and_weighed() {
    let name = "n".repeat(200);
    let one = Message::Nil.queue_weight() + name.len();
    let limits = EventLimits {
        pending: 100,
        bytes: 2 * one + one / 2,
        per_drain: 100,
        name: 256,
    };
    let harness = limited(limits, "return {}", &[]);
    let domain = &harness.domain;
    let long = emit_error(domain, "require('@jess/events').emit(string.rep('X', 1024 * 1024))");
    assert!(
        long.contains("events.emit: an event name is at most 256 bytes, and this one is 1048576"),
        "{long}"
    );
    let flood = format!("require('@jess/events').emit(string.rep('n', {}))", name.len());
    support::eval::<()>(domain, &flood);
    support::eval::<()>(domain, &flood);
    let full = emit_error(domain, &flood);
    assert!(full.contains(&format!("{} bytes of payload are waiting", 2 * one)), "{full}");

    // A host's named post with a name past the limit is refused for it, and queues nothing.
    let Err(PostRefused::Full(refused)) = domain.mailbox().unwrap().post_named(&"X".repeat(300), Message::Nil) else {
        panic!("a name past the limit is refused");
    };
    assert!(
        refused
            .to_string()
            .contains("takes event names of at most 256 bytes, and this one is 300"),
        "{refused}"
    );
    assert_eq!(domain.drain_events().unwrap().unhandled, 2);
}
