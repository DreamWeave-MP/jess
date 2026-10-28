---
title: 'Built-in Modules'
description: 'The @jess/ namespace: jess-owned Luau modules, embedded, frozen, typed, and unshadowable.'
weight: 30
extra:
  kind: guide
---

`@jess/` is jess's own namespace inside `require`. Three kinds of thing live there, and all are jess's to define:

- **jess's members.** Every table under `jess` is a module too: `require('@jess/worker')` is `jess.worker`, and `require('@jess/shader')` is what a shader module starts with.
- **The built-in modules.** [`@jess/log`](@/docs/api/log.md), [`@jess/test`](@/docs/api/test.md), [`@jess/jpk`](@/docs/api/jpk.md), and [`@jess/package`](@/docs/api/package.md): libraries jess ships for scripts and packages, written in Luau and embedded in the binary.
- **The built-in package's exports.** `org.dreamweave.jess`, the one package that may export under `@jess/`, which the binary also carries. Fifteen modules: [`@jess/style`](@/docs/api/style.md), [`@jess/analysis`](@/docs/api/analysis.md), [`@jess/json`](@/docs/api/json.md), [`@jess/yaml`](@/docs/api/yaml.md), [`@jess/data/toml`](@/docs/api/toml.md), [`@jess/data/value`](@/docs/api/value.md), [`@jess/l10n`](@/docs/api/l10n.md), [`@jess/input`](@/docs/api/input.md), [`@jess/audio`](@/docs/api/audio.md), [`@jess/render` and `@jess/render/camera`](@/docs/api/render.md), [`@jess/backup`](@/docs/api/backup.md), [`@jess/http`](@/docs/api/http.md), [`@jess/net`](@/docs/api/net.md), and [`@jess/websocket`](@/docs/api/websocket.md). They resolve in every runtime, like the built-in modules, and [Packages](@/docs/scripting/packages.md#the-built-in-package) says how the package itself is handled.

"Built-in" says who owns the API and who resolves the name. It doesn't say what language the module is written in. `@jess/log` is a hundred lines of Luau over one native function, a structured-log sink the host owns; `@jess/test` is Luau over `@jess/log`. `@jess/jpk` is a ZIP reader and writer over `@dream/bytes`' codecs, and `@jess/package` is the package model over `@jess/jpk` and a handful of file reads only it is handed. They ship inside the jess binary, install through the same runtime plan as the native `@dream/*` modules, and version with jess.

## The rules

**The namespace is closed.** `require` resolves a `@jess/` name to a member of `jess`, a built-in module, or an export of the built-in package, or fails: `module not found: @jess/whatever: @jess/ is jess's own namespace (its members, such as @jess/shader, @jess/log, @jess/test, @jess/jpk, @jess/package, and the built-in package's exports, such as @jess/style), never a file`. No file beside your script, no module in another package, and no `.jessrc` alias can stand in for one. The built-in package can't take a name jess already provides either: `@jess/test` stays the built-in module.

**Exports are frozen.** `require('@jess/test').expect = evil` is an error (`attempt to modify a readonly table`), and so is changing a logger or an expectation. The same table comes back from every `require`. This holds for every package's exports, not only jess's: one package can't patch another's module for the whole plan.

**They're typed like everything else.** The folder `jess luau-types` writes has a stub for each built-in module, `types/jess/log.jsl` and the rest, and a copy of each of the built-in package's modules, `types/jess/data/json.jsl` under its own name, since a Luau module's type is its source; the `.jessrc` `jess init` writes aliases `@jess/` to them, so `local log = require '@jess/log'` is fully typed under `--!strict` with no annotation, the same way as `@jess/worker` or `@dream/fs`.

**They stay small.** Four built-in modules: two for scripts, and two that packages needed before a package could exist, since the thing that opens packages can't itself come in one. Everything else jess ships in Luau is in the built-in package, which goes through the same package model as yours. A package that wants more brings it as a package.

## Using them

```luau
--!strict
local log = require '@jess/log'
local test = require '@jess/test'

local logger = log.new('build', { stage = 'render' })
logger.info('wrote', { pages = 3 })

local suite = test.new()

suite.it('logs inside a capture', function()
  local records = log.capture(function() logger.warn 'slow' end)
  test.expect(#records).toBe(1)
  test.expect(records[1].fields).toEqual { stage = 'render' }
end)

return suite
```

Where records go depends on the host. `jess run` writes them to stderr as JSON lines, one per record, dropping `debug`:

```text
{"level":"info","target":"build","message":"wrote","fields":{"pages":3.0,"stage":"render"},"source":"build.jsl"}
```

`jess test` collects them per case. A Rust program embedding jess sets its own sink, or gives a script domain one log stream for all its contexts and workers. A script can't choose: that's the point of the sink living outside the VM.

## Under the hood

The built-in modules' source is `src/lua/stdlib/*.jsl`, embedded with `include_str!`. `src/lua/stdlib.rs` declares each module and its members to the plan, with their Luau types, then at install compiles the source (once per thread, cached as bytecode), runs it as a factory that receives the values it's built on (`emit` for the logger, the logger module for the test module, the file reads and `@jess/jpk` for the package module), freezes what comes back, and installs the members as the plan's module. The plan freezes the module table and registers it with Luau's `require`; jess's own `require` sends `@jess/` names there and nowhere else.

The built-in package is the source tree under `jpk/builtin`, embedded file by file through `src/lua/builtin.rs`. A package claiming its id is accepted only when it is byte-for-byte the embedded copy, and a test holds the embedded file list to the manifest's payload.

`tests/lua/stdlib.rs` pins the rules above: frozen, private sink, closed namespace, no shadowing by a file beside the script or a package's own module, and every module typed through `require`.
