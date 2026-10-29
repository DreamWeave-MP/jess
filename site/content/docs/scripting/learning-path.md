---
title: 'Building a Larger Application'
description: 'A learning path through application composition, authority, I/O, presentation, and verification.'
weight: 55
extra:
  kind: guide
---

Once [your first application](@/docs/getting-started/first-application.md) runs, the next challenge is deciding where work and state belong. Follow these steps in order or enter at the problem you are solving. The linked guides explain relationships; the references define the individual operations.

## Compose and own state

Read [Modules, Layers, and Dependencies](@/docs/getting-started/modules-and-layers.md), then [Application Lifecycle and State](@/docs/getting-started/application-lifecycle.md). Decide which package owns each public API and which managed module owns mutable state. An exported module, a layer entrypoint, and a managed module have different jobs.

The [headless application recipe](@/docs/cookbook/headless.md) runs a host until a frame limit or interrupt, then exercises its shutdown handler.

## Communicate deliberately

[Applications](@/docs/scripting/applications.md) explains managed handlers, startup order, named events, and interface composition. Try the [named-event recipe](@/docs/cookbook/named-event.md), then consult [events](@/docs/api/events.md) for queueing and payload rules and [interfaces](@/docs/api/interfaces.md) for domain-local contracts. A worker is a separate Luau state communicating through messages; its [reference](@/docs/api/worker.md) covers delegation, queues, and shutdown.

## Plan authority and I/O

[Capabilities and Trust](@/docs/getting-started/authority.md) explains what the target requests and what the caller grants. For files and data, start with [Files, TOML, and Regex](@/docs/scripting/files-toml-regex.md) and the [backup recipe](@/docs/cookbook/safe-write.md). For HTTP, read [@jess/http](@/docs/api/http.md) and the [L3i module map](@/docs/api/l3i.md) for the underlying transport. Capability gates are not an operating-system sandbox.

## Add input, graphics, or audio

The [input reference](@/docs/api/input.md) covers binding documents and the native input runtime. [Applications: A window](@/docs/scripting/applications.md#a-window) walks through a window; [GPU](@/docs/api/gpu.md), [shader](@/docs/api/shader.md), and [render](@/docs/api/render.md) define the parts. [Audio](@/docs/api/audio.md) covers offline and device-backed contexts. Device and platform availability are runtime conditions, not guarantees made by a successful type check.

## Test, measure, and ship

[Checking and Testing](@/docs/getting-started/testing.md) separates static checking from execution and source from packed providers. Use [Debugging](@/docs/getting-started/debugging.md) when a boundary fails and [Performance](@/docs/scripting/performance.md) before changing a hot path. [From Workspace to Distribution](@/docs/getting-started/first-workspace.md) builds a small `.jsw`; the [workspace reference](@/docs/cli/workspace.md) covers dependencies, offline behavior, and standalone runtimes.
