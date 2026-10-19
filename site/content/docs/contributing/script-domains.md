---
title: 'Script Domains'
description: 'What jess-runtime is, what it deliberately is not, and how domains, contexts, capabilities, workers, generations, and teardown work.'
weight: 100
extra:
  kind: guide
---

`jess run` makes one Luau VM, installs jess in it, runs a script, and exits. That is the right shape for a command-line tool and the wrong one for an application that hosts an editor, a running game session, a headless server, and a profiler at once, each in Luau, each with its own authority, each able to reload without the others noticing.

`jess-runtime` (`libs/runtime`) is the piece between those two: one process, many isolated Luau worlds, and one place that decides when each may run and what each may do. It sits on [l3i](https://DreamWeave-MP.github.io/l3i/) and knows nothing about Morrowind, OpenMW, or anything with a window.

## What It Is

A `ScriptDomain` is one isolated Luau authority. It owns:

- one primary l3i `Runtime`, built by a `RuntimeFactory` the host hands it;
- its contexts, which say when its code may run;
- its capability grant, which says what its code may do;
- its workers, isolated Luau states that run beside it;
- the native resources services created on its behalf;
- the queue of events waiting for it.

It has an explicit lifecycle (`Created`, `Loading`, `Ready`, `Running`, `Stopping`, `Stopped`, `Failed`), a stable `DomainId`, and a `Generation` that moves on every restart. Two domains made from the same plan share nothing: not globals, not module tables, not a userdata. The fixture in `libs/runtime/tests/runtime/fixture.rs` proves that with two domains in one process and is the definition of the crate being done.

## What It Is Not

- **Not a session, workspace, or GUI.** No world state, no player, no windows. A session may later own domains; a tool domain may exist with no session.
- **Not an OpenMW script system.** There is no `Player` or `Menu` phase and there never will be. The generic phases below are scheduler entries; a layer that wants `openmw.load` builds it on `onDemand` or events.
- **Not an actor framework.** Workers exchange a small set of values over a channel. There is no object RPC, no shared table, no transparent anything.
- **Not `jess_lua`.** The crate takes a factory. `jess_lua::plan` is one plan it can run, and `jess_lua::runtime` is one factory; the crate never names either.
- **Not an application.** It carries a host's services by type ([Application Kernel](@/docs/contributing/application-kernel.md)) and never looks inside one.

## Threads

l3i's `Runtime` is not `Send`: it owns a `lua_State` and `Rc` state. Neither is a `RuntimePlan`. The crate is built on that fact rather than around it, and its docs carry `compile_fail` tests that pin both.

So a domain lives on the thread that created it, a worker builds and drops its own runtime on its own lane, and the factory is a plan *function*, not a plan: `FromPlan(jess_lua::plan)` calls `jess_lua::plan()` on whatever thread needs a runtime and gets that thread's cached plan. A closure returning a finished `Runtime` works too, for hosts that set up globals after `Runtime::from_plan`.

Other threads reach a domain through a `DomainMailbox`, which is `Send` and only queues. Nothing else crosses.

## Contexts: When Code May Run

The host declares contexts with a `ContextSpec`: a name, the phases it schedules, a priority, an optional capability subset, an optional budget. Scripts bind handlers to a declared context by name:

```luau
--!strict
local context = require('@jess/context')

context.bind('game', {
  startup = function()
    print('the domain started')
  end,

  update = function(deltaSeconds: number)
    print(`{deltaSeconds} seconds passed`)
  end,

  shutdown = function()
    print('the domain is stopping')
  end,
})
```

Binding to a phase the host did not schedule is an error. Binding never widens authority; the host's declaration is the ceiling.

The phases are generic: `load`, `startup`, `fixedUpdate`, `update`, `lateUpdate`, `input`, `renderPrepare`, `message`, `timer`, `event`, `onDemand`, `shutdown`. Within a phase, contexts run by priority, then registration order. No hash map decides anything.

Every entry into script code goes through the scheduler: `run_phase` broadcasts one phase, `advance(delta)` runs the fixed steps the accumulator owes (bounded by a catch-up limit, the rest dropped and counted), then `update`, `lateUpdate`, and an event drain. A native event never calls Luau directly; it is posted to the mailbox and delivered on the next drain. A handler that tries to re-enter the scheduler through a native service gets `RuntimeError::Reentrant` and a hint to queue an event instead.

Each handler call runs under the context's authority, inside its budget (l3i's watchdog), inside an l3i call scope carrying its `ContextId`, and timed. A Luau error in it becomes a `Diagnostic` and a `RuntimeError::Script` in the phase report; the phase goes on, and so does the domain.

## Events: Raw and Named

The queue carries two kinds of event, structurally apart. A **raw** event (`DomainMailbox::post(EventTarget, Message)`) goes to one context's `event` handler, or to every context's. A **named** event goes to every handler bound to its name, in scheduler order: script code queues one with [`@jess/events`](@/docs/api/events.md)'s `emit`, a native host with `DomainMailbox::post_named`, and a host binds a handler with `ScriptDomain::bind_event_handler` (an application's loader binds a managed module's `on.event` table this way). A name is not a target and a raw event has no name, so neither can pass for the other, and neither ever runs a phase handler. `emit('startup')` reaches handlers named `startup`, never the `startup` phase.

Both kinds share one FIFO queue and one drain: emission order is delivery order, an event queued during a drain waits for the next one, every envelope is stamped with its generation, and an event nothing handles is counted in `EventReport::unhandled` and dropped. Payloads are `Message`s either way, so an event that works inside a domain works across one. Crossing is the host's call: `ScriptDomain::route(name, mailbox)` posts a copy of every `name` the domain emits to another domain's mailbox. Nothing is broadcast, and a stopped or full recipient is a warning diagnostic, not the emitter's error.

The queue is bounded by `DomainSpec::event_limits`: `EventLimits { pending, bytes, per_drain }`, by default 65,536 events, 64 MiB of payload, and 4,096 events a drain. A named event that doesn't fit is refused, never dropped: `emit` raises `RuntimeError::QueueFull` with a `QueueFull` diagnostic, and `post_named` returns `PostRefused::Full`. A raw post, the host's own, counts toward the limits and is always queued. A drain takes at most `per_drain` envelopes and leaves the rest, in order; `EventReport::pending` says how many wait.

Interfaces, [`@jess/interfaces`](@/docs/api/interfaces.md), are the other half of an application, and the half that never crosses: the effective tables an application's packages composed, kept with the runtime, so they are one domain's and one generation's, and a restart starts without them.

## Capabilities: What Code May Do

A capability is a dotted lowercase name: `input.observe`, `render.scene`, `worker.spawn`. The host holds `RootAuthority` and grants a `CapabilitySet` to a domain. From there the math only shrinks: a child domain (`delegate`), a context (`ContextSpec::capabilities`), a worker (`WorkerSpec::capabilities`) each hold a subset of whoever made them, and asking for more fails with `RuntimeError::OverDelegation` naming what is missing. A script cannot grant itself anything, and a script spawning a worker cannot give it more than the calling context holds.

A native service checks at its entry point:

```rust
pub const INPUT_OBSERVE: &str = "input.observe";

fn observe(/* ... */) -> l3i::Result<()> {
    jess_runtime::require_capability(INPUT_OBSERVE)?;
    Ok(())
}
```

`require_capability` reads the authority the scheduler installed on the thread for the running handler (or worker), so it is right under several domains and workers without a process-global anywhere. A denial is structured (`CapabilityDenied { capability, attribution }`), counted, and reported; the script sees a permission error naming the capability and the domain, generation, and context that asked.

Context and capability are separate fields on purpose. A context is *when*; a grant is *what*. Fusing them into one enum was on the list of ways to get this wrong.

## Workers: Cheap Parallel Luau

A worker is a second Luau state the domain owns, built from the same factory on a lane of its own (today one OS thread per worker; the contract is an isolated state that may run concurrently, and nothing in the API names a thread). Its module returns one function, the message handler; the lane calls it once per message with the message as a Luau value, and what it returns goes back:

```luau
--!strict
local worker = require('@jess/worker')

local decoder = worker.spawn {
  name = 'decoder',
  capabilities = { 'vfs.read' },
  source = [[
    return function(message)
      return { decoded = message.path }
    end
  ]],
}

decoder:send { path = 'music/battle01.ogg' }
local reply = decoder:receive(5)
print(reply.decoded)
decoder:join()
```

Only messages cross. A message is nil, a boolean, a number, an integer, a string, a vector, a buffer, a list or map of those, or an explicit `ResourceHandle`. A function, a thread, another userdata, or a boolean key fails with the path to it: `message.items[3].callback: a function can't cross a worker or event boundary`. Nothing is dropped quietly.

Buffers travel as bytes: one copy out of the sender, one fresh `buffer` in the receiver, and no copy at all between native parties that share the `Arc`. A zero-copy buffer shared between two VMs would need Luau to alias external memory, which l3i does not expose; that is the documented follow-up, not a hidden limitation.

A worker's failure is structured (`WorkerError { attribution, module, stage, message }`) and never fails its domain. The default policy stops the worker on its first handler failure; `Continue` keeps it handling. A worker that overruns its budget fails like any other.

## Generations and Restart

`domain.restart(load)` ends the current generation and starts the next: shutdown handlers run, workers stop, owned resources are released, bound handlers are dropped, the runtime is dropped, a fresh one is built, the new code loads, and a domain that was running starts again. Context declarations survive, because they are the host's; nothing else does.

Every event a mailbox posts is stamped with the generation at posting. After a restart, stale events are discarded, counted, and reported, and they never reach the new code. A native service completing work later should capture the generation when the work was requested and `post_for` it, so a late completion is dropped instead of delivered to code that never asked.

No state migrates. This is the coarse, correct reload boundary; clever hot swapping can come later, on top of it.

It is also destructive. The old generation is gone before the new code loads, so a load that fails leaves the domain `Failed` with nothing running. That is right for "restart this domain" and wrong for "reload this shader, this UI, this package source, and keep the last good one if the new one is broken". That reload builds its replacement beside the running generation, validates it, and swaps only on success. It is a different operation, it does not exist yet, and `restart` is not it.

## Teardown and Ownership

Anything native a service creates for a domain is registered with `domain.own(kind, release)`. It is released on `release(id)`, at the end of the generation that created it, or at `stop`, newest first. `owned_resources()` says what is still owned; after `stop` the answer is nothing.

`stop()` runs the shutdown handlers, drops and counts queued events, closes and joins every worker, releases every resource, removes the contexts, and drops the runtime. Dropping a `ScriptDomain` without calling `stop` does the same. Nothing a domain owns waits for process exit.

## Diagnostics and Profiling

Everything the runtime reports carries an `Attribution`: domain, generation, and the context or worker if there was one. Failures go to a `DiagnosticSink` as `Diagnostic { attribution, severity, kind, message, phase, event, capability, source }`, where `event` names the named event being delivered and `source` is `chunk:line` when the runtime knows it (where a handler was bound, where a bad message was sent). Costs go to a `ProfileSink` as `ProfileEvent`s: domain creation and startup, every context call with its queue delay and duration, every worker spawn, call, and message, every stale event and denial, and the teardown. `domain.stats()` is the counters at any moment.

What scripts log is a third stream. `DomainSpec::logs` gives a domain a `LogSink`, and every `@jess/log` record its contexts and workers write goes there as a `LogRecord { attribution, level, target, message, fields, source }`, attributed by the runtime, not by the script. A worker's logs are the application's whether or not it shares a service. A domain without a stream leaves `@jess/log` writing where its runtime's own sink points.

Every handler call also runs inside an l3i `CallScope` whose id is the `ContextId`, so l3i's own profiler and sampler attribute by the same ids. If somebody gets clever, the cost has a name.

## Where It Lives

- `src/runtime/domain.rs`: `ScriptDomain`, `DomainSpec`, the scheduler, the mailbox, raw and named events, routes, restart, and teardown.
- `src/runtime/context.rs`: `Phase`, `ContextSpec`, `Budget`, `FixedStep`.
- `src/runtime/capability.rs`: `CapabilityId`, `CapabilitySet`, `Grant`, `RootAuthority`, `Authority`, `require_capability`.
- `src/runtime/worker.rs` and `message.rs`: lanes, `WorkerSpec`, `Message`, `TransferError`.
- `src/runtime/service.rs`: `Services`, the typed slot a host attaches its input and localization to; `jess::runtime::service::<T>()` reads it from a binding.
- `src/runtime/luau.rs`: the `@jess/context`, `@jess/worker`, `@jess/events`, and `@jess/interfaces` extension.
- `src/lua/application.rs`: the application supervisor that loads managed modules into a domain, and `@jess/application`.

The tests are one binary, `tests/runtime/main.rs`, because every test binary pays l3i's link-time codegen: the fixture, the lifecycle and authority math, the scheduler, named events and routes, the workers, the threading proof (two workers meeting at a rendezvous inside their handlers), and a create-spawn-message-stop stress loop that checks no lane outlives its domain. Applications, end to end, are `tests/lua/application.rs` in `lua_package` and `tests/cli/application.rs`.
