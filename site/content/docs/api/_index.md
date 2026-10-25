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

One page per module. Each entry leads with the signature and a one-line summary, then behavior, parameters, returns, errors, and an example. This section is for looking things up, not for reading front to back; [Scripting](@/docs/scripting/_index.md) is the part that explains how the pieces fit.

Three kinds of module live here, and `require` resolves all of them the same way:

- **The `jess` global and its members**: `jess.context`, `jess.worker`, `jess.events`, `jess.interfaces`, `jess.application`, `jess.shader`, `jess.input`, `jess.audio`, and the values that belong to no module. Each member is also `require('@jess/<name>')`.
- **The built-in modules**, Luau that ships inside the binary: `@jess/log`, `@jess/test`, `@jess/jpk`, `@jess/package`.
- **The built-in package's exports**: `@jess/style`, `@jess/analysis`, `@jess/json`, `@jess/yaml`, `@jess/data/toml`, `@jess/data/value`, `@jess/l10n`, `@jess/input`, `@jess/audio`, `@jess/backup`, `@jess/http`.

The native primitives under all of it, files, bytes, processes, the network, and text, are [L3i's `@dream/*` modules](@/docs/api/l3i.md), documented on L3i's own site.

Option tables, the last argument most functions take, reject keys they don't take: the error names the key and lists the ones the function does take, so a typo fails instead of quietly doing the default.

The examples are written against the types `jess luau-types` writes, under `--!strict`. If one disagrees with the jess you have, the jess you have wins, and the page is wrong.
