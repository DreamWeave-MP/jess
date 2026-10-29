---
title: 'API Documentation Coverage'
description: 'Map the public runtime surface to its references and identify where contract verification is still needed.'
weight: 80
extra:
  kind: reference
---

This is a review map, not a promise that every listed function has been checked. The generated type definitions are the inventory for the runtime the checker sees; implementations and tests establish behavior that a type cannot express. Run `jess luau-types` for the installed version. The [machine-readable catalog](@/docs/api/_index.md) names the modules whose top-level signatures the site currently checks automatically and includes generated type and native handle declarations without claiming their behavior has been audited.

## Native runtime services

| Surface | Canonical reference | Source and tests to review | Current status |
| --- | --- | --- | --- |
| Global values and `require` | [jess](@/docs/api/jess.md) | `src/lua/definitions.jsl`, `src/lua/plan.rs`, `tests/lua/contracts.rs` | Reference exists; full signature and behavior audit remains. |
| Contexts, workers, events, interfaces, application | [context](@/docs/api/context.md), [worker](@/docs/api/worker.md), [events](@/docs/api/events.md), [interfaces](@/docs/api/interfaces.md), [application](@/docs/api/application.md) | `src/lua/definitions.jsl`, `src/runtime/`, `tests/lua/package/application.rs` | Top-level signatures checked by the catalog; worker methods, transfer rules, queues, lifecycle, and error contracts need a complete source audit. |
| Input | [input](@/docs/api/input.md) | `src/input/`, `src/lua/definitions.jsl`, `jpk/builtin/src/input/`, `tests/input/` | Reference exists; native and package APIs share a page and need a symbol-by-symbol audit. |
| Audio | [audio](@/docs/api/audio.md) | `src/lua/audio.rs`, `src/audio/`, `src/lua/definitions.jsl`, `jpk/builtin/src/audio/`, `tests/lua/audio_*` | Detailed reference; scheduling, resources, platform behavior, and every method still need systematic verification. |
| GPU and shader | [GPU](@/docs/api/gpu.md), [shader](@/docs/api/shader.md) | `src/lua/gpu/`, `src/shader/`, `src/lua/definitions.jsl`, `tests/gpu/`, `tests/shader/` | Detailed references; methods, ownership, limits, and platform conditions need systematic verification. |
| Measurement | [bench](@/docs/api/bench.md) | `src/lua/definitions.jsl`, `src/lua/`, `tools/bench/` | Top-level signatures checked by the catalog; verify exact build and measurement requirements against the implementation. |

## Luau modules shipped with Jess

| Surface | Canonical reference | Source and tests to review | Current status |
| --- | --- | --- | --- |
| Logging and tests | [log](@/docs/api/log.md), [test](@/docs/api/test.md) | `src/lua/stdlib/`, `tests/lua/stdlib.rs` | Top-level signatures checked by the catalog; audit all expectations, captures, and errors. |
| JPK and package resolution | [JPK](@/docs/api/jpk.md), [package](@/docs/api/package.md), [manifest](@/docs/reference/manifest.md) | `src/lua/stdlib/jpk.jsl`, `src/lua/stdlib/package.jsl`, `tests/lua/package/` | Top-level signatures checked by the catalog; verify budgets, version rules, activation, and provider parity. |
| JSON, YAML, TOML, and values | [JSON](@/docs/api/json.md), [YAML](@/docs/api/yaml.md), [TOML](@/docs/api/toml.md), [values](@/docs/api/value.md) | `jpk/builtin/src/data/`, `jpk/builtin/tests/` | References exist; audit loss, null, ordering, and error behavior. |
| Localization | [localization](@/docs/api/l10n.md) | `jpk/builtin/src/l10n/`, `jpk/builtin/tests/` | Reference exists; audit providers and invalidation. |
| Analysis and style | [analysis](@/docs/api/analysis.md), [style](@/docs/api/style.md) | `jpk/builtin/src/analysis/`, `jpk/builtin/src/style/`, `tests/lua/luau_style.rs` | References exist; audit every returned fact and checker option. |
| Backup, HTTP, rendering helpers | [backup](@/docs/api/backup.md), [HTTP](@/docs/api/http.md), [render](@/docs/api/render.md) | `jpk/builtin/src/backup/`, `http/`, `render/`, `jpk/builtin/tests/` | References exist; audit failure, ownership, and limits. |
| L3i primitives | [@dream modules](@/docs/api/l3i.md) | The L3i definitions emitted by `jess luau-types`; L3i's reference | Top-level stub signatures are in the catalog, marked unchecked against local pages. Jess documents its gates and integration; L3i owns the primitive contracts. |

## CLI and distribution

The [CLI reference](@/docs/cli/_index.md), [workspace reference](@/docs/cli/workspace.md), and [capabilities](@/docs/reference/capabilities.md) map to `src/cli/`, `src/main.rs`, `src/lua/stdlib/workspace.jsl`, and `tests/cli/`. A complete parser-to-reference audit, including exit statuses, flag placement, install behavior, and platform-specific standalone artifacts, remains open.

The tutorial checks exercise a small set of complete paths. They do not replace the remaining API audit, browser and keyboard testing, or platform verification. For entries marked `checked`, the build verifies the generated signature and handwritten heading. Other entries are indexed without that signature check.

A WebKit visual review covered the first application page at 1440 and 390 CSS pixels and the GPU reference at 390 pixels. The pages and navigation rendered at those widths; this is not a complete keyboard, assistive technology, browser, or platform audit.

## Executable coverage and restrictions

The fixtures below are run or checked by `tools/site/tests/tutorials.jsl` unless another check is named. Paths are relative to the repository root. These examples cover representative paths, not every option or error in the corresponding reference.

| Surface | Example and evidence | Requirements and limits |
| --- | --- | --- |
| Luau, JSL, and grants | `tests/fixtures/docs/luau-basics.jsl`, `jsl-dialect.jsl`, and `capabilities.jsl`; [first script](@/docs/scripting/intro.md) | Loose scripts run under the CLI; the grant fixture checks the default capability set. |
| Package source and JPK | The generated first package and `tests/fixtures/docs/multi`; [first package](@/docs/getting-started/first-package.md), [two packages](@/docs/cookbook/multi-package.md) | Source tests run before packing; packed execution uses the embedded payload and resolved dependencies. |
| Host lifecycle, events, and workers | `tests/fixtures/docs/headless`, `counter`, `named-event`, and `worker`; [headless](@/docs/cookbook/headless.md), [events](@/docs/cookbook/named-event.md), [worker](@/docs/cookbook/worker.md) | Workers need an application domain and `worker.spawn`; the headless case needs no display. |
| HTTP and capability denial | `tests/fixtures/docs/http-get.jsl` with the loopback server in `tests/cli/docs.rs`; `http-denied.jsl` | A real request needs DNS and TCP grants. CI uses loopback; TLS trust stores and remote servers are environment dependent. |
| Structured data and files | `tests/fixtures/docs/json-report.jsl`, `config.jsl`, and `backup.jsl` | File recipes run in scratch with filesystem grants. A backup is not a transaction across concurrent writers. |
| Audio and window | `tests/fixtures/docs/offline-audio.jsl` and `window`; [audio](@/docs/cookbook/offline-audio.md), [window](@/docs/cookbook/window.md) | Offline mixing needs no output device. CI checks the window package without opening a device; rendering requires an available display, adapter, and build features. |
| Workspace distribution | `tests/fixtures/docs/counter` and `multi` | The suite runs source, a `.jsw`, a standalone executable made from the test runtime, and an offline local dependency lock. Standalone formats and destination libraries remain platform dependent. |

## Writing and navigation review

The reference pages generally put signatures and concrete behavior before examples, and the larger GPU, shader, and audio pages keep related operations together under a local table of contents. The getting-started landing page now separates a first script, application distribution, and larger application development; the cookbook links complete fixtures where it promises runnable programs. Guides link back to the reference for contracts rather than copying it wholesale.

The audit found absolute claims that contradicted the implementation, especially around default grants, returned errors, scheduling, and recovery. The findings below record corrections and their sources. Some older reference snippets are fragments or depend on a host, a device, or data that the snippet does not create; they are not covered by the tutorial fixture gate. Some generated signatures remain unchecked against the prose, and platform behavior needs testing on the destination. Treat the catalog as a discovery aid and its `checked` field as a signature-check status, not proof that every behavioral claim has been reviewed.

## Findings from this pass

| Classification | Surface | Evidence and action |
| --- | --- | --- |
| Incorrect | Default target grants and host file reads | `src/lua/package.rs` and `src/lua/mod.rs` distinguish the host's default grants from a target's declared authority. [Capabilities](@/docs/reference/capabilities.md) now states that boundary. |
| Incorrect | Package inspection and resolution | `src/lua/stdlib/package.jsl` returns both the plan and selected packages from `resolve` and exposes `peek` and `candidates`; [package](@/docs/api/package.md) now describes them. |
| Incorrect | Message transfer | `src/runtime/message.rs` accepts integers and resource handles and limits nested values to 32 levels; [events](@/docs/api/events.md) and [workers](@/docs/api/worker.md) now describe the shared boundary. |
| Incorrect | Platform distribution | Release archives do not establish Android or 32-bit support. [Installation](@/docs/getting-started/install.md) no longer directs those users to an unverified source build. |
| Incorrect | CLI state and option placement | `src/cli/run.rs` and `src/cli/package.rs` accept trailing script arguments, and workspace and install commands retain state. [Global options](@/docs/cli/global-options.md) now explains the boundary; `tools/site/tests/cli-reference.jsl` exercises argument forwarding and checks documented options against help. |
| Incorrect | GPU adapter selection | `src/gpu/mod.rs` asks for a low-power adapter, then a fallback. [GPU](@/docs/api/gpu.md) no longer promises a hardware adapter or universal software support. |
| Incorrect | HTTP failure behavior | `jpk/builtin/src/http/init.jsl` waits for requests and bounds response heads; network capability denial raises from the underlying primitive. [HTTP](@/docs/api/http.md) now distinguishes denial from returned request failures, and the docs checks exercise the denial without contacting a network. |
| Incorrect | Backup cleanup and concurrency | `jpk/builtin/src/backup/init.jsl` attempts to remove a failed backup but does not check that removal, and its copy and replacement are separate operations. [Backup](@/docs/api/backup.md) now states those limits. |
| Incorrect | Audio scheduling and stream waits | `src/audio/stream.rs` clamps a wait to ring capacity and sleeps in intervals, while late engine commands execute on a later frame. [Audio](@/docs/api/audio.md) now describes the timing and blocking limits. |
| Incomplete | GPU readback | `src/gpu/read.rs` polls without waiting for GPU completion, copies mapped bytes once ready, and can report device or mapping failures. [GPU](@/docs/api/gpu.md#gpuread) now distinguishes that from waiting for submitted work. |
| Incomplete | Test hooks and assertions | `src/lua/stdlib/test.jsl` skips the body after a setup failure, continues cleanup after cleanup failures, and searches table values in `toContain`. [Test](@/docs/api/test.md) now describes those cases and recursive-run errors. |
| Incomplete | Exact behavior of generated types and handle methods | The catalog exposes their declarations for search, but `checked = false` does not certify lifecycle, error, scheduling, or resource contracts. The API pages and tests remain the behavioral evidence. |
| Incomplete | CLI and platform contracts | The command groups have reference pages and exercised examples, but a complete parser-to-reference and cross-platform audit remains open. |
| Lacking examples | Device-dependent operations | The window fixture is checked without a device in CI; the offline audio fixture exercises processing without an audio device. Driver and hardware behavior still needs destination testing. |
