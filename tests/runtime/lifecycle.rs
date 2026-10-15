//! The lifecycle, the authority math, and the ownership registry.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use jess::runtime::l3i::Runtime;
use jess::runtime::{
    CapabilityId, CapabilitySet, ContextSpec, DiagnosticKind, DomainSpec, DomainState, EventTarget, Grantor, Message, Phase,
    RuntimeError, RuntimeFactory, ScriptDomain, WORKER_SPAWN, WorkerSpec,
};

use crate::support;

fn lifecycle(error: &RuntimeError) -> (DomainState, &'static str) {
    match error {
        RuntimeError::Lifecycle { state, action, .. } => (*state, *action),
        other => panic!("expected a lifecycle error, got {other:?}"),
    }
}

#[test]
fn invalid_transitions_fail_loudly_and_are_diagnosed() {
    let harness = support::domain("lifecycle", &[]);
    let domain = &harness.domain;
    assert_eq!(lifecycle(&domain.start().unwrap_err()), (DomainState::Created, "start"));
    assert_eq!(
        lifecycle(&domain.run_phase(Phase::Update, &Message::Nil).unwrap_err()),
        (DomainState::Created, "run a phase of")
    );

    domain.load(|_| Ok(())).unwrap();
    assert_eq!(domain.state(), DomainState::Ready);
    assert_eq!(lifecycle(&domain.load(|_| Ok(())).unwrap_err()), (DomainState::Ready, "load"));
    assert_eq!(
        lifecycle(&domain.drain_events().unwrap_err()),
        (DomainState::Ready, "drain the events of")
    );

    domain.start().unwrap();
    assert_eq!(domain.state(), DomainState::Running);
    assert_eq!(lifecycle(&domain.start().unwrap_err()), (DomainState::Running, "start"));

    let mailbox = domain.mailbox().unwrap();
    domain.stop().unwrap();
    assert_eq!(domain.state(), DomainState::Stopped);
    assert_eq!(lifecycle(&domain.start().unwrap_err()), (DomainState::Stopped, "start"));
    assert_eq!(lifecycle(&domain.stop().unwrap_err()), (DomainState::Stopped, "stop"));
    assert_eq!(
        lifecycle(&domain.restart(|_| Ok(())).unwrap_err()),
        (DomainState::Stopped, "restart")
    );
    assert_eq!(
        lifecycle(&domain.register_context(ContextSpec::new("late")).unwrap_err()),
        (DomainState::Stopped, "register a context in")
    );
    assert_eq!(
        lifecycle(
            &domain
                .spawn_worker(WorkerSpec::new("late", "return function() end"))
                .unwrap_err()
        ),
        (DomainState::Stopped, "spawn a worker in")
    );
    assert_eq!(
        lifecycle(&domain.mailbox().unwrap_err()),
        (DomainState::Stopped, "open a mailbox of")
    );
    assert_eq!(mailbox.post(EventTarget::All, Message::Nil).unwrap_err().0, domain.id());

    let kinds: Vec<DiagnosticKind> = harness
        .diagnostics
        .entries()
        .iter()
        .map(|diagnostic| diagnostic.kind)
        .collect();
    assert_eq!(kinds.iter().filter(|kind| **kind == DiagnosticKind::SpawnRefused).count(), 1);
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == DiagnosticKind::InvalidLifecycleTransition)
            .count(),
        10
    );
    let message = &harness.diagnostics.of_kind(DiagnosticKind::SpawnRefused)[0].message;
    assert_eq!(
        *message,
        format!("cannot spawn a worker in {} while it is stopped", domain.id())
    );
}

#[test]
fn code_that_fails_to_load_leaves_a_failed_domain_that_can_only_stop() {
    let harness = support::domain("broken", &[]);
    let domain = &harness.domain;
    let error = domain.load_source("=broken", "error('no good', 0)").unwrap_err();
    let RuntimeError::Script {
        attribution,
        phase,
        message,
    } = &error
    else {
        panic!("{error:?}")
    };
    assert_eq!(
        (attribution.domain, *phase, message.as_str()),
        (domain.id(), Some(Phase::Load), "no good")
    );
    assert_eq!(domain.state(), DomainState::Failed);
    assert_eq!(lifecycle(&domain.start().unwrap_err()), (DomainState::Failed, "start"));
    assert_eq!(harness.diagnostics.of_kind(DiagnosticKind::HandlerFailed).len(), 1);
    domain.stop().unwrap();
    assert_eq!(domain.state(), DomainState::Stopped);

    let syntax = support::domain("syntax", &[]);
    let error = syntax.domain.load_source("=syntax", "local = 1").unwrap_err();
    assert!(error.to_string().contains("Expected identifier"), "{error}");
    assert_eq!(syntax.domain.state(), DomainState::Failed);
}

#[test]
fn a_child_never_holds_more_than_its_parent() {
    let parent = support::domain("parent", &["vfs.read", "input.actions", WORKER_SPAWN]);
    assert_eq!(parent.domain.grant().granted_by(), Grantor::Root);

    // A child domain with a subset.
    let grant = parent
        .domain
        .delegate(CapabilitySet::from_names(["vfs.read"]).unwrap())
        .unwrap();
    assert_eq!(grant.granted_by(), Grantor::Domain(parent.domain.id()));
    let child =
        ScriptDomain::create(support::factory(), DomainSpec::new("child", grant).parent(parent.domain.id())).unwrap();
    assert_eq!(child.parent(), Some(parent.domain.id()));
    assert!(child.grant().capabilities().contains_name("vfs.read"));
    assert!(!child.grant().capabilities().contains_name("input.actions"));

    // The parent lacks render.scene, so it can't grant it; the child can't pass on what it lacks.
    let render = CapabilityId::intern("render.scene").unwrap();
    let over = parent
        .domain
        .delegate(CapabilitySet::from_names(["vfs.read", "render.scene"]).unwrap())
        .unwrap_err();
    assert_eq!(
        over,
        RuntimeError::OverDelegation {
            grantor: Grantor::Domain(parent.domain.id()),
            missing: vec![render]
        }
    );
    assert_eq!(
        over.to_string(),
        format!(
            "{} cannot delegate capabilities it does not hold: render.scene",
            parent.domain.id()
        )
    );
    let input = CapabilityId::intern("input.actions").unwrap();
    let over = child
        .delegate(CapabilitySet::from_names(["input.actions"]).unwrap())
        .unwrap_err();
    assert_eq!(
        over,
        RuntimeError::OverDelegation {
            grantor: Grantor::Domain(child.id()),
            missing: vec![input]
        }
    );

    // The same math for a context and for a worker.
    let context = ContextSpec::new("greedy").capabilities(CapabilitySet::from_names(["render.scene"]).unwrap());
    assert!(matches!(
        parent.domain.register_context(context),
        Err(RuntimeError::OverDelegation { .. })
    ));
    let narrow = ContextSpec::new("narrow").capabilities(CapabilitySet::from_names(["input.actions"]).unwrap());
    parent.domain.register_context(narrow).unwrap();
    let info = parent.domain.context("narrow").unwrap();
    assert_eq!(
        info.capabilities.len(),
        1,
        "a context's grant is an explicit subset, not the domain's"
    );
    let worker = WorkerSpec::new("greedy", "return function() end")
        .capabilities(CapabilitySet::from_names(["render.scene"]).unwrap());
    assert!(matches!(
        parent.domain.spawn_worker(worker),
        Err(RuntimeError::OverDelegation { .. })
    ));
    assert_eq!(parent.domain.workers(), []);
    assert_eq!(parent.diagnostics.of_kind(DiagnosticKind::SpawnRefused).len(), 1);
}

#[test]
fn a_script_cannot_widen_its_own_authority() {
    // The context holds worker.spawn and nothing else, though its domain holds more.
    let harness = support::domain("narrow", &["vfs.read", WORKER_SPAWN]);
    let domain = &harness.domain;
    let spec = ContextSpec::new("main")
        .schedule([Phase::OnDemand])
        .capabilities(CapabilitySet::from_names([WORKER_SPAWN]).unwrap());
    domain.register_context(spec).unwrap();
    domain
        .load_source(
            "=narrow",
            r"
            local context = require('@jess/context')
            local worker = require('@jess/worker')

            context.bind('main', {
              onDemand = function()
                worker.spawn { source = 'return function() end', capabilities = { 'vfs.read' } }
              end,
            })
            ",
        )
        .unwrap();
    domain.start().unwrap();
    let report = domain.run_phase(Phase::OnDemand, &Message::Nil).unwrap();
    let message = report.failures[0].to_string();
    assert!(
        message.contains("cannot delegate capabilities it does not hold: vfs.read"),
        "{message}"
    );
    assert_eq!(domain.workers(), []);

    // Binding to a phase the host didn't schedule, or to a context it didn't declare, fails too.
    let error = domain.with_runtime(|runtime| {
        runtime
            .exec("require('@jess/context').bind('main', { update = function() end })")
            .unwrap_err()
    });
    assert!(error.unwrap().to_string().contains("does not schedule the update phase"));
    let error = domain.with_runtime(|runtime| runtime.exec("require('@jess/context').bind('other', {})").unwrap_err());
    assert_eq!(error.unwrap().to_string(), format!("{} has no context 'other'", domain.id()));
    let error = domain.with_runtime(|runtime| {
        runtime
            .exec("require('@jess/context').bind('main', { menu = function() end })")
            .unwrap_err()
    });
    assert_eq!(error.unwrap().to_string(), "'menu' is not a scheduler phase");
}

#[test]
fn a_runtime_no_domain_owns_has_the_modules_and_refuses_to_use_them() {
    let runtime = Runtime::from_plan(&support::plan().unwrap()).unwrap();
    let bind = runtime.exec("require('@jess/context').bind('main', {})").unwrap_err();
    assert_eq!(
        bind.to_string(),
        "context.bind: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and package targets run scripts outside one"
    );
    let spawn = runtime.exec("require('@jess/worker').spawn { source = '' }").unwrap_err();
    assert_eq!(
        spawn.to_string(),
        "worker.spawn: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and package targets run scripts outside one"
    );
    let send = runtime.exec("require('@jess/worker').send(1)").unwrap_err();
    assert_eq!(
        send.to_string(),
        "worker.send: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and package targets run scripts outside one"
    );
}

#[test]
fn owned_resources_are_released_once_newest_first() {
    let harness = support::domain("owner", &[]);
    let domain = &harness.domain;
    let log = Rc::new(RefCell::new(Vec::new()));
    let note = |name: &'static str| {
        let log = Rc::clone(&log);
        move || log.borrow_mut().push(name)
    };
    let timer = domain.own("timer", note("timer"));
    domain.own("watch", note("watch"));
    domain.own("target", note("target"));
    let kinds: Vec<&str> = domain.owned_resources().iter().map(|resource| resource.kind).collect();
    assert_eq!(kinds, ["timer", "watch", "target"]);
    assert!(
        domain
            .owned_resources()
            .iter()
            .all(|resource| resource.generation == domain.generation())
    );

    assert!(domain.release(timer));
    assert!(!domain.release(timer), "released once");
    let teardown = domain.stop().unwrap();
    assert_eq!(teardown.resources_released, 2);
    assert_eq!(*log.borrow(), ["timer", "target", "watch"]);
    assert_eq!(domain.owned_resources(), []);
}

#[test]
fn dropping_a_domain_tears_it_down() {
    let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker = {
        let harness = support::domain("dropped", &[]);
        let flag = Arc::clone(&released);
        harness
            .domain
            .own("flag", move || flag.store(true, std::sync::atomic::Ordering::SeqCst));
        let worker = harness
            .domain
            .spawn_worker(WorkerSpec::new("orphan", "return function() end"))
            .unwrap();
        harness.domain.send_to_worker(worker, Message::Nil).unwrap();
        worker
    };
    assert!(
        released.load(std::sync::atomic::Ordering::SeqCst),
        "drop released what the domain owned"
    );
    #[cfg(target_os = "linux")]
    assert!(support::lane_gone(worker), "and joined its worker");
    let _ = worker;
}

#[test]
fn a_factory_can_be_a_plan_function_or_a_runtime_closure() {
    // A closure that finishes the runtime itself: what a host does to set globals after
    // `Runtime::from_plan`.
    let factory: Arc<dyn RuntimeFactory> = Arc::new(|| {
        let runtime = Runtime::from_plan(&support::plan()?)?;
        runtime.set_global("hostGlobal", &42.0)?;
        Ok(runtime)
    });
    let (spec, _, _) = support::spec("closure", &[]);
    let spec = spec.setup(|runtime| runtime.set_global("fromSetup", "yes"));
    let domain = ScriptDomain::create(factory, spec).unwrap();
    assert_eq!(support::eval::<i64>(&domain, "return hostGlobal"), 42);
    assert_eq!(support::eval::<String>(&domain, "return fromSetup"), "yes");

    // The factory and the setup hook build the worker's runtime too, on its lane.
    let worker = domain
        .spawn_worker(WorkerSpec::new("w", "return function() return { hostGlobal, fromSetup } end"))
        .unwrap();
    domain.send_to_worker(worker, Message::Nil).unwrap();
    let reply = domain
        .receive_from_worker(worker, Some(std::time::Duration::from_secs(20)))
        .unwrap();
    assert_eq!(reply, Some(Message::List(vec![Message::Number(42.0), Message::text("yes")])));
}

#[test]
fn the_plan_declares_types_luau_accepts() {
    support::plan().unwrap().check_definitions().unwrap();
    let definitions = support::plan().unwrap().type_definitions();
    for declared in [
        "declare extern type jess_runtime_Worker with",
        "declare extern type jess_runtime_Handle with",
        "export type Module__jess_context = {",
        "export type Module__jess_worker = {",
    ] {
        assert!(
            definitions.contains(declared),
            "the definitions lack `{declared}`:\n{definitions}"
        );
    }
}
