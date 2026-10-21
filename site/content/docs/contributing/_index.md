---
title: 'Contributing'
description: 'Build jess, run its gates, understand its internals, and get changes released.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 90

extra:
  kind: guide
---

jess is one Rust 2024 crate with a library and a binary. These pages cover the gates every change passes, where things live, and how the runtime is put together.

- [Building and Testing](@/docs/contributing/building-and-testing.md): the exact checks CI runs and how to run them locally.
- [Architecture](@/docs/contributing/architecture.md): the module map, and the rules the layout enforces.
- [Script Domains](@/docs/contributing/script-domains.md): isolated Luau authorities in one process, and how contexts, capabilities, workers, generations, and teardown work.
- [Application Kernel](@/docs/contributing/application-kernel.md): a script domain joined to live input, advancing frames, with no window and no renderer.
- [Render Graph](@/docs/contributing/render-graph.md): the backend-independent render graph IR, what it deliberately isn't, and how passes, targets, ordering, culling, lifetimes, and aliasing work.
- [Shader IR](@/docs/contributing/shader-ir.md): the typed shader IR, and how validation, source attribution, constant folding, and WGSL lowering work.
- [Input Runtime](@/docs/contributing/input-runtime.md): device truth, the serializable action graph, sets and layers, ticks, intents, binding presentation, and the platform backends.
- [Localization](@/docs/contributing/localization.md): message bundles, the ICU MessageFormat they speak, how locales fall back, and why every pattern compiles once.
- [Luau Style](@/docs/contributing/luau-style.md): how jess's own Luau and every example here are written, and the checker that enforces it.
- [Benchmarks](@/docs/contributing/benchmarks.md): the harness, why it refuses unverified builds, and how to compare two runs.
- [Releasing](@/docs/contributing/releasing.md): tags, crates.io, and what the release pipeline does with them.

Bugs and patches go to [GitHub](https://github.com/DreamWeave-MP/jess). Commits are one line, prefixed with what kind of change they are (`FEAT:`, `FIX:`, `CLEANUP:`, `REFACTOR:`, `PERF:`, `TEST:`, `DOCS:`, `DEPS:`, `CI:`), and say what changed in plain words.
