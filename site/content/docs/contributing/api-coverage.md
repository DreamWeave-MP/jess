---
title: 'API Documentation Coverage'
description: 'Map the public runtime surface to its references and identify where contract verification is still needed.'
weight: 80
extra:
  kind: reference
---

This is a review map, not a promise that every listed function has been checked. The generated type definitions are the inventory for the runtime the checker sees; implementations and tests establish behavior that a type cannot express. Run `jess luau-types` for the installed version. The [machine-readable catalog](@/docs/api/_index.md) names the modules whose top-level signatures the site currently checks automatically and includes generated type and native handle declarations without claiming their behavior has been audited.

For the build reviewed here, the catalog contains 2,496 symbol records; 52 have `checked = true`, meaning their generated signature and a handwritten heading are checked together. The other records remain searchable and are covered by the per-module drift fingerprints, but their signatures are not individually matched to the prose. Counts vary with the runtime and enabled features; neither status proves a behavioral audit.

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
| JSON, YAML, TOML, and values | [JSON](@/docs/api/json.md), [YAML](@/docs/api/yaml.md), [TOML](@/docs/api/toml.md), [values](@/docs/api/value.md) | `jpk/builtin/src/data/`, `jpk/builtin/tests/` | Public functions are matched to generated signatures; audit remaining loss, null, ordering, and error behavior across codecs. |
| Localization | [localization](@/docs/api/l10n.md) | `jpk/builtin/src/l10n/`, `jpk/builtin/tests/` | Reference exists; audit providers and invalidation. |
| Analysis and style | [analysis](@/docs/api/analysis.md), [style](@/docs/api/style.md) | `jpk/builtin/src/analysis/`, `jpk/builtin/src/style/`, `tests/lua/luau_style.rs` | References exist; audit every returned fact and checker option. |
| Backup, HTTP, rendering helpers | [backup](@/docs/api/backup.md), [HTTP](@/docs/api/http.md), [render](@/docs/api/render.md) | `jpk/builtin/src/backup/`, `http/`, `render/`, `jpk/builtin/tests/` | Backup and HTTP functions are matched to generated signatures; audit remaining failure, ownership, and limits. |
| L3i primitives | [@dream modules](@/docs/api/l3i.md) | The L3i definitions emitted by `jess luau-types`; L3i's reference | Top-level stub signatures are in the catalog, marked unchecked against local pages. Jess documents its gates and integration; L3i owns the primitive contracts. |

### Built-in package export map

The manifest is `jpk/builtin/jess.toml`; `jess luau-types` copies these sources for type checking, and the generated catalog records their public exports and types. In the source column, `src/` paths are under `jpk/builtin/` and `src/lua/` paths are at the Jess repository root. Unqualified test paths are under `jpk/builtin/tests/`. A listed test establishes the cases it runs, not every possible input or platform.

| Export | Source | Reference | Tests and example coverage |
| --- | --- | --- | --- |
| `@jess/style` | `src/style/` | [style](@/docs/api/style.md) | `style/`; `tests/lua/luau_style.rs` runs the checker over repository sources and documentation blocks. |
| `@jess/analysis` | `src/analysis/` | [analysis](@/docs/api/analysis.md) | `analysis/`; the reference examples illustrate AST and workspace analysis. |
| `@jess/json` | `src/data/json.jsl`, `src/data/value.jsl` | [JSON](@/docs/api/json.md) | `data/`; [JSON report](@/docs/cookbook/json-report.md) is executable. |
| `@jess/yaml` | `src/data/yaml.jsl`, `src/data/value.jsl` | [YAML](@/docs/api/yaml.md) | `data/`; the reference demonstrates aliases and values. |
| `@jess/data/toml` | `src/data/toml.jsl`, `src/data/tomldecode.jsl` | [TOML](@/docs/api/toml.md) | `data/`; workspace and manifest fixtures also exercise the decoder. |
| `@jess/data/value` | `src/data/value.jsl` | [values](@/docs/api/value.md) | `data/`; reference snippets demonstrate encoding and explicit null/none. |
| `@jess/l10n` | `src/l10n/` | [localization](@/docs/api/l10n.md) | `l10n/`; directory providers need filesystem access, while in-memory providers do not. |
| `@jess/input` | `src/input/` and `src/lua/input.rs` | [input](@/docs/api/input.md) | `input/`, `tests/input/`; device backends and window input depend on build features and host grants. |
| `@jess/audio` | `src/audio/` and `src/lua/audio.rs` | [audio](@/docs/api/audio.md) | `audio/`, `tests/lua/audio_*`; [offline audio](@/docs/cookbook/offline-audio.md) needs no device, while output depends on `audio-device` and the host. |
| `@jess/backup` | `src/backup/` | [backup](@/docs/api/backup.md) | `backup/`; [configuration](@/docs/cookbook/configuration.md) exercises scratch writes with filesystem grants. |
| `@jess/http` | `src/http/` | [HTTP](@/docs/api/http.md) | `http/`, `tests/cli/docs.rs`; the [HTTP recipe](@/docs/cookbook/http-get.md) uses loopback in CI and needs network grants. |
| `@jess/render`, `@jess/render/camera` | `src/render/` | [render](@/docs/api/render.md) | `render/`; actual presentation requires a compatible GPU and, for windows, a display. |

### Public Rust modules

`src/lib.rs` exports `audio`, `host`, `input`, `lua`, `render`, `runtime`, and `shader`, plus `gpu` and `window` when their features are enabled. Rustdoc generated by `cargo doc --workspace --all-features --no-deps` is the symbol reference for the native API; the following pages explain its architecture and ownership boundaries. These are contributor and embedding-host surfaces, not extra Luau modules.

| Rust surface | Architecture and tests | Build restrictions |
| --- | --- | --- |
| `runtime`, `host` | [Script Domains](@/docs/contributing/script-domains.md), [Application Kernel](@/docs/contributing/application-kernel.md); `tests/runtime/`, `tests/host/` | The headless host does not require a window. |
| `lua` | [Architecture](@/docs/contributing/architecture.md); `tests/lua/` and `tests/cli/` | Its generated types and bindings depend on the features in the build. |
| `input` | [Input Runtime](@/docs/contributing/input-runtime.md); `tests/input/` | `linux-evdev` and `gilrs` are optional backends with platform-specific dependencies. |
| `render`, `shader` | [Render Graph](@/docs/contributing/render-graph.md), [Shader IR](@/docs/contributing/shader-ir.md); `tests/render/`, `tests/shader/` | The IR modules do not require a window. |
| `audio` | [Audio Engine](@/docs/contributing/audio-engine.md), [Audio Graph](@/docs/contributing/audio-graph.md); `tests/lua/audio_*` | Device output is feature-dependent; offline processing remains available without it. |
| `gpu`, `window` | [GPU](@/docs/contributing/gpu.md); `tests/gpu/`, `tests/lua/gpu.rs`, `src/window/` | `gpu` and `window` are feature-gated; a window also needs a display, and GPU operations need a compatible adapter. |

## CLI and distribution

The [CLI reference](@/docs/cli/_index.md), [workspace reference](@/docs/cli/workspace.md), and [capabilities](@/docs/reference/capabilities.md) map to `src/cli/`, `src/main.rs`, `src/lua/stdlib/workspace.jsl`, and `tests/cli/`. `tools/site/tests/cli-reference.jsl` compares the command tree and option names with the actual help and exercises trailing argument forwarding. A complete behavioral audit, including every exit path, install behavior, and platform-specific standalone artifact, remains open.

The tutorial checks exercise a small set of complete paths. They do not replace the remaining API audit, browser and keyboard testing, or platform verification. For entries marked `checked`, the build verifies the generated signature and handwritten heading. For the catalog as a whole, the build also compares per-module fingerprints to a recorded baseline so a generated signature change requires review. A matching fingerprint does not establish the accuracy or completeness of behavioral prose.

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

The reference pages generally put signatures and concrete behavior before examples, and the larger GPU, shader, and audio pages keep related operations together under a local table of contents. The getting-started landing page now separates a first script, application distribution, and larger application development; the cookbook links complete fixtures where it promises runnable programs. The generated symbol index provides direct signature links without JavaScript and points each entry to its handwritten reference. Guides link back to the reference for contracts rather than copying it wholesale.

The audit found absolute claims that contradicted the implementation, especially around default grants, returned errors, scheduling, and recovery. The findings below record corrections and their sources. Some older reference snippets are fragments or depend on a host, a device, or data that the snippet does not create; they are not covered by the tutorial fixture gate. Some generated signatures remain unchecked against the prose, and platform behavior needs testing on the destination. Treat the catalog as a discovery aid and its `checked` field as a signature-check status, not proof that every behavioral claim has been reviewed.

A native message-transfer defect was reproduced: rebuilding a table changes non-UTF-8 string keys, and distinct keys can collide. `src/runtime/message.rs` uses a lossy UTF-8 conversion because the pinned L3i table setter accepts `&str`. A byte-key setter in L3i and its use in Jess are needed to preserve these keys; [events](@/docs/api/events.md) and [workers](@/docs/api/worker.md) identify the affected operations. The remaining audits below are not evidence that there are no other defects.

## Findings from this pass

| Classification | Surface | Evidence and action |
| --- | --- | --- |
| Incorrect | Default target grants and host file reads | `src/lua/package.rs` and `src/lua/mod.rs` distinguish the host's default grants from a target's declared authority. [Capabilities](@/docs/reference/capabilities.md) now states that boundary. |
| Incorrect | Package inspection and resolution | `src/lua/stdlib/package.jsl` returns both the plan and selected packages from `resolve` and exposes `peek` and `candidates`; [package](@/docs/api/package.md) now describes them. |
| Incomplete | Resolution roots and development dependencies | `src/lua/stdlib/package.jsl` selects requested roots independently of layers and adds development dependencies only for selected ids named by `request.dev`. [Package](@/docs/api/package.md#package-resolve) now describes those options and the module authority boundary. |
| Incomplete | Package access and digests | `src/lua/package.rs` gates file reads and writes, while `openJpk` reads through its supplied source. [Package](@/docs/api/package.md) now states those capability boundaries and describes digest framing without promising impossible collision freedom. |
| Incomplete | Opened package lifetime | `src/lua/stdlib/package.jsl` seals each opened payload with per-file digests and rechecks reads; a source manifest uses its retained opening bytes. [Package](@/docs/api/package.md#package-open) now states the read and ownership behavior. |
| Incorrect | JPK integrity and interoperability | `src/lua/stdlib/jpk.jsl` checks decoded length and CRC-32; these checks do not establish cryptographic integrity. [JPK](@/docs/api/jpk.md) now states that limit and requires ZIP method 93 support for Zstandard entries. |
| Incomplete | Export ownership | `src/lua/require.rs` freezes only a returned package table's top level. [require](@/docs/api/jess.md#require) and [Modules, Layers, and Dependencies](@/docs/getting-started/modules-and-layers.md) now describe nested mutable state; `tests/fixtures/docs/module-ownership` checks source and JPK behavior. |
| Incomplete | Interface ownership and overrides | `src/lua/application.rs` freezes each provider's top-level table after its override callback, and calls the callback only when an earlier provider exists. [Interfaces](@/docs/api/interfaces.md) now explains the shared nested state, callback order, and failure behavior; `tests/lua/package/application.rs` exercises composition. |
| Incomplete | Context binding | `src/runtime/luau.rs` reads all handlers before `src/runtime/domain.rs` validates their schedule and installs them. [Context](@/docs/api/context.md) now describes failed-bind behavior and the domain states that accept bindings. |
| Incorrect | Message transfer | `src/runtime/message.rs` accepts integers and resource handles and limits nested values to 32 levels; [events](@/docs/api/events.md) and [workers](@/docs/api/worker.md) now describe the shared boundary. |
| Incomplete | Worker reply scheduling and ownership | `src/runtime/worker.rs` uses a bounded channel for replies and drains it during `join`; explicit `worker.send` refuses a full channel. `src/runtime/message.rs` reconstructs mutable worker messages and copies buffers. [Workers](@/docs/api/worker.md) now distinguishes these behaviors; `tests/runtime/workers.rs` covers queues and joining. |
| Incorrect | Platform distribution | Release archives do not establish Android or 32-bit support. [Installation](@/docs/getting-started/install.md) no longer directs those users to an unverified source build. |
| Incorrect | Source installation flags | The pinned L3i `build.rs` says Apple targets cannot use the Linux linker-plugin-LTO flag. [Installation](@/docs/getting-started/install.md#from-crates-io) now separates Linux and macOS commands and names clang for the unverified example. The commands still need verification on each destination platform. |
| Incorrect | CLI state and option placement | `src/cli/run.rs` and `src/cli/package.rs` accept trailing script arguments, and workspace and install commands retain state. [Global options](@/docs/cli/global-options.md) now explains the boundary; `tools/site/tests/cli-reference.jsl` exercises argument forwarding and checks documented options against help. |
| Incorrect | GPU adapter selection | `src/gpu/mod.rs` asks for a low-power adapter, then a fallback. [GPU](@/docs/api/gpu.md) no longer promises a hardware adapter or universal software support. |
| Incorrect | HTTP failure behavior | `jpk/builtin/src/http/init.jsl` waits for requests and bounds response heads; network capability denial raises from the underlying primitive. [HTTP](@/docs/api/http.md) now distinguishes denial from returned request failures, and the docs checks exercise the denial without contacting a network. |
| Incomplete | Filesystem traversal order | The pinned L3i `src/fs/mod.rs` sorts `list` names by bytes, while `src/fs/walk.rs` defaults its `sort` option to false. [Files](@/docs/api/fs.md) now distinguishes the two; the L3i reference owns the other filesystem contracts. |
| Incorrect | Backup cleanup and concurrency | `jpk/builtin/src/backup/init.jsl` attempts to remove a failed backup but does not check that removal, and its copy and replacement are separate operations. [Backup](@/docs/api/backup.md) now states those limits. |
| Incorrect | Audio scheduling and stream waits | `src/audio/stream.rs` clamps a wait to ring capacity and sleeps in intervals, while late engine commands execute on a later frame. [Audio](@/docs/api/audio.md) now describes the timing and blocking limits. |
| Incomplete | GPU readback | `src/gpu/read.rs` polls without waiting for GPU completion, copies mapped bytes once ready, and can report device or mapping failures. [GPU](@/docs/api/gpu.md#gpuread) now distinguishes that from waiting for submitted work. |
| Incomplete | GPU buffer ranges | `src/gpu/buffer.rs` rejects a zero-sized allocation, and `src/lua/gpu/resources.rs` treats the write source offset separately from GPU alignment. [GPU buffers](@/docs/api/gpu.md#gpubuffer) now states those boundaries and defaults. |
| Incorrect | 3D texture transfers | `src/gpu/texture.rs` includes every slice of the selected mip in a 3D transfer. [GPU textures](@/docs/api/gpu.md#gputexture) now includes depth in the byte count and states the layer constraint. |
| Incomplete | Test hooks and assertions | `src/lua/stdlib/test.jsl` skips the body after a setup failure, continues cleanup after cleanup failures, and searches table values in `toContain`. [Test](@/docs/api/test.md) now describes those cases and recursive-run errors. |
| Incomplete | Log capture scope | `src/lua/stdlib/log.jsl` keys capture stacks by `coroutine.running()` and returns a mutable list of frozen records. [Log](@/docs/api/log.md) now distinguishes coroutine scope from an OS thread and states result ownership. |
| Incomplete | Analysis file discovery | `jpk/builtin/src/analysis/init.jsl` selects `.lua` and `.jsl` by default, skips hidden entries during directory traversal, and retains physical paths. [Analysis](@/docs/api/analysis.md) now states these options, capability requirements, and path ownership. |
| Incomplete | Localization ownership and invalidation | `jpk/builtin/src/l10n/init.jsl` returns fresh lists, deduplicates canonical locale tags, validates before replacing preferences, and increments the generation on a successful `setLocales`. [Localization](@/docs/api/l10n.md) now states these details; `jpk/builtin/tests/l10n/run.jsl` exercises canonicalization and reload behavior. |
| Incomplete | Structured value ownership and encoding | `jpk/builtin/src/data/value.jsl` marks arrays in place, mutates maps, and writes non-finite numbers differently by format. [Values](@/docs/api/value.md) now states those contracts and the TOML root restriction; `jpk/builtin/tests/data/` covers the encoders. |
| Incomplete | Plain-table JSON encoding | `jpk/builtin/src/data/json.jsl` escapes bytes individually in Latin-1 mode and recursively encodes tables without cycle detection. [JSON](@/docs/api/json.md) now states the UTF-8 consequence and cyclic-input limit. |
| Incomplete | YAML limits and keys | `jpk/builtin/src/data/yaml.jsl` defaults to a depth of 512 and rejects duplicate or NaN mapping keys. [YAML](@/docs/api/yaml.md) now states those limits for both decoding functions. |
| Defect fixed | Ordered map keys | Assigning `nil` with `value.set` removed the stored value but left its key in the ordering list; setting it again inserted a duplicate key. `set` now records `None`, matching `struct`, and the built-in data tests cover replacement and JSON output. |
| Defect fixed | JSON number overflow | `json.decode` previously accepted an out-of-range number as infinity, which `json.encode` then wrote as `null`. The decoder now rejects overflow, and the built-in data tests cover positive and negative cases. |
| Defect fixed | Checking scripts and modules together | `jess check` previously marked every named path as a script in one shared analysis, so a named file required by another was rejected as a module. Each entry now has its own analysis; `tests/cli/check.rs` covers both orders and deduplicated diagnostics. |
| Defect fixed | Package Source include roots | `src/lua/stdlib/package.jsl` followed a symlink when it was an included root or an intermediate component, despite rejecting links found inside a walked folder. It now checks each include component and the manifest; `tests/lua/package.rs` covers inspection, packing, and peeking. |
| Incomplete | Exact behavior of generated types and handle methods | The catalog exposes their declarations for search, but `checked = false` does not certify lifecycle, error, scheduling, or resource contracts. The API pages and tests remain the behavioral evidence. |
| Incomplete | CLI and platform contracts | The command groups have reference pages and exercised examples, but a complete parser-to-reference and cross-platform audit remains open. |
| Lacking examples | Device-dependent operations | The window fixture is checked without a device in CI; the offline audio fixture exercises processing without an audio device. Driver and hardware behavior still needs destination testing. |
