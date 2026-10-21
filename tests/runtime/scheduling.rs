//! The scheduler: ordering, fixed steps, events, budgets, and reentrancy.

use std::rc::Rc;
use std::time::Duration;

use jess::runtime::{
    Budget, ContextSpec, DiagnosticKind, DomainSpec, EventTarget, FixedStep, Generation, Message, Phase, ProfileEvent,
    RootAuthority, RuntimeError, ScriptDomain,
};

use super::support;

/// Binds, for each named context, handlers that journal `<context> <phase> <payload>`.
fn journaling(contexts: &[(&str, &[&str])]) -> String {
    use std::fmt::Write as _;

    let mut source = String::from("local context = require('@jess/context')\nlocal journal = require('@test/journal')\n");
    for (name, phases) in contexts {
        let _ = write!(source, "\ncontext.bind('{name}', {{\n");
        for phase in *phases {
            let _ = write!(
                source,
                "  {phase} = function(payload)\n    journal.write(`{name} {phase} {{tostring(payload)}}`)\n  end,\n\n"
            );
        }
        source.push_str("})\n");
    }
    source
}

#[test]
fn a_phase_runs_contexts_by_priority_then_registration_order() {
    let harness = support::domain("ordered", &[]);
    let domain = &harness.domain;
    for (name, priority) in [("first", 0), ("early", -5), ("second", 0), ("late", 9), ("silent", 0)] {
        domain
            .register_context(
                ContextSpec::new(name)
                    .schedule([Phase::Update, Phase::Startup])
                    .priority(priority),
            )
            .unwrap();
    }
    // Binding order is not scheduling order: `late` binds first and still runs last.
    let phases: &[&str] = &["update"];
    domain
        .load_source(
            "=ordered",
            &journaling(&[("late", phases), ("second", phases), ("early", phases), ("first", phases)]),
        )
        .unwrap();
    domain.start().unwrap();
    for _ in 0..3 {
        let report = domain.run_phase(Phase::Update, &Message::Number(1.0)).unwrap();
        assert_eq!(report.ran, 4, "a context with no handler for the phase isn't called");
        assert_eq!(
            support::journal(),
            ["early update 1", "first update 1", "second update 1", "late update 1"]
        );
    }
    let bound: Vec<(String, Vec<Phase>)> = domain
        .contexts()
        .into_iter()
        .map(|context| (context.name, context.bound))
        .collect();
    assert_eq!(bound[0], ("first".to_owned(), vec![Phase::Update]));
    assert_eq!(bound[4], ("silent".to_owned(), Vec::new()));
    assert_eq!(domain.context("early").unwrap().calls, 3);
}

#[test]
fn a_failing_handler_is_attributed_and_the_phase_goes_on() {
    let harness = support::domain("failing", &[]);
    let domain = &harness.domain;
    let bad = domain
        .register_context(ContextSpec::new("bad").schedule([Phase::Update]))
        .unwrap();
    domain
        .register_context(ContextSpec::new("good").schedule([Phase::Update]))
        .unwrap();
    domain
        .load_source(
            "=failing",
            r"
            local context = require('@jess/context')
            local journal = require('@test/journal')

            context.bind('bad', {
              update = function()
                error('handler broke')
              end,
            })

            context.bind('good', {
              update = function()
                journal.write('good ran')
              end,
            })
            ",
        )
        .unwrap();
    domain.start().unwrap();
    let report = domain.run_phase(Phase::Update, &Message::Nil).unwrap();
    assert_eq!(report.ran, 2);
    assert_eq!(support::journal(), ["good ran"]);
    let [
        RuntimeError::Script {
            attribution,
            phase,
            message,
        },
    ] = report.failures.as_slice()
    else {
        panic!("one failure, got {:?}", report.failures);
    };
    assert_eq!(
        (attribution.domain, attribution.generation, attribution.context),
        (domain.id(), Generation::FIRST, Some(bad))
    );
    assert_eq!(*phase, Some(Phase::Update));
    assert_eq!(message, "failing:7: handler broke");

    let [diagnostic] = harness
        .diagnostics
        .of_kind(DiagnosticKind::HandlerFailed)
        .try_into()
        .expect("one diagnostic");
    assert_eq!(
        (diagnostic.attribution, diagnostic.phase),
        (*attribution, Some(Phase::Update))
    );
    assert_eq!(diagnostic.source.as_deref(), Some("failing:5"), "where the handler was bound");
    assert_eq!(
        domain.context("bad").map(|context| (context.calls, context.failures)),
        Some((1, 1))
    );
    assert_eq!((domain.stats().context_calls, domain.stats().context_failures), (2, 1));
    let failed: Vec<bool> = harness
        .profile
        .events()
        .iter()
        .filter_map(|event| match event {
            ProfileEvent::ContextCall { failed, .. } => Some(*failed),
            _ => None,
        })
        .collect();
    assert_eq!(failed, [true, false]);
}

#[test]
fn fixed_steps_accumulate_and_never_catch_up_without_bound() {
    let (spec, _, _) = support::spec("fixed", &[]);
    let spec = spec.fixed_step(FixedStep {
        step: Duration::from_millis(10),
        max_catch_up: 3,
    });
    let domain = ScriptDomain::create(support::factory(), spec).unwrap();
    domain
        .register_context(ContextSpec::new("sim").schedule([Phase::FixedUpdate, Phase::Update, Phase::LateUpdate]))
        .unwrap();
    domain
        .load_source("=fixed", &journaling(&[("sim", &["fixedUpdate", "update", "lateUpdate"])]))
        .unwrap();
    domain.start().unwrap();

    // 25 ms owes two 10 ms steps; 5 ms stays in the accumulator.
    let tick = domain.advance(Duration::from_millis(25)).unwrap();
    assert_eq!((tick.fixed_steps, tick.dropped_steps), (2, 0));
    assert_eq!(
        support::journal(),
        [
            "sim fixedUpdate 0.01",
            "sim fixedUpdate 0.01",
            "sim update 0.025",
            "sim lateUpdate 0.025"
        ]
    );

    // A stalled host owes ten steps and gets three; the other seven are dropped, not queued.
    let tick = domain.advance(Duration::from_millis(100)).unwrap();
    assert_eq!((tick.fixed_steps, tick.dropped_steps), (3, 7));
    assert_eq!(support::journal().len(), 5);

    // The 5 ms remainder survived both: 5 ms more makes one step.
    let tick = domain.advance(Duration::from_millis(5)).unwrap();
    assert_eq!((tick.fixed_steps, tick.dropped_steps), (1, 0));
    assert_eq!((domain.stats().fixed_updates, domain.stats().updates), (6, 3));
    support::journal();

    // No fixed step configured: no fixed updates.
    let plain = support::domain("plain", &[]);
    plain
        .domain
        .register_context(ContextSpec::new("sim").schedule([Phase::FixedUpdate, Phase::Update]))
        .unwrap();
    plain
        .domain
        .load_source("=plain", &journaling(&[("sim", &["fixedUpdate", "update"])]))
        .unwrap();
    plain.domain.start().unwrap();
    assert_eq!(plain.domain.advance(Duration::from_secs(1)).unwrap().fixed_steps, 0);
    assert_eq!(support::journal(), ["sim update 1"]);
}

#[test]
fn events_queue_from_any_thread_and_are_delivered_in_order_by_the_scheduler() {
    let harness = support::domain("events", &[]);
    let domain = &harness.domain;
    let input = domain
        .register_context(ContextSpec::new("input").schedule([Phase::Event]))
        .unwrap();
    let audio = domain
        .register_context(ContextSpec::new("audio").schedule([Phase::Event]).priority(-1))
        .unwrap();
    let mute = domain
        .register_context(ContextSpec::new("mute").schedule([Phase::Update]))
        .unwrap();
    domain
        .load_source(
            "=events",
            r"
            local context = require('@jess/context')
            local journal = require('@test/journal')

            for _, name in { 'input', 'audio' } do
              context.bind(name, {
                event = function(payload)
                  journal.write(`{name} {payload}`)
                end,
              })
            end
            ",
        )
        .unwrap();
    domain.start().unwrap();

    // Posting runs nothing: the native side only queues.
    let mailbox = domain.mailbox().unwrap();
    let from_another_thread = mailbox.clone();
    std::thread::spawn(move || {
        from_another_thread
            .post(EventTarget::Context(input), Message::text("one"))
            .unwrap();
        from_another_thread.post(EventTarget::All, Message::text("two")).unwrap();
    })
    .join()
    .unwrap();
    mailbox.post(EventTarget::Context(mute), Message::text("three")).unwrap();
    mailbox.post(EventTarget::Context(audio), Message::text("four")).unwrap();
    assert_eq!(support::journal(), Vec::<String>::new());

    let report = domain.drain_events().unwrap();
    assert_eq!((report.delivered, report.unhandled, report.stale), (4, 1, 0));
    assert_eq!(support::journal(), ["input one", "audio two", "input two", "audio four"]);
    assert_eq!(domain.stats().events_delivered, 4);
    let delays: Vec<Duration> = harness
        .profile
        .events()
        .iter()
        .filter_map(|event| match event {
            ProfileEvent::ContextCall {
                phase: Phase::Event,
                queue_delay,
                ..
            } => Some(*queue_delay),
            _ => None,
        })
        .collect();
    assert_eq!(delays.len(), 4);
    assert!(
        delays.iter().all(|delay| !delay.is_zero()),
        "an event's queue delay is measured"
    );

    // `advance` drains too, after late update.
    mailbox.post(EventTarget::Context(input), Message::text("five")).unwrap();
    assert_eq!(domain.advance(Duration::ZERO).unwrap().events.delivered, 1);
    assert_eq!(support::journal(), ["input five"]);

    // Work a service started under one generation completes as that generation's.
    mailbox
        .post_for(EventTarget::Context(input), Generation::FIRST.next(), Message::text("early"))
        .unwrap();
    assert_eq!(domain.drain_events().unwrap().stale, 1);
}

#[test]
fn a_context_runs_inside_its_budget() {
    let (spec, diagnostics, _) = support::spec("budget", &[]);
    let domain = ScriptDomain::create(support::factory(), spec.budget(Budget::time(Duration::from_secs(30)))).unwrap();
    let tight = ContextSpec::new("tight")
        .schedule([Phase::Update])
        .budget(Budget::time(Duration::from_millis(30)));
    domain.register_context(tight).unwrap();
    domain
        .register_context(ContextSpec::new("roomy").schedule([Phase::Update]))
        .unwrap();
    domain
        .load_source(
            "=budget",
            r"
            local context = require('@jess/context')
            local journal = require('@test/journal')

            context.bind('tight', {
              update = function(spin)
                while spin do
                end
              end,
            })

            context.bind('roomy', {
              update = function()
                local total = 0
                for index = 1, 200000 do
                  total += index
                end

                journal.write(`roomy {total}`)
              end,
            })
            ",
        )
        .unwrap();
    domain.start().unwrap();

    let report = domain.run_phase(Phase::Update, &Message::Boolean(true)).unwrap();
    let [failure] = report.failures.as_slice() else {
        panic!("the spinning handler alone fails: {:?}", report.failures)
    };
    assert!(failure.to_string().contains("Lua execution time limit exceeded"), "{failure}");
    assert_eq!(
        support::journal(),
        ["roomy 20000100000"],
        "the next context ran, under the domain's roomier budget"
    );
    assert_eq!(diagnostics.of_kind(DiagnosticKind::HandlerFailed).len(), 1);

    // The context is not poisoned: it runs again, inside its budget this time.
    assert_eq!(
        domain.run_phase(Phase::Update, &Message::Boolean(false)).unwrap().failures,
        []
    );
    support::journal();
}

#[test]
fn the_scheduler_refuses_entry_from_inside_its_own_dispatch() {
    let grant = RootAuthority::new().grant(jess::runtime::CapabilitySet::empty());
    let domain = Rc::new(ScriptDomain::create(support::factory(), DomainSpec::new("reentrant", grant)).unwrap());
    domain
        .register_context(ContextSpec::new("main").schedule([Phase::Update, Phase::Event]))
        .unwrap();

    // A native service that, handed control by a handler, tries every scheduler entry.
    let weak = Rc::downgrade(&domain);
    domain
        .with_runtime(|runtime| {
            let reenter = runtime.bind_function("test.reenter", move || {
                let domain = weak.upgrade().expect("the domain is alive while its handler runs");
                let attempts = [
                    domain.run_phase(Phase::Update, &Message::Nil).map(|_| ()),
                    domain.advance(Duration::ZERO).map(|_| ()),
                    domain.drain_events().map(|_| ()),
                    domain.restart(|_| Ok(())).map(|_| ()),
                    domain.stop().map(|_| ()),
                ];
                attempts.map(|attempt| attempt.unwrap_err().to_string()).join("\n")
            })?;
            runtime.set_global("reenter", &reenter)
        })
        .unwrap()
        .unwrap();
    domain
        .load_source(
            "=reentrant",
            r"
            local context = require('@jess/context')
            local journal = require('@test/journal')

            context.bind('main', {
              update = function()
                journal.write(reenter())
              end,

              event = function(payload)
                journal.write(`event {payload}`)
              end,
            })
            ",
        )
        .unwrap();
    domain.start().unwrap();

    // What a handler may do instead: queue. The event is delivered after the handler returns.
    let mailbox = domain.mailbox().unwrap();
    mailbox.post(EventTarget::All, Message::text("queued")).unwrap();
    let tick = domain.advance(Duration::ZERO).unwrap();
    assert!(tick.failures().is_empty(), "{:?}", tick.failures());
    let id = domain.id();
    let refused = |action: &str| format!("cannot {action} {id} from inside its own update dispatch; queue an event instead");
    let expected = [
        refused("run a phase of"),
        refused("advance"),
        refused("drain the events of"),
        refused("restart"),
        refused("stop"),
    ]
    .join("\n");
    assert_eq!(support::journal(), [expected, "event queued".to_owned()]);
    assert_eq!(
        domain.state(),
        jess::runtime::DomainState::Running,
        "none of the refused entries changed the domain"
    );
}
