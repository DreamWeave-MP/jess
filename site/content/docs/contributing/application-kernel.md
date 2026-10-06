---
title: 'Application Kernel'
description: 'What jess-host is: a script domain with its live input and localization services, the frame a host runs, how the input phase and locale events arrive, and what a capability denial looks like.'
weight: 102
extra:
  kind: guide
---

The substrates exist on their own terms. [Script Domains](@/docs/contributing/script-domains.md) run code and know nothing about devices or text. [Input](@/docs/contributing/input-runtime.md) knows devices and action graphs and has never heard of a domain. [Localization](@/docs/contributing/localization.md) formats messages and does not care who asks. Something has to put one application together out of them, and it should be small, because the GUI, the render executor, and every application above them will stand on it.

`jess-host` (`libs/host`) is that something: one `ScriptDomain`, the services it carries, and a frame.

## What a Host Is

A `Host` owns:

- a `ScriptDomain` built from any `RuntimeFactory` (`jess_lua::runtime` is the one jess uses);
- the domain's **input service**, a `Mutex<jess_input::InputService>`: the live `InputRuntime`, its intent routes by name, and a synthetic side;
- the domain's **localization**, a `RwLock<jess_l10n::Registry>`;
- the clock the input ticks on: the monotonic clock behind a platform backend, or the synthetic side's when there is none.

It declares no contexts, reserves no action names, formats nothing itself, and draws nothing. The application registers contexts on `host.domain()`, loads its code, starts it, and then calls `host.frame(delta)` until it is done. `Host::create` leaves the domain `Created`, like `ScriptDomain::create` does.

```rust
let grant = RootAuthority::new().grant(CapabilitySet::from_names(["input.actions", "input.configure", "l10n.read"])?);
let mut host = Host::create(
    Arc::new(|| jess_lua::runtime(Options::default())),
    HostSpec::new(DomainSpec::new("editor", grant))
        .input(InputSource::platform()?)
        .localization(registry),
)?;
host.domain().register_context(ContextSpec::new("main").schedule([Phase::Startup, Phase::Update, Phase::Event, Phase::Shutdown]))?;
host.domain().register_context(ContextSpec::new("input").schedule([Phase::Input]).capabilities(CapabilitySet::from_names(["input.actions"])?))?;
host.domain().load_source("@editor/main.luau", &source)?;
host.domain().start()?;
```

## Services: What a Domain Carries

The runtime gained one piece of mechanism for this: a typed service slot. `DomainSpec::service(Arc<T>)` attaches any `Send + Sync` value by type; `jess_runtime::service::<T>()` finds it again from a binding, through the same thread-scoped ticket `current_authority()` reads, so it is `Some` exactly while script code runs under the domain and `None` for host code calling a runtime directly. The runtime never looks inside a service and depends on no service crate. The *type* is the contract: `jess-host` attaches a `Mutex<InputService>` and `jess.input` looks up a `Mutex<InputService>`, and the two crates agree on nothing else.

Services are the host's, not the script generation's. A `restart` keeps them; what a generation configured in one (the graph it installed) is the host's to reset, and `Host::restart` does. A worker sees only what its spawner shares: `WorkerSpec::share::<T>()` by type, `share_all()` for everything, or from Luau `worker.spawn { shareServices = true }`, since a script cannot name a Rust type. A worker not given a service has none, and the jess modules fall back to state of the worker's own.

`jess.input.runtime()` and `jess.l10n` follow the same rule. Under a domain that carries the service, they bind to it; anywhere else (a plain `jess run`, a domain without the service, a worker not given it) each runtime keeps a headless input runtime and a registry of its own, and the API is the same. The contract fixtures run outside any domain and did not change.

## The Frame

`Host::frame(delta)` is three steps, always in this order. `pump_input()` and `pump_localization()` are the first two on their own, for a host with its own loop shape, and `FrameReport` carries the input `PhaseReport`, the localization generation broadcast, and the `TickReport`.

### Localization

If the registry's generation moved since the last frame (a locale change, a reload that swapped a bundle in, whoever made it: a script through `jess.l10n.setLocales`, or the host through `host.localization()`), the host posts one event to every context through the domain's mailbox:

```luau
local payload = { event = 'l10n.changed', generation = 2, locales = { 'pl-PL', 'en' } }
```

`generation` is the number `jess.l10n.generation()` returns; `locales` are the preferred locales as set. It arrives in this frame's event drain, stamped with the domain generation at posting, so it never crosses a restart. A context that cached formatted text formats again on it. The host does not intercept writes to the registry; it watches the one number the registry already keeps for exactly this.

### Input

The host polls the backend into the service's queue, ticks the service once at the host's clock, and, if the tick produced action events, runs `Phase::Input` with this payload:

```luau
local payload = {
  time = 0.016, -- the tick's time on the host's clock, in seconds
  generation = 1, -- the graph generation the events came from
  events = { { action = 'ui.accept', kind = 'pressed', value = true, time = 0.016, device = 0 } },
  intents = { { intent = 'ui.activate', action = 'ui.accept', kind = 'pressed', value = true, time = 0.016, device = 0 } },
}
```

The rows are exactly what `InputRuntime:tick()` and `InputRuntime:intents()` return to a script, so reading a phase payload and reading a headless tick are the same code. `Phase::Input` is the runtime's existing generic phase for this; nothing was added. A quiet tick enters no Luau at all, which is the no-hidden-per-frame-Luau rule applied to input: a stable frame with nobody touching a key runs no handler.

### Advance

`ScriptDomain::advance(delta)`: fixed steps, update, late update, and the event drain that delivers the localization event.

## Input Sources

| `InputSource` | Clock | For |
| --- | --- | --- |
| `Backend(Box<dyn Backend>)`, `platform()` | monotonic | real devices: evdev with `linux-evdev`, gilrs with `gilrs`, the stub otherwise |
| `stub()` | monotonic | the backend path with no devices behind it |
| `Synthetic` | the service's synthetic side, moved by `InputService::advance` | tests, replays, headless runs |

A synthetic host's frames read the synthetic clock and never move it: `frame(delta)` advances the domain by `delta`, and input time moves only when whoever injects the events calls `InputService::advance`. That is a decision, not an accident. A replay keeps its recorded timing whatever frame cadence plays it back, and a tap or a hold is decided by input time, not by how many frames ran. A headless run that wants one fake clock for everything advances both by the same amount. `libs/host/tests/kernel.rs` pins it.

The device features are forwarded to `jess-input` and off by default. No test opens a real device: the backends are compile-checked, and every fixture presses keys through `host.input().lock().synthetic()` or through `InputRuntime:press` from a script holding `input.configure`.

On a live runtime, `InputRuntime:tick()` and `InputRuntime:advance()` are refused (`InputRuntime:tick is the host's on a live input runtime: it ticks in the input phase; bind a context to that phase and read the payload`): the host owns the tick and the clock.

## Logs

The host adds no logging of its own: the application's log stream is the domain's. Give the `DomainSpec` a `LogSink` (`DomainSpec::logs`) and every `@jess/log` record from every context and every worker arrives there as a `jess_runtime::LogRecord`, with the domain, generation, and context or worker that wrote it. That stream, the diagnostics, and the profile events are what an Output, Problems, Logs, or Profiler view reads.

```rust
let logs = Arc::new(CollectedLogs::new());
let spec = DomainSpec::new("editor", grant).diagnostics(diagnostics).logs(logs.clone());
```

## Capabilities: Who May Do What

The capabilities the input and localization crates named are now checked, at the script boundary, by `jess.input` and `jess.l10n`, through `jess_runtime::require_capability`:

| Capability | `jess.input` | `jess.l10n` |
| --- | --- | --- |
| `input.actions` | `action`, `intents`, `stats`, `now`, `tick` | |
| `input.observe` | `devices` | |
| `input.configure` | `install`, `uninstall`, `restoreDefaults`, `setActive`, `pushLayer`, `popLayer`, `route`, `connect`, `disconnect`, `press`, `release`, `axis`, `focus`, `advance` | |
| `l10n.read` | | `bundle`, `bundles`, `locales`, `generation`, calling a bundle, `format`, `has` |
| `l10n.configure` | | `setLocales`, `addDirectory`, `reload` |

The rule for the boundary: **no authority, no check.** Under a domain, the check is the broker's, and a context or worker without the capability gets the broker's permission error, attributed:

```text
capability 'input.actions' denied to domain#1 gen#1 context#2
```

The denial is also counted (`DomainStats::capability_denials`) and reported as a `DiagnosticKind::CapabilityDenied` carrying the capability and the attribution. Outside any domain (a plain `jess run`, a CLI command, a test evaluating a runtime directly) nothing holds authority and every call succeeds: the process's own user is running the script, and they already have the keyboard. The authoring functions (`jess.input.key`, `chord`, `graph`, `fromJson`, and the graph methods) check nothing, because they touch no device and no runtime: they are data.

### What Is Not Checked Yet

Input, localization, and `worker.spawn` are the whole list. Everything older than the broker runs under a domain exactly as it runs in `jess run`: with the process's own file permissions. The capability model is not a security boundary for third-party packages until every surface below checks something:

| Surface | Reaches |
| --- | --- |
| `jess.fs` (`write`, `replace`, `copy`, `remove`, `createDir`; `read`, `list`, `stat`, `cwd`, ...) | any path the process can, both ways |
| `Plugin:save`, `jess.plugin.convert`, `jess.save.strip`, `jess.nif.fix`, `Nif:save` | writes at any path, or in place |
| `jess.plugin.open`, `jess.save.open`, `jess.nif.open`, `jess.compare.bytes`, `jess.openmw.load` | reads at any path, or through the config's data folders |
| `OpenMwContext:save`, `openmwConfig` `saveUser`/`saveToPath`/`saveSubconfig`/`saveResolvedToPath` | writes the user's `openmw.cfg` |
| `jess.openmw.configPath`, the `config =` option, default config loading | re-points resolution at any config; reads `OPENMW_CONFIG*`, `HOME`, `XDG_*` |
| `vfstool.host.*`, `vfs:setWriteRoot`, `dumpToDirectory`, `collapseInto`, `extractFile`, `runSetup*`/`runFinalize*` | anything: the plan grants l3i's `filesystem.host` and `filesystem.write` to every runtime, so those gates are open |
| `VFS.fromDirectories`, `pushArchive`, `VfsFile.from` | reads at any path |
| Dreamwind's `@dreamwind/archive` (`openPath`, `extractTo*`, builders' `addDir`/`writePath`) and `@dreamwind/archive/tool` (`extract*`, `create`, `add`) | reads and writes at any path, through `vfstool.host` |
| `require('./...')` from a script on disk | reads from disk relative to the caller; `..` is not clamped |

What is fine without a check: anything that works on bytes or records already in the VM (`parse`, `openBytes`, `luadata`, `toml`, `regex`, `shader`, `dreamPath`, record handles), `print` and `@jess/log`, and `os.time`/`os.clock`. Nothing in the plan spawns a process or opens a socket.

The vocabulary for closing this (`filesystem.profile`, `filesystem.user-selected`, `filesystem.unrestricted`, `vfs.read`, `vfs.mount`) is written down in the design and not yet in code. Until it is, run only trusted code in a domain.

## Restart

`Host::restart(load)` is `ScriptDomain::restart` with one thing in front of it: the input service forgets the graph and the intent routes the old code installed, and keeps the devices and the queue, because the devices did not restart. The registry stays as it is, locale included. Events queued for the old generation are discarded by the domain and counted; an action event from an older graph generation is dropped by the intent router; a key held across the restart is invisible to the new code's graph until it is released, the same going-live rule a set change applies.

It is as destructive as the domain's restart: the graph is forgotten and the old generation ends before the new code loads, and a load that fails leaves nothing running. A reload that keeps the last good generation alive is not this; see [Generations and Restart](@/docs/contributing/script-domains.md#generations-and-restart).

## Proof

Two fixtures, both headless:

- `libs/host/tests/kernel.rs` drives a host with the runtime's own modules and a journal: the quiet frame, the press that becomes an input phase with its intent and generation, the locale change that becomes an event, the restart that keeps the keyboard and drops the graph and the stale event, the stub backend's poll loop.
- `libs/lua/tests/kernel.rs` is the substrate handoff's integration fixture over jess's real plan: one host domain with a registry and a live input runtime; a `main` context and an `input` context holding `input.actions` alone; a script that installs the graph and formats a message at startup; a worker spawned with `shareServices = true` and `l10n.read` that formats from the shared registry and is denied input; keys pressed on the host's synthetic side and from the script, arriving as action events and intents; a locale switch that arrives as `l10n.changed` and formats in Polish; two denials with the input context's attribution; a restart to a second generation with the held key, the graph, and a queued event gone and the registry kept; and a teardown that owns nothing.

## Where It Lives

- `libs/host/src/lib.rs`: `Host`, `HostSpec`, `FrameReport`, `SharedInput`, `SharedRegistry`.
- `libs/host/src/input.rs`: `InputSource`, the driver, `payload`.
- `libs/host/src/l10n.rs`: `CHANGED`, the watch, `changed`.
- `libs/runtime/src/service.rs`: `Services`; `DomainSpec::service`, `ScriptDomain::service`, `jess_runtime::service`, `WorkerSpec::share`.
- `libs/input/src/service.rs`: `InputService`.
- `libs/lua/src/input.rs` and `l10n.rs`: the modules that bind to the services and check the capabilities.
- `libs/runtime/README.md`: the contract, call by call, services and events included.
