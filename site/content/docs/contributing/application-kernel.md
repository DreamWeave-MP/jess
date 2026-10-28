---
title: 'Application Kernel'
description: 'What jess::host is: a script domain with its live input service, the frame a host runs, how the input phase arrives, and what a capability denial looks like.'
weight: 102
extra:
  kind: guide
---

The substrates exist on their own terms. [Script Domains](@/docs/contributing/script-domains.md) run code and know nothing about devices or text. [Input](@/docs/contributing/input-runtime.md) knows devices and action graphs and has never heard of a domain. [Localization](@/docs/contributing/localization.md) is a Luau module with a registry in every runtime, so it isn't a host's business at all. Something has to put one application together out of them, and it should be small, because the GUI, the render executor, and every application above them will stand on it.

`jess::host` (`src/host`) is that something: one `ScriptDomain`, the services it carries, and a frame.

## What a Host Is

A `Host` owns:

- a `ScriptDomain` built from any `RuntimeFactory` (`jess::lua::runtime` is the one jess uses);
- the domain's **input service**, a `Mutex<jess::input::InputService>`: the live `InputRuntime` and a synthetic side;
- the clock the input ticks on: the monotonic clock behind a platform backend, or the synthetic side's when there is none.

It declares no contexts, reserves no action names, formats nothing itself, and draws nothing. The application registers contexts on `host.domain()`, loads its code, starts it, and then calls `host.frame(delta)` until it is done. `Host::create` leaves the domain `Created`, like `ScriptDomain::create` does.

```rust
let grant = RootAuthority::new().grant(CapabilitySet::from_names(["input.actions", "input.configure"])?);
let mut host = Host::create(
    Arc::new(|| jess::lua::runtime(Options::default())),
    HostSpec::new(DomainSpec::new("editor", grant))
        .input(InputSource::platform()?),
)?;
host.domain().register_context(ContextSpec::new("main").schedule([Phase::Startup, Phase::Update, Phase::Event, Phase::Shutdown]))?;
host.domain().register_context(ContextSpec::new("input").schedule([Phase::Input]).capabilities(CapabilitySet::from_names(["input.actions"])?))?;
host.domain().load_source("@editor/main.jsl", &source)?;
host.domain().start()?;
```

An [application target](@/docs/scripting/applications.md) is this, assembled from a manifest: `jess::lua::package::launch` builds a `Host`, loads every selected package's managed modules into its domain, binds their callbacks, and drives frames until the application asks to stop.

## Services: What a Domain Carries

The runtime has one piece of mechanism for this: a typed service slot. `DomainSpec::service(Arc<T>)` attaches any `Send + Sync` value by type; `jess::runtime::service::<T>()` finds it again from a binding, through the same thread-scoped ticket `current_authority()` reads, so it is `Some` exactly while script code runs under the domain and `None` for host code calling a runtime directly. The runtime never looks inside a service and depends on no service type. The *type* is the contract: the host attaches a `Mutex<InputService>` and `jess.input` looks up a `Mutex<InputService>`, and the two modules agree on nothing else.

Services are the host's, not the script generation's. A `restart` keeps them; what a generation configured in one (the graph it installed) is the host's to reset, and `Host::restart` does. A worker sees only what its spawner shares: `WorkerSpec::share::<T>()` by type, `share_all()` for everything, or from Luau `worker.spawn { shareServices = true }`, since a script cannot name a Rust type. A worker not given a service has none, and the jess modules fall back to state of the worker's own.

`jess.input.runtime()` follows the same rule. Under a domain that carries the service, it binds to it; anywhere else (a plain `jess run`, a domain without the service, a worker not given it) each runtime keeps a headless input runtime of its own, and the API is the same. The contract fixtures run outside any domain and did not change.

## The Frame

`Host::frame(delta)` is two steps, always in this order. `pump_input()` is the first on its own, for a host with its own loop shape, and `FrameReport` carries the input `PhaseReport` and the `TickReport`.

### Input

The host polls the backend into the service's queue, ticks the service once at the host's clock, and, if the tick produced action events, runs `Phase::Input` with this payload:

```luau
local payload = {
  time = 0.016, -- the tick's time on the host's clock, in seconds
  generation = 1, -- the graph generation the events came from
  events = { { action = 'ui.accept', kind = 'pressed', value = true, time = 0.016, device = 0 } },
}
```

The rows are exactly what `InputRuntime:tick()` returns to a script, so reading a phase payload and reading a headless tick are the same code, and a script's `@jess/input` router makes intents of either with one `router:dispatch(payload.events)`. `Phase::Input` is the runtime's existing generic phase for this; nothing was added. A quiet tick enters no Luau at all, which is the no-hidden-per-frame-Luau rule applied to input: a stable frame with nobody touching a key runs no handler.

### Advance

`ScriptDomain::advance(delta)`: fixed steps, update, late update, and the event drain.

## Input Sources

| `InputSource` | Clock | For |
| --- | --- | --- |
| `Backend(Box<dyn Backend>)`, `platform()` | monotonic | real devices: evdev with `linux-evdev`, gilrs with `gilrs`, the stub otherwise |
| `stub()` | monotonic | the backend path with no devices behind it |
| `Synthetic` | the service's synthetic side, moved by `InputService::advance` | tests, replays, headless runs |

A synthetic host's frames read the synthetic clock and never move it: `frame(delta)` advances the domain by `delta`, and input time moves only when whoever injects the events calls `InputService::advance`. That is a decision, not an accident. A replay keeps its recorded timing whatever frame cadence plays it back, and a tap or a hold is decided by input time, not by how many frames ran. A headless run that wants one fake clock for everything advances both by the same amount. `tests/host/kernel.rs` pins it.

The device features are the input module's and off by default. No test opens a real device: the backends are compile-checked, and every fixture presses keys through `host.input().lock().synthetic()` or through `InputRuntime:press` from a script holding `input.configure`.

On a live runtime, `InputRuntime:tick()` and `InputRuntime:advance()` are refused (`InputRuntime:tick is the host's on a live input runtime: it ticks in the input phase; bind a context to that phase and read the payload`): the host owns the tick and the clock.

## Logs

The host adds no logging of its own: the application's log stream is the domain's. Give the `DomainSpec` a `LogSink` (`DomainSpec::logs`) and every `@jess/log` record from every context and every worker arrives there as a `jess::runtime::LogRecord`, with the domain, generation, and context or worker that wrote it. That stream, the diagnostics, and the profile events are what an Output, Problems, Logs, or Profiler view reads. An application's diagnostics sit in a bounded sink that reports a run of identical ones as the first and a count, so an embedder that never drains them doesn't grow without end.

```rust
let logs = Arc::new(CollectedLogs::new());
let spec = DomainSpec::new("editor", grant).diagnostics(diagnostics).logs(logs.clone());
```

## Capabilities: Who May Do What

Every native call that reaches outside the VM is behind a named capability, checked at the script boundary through `jess::runtime::require_capability`. The [capabilities reference](@/docs/reference/capabilities.md) has the whole list; the input ones are the host's own concern:

| Capability | `jess.input` |
| --- | --- |
| `input.actions` | `action`, `stats`, `now`, `tick` |
| `input.observe` | `devices` |
| `input.configure` | `install`, `uninstall`, `restoreDefaults`, `setActive`, `pushLayer`, `popLayer`, `connect`, `disconnect`, `press`, `release`, `axis`, `focus`, `advance` |

The rule for the boundary: **someone always decides.** Under a domain, the check is the broker's, and a context or worker without the capability gets the broker's permission error, attributed:

```text
capability 'input.actions' denied to domain#1 gen#1 context#2
```

The denial is also counted (`DomainStats::capability_denials`) and reported as a `DiagnosticKind::CapabilityDenied` carrying the capability and the attribution. Outside any domain the runtime's own grant decides, the same set its l3i plan was built from: `jess run` and `jess test` grant a script its terminal (`process.stdio`), the input devices in front of the person running it (`input.actions`, `input.observe`, `input.configure`), and their GPU (`gpu.device`), since they already have the keyboard and the screen; a package target holds what it declares, once the operator grants it; and a library caller grants what it means to (`jess::lua::host_script_grant`), or gets `jess.input: needs the 'input.configure' capability, which this runtime does not grant`. Everything else in `@jess/input` (the builders, `graph`, `fromJson`, labels, presentation, conflicts, and the router) checks nothing, because it touches no device and no runtime: it is data.

What needs no capability: anything that works on values already in the VM (the codecs, `@jess/style`, `jess.shader`, `@jess/l10n`'s formatting), `print` and `@jess/log`, `os.time` and `os.clock`, and `require` of a module beside the caller or exported by an activated package, which reads through the script's folder or the package's provider rather than through `@dream/fs`. A package's modules may require only what its manifest declares, so that read is bounded by the plan, not by a grant.

## Restart

`Host::restart(load)` is `ScriptDomain::restart` with one thing in front of it: the input service forgets the graph the old code installed, and keeps the devices and the queue, because the devices did not restart. Events queued for the old generation are discarded by the domain and counted; the old code's routers went with its Luau state; a key held across the restart is invisible to the new code's graph until it is released, the same going-live rule a set change applies.

It is as destructive as the domain's restart: the graph is forgotten and the old generation ends before the new code loads, and a load that fails leaves nothing running. A reload that keeps the last good generation alive is not this; see [Generations and Restart](@/docs/contributing/script-domains.md#generations-and-restart).

## Proof

Two fixtures, both headless:

- `tests/host/kernel.rs` drives a host with the runtime's own modules and a journal: the quiet frame, the press that becomes an input phase with its generation, the restart that keeps the keyboard and drops the graph and the stale event, the stub backend's poll loop.
- `tests/lua/kernel.rs` is the integration fixture over jess's real plan: one host domain with a live input runtime; a `main` context and an `input` context holding `input.actions` alone; a script that installs the graph and formats a message from its own `@jess/l10n` registry at startup; a worker spawned with `shareServices = true` and no capabilities that formats from a registry of its own and is denied input; keys pressed on the host's synthetic side and from the script, arriving as action events and, through the script's router, intents; a locale switch that formats in Polish at once; the input context's denial in every input phase; a restart to a second generation with the held key, the graph, and a queued event gone; and a teardown that owns nothing.

## Where It Lives

- `src/host/mod.rs`: `Host`, `HostSpec`, `FrameReport`, `SharedInput`.
- `src/host/input.rs`: `InputSource`, the driver, `payload`.
- `src/runtime/service.rs`: `Services`; `DomainSpec::service`, `ScriptDomain::service`, `jess::runtime::service`, `WorkerSpec::share`.
- `src/input/service.rs`: `InputService`.
- `src/lua/input.rs`: the module that binds to the service and checks the capabilities.
- `src/lua/application.rs`: the supervisor that turns an application target into a host, its managed modules, and a frame loop.
