---
title: 'Contributing'
description: 'Build jess, run its tests against real data, and get changes released.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 90

extra:
  kind: guide
---

jess is a Rust 2024 workspace. These pages cover the gates every change passes, where things live, and how releases happen.

- [Building and Testing](@/docs/contributing/building-and-testing.md): the exact checks CI runs, and the real-data tests you should run before touching a parser.
- [Architecture](@/docs/contributing/architecture.md): the crate map, and the rules the layout enforces.
- [Releasing](@/docs/contributing/releasing.md): tags, crates.io, and what StroggForge does with them.
- [Benchmarks](@/docs/contributing/benchmarks.md): the harness behind every performance number here, the runtime canaries, and comparing two runs.
- [Format Notes](@/docs/contributing/format-notes.md): byte-identical round trips, OpenMW save format bumps, and the engine behavior behind the stranger rules.
- [Luau Style](@/docs/contributing/luau-style.md): how the bundled scripts and every Luau example are written, and the test that checks them.
- [Render Graph](@/docs/contributing/render-graph.md): the backend-independent render graph IR, what it deliberately isn't, and how passes, targets, ordering, culling, lifetimes, and aliasing work.
- [Shader IR](@/docs/contributing/shader-ir.md): the typed shader IR, what it deliberately isn't, and how validation, source attribution, constant folding, and WGSL lowering work.
- [Localization](@/docs/contributing/localization.md): the OpenMW-shaped message bundles, the ICU MessageFormat they speak, how locales fall back, and why every pattern compiles once.
- [Input Runtime](@/docs/contributing/input-runtime.md): device truth, the serializable action graph, sets and layers, ticks, intents, binding presentation, and the platform backends.
- [NIF Render Data](@/docs/contributing/nif-render-data.md): what Morrowind's NIFs actually use to draw, counted across `Morrowind.bsa`, and the `NifScene` translation that count decided.
- [Script Domains](@/docs/contributing/script-domains.md): `jess-runtime`, isolated Luau authorities in one process, and how contexts, capabilities, workers, generations, and teardown work.

Bugs and patches go to [GitHub](https://github.com/DreamWeave-MP/jess). Commits are one line, prefixed with what kind of change they are (`FEAT:`, `FIX:`, `CLEANUP:`, `REFACTOR:`, `PERF:`, `TEST:`, `DOCS:`, `DEPS:`, `CI:`), and say what changed in plain words.
