//! The host, headless: a domain with a live input service, driven a frame at a time from a synthetic source. The plan is the runtime's own modules plus a journal,
//! so what is proven here is the host's plumbing, not any script API over it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use jess::host::{ExitRequest, Host, HostSpec, InputSource};
use jess::input::{Control, GraphSpec, Key, NodeSpec, Owner, ValueType};
use jess::runtime::l3i;
use jess::runtime::l3i::extension::{Extension, ExtensionDescriptor, RuntimePlan, RuntimePolicy};
use jess::runtime::luau::RuntimeExtension;
use jess::runtime::{
    CapabilitySet, CollectedDiagnostics, ContextSpec, DiagnosticKind, DomainSpec, EventTarget, FromPlan, Message, Phase,
    RootAuthority, RuntimeFactory,
};

thread_local! {
    static JOURNAL: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PLAN: RefCell<Option<Rc<RuntimePlan>>> = const { RefCell::new(None) };
}

fn journal() -> Vec<String> {
    JOURNAL.with_borrow_mut(std::mem::take)
}

struct Journal;

impl Extension for Journal {
    fn id(&self) -> &'static str {
        "test"
    }

    fn describe(&self, d: &mut ExtensionDescriptor) -> l3i::Result<()> {
        d.module("@test/journal")
            .function("write", |line: String| JOURNAL.with_borrow_mut(|journal| journal.push(line)))
            .signature("(line: string) -> ()");
        Ok(())
    }
}

fn plan() -> l3i::Result<Rc<RuntimePlan>> {
    if let Some(plan) = PLAN.with_borrow(Clone::clone) {
        return Ok(plan);
    }
    let plan = RuntimePlan::builder()
        .policy(RuntimePolicy::new().debug_root("test"))
        .extension(RuntimeExtension)
        .extension(Journal)
        .finalize()?;
    PLAN.with_borrow_mut(|cached| *cached = Some(Rc::clone(&plan)));
    Ok(plan)
}

fn factory() -> Arc<dyn RuntimeFactory> {
    Arc::new(FromPlan(plan))
}

/// `ui.accept` on Enter, in a `ui` set.
fn accept() -> GraphSpec {
    let mut spec = GraphSpec::new();
    spec.action("ui.accept", ValueType::Digital).set("ui", "@test/kernel");
    spec.add_binding(
        &Owner::Set("ui".to_owned()),
        "ui.accept",
        NodeSpec::button(Control::Key(Key::Enter)),
    )
    .unwrap();
    spec
}

const MAIN: &str = r"
local context = require('@jess/context')
local journal = require('@test/journal')

context.bind('main', {
  input = function(payload)
    for _, event in payload.events do
      journal.write(`input {event.action} {event.kind} {tostring(event.value)} device {event.device} at {event.time} gen {payload.generation}`)
    end
  end,

  update = function(delta)
    journal.write(`update {delta}`)
  end,

  event = function(payload)
    journal.write(`event {payload.event}`)
  end,

  shutdown = function()
    journal.write('shutdown')
  end,
})
";

fn host(diagnostics: &Arc<CollectedDiagnostics>) -> Host {
    let grant = RootAuthority::new().grant(CapabilitySet::empty());
    let sink: Arc<dyn jess::runtime::DiagnosticSink> = diagnostics.clone();
    let spec = DomainSpec::new("kernel", grant).diagnostics(sink);
    let host = Host::create(factory(), HostSpec::new(spec).input(InputSource::Synthetic)).unwrap();
    host.domain()
        .register_context(ContextSpec::new("main").schedule([Phase::Input, Phase::Update, Phase::Event, Phase::Shutdown]))
        .unwrap();
    host.domain().load_source("=main", MAIN).unwrap();
    host.domain().start().unwrap();
    host
}

#[test]
fn a_synthetic_host_leaves_the_input_clock_to_whoever_drives_it() {
    let diagnostics = Arc::new(CollectedDiagnostics::new());
    let mut host = host(&diagnostics);
    let input = Arc::clone(host.input().unwrap());
    let start = input.lock().unwrap().now();

    for _ in 0..3 {
        host.frame(Duration::from_millis(100)).unwrap();
    }
    assert_eq!(
        input.lock().unwrap().now(),
        start,
        "a frame's delta is the domain's time, not input's"
    );

    input.lock().unwrap().advance(Duration::from_millis(5));
    host.frame(Duration::from_millis(100)).unwrap();
    assert_eq!(input.lock().unwrap().now().nanos() - start.nanos(), 5_000_000);

    journal();
    host.stop().unwrap();
    journal();
}

#[test]
fn a_frame_ticks_input_into_the_input_phase() {
    let diagnostics = Arc::new(CollectedDiagnostics::new());
    let mut host = host(&diagnostics);
    assert!(host.domain().service::<jess::host::SharedInput>().is_some());

    // The host configures input here; under jess::lua's plan the script would, through the same service.
    let keyboard = {
        let mut input = host.input().unwrap().lock().unwrap();
        input.install(accept()).unwrap();
        input.set_active("ui", true).unwrap();
        input.synthetic().connect_keyboard()
    };

    // A quiet frame: the connect applies to device state, no action event, no input phase.
    let report = host.frame(Duration::from_millis(16)).unwrap();
    assert!(report.input.is_none());
    assert_eq!(journal(), ["update 0.016"]);

    // A key press: one input phase with the event and the graph generation.
    {
        let mut input = host.input().unwrap().lock().unwrap();
        input.advance(Duration::from_millis(16));
        input.synthetic().press(keyboard, Control::Key(Key::Enter));
    }
    let report = host.frame(Duration::from_millis(16)).unwrap();
    let phase = report.input.expect("the press is an action event");
    assert_eq!((phase.ran, phase.failures.len()), (1, 0));
    assert_eq!(
        journal(),
        ["input ui.accept pressed true device 0 at 0.016 gen 1", "update 0.016",]
    );

    // The key is still held: no edge, no phase.
    let report = host.frame(Duration::from_millis(16)).unwrap();
    assert!(report.input.is_none());
    assert_eq!(journal(), ["update 0.016"]);

    let teardown = host.stop().unwrap();
    assert_eq!(teardown.shutdown_failures, []);
    assert_eq!(journal(), ["shutdown"]);
    assert!(diagnostics.entries().is_empty(), "{:?}", diagnostics.entries());
}

#[test]
fn a_restart_keeps_the_devices_and_drops_what_the_old_generation_configured_or_queued() {
    let diagnostics = Arc::new(CollectedDiagnostics::new());
    let mut host = host(&diagnostics);
    let keyboard = {
        let mut input = host.input().unwrap().lock().unwrap();
        input.install(accept()).unwrap();
        input.set_active("ui", true).unwrap();
        input.synthetic().connect_keyboard()
    };
    host.frame(Duration::from_millis(16)).unwrap();
    journal();

    // A key goes down and an event is posted for the old generation, then the code restarts.
    {
        let mut input = host.input().unwrap().lock().unwrap();
        input.advance(Duration::from_millis(16));
        input.synthetic().press(keyboard, Control::Key(Key::Enter));
    }
    let mailbox = host.domain().mailbox().unwrap();
    mailbox
        .post(EventTarget::All, Message::map([("event", Message::text("old"))]))
        .unwrap();
    let generation = host
        .restart(|runtime| jess::runtime::chunk::load(runtime, "=main", MAIN)?.invoke::<(), ()>(&runtime.stack(), ()))
        .unwrap();
    assert_eq!(generation.raw(), 2);
    assert_eq!(journal(), ["shutdown"]);
    {
        let input = host.input().unwrap().lock().unwrap();
        assert!(input.runtime().graph().is_none(), "the old code's graph is gone");
        assert_eq!(input.device(keyboard.index()), Some(keyboard), "the keyboard is still there");
    }

    // The new generation's first frame: the press reaches device state, but no graph means no
    // action event, and the old event is discarded, not delivered.
    host.input().unwrap().lock().unwrap().advance(Duration::from_millis(16));
    let report = host.frame(Duration::from_millis(16)).unwrap();
    assert!(report.input.is_none());
    assert_eq!((report.tick.events.stale, report.tick.events.delivered), (1, 0));
    assert_eq!(journal(), ["update 0.016"]);
    assert_eq!(diagnostics.of_kind(DiagnosticKind::StaleGenerationDiscarded).len(), 1);

    // The new code installs its graph; the key held across the swap stays invisible until it is
    // released and pressed again.
    {
        let mut input = host.input().unwrap().lock().unwrap();
        input.install(accept()).unwrap();
        input.set_active("ui", true).unwrap();
        input.advance(Duration::from_millis(16));
    }
    assert!(host.frame(Duration::from_millis(16)).unwrap().input.is_none());
    journal();
    {
        let mut input = host.input().unwrap().lock().unwrap();
        input.synthetic().release(keyboard, Control::Key(Key::Enter));
        input.advance(Duration::from_millis(16));
        input.synthetic().press(keyboard, Control::Key(Key::Enter));
    }
    // The release applies and then the press, in one tick: the action ends pressed.
    let report = host.frame(Duration::from_millis(16)).unwrap();
    assert!(report.input.is_some());
    assert_eq!(
        journal(),
        ["input ui.accept pressed true device 0 at 0.064 gen 3", "update 0.016"]
    );
    host.stop().unwrap();
}

#[test]
fn the_stub_backend_is_a_real_poll_loop_over_nothing() {
    let grant = RootAuthority::new().grant(CapabilitySet::empty());
    let mut host = Host::create(
        factory(),
        HostSpec::new(DomainSpec::new("stub", grant)).input(InputSource::stub()),
    )
    .unwrap();
    assert!(format!("{host:?}").contains("stub"));
    host.domain()
        .register_context(ContextSpec::new("main").schedule([Phase::Input]))
        .unwrap();
    host.domain().load_source("=main", "").unwrap();
    host.domain().start().unwrap();
    assert!(host.pump_input().unwrap().is_none());
    assert!(host.frame(Duration::from_millis(16)).unwrap().input.is_none());
    // The platform backend of a build without a device feature is the stub too. With one, it
    // would open real devices, which no test does.
    #[cfg(not(any(feature = "linux-evdev", feature = "gilrs")))]
    assert_eq!(format!("{:?}", InputSource::platform().unwrap()), "Backend(\"stub\")");
    host.stop().unwrap();
}

#[test]
fn a_host_without_services_carries_only_its_exit_request() {
    let grant = RootAuthority::new().grant(CapabilitySet::empty());
    let mut host = Host::create(factory(), HostSpec::new(DomainSpec::new("bare", grant))).unwrap();
    assert!(host.input().is_none());
    assert_eq!(host.domain().services().len(), 1);
    assert!(host.domain().service::<ExitRequest>().is_some());
    assert_eq!(host.exit_requested(), None);
    host.domain().load_source("=main", "").unwrap();
    host.domain().start().unwrap();
    let report = host.frame(Duration::from_millis(16)).unwrap();
    assert!(report.input.is_none());
    assert_eq!(report.failures(), Vec::<&jess::runtime::RuntimeError>::new());
    host.stop().unwrap();
}
