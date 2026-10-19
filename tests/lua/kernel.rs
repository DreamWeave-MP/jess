//! The application kernel, end to end and headless: one host domain from jess's own plan, with a
//! live input runtime attached, walked through everything the GUI pass will stand on. This is the substrate handoff's integration fixture (section 18), minus
//! the package and presentation items that belong to later passes.
//!
//! ```text
//! Host
//! ├── services: Mutex<InputService> (synthetic)
//! └── domain "app": input.actions, input.configure, worker.spawn
//!     ├── context main:  startup / update / event / shutdown   (the domain's grant)
//!     ├── context input: input                                  (input.actions only)
//!     └── worker greeter: no capabilities, its own @jess/l10n registry, no input at all
//! ```
//!
//! Each numbered step below is one thing the fixture proves. Globals are read-only in a jess
//! runtime, so the script keeps its log in a local and returns two functions from its chunk: one
//! that reads the log (called at host level, no authority needed) and an `onDemand` handler the
//! test binds, through which it calls into the script under the `main` context's authority.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jess::host::{Host, HostSpec, InputSource, SharedInput};
use jess::input::{Control, Key};
use jess::lua::Options;
use jess::lua::l3i::Runtime;
use jess::lua::l3i::value::Function;
use jess::runtime::{
    CapabilitySet, CollectedDiagnostics, ContextSpec, DiagnosticKind, DiagnosticSink, DomainSpec, DomainState, EventTarget,
    Message, Phase, RootAuthority, RuntimeFactory, WORKER_SPAWN,
};

/// The application: what a package's `main.jsl` would be.
const MAIN: &str = r"
local context = require('@jess/context')
local worker = require('@jess/worker')
local l10n = require('@jess/l10n')
local input = require('@jess/input')

local log = {}

local function note(line: string)
  table.insert(log, line)
end

-- Localization is the runtime's own: a registry over bundles the script holds.
l10n.addProvider(l10n.files('kernel', {
  ['l10n/common/en.toml'] = [[hello = 'Hello, {name}!']],
  ['l10n/common/pl.toml'] = [[hello = 'Cześć, {name}!']],
}, 'l10n'))
assert(#l10n.reload() == 0)

local common = l10n.bundle 'common'
local runtime = input.runtime()
local router = input.router()
local greeter

local calls = {}

context.bind('main', {
  startup = function()
    -- The live runtime: the host ticks it, the script configures it.
    runtime:install(input.graph {
      actions = {
        { name = 'ui.accept', valueType = 'digital' },
        { name = 'quickSave', valueType = 'digital' },
      },
      sets = {
        {
          name = 'ui',
          owner = '@test/kernel',
          bindings = {
            { action = 'ui.accept', input = input.key 'Enter' },
            { action = 'quickSave', input = input.chord { input.key 'ControlLeft', input.key 'KeyS' } },
          },
        },
      },
    })
    runtime:setActive 'ui'
    router:route('ui.accept', 'pressed', 'ui.activate')
    note(`startup {common('hello', { name = 'Fargoth' })} gen {l10n.generation()}`)

    -- A worker that localizes with a registry of its own and has no say over input: it sees
    -- the domain's services but holds no capability.
    greeter = worker.spawn {
      name = 'greeter',
      capabilities = {},
      shareServices = true,
      source = [[
        local l10n = require('@jess/l10n')
        l10n.addProvider(l10n.files('greeter', { ['common/en.toml'] = [=[hello = 'Hello, {name}!']=] }))
        l10n.reload()
        local common = l10n.bundle 'common'

        return function(message)
          local ok, problem = pcall(function()
            return jess.input.runtime():action 'ui.accept'
          end)
          return { text = common('hello', { name = message }), input = ok and 'allowed' or tostring(problem) }
        end
      ]],
    }
  end,

  update = function(delta)
    local state = runtime:action 'ui.accept'
    note(`update accept={tostring(state.active)}`)
  end,

  event = function(payload)
    note(`event {payload.event}`)
  end,

  shutdown = function()
    note('shutdown')
  end,
})

context.bind('input', {
  input = function(payload)
    for _, event in payload.events do
      note(`input {event.action} {event.kind} device {event.device} gen {payload.generation}`)
    end

    for _, intent in router:dispatch(payload.events) do
      note(`intent {intent.intent} <- {intent.action} {intent.kind} value {tostring(intent.value)}`)
    end

    -- This context holds input.actions only: it may read input and nothing else.
    local ok, problem = pcall(runtime.setActive, runtime, 'ui')
    note(`input-context configure {ok and 'allowed' or problem}`)
  end,
})

function calls.switchLocale(tag: string)
  note(`setLocales {l10n.setLocales(tag)} now {common('hello', { name = 'Fargoth' })}`)
end

function calls.pressEnter()
  -- Synthetic events on the live runtime need input.configure, which this context holds.
  local keyboard = runtime:connect 'keyboard'
  runtime:press(keyboard, 'key:Enter')
end

function calls.askGreeter(name: string)
  greeter:send(name)
  local reply = greeter:receive(20)
  note(`greeter {reply.text} / {reply.input}`)
end

return function()
  return table.concat(log, '\n')
end, function(payload)
  calls[payload.call](payload.argument)
end
";

/// The second generation: a smaller program that only reports what it sees.
const MAIN_2: &str = r"
local context = require('@jess/context')

local log = {}

context.bind('main', {
  update = function()
    table.insert(log, `update2 graph={tostring(jess.input.runtime():action 'ui.accept' ~= nil)}`)
  end,

  event = function(payload)
    table.insert(log, `event2 {payload.event}`)
  end,

  shutdown = function()
    table.insert(log, 'shutdown2')
  end,
})

context.bind('input', {
  input = function(payload)
    table.insert(log, `input2 {#payload.events}`)
  end,
})

return function()
  return table.concat(log, '\n')
end
";

/// Loads `source` as the domain's code and keeps what the chunk returned: the log reader, and
/// for the first generation the `onDemand` handler, bound to `main`.
fn load(host: &mut Host, source: &str, with_calls: bool) -> Function {
    let functions: RefCell<Option<(Function, Option<Function>)>> = RefCell::new(None);
    let body = |runtime: &Runtime| {
        let chunk = jess::runtime::chunk::load(runtime, "@test/kernel/main.jsl", source)?;
        let returned = if with_calls {
            let (log, calls) = chunk.invoke::<(Function, Function), ()>(&runtime.stack(), ())?;
            (log, Some(calls))
        } else {
            (chunk.invoke::<Function, ()>(&runtime.stack(), ())?, None)
        };
        *functions.borrow_mut() = Some(returned);
        Ok(())
    };
    if host.domain().state() == DomainState::Created {
        host.domain().load(body).unwrap();
    } else {
        host.restart(body).unwrap();
    }
    let (log, calls) = functions.into_inner().expect("the chunk ran");
    if let Some(calls) = calls {
        host.domain().bind_handler("main", Phase::OnDemand, calls).unwrap();
    }
    log
}

/// The script's log so far, read at host level: no authority is needed to read a local.
fn log(host: &Host, reader: &Function) -> Vec<String> {
    host.domain()
        .with_runtime(|runtime| reader.invoke::<String, ()>(&runtime.stack(), ()))
        .expect("the domain has a runtime")
        .unwrap_or_else(|error| panic!("{error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Calls one of the script's entry points under the `main` context's authority, through the
/// `onDemand` handler the chunk returned.
fn call(host: &Host, function: &str, argument: Message) {
    let payload = Message::map([("call", Message::text(function)), ("argument", argument)]);
    let report = host.domain().run_phase(Phase::OnDemand, &payload).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
}

#[test]
#[allow(clippy::too_many_lines)] // One walk through the whole kernel, in the order an application meets it.
fn the_application_kernel_end_to_end() {
    let diagnostics = Arc::new(CollectedDiagnostics::new());
    let sink: Arc<dyn DiagnosticSink> = diagnostics.clone();
    let factory: Arc<dyn RuntimeFactory> = Arc::new(|| jess::lua::runtime(Options::default()));

    // 1. One host domain from jess's plan, restricted grant, input attached.
    let grant =
        RootAuthority::new().grant(CapabilitySet::from_names(["input.actions", "input.configure", WORKER_SPAWN]).unwrap());
    let mut host = Host::create(
        factory,
        HostSpec::new(DomainSpec::new("app", grant).diagnostics(sink)).input(InputSource::Synthetic),
    )
    .unwrap();
    assert!(host.domain().service::<SharedInput>().is_some());

    // 2. Two contexts: the application's, and an input context holding only input.actions.
    let main = host
        .domain()
        .register_context(ContextSpec::new("main").schedule([
            Phase::Startup,
            Phase::Update,
            Phase::Event,
            Phase::OnDemand,
            Phase::Shutdown,
        ]))
        .unwrap();
    let input = host
        .domain()
        .register_context(
            ContextSpec::new("input")
                .schedule([Phase::Input])
                .capabilities(CapabilitySet::from_names(["input.actions"]).unwrap()),
        )
        .unwrap();
    let reader = load(&mut host, MAIN, true);

    // 3. Startup: the script installs the graph, formats a message, spawns the worker.
    let started = host.domain().start().unwrap();
    assert!(started.failures.is_empty(), "{:?}", started.failures);
    assert_eq!(log(&host, &reader), ["startup Hello, Fargoth! gen 1"]);
    assert_eq!(host.domain().workers().len(), 1);
    assert!(host.input().unwrap().lock().unwrap().runtime().graph().is_some());

    // 4. The worker formats from its own registry and is refused input: it holds no grant.
    call(&host, "askGreeter", Message::text("Fargoth"));
    let greeted = log(&host, &reader).pop().unwrap();
    let worker = host.domain().workers()[0].id;
    assert_eq!(
        greeted,
        format!(
            "greeter Hello, Fargoth! / capability 'input.actions' denied to {}",
            host.domain().attribution().in_worker(worker)
        )
    );

    // 5. Keys pressed on the host's synthetic side become action events in the input phase,
    //    with provenance and the graph generation, and the script's router makes intents of
    //    them; the input context's own capability checks hold.
    let keyboard = {
        let mut service = host.input().unwrap().lock().unwrap();
        let keyboard = service.synthetic().connect_keyboard();
        service.advance(Duration::from_millis(16));
        service.synthetic().press(keyboard, Control::Key(Key::Enter));
        keyboard
    };
    let frame = host.frame(Duration::from_millis(16)).unwrap();
    assert!(frame.failures().is_empty(), "{:?}", frame.failures());
    let input_attribution = host.domain().attribution().in_context(input);
    assert_eq!(
        log(&host, &reader),
        [
            "startup Hello, Fargoth! gen 1".to_owned(),
            greeted,
            "input ui.accept pressed device 0 gen 1".to_owned(),
            "intent ui.activate <- ui.accept pressed value true".to_owned(),
            format!("input-context configure capability 'input.configure' denied to {input_attribution}"),
            "update accept=true".to_owned(),
        ]
    );
    let denials = diagnostics.of_kind(DiagnosticKind::CapabilityDenied);
    let denied: Vec<(&str, jess::runtime::Attribution)> = denials
        .iter()
        .map(|denial| (denial.capability.unwrap().name(), denial.attribution))
        .collect();
    assert_eq!(
        denied,
        [
            ("input.actions", host.domain().attribution().in_worker(worker)),
            ("input.configure", input_attribution),
        ]
    );

    // 6. The release is an edge too; then the script presses a key itself, on a second keyboard:
    //    synthetic injection on the live runtime, under input.configure.
    {
        let mut service = host.input().unwrap().lock().unwrap();
        service.synthetic().release(keyboard, Control::Key(Key::Enter));
        service.advance(Duration::from_millis(16));
    }
    let frame = host.frame(Duration::from_millis(16)).unwrap();
    assert_eq!(frame.input.map(|phase| phase.ran), Some(1));
    assert!(log(&host, &reader).contains(&"input ui.accept released device 0 gen 1".to_owned()));
    call(&host, "pressEnter", Message::Nil);
    host.input().unwrap().lock().unwrap().advance(Duration::from_millis(16));
    let frame = host.frame(Duration::from_millis(16)).unwrap();
    assert_eq!(frame.input.map(|phase| phase.ran), Some(1));
    let entries = log(&host, &reader);
    assert!(
        entries.contains(&"input ui.accept pressed device 1 gen 1".to_owned()),
        "{entries:?}"
    );

    // 7. A locale switch from the script raises its registry's generation and changes the text at
    //    once: no reload, and nothing for the host to do.
    call(&host, "switchLocale", Message::text("pl-PL"));
    let entries = log(&host, &reader);
    assert_eq!(entries.last().unwrap(), "setLocales 2 now Cześć, Fargoth!");

    // 8. A restart: the old generation's handlers, worker, graph, and queued events go; the
    //    devices stay; a key held across the restart is not the new code's press.
    host.input()
        .unwrap()
        .lock()
        .unwrap()
        .synthetic()
        .press(keyboard, Control::Key(Key::ControlLeft));
    host.domain()
        .mailbox()
        .unwrap()
        .post(EventTarget::Context(main), Message::map([("event", Message::text("stale"))]))
        .unwrap();
    drop(reader);
    let reader = load(&mut host, MAIN_2, false);
    assert_eq!(host.domain().generation().raw(), 2);
    assert_eq!(host.domain().state(), DomainState::Running);
    assert!(host.domain().workers().is_empty(), "the worker went with its generation");
    assert!(host.input().unwrap().lock().unwrap().runtime().graph().is_none());
    host.input().unwrap().lock().unwrap().advance(Duration::from_millis(16));
    let frame = host.frame(Duration::from_millis(16)).unwrap();
    assert!(frame.input.is_none(), "no graph, no action events");
    assert_eq!((frame.tick.events.stale, frame.tick.events.delivered), (1, 0));
    assert_eq!(log(&host, &reader), ["update2 graph=false"]);
    assert_eq!(host.domain().stats().stale_discarded, 1);
    assert_eq!(diagnostics.of_kind(DiagnosticKind::StaleGenerationDiscarded).len(), 1);
    assert!(
        host.input().unwrap().lock().unwrap().device(keyboard.index()).is_some(),
        "so did the keyboard"
    );

    // 9. Teardown: shutdown ran, nothing is owned, and the services are the host's to drop.
    drop(reader);
    let teardown = host.stop().unwrap();
    assert_eq!(teardown.shutdown_failures, []);
    assert_eq!(
        (
            teardown.workers_stopped,
            teardown.resources_released,
            teardown.contexts_removed
        ),
        (0, 0, 2)
    );
    let denials = diagnostics.of_kind(DiagnosticKind::CapabilityDenied).len();
    assert_eq!(
        denials,
        1 + 3,
        "the worker's one, and the input context's one per input phase, in three phases"
    );
    let worker_failures = diagnostics.of_kind(DiagnosticKind::WorkerFailed);
    assert!(worker_failures.is_empty(), "{worker_failures:?}");
}

/// The same input service type the host attaches is what `jess.input.runtime()` binds to, so a
/// host that configures input itself and a script that reads it agree on one object.
#[test]
fn the_service_types_are_the_contract() {
    let service: Arc<SharedInput> = Arc::new(Mutex::new(jess::input::InputService::new()));
    assert_eq!(
        std::any::type_name::<SharedInput>(),
        std::any::type_name::<Mutex<jess::input::InputService>>()
    );
    drop(service);
}

/// `@jess/log` under a domain with a log stream: a context's records and a worker's arrive in
/// the one stream, attributed by the runtime, and none reach the runtime's own sink.
#[test]
fn a_domain_log_stream_collects_its_contexts_and_its_workers() {
    const SOURCE: &str = r"
local context = require('@jess/context')
local worker = require('@jess/worker')
local log = require('@jess/log')

context.bind('main', {
  startup = function()
    log.new('app', { stage = 'startup' }).info('started', { count = 2, ready = true })

    local lane = worker.spawn {
      name = 'logger',
      source = [[
        local log = require('@jess/log')

        return function(message)
          log.new('lane').warn(`got {message}`)
          return 'ok'
        end
      ]],
    }

    lane:send 'work'
    assert(lane:receive(20) == 'ok')
  end,
})
";
    let logs = Arc::new(jess::runtime::CollectedLogs::new());
    let factory: Arc<dyn RuntimeFactory> = Arc::new(|| {
        let runtime = jess::lua::runtime(Options::default())?;
        jess::lua::stdlib::set_log_sink(&runtime, |record| panic!("the runtime's own sink saw {record:?}"))?;
        Ok(runtime)
    });
    let grant = RootAuthority::new().grant(CapabilitySet::from_names([WORKER_SPAWN]).unwrap());
    let domain = jess::runtime::ScriptDomain::create(factory, DomainSpec::new("logs", grant).logs(logs.clone())).unwrap();
    let main = domain
        .register_context(ContextSpec::new("main").schedule([Phase::Startup]))
        .unwrap();
    domain.load_source("@test/kernel/logs.jsl", SOURCE).unwrap();
    let started = domain.start().unwrap();
    assert!(started.failures.is_empty(), "{:?}", started.failures);
    let worker = domain.workers()[0].id;

    let records = logs.drain();
    let summary: Vec<String> = records
        .iter()
        .map(|record| {
            format!(
                "{} {} {}: {}",
                record.level, record.attribution, record.target, record.message
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            format!("info {} app: started", domain.attribution().in_context(main)),
            format!("warn {} lane: got work", domain.attribution().in_worker(worker)),
        ]
    );
    assert_eq!(
        records[0].fields,
        [
            ("count".to_owned(), Message::Number(2.0)),
            ("ready".to_owned(), Message::Boolean(true)),
            ("stage".to_owned(), Message::text("startup")),
        ]
        .into()
    );
    domain.stop().unwrap();
}

/// Under a domain `jess.granted` answers from the broker, whatever the runtime's own plan grants,
/// and asking is no denial.
#[test]
fn granted_asks_the_broker_under_a_domain() {
    let logs = Arc::new(jess::runtime::CollectedLogs::new());
    let factory: Arc<dyn RuntimeFactory> = Arc::new(|| jess::lua::runtime(Options::default()));
    let grant = RootAuthority::new().grant(CapabilitySet::from_names(["process.environment"]).unwrap());
    let domain = jess::runtime::ScriptDomain::create(factory, DomainSpec::new("asking", grant).logs(logs.clone())).unwrap();
    domain
        .register_context(ContextSpec::new("main").schedule([Phase::Startup]))
        .unwrap();
    domain
        .load_source(
            "@test/kernel/granted.jsl",
            "require('@jess/log').new('asking').info(`{jess.granted('process.environment')} {jess.granted('process.spawn')}`)",
        )
        .unwrap();
    let messages = logs.drain().into_iter().map(|record| record.message).collect::<Vec<_>>();
    assert_eq!(messages, ["true false"]);
    assert_eq!(domain.stats().capability_denials, 0);
}

/// A script under a host asks it to stop through `@jess/application`; the host reads the status,
/// a restart forgets it, and nothing outside a host, or in a worker, can ask.
#[test]
fn a_script_asks_its_host_to_stop() {
    let factory: Arc<dyn RuntimeFactory> = Arc::new(|| jess::lua::runtime(Options::default()));
    let grant = RootAuthority::new().grant(CapabilitySet::from_names([WORKER_SPAWN]).unwrap());
    let mut host = Host::create(factory, HostSpec::new(DomainSpec::new("stopping", grant))).unwrap();
    host.domain()
        .register_context(ContextSpec::new("main").schedule([Phase::Update, Phase::OnDemand]))
        .unwrap();
    host.domain()
        .load_source(
            "@test/kernel/exit.jsl",
            r"
            local application = require('@jess/application')
            local worker = require('@jess/worker')
            local frames = 0

            require('@jess/context').bind('main', {
              update = function()
                frames += 1
                if frames == 2 then application.exit(3) end
              end,
              onDemand = function(case)
                if case == 'bad' then
                  application.exit(2.5)
                end

                local asker = worker.spawn {
                  shareServices = true,
                  source = [[return function() require('@jess/application').exit(1) end]],
                }
                asker:send(true)
                -- The worker's refusal comes back here, and fails this handler with it.
                asker:receive(20)
              end,
            })
            ",
        )
        .unwrap();
    host.domain().start().unwrap();
    host.frame(Duration::from_millis(16)).unwrap();
    assert_eq!(host.exit_requested(), None);
    host.frame(Duration::from_millis(16)).unwrap();
    assert_eq!(host.exit_requested(), Some(3));

    let refused = |case: &str| {
        let report = host.domain().run_phase(Phase::OnDemand, &Message::text(case)).unwrap();
        report.failures[0].to_string()
    };
    let bad = refused("bad");
    assert!(
        bad.contains("application.exit: the status is an integer from 0 to 255, not 2.5"),
        "{bad}"
    );
    let from_worker = refused("worker");
    assert!(
        from_worker.contains("application.exit is for the domain's own code"),
        "{from_worker}"
    );
    assert_eq!(host.exit_requested(), Some(3));

    host.restart(|_| Ok(())).unwrap();
    assert_eq!(host.exit_requested(), None, "a new generation hasn't asked anything");
    host.stop().unwrap();

    let outside = jess::lua::runtime(Options::default()).unwrap();
    let problem = outside
        .eval::<()>("require('@jess/application').exit()")
        .unwrap_err()
        .to_string();
    assert!(problem.contains("application.exit: needs an application host"), "{problem}");
}
