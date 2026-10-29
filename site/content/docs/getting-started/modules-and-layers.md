---
title: 'Modules, Layers, and Dependencies'
description: 'Follow a require, understand activation, and distinguish the two dependency graphs.'
weight: 32
extra:
  kind: guide
---

Several Jess concepts have similar names because they solve related problems. They do not do the same job. A module supplies code; a package supplies modules; a layer describes a system; a dependency makes another package available. Keep those edges separate and resolution stops looking like magic.

## Start at the call site

When you see `require('@greeter')`, look for the export named `@greeter`. Its package's manifest maps the name to a file under `[modules]`, and a layer exports that name. The module runs when first required and its returned value is cached. A relative require such as `./words` instead resolves from the requiring file, first as `words.jsl`, then as `words/init.jsl`.

An exported name is a contract between packages. A relative path is an implementation detail inside one package. Keep internal files private unless another package needs a supported API.

Jess makes a returned package-module table read-only at its top level, but it does not recursively freeze nested tables or buffers. Decide explicitly whether an export shares mutable state; [require](@/docs/api/jess.md#require) describes caching and ownership.

For example, if `@example/state` returns `{ nested = { count = 1 } }`, both calls to `require` below get the same outer table. The outer table is read-only; its nested table is not:

```luau
--!strict
local state = require '@example/state'
local same = require '@example/state'

assert(rawequal(state, same))
assert(table.isfrozen(state))
assert(not table.isfrozen(state.nested))
state.nested.count += 1
print(same.nested.count)
```

This is a shared-state example, not a recommendation that every export share state. The complete package is `tests/fixtures/docs/module-ownership` in the Jess repository. The documentation tests check it and run both its source and a packed JPK; each prints `2`.

## Dependency is not activation

Suppose a package supplies a formatter and a command uses it. The command package declares the formatter package in `[dependencies]`, then requires its export. That declaration tells resolution which package is needed and permits the command package's modules to use its exports. It does not request that every layer the formatter provides start up.

A layer's `requires` is a different graph: it names other layers required when that layer is resolved for activation. An entrypoint is code run because its layer was activated. Requiring an export is not the same operation as activating that entrypoint.

This distinction lets a package offer a library without making every caller start a background system just to use one function. It also gives a host a plan to inspect before it grants authority and activates behavior.

## When resolution fails

First decide which name failed: the package id, the layer id, or the module export. Check its spelling and the corresponding manifest field. Then check that the right package and version were offered. `jess package inspect` shows what a package declares; `jess package graph` shows the resolved layers and entrypoints. [Troubleshooting](@/docs/reference/troubleshooting.md#resolution) covers the actual error messages.

A workspace makes packages available for development, but it is not a substitute for `[dependencies]`. The package has to describe its own requirements when it is distributed too. The full rules, including version selection and duplicate providers, are in [Packages](@/docs/scripting/packages.md).
