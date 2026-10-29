---
title: 'JSL, Luau, and L3i'
description: 'Which language and libraries a Jess script receives, and where the dialect is specified.'
weight: 15
extra:
  kind: guide
---

Jess runs Luau source in `.jsl` files through L3i. The extension identifies a Jess script; it is not a different spelling for a Roblox script. A script has Luau's language and standard libraries, L3i's dialect and `@dream/*` primitives, and Jess's own runtime and package APIs. Roblox services and object APIs do not come with Jess.

## The language and the dialect

Ordinary Luau syntax works in a `.jsl` file. L3i also accepts syntax that it lowers to ordinary Luau before compilation. For example, save this as `select.jsl`:

```luau
--!strict
local Values = { 1, 2, 3, 4 }
local doubled = [for item in Values if item > 2 => item * 2]

print(table.concat(doubled, ', '))
```

`jess check select.jsl` checks it; `jess run select.jsl` prints `6, 8`. The comprehension builds a dense list from the accepted elements. It is syntax handled by L3i's frontend, not a global function named `for` or a Roblox API. The matching fixture is `tests/fixtures/docs/jsl-dialect.jsl` in the Jess repository.

The [L3i dialect reference](https://DreamWeave-MP.github.io/l3i/docs/dialect/) explains the syntax, lowering, restrictions, and error cases. It describes L3i's published documentation; use the definitions and checks from the Jess version you actually run to verify version-specific behavior.

## The APIs

- Luau provides values, tables, functions, buffers, and its standard libraries. [Luau's documentation](https://luau.org) explains the language.
- [L3i's `@dream/*` modules](@/docs/api/l3i.md) provide primitives such as filesystem, bytes, networking, and process access. Jess gates operations that require authority.
- The [`jess` global](@/docs/api/jess.md), its `@jess/*` runtime modules, and the built-in package provide Jess's domains, application services, codecs, tools, and other APIs. Package exports add application-specific behavior on top.

An editor's types are generated for one installed Jess. [Editor Setup](@/docs/getting-started/editor-setup.md) explains how to generate and refresh them. The type checker and runtime are separate: check code before running it, and remember that a successful check cannot grant a capability or guarantee a file exists.
