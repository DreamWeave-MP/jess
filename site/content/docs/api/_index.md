---
title: 'Luau API Reference'
description: 'Lookup-first reference for every module, type, and function in the jess Luau API.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 60

extra:
  kind: api
---

This section is for looking things up, not for reading front to back; [Scripting](@/docs/scripting/_index.md) explains how the pieces fit. Most pages cover one module. Larger surfaces group related operations and types on one page.

Three kinds of module live here:

- **The `jess` global and its members**: `jess.context`, `jess.worker`, `jess.events`, `jess.interfaces`, `jess.application`, `jess.shader`, `jess.gpu`, `jess.input`, `jess.audio`, `jess.bench`, and the values that belong to no module. Each member is also `require('@jess/<name>')`.
- **The built-in modules**, Luau that ships inside the binary: `@jess/log`, `@jess/test`, `@jess/jpk`, `@jess/package`.
- **The built-in package's exports**: `@jess/style`, `@jess/analysis`, `@jess/json`, `@jess/yaml`, `@jess/data/toml`, `@jess/data/value`, `@jess/l10n`, `@jess/input`, `@jess/audio`, `@jess/render`, `@jess/render/camera`, `@jess/backup`, `@jess/http`.

The native primitives under all of it, files, bytes, processes, the network, and text, are [L3i's `@dream/*` modules](@/docs/api/l3i.md), documented on L3i's own site.

## Find a module

| What you need | Reference |
| --- | --- |
| Runtime values, grants, and `require` | [jess](@/docs/api/jess.md), [context](@/docs/api/context.md), [workers](@/docs/api/worker.md) |
| Application lifecycle and communication | [application](@/docs/api/application.md), [events](@/docs/api/events.md), [interfaces](@/docs/api/interfaces.md) |
| Packages and containers | [package](@/docs/api/package.md), [JPK](@/docs/api/jpk.md) |
| Tests, logging, and analysis | [test](@/docs/api/test.md), [log](@/docs/api/log.md), [analysis](@/docs/api/analysis.md), [style](@/docs/api/style.md) |
| Structured data and localization | [JSON](@/docs/api/json.md), [YAML](@/docs/api/yaml.md), [TOML](@/docs/api/toml.md), [values](@/docs/api/value.md), [localization](@/docs/api/l10n.md) |
| Files, network, and text | [files](@/docs/api/fs.md), [HTTP](@/docs/api/http.md), [regular expressions](@/docs/api/regex.md), [backup](@/docs/api/backup.md), [L3i](@/docs/api/l3i.md) |
| Input, rendering, and audio | [input](@/docs/api/input.md), [render](@/docs/api/render.md), [GPU](@/docs/api/gpu.md), [shader](@/docs/api/shader.md), [audio](@/docs/api/audio.md) |
| Measurement | [bench](@/docs/api/bench.md) |

Some pages describe several related APIs rather than one module. In particular, input and audio cover both the built-in package and native runtime surfaces; the page identifies each one.

Option tables, the last argument most functions take, reject keys they don't take: the error names the key and lists the ones the function does take, so a typo fails instead of quietly doing the default.

The examples are written against the types `jess luau-types` writes, under `--!strict`. If one disagrees with the jess you have, the jess you have wins, and the page is wrong.
