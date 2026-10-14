---
title: 'Built-in Modules'
description: 'The @jess/ namespace: jess-owned Luau modules, embedded, frozen, typed, and unshadowable.'
weight: 95
extra:
  kind: guide
---

`@jess/` is jess's own namespace inside `require`. Three kinds of thing live there, and all are jess's to define:

- **jess's members.** Every table under `jess` is a module too: `require('@jess/plugin')` is `jess.plugin`, `require('@jess/shader')` is what a shader module starts with.
- **The built-in modules.** [`@jess/log`](@/docs/api/log.md), [`@jess/test`](@/docs/api/test.md), [`@jess/jpk`](@/docs/api/jpk.md), and [`@jess/package`](@/docs/api/package.md): libraries jess ships for scripts and packages, written in Luau.
- **The built-in package's exports.** [`@jess/style`](@/docs/api/style.md), the Luau style checker, is a module of [jess's built-in package](@/docs/scripting/packages.md#the-built-in-package), `jpk/builtin`, which the binary carries. It resolves in every runtime, like the built-in modules.

"Built-in" says who owns the API and who resolves the name. It doesn't say what language the module is written in. `@jess/log` is a hundred lines of Luau over one native function, a structured-log sink the host owns; `@jess/test` is Luau over `@jess/log`. `@jess/jpk` is a ZIP reader and writer over `@dream/bytes`' codecs, and `@jess/package` is the package model over `@jess/jpk` and a handful of file reads only it is handed. They ship inside the jess binary, install through the same runtime plan as the native `@dream/*` modules, and version with jess.

## The rules

**The namespace is closed.** `require` resolves a `@jess/` name to a member of `jess`, a built-in module, or an export of the built-in package, or fails: `module not found: @jess/whatever: @jess/ is jess's own namespace (...), never a file`. No file in a data folder, no module in another package, and no `.jessrc` alias can stand in for one. The built-in package can't take a name jess already provides either: `@jess/test` stays the built-in module. `jess/log.lua` in your data folder is `require('jess.log')`, a different name, and stays that.

**Exports are frozen.** `require('@jess/test').expect = evil` is an error (`attempt to modify a readonly table`), and so is changing a logger or an expectation. The same table comes back from every `require`.

**They're typed like everything else.** The folder `jess luau-types` writes has a stub for each, `types/jess/log.jsl` and the rest, and the `.jessrc` `jess init` writes aliases `@jess/` to them, so `local log = require '@jess/log'` is fully typed under `--!strict` with no annotation, the same way as `@jess/plugin` or `@dream/vfs`.

**They stay small.** Four modules: two for scripts, and two that packages needed before a package could exist, since the thing that opens packages can't itself come in one. A package that wants more brings it as a package.

## Using them

```luau
--!strict
local log = require '@jess/log'
local test = require '@jess/test'

local logger = log.new('mymod', { version = '1.2' })
logger.info('starting', { plugins = 3 })

local suite = test.new()
suite.it('logs inside a capture', function()
  local records = log.capture(function() logger.warn 'inside' end)
  test.expect(#records).toBe(1)
  test.expect(records[1].fields).toEqual { version = '1.2' }
end)
```

Where records go depends on the host. `jess run` writes them to stderr as JSON lines, one per record, dropping `debug`. `jess test` collects them per case. A Rust program embedding `jess-lua` sets its own sink with `jess_lua::stdlib::set_log_sink`, or gives a script domain one log stream for all its contexts and workers with `DomainSpec::logs`. A script can't choose: that's the point of the sink living outside the VM.

## Under the hood

The source is `libs/lua/stdlib/*.jsl`, embedded with `include_str!`. `jess_lua::stdlib::StdlibExtension` declares each module and its members to the plan, with their Luau types, then at install compiles the source (once per thread, cached as bytecode), runs it as a factory that receives the values it's built on (`emit` for the logger, the logger module for the test module, the file reads and `@jess/jpk` for the package module), freezes what comes back, and installs the members as the plan's module. The plan freezes the module table and registers it with Luau's `require`; jess's own `require` sends `@jess/` names there and nowhere else.

The contract inventory (`libs/lua/tests/contracts/inventory.toml`) lists both modules' members, the surface probe reaches them through `require`, and `libs/lua/tests/stdlib.rs` pins the rules above: frozen, private sink, closed namespace, no shadowing by a data folder's file or a package's own module.
