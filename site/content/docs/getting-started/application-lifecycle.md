---
title: 'Application Lifecycle and State'
description: 'Place state and work in the right part of an application’s lifecycle.'
weight: 35
extra:
  kind: guide
---

An application is not a command that happens to take a long time. Its target creates a host and a ScriptDomain, loads managed modules, and lets the host drive callbacks. [Your First Application](@/docs/getting-started/first-application.md) gives you a small running example.

## What runs, and when

Before application code runs, Jess checks the target's grant and resolves its plan. It then activates the plan, loads and validates managed modules, composes their interfaces, and runs the target entry. The managed `load` and `startup` handlers follow; then the host advances frames. On exit or restart, shutdown handlers run.

That ordering answers practical questions. A managed module's top-level code runs while it is being loaded. An interface lookup belongs in the entry or a handler, after composition, not in that top-level code while the registry is still being assembled. A handler name that Jess does not dispatch is rejected when the module is loaded, rather than quietly sitting there forever.

## Put state with its owner

The counter in the first application keeps its frame count in its managed module. That module owns both the state and the handlers that update it. For a larger application, decide which module owns a piece of state before choosing how other modules reach it.

- Call a package export for an ordinary reusable API.
- Publish an interface when modules in the same domain need a shared contract whose provider is composed at startup.
- Emit a named event when a producer needs to announce something to interested handlers.
- Use a worker for suitable independent work, with explicit communication; it does not share the domain's interface tables.

These choices have different lifetimes and failure modes. See [Applications](@/docs/scripting/applications.md), [events](@/docs/api/events.md), [interfaces](@/docs/api/interfaces.md), and [workers](@/docs/api/worker.md).

## Work only when it is needed

An `update` callback runs each frame. That is appropriate for work that changes with time, not automatically for every task in an application. Use a named event for a discrete occurrence and keep work near the state it changes. If work is expensive, measure it before deciding that a lower-level implementation will help. [Performance](@/docs/scripting/performance.md) explains the measurement tools.

The [application guide](@/docs/scripting/applications.md#how-an-application-starts) documents the exact startup order, callback names, error behavior, and interface override rules. It is the reference to consult when the small example stops being small.
