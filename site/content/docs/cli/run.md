---
title: 'jess run'
description: 'Run a Luau script with the jess API, optionally with packages activated.'
weight: 40
extra:
  kind: reference
---

{{ api_signature(value="jess run [--package PACKAGE]... [--layer ID]... [--capability NAME]... <SCRIPT> [ARGS]...") }}

Runs a Luau script with the whole jess API loaded.

| Argument / option | Does |
| --- | --- |
| `<SCRIPT>` | The Luau file to run, by its path. Required: a missing one is `error: 'nope.jsl' isn't a file`. |
| `[ARGS]...` | Everything after the script, passed to it as strings. Flags here belong to the script, not to jess: `jess run fix.jsl --verbose` hands `--verbose` to `fix.jsl`. |
| `--package PACKAGE` | Activate a [package](@/docs/scripting/packages.md), a Package Source folder or a `.jpk`, before the script runs (repeatable). Its exports are then `require`-able by the script, and the activated layers' entrypoints have run. |
| `--layer ID` | Activate only this layer of the packages, and the layers it requires (repeatable). Without any, every layer they declare. Needs `--package`. |
| `--capability NAME` | Grant the script a capability it doesn't hold by default (repeatable): `filesystem.read`, `network.transport`, ... A name no part of jess declares is an error, not a grant of nothing. |

Plus the [global options](@/docs/cli/global-options.md).

## Running one script

```sh
jess run count-books.jsl "Data Files/Morrowind.esm" --verbose
```

The script's arguments arrive two ways:

- as `...`, the script's varargs: `local pluginPath, flag = ...`
- in the global `arg` table: `arg[1]`, `arg[2]`, ..., with the script's own path in `arg[0]`

```luau
-- count-books.jsl
local pluginPath, flag = ...
print('counting books in', pluginPath, 'with', #arg, 'args; I am', arg[0], flag)

local plugin = jess.plugin.open(pluginPath, { types = { 'BOOK' } })
print(plugin:select({ type = 'book' }):count())
```

```text
counting books in	Data Files/Morrowind.esm	with	2	args; I am	count-books.jsl	--verbose
574
```

The globals are all there: `jess`, `vfstool`, `openmwConfig`, `dreamPath`, and `require`. Under `jess run`, `require` finds files relative to the script (`require('./lib/util')`), jess's built-in modules (`@jess/log`), and the exports of `--package` packages. [Your First Script](@/docs/scripting/intro.md) starts from here.

## Running over a load order

`jess run` doesn't read an `openmw.cfg`, so a script starts with no load order: `jess.openmw.default()` is `nil`, and plugin names that aren't paths find nothing. A script that wants one loads it, from a path it was given:

```luau
--!strict
-- merge.jsl: run TES3Merge over a load order.
local config: string = ...
local openmw = jess.openmw.load(config)
local tes3merge = require '@dreamwind/tools/tes3merge'

tes3merge({}, openmw)
```

```text
$ jess run --package jess/jpk/dreamwind merge.jsl ~/.config/openmw/openmw.cfg
tes3merge: wrote 22 merged records to /home/you/.local/share/openmw/data/Merged Objects.esp
```

`jess.openmw.load` takes an `openmw.cfg` or the folder holding one, and with no argument finds OpenMW's own the way OpenMW does. `jess.openmw.configPath(path)` makes a config the script's default, so plugin names and `require` resolve through it. [Merging and Fixing a Load Order](@/docs/guides/load-order-scripts.md) is the guide; [Load-Order Scripts](@/docs/scripting/load-order-scripts.md) covers writing your own.

## Running with packages

`--package` resolves the packages' layers into a plan and activates it before the first line of the script runs: their exports resolve through `require`, and the activated layers' entrypoints run, in order:

```luau
-- hello.jsl
local greeter = require '@greeter'
print(greeter.greet 'Nerevarine')
```

```text
$ jess run --package greeter.jpk hello.jsl
Wake up, Nerevarine
```

A plan that doesn't resolve stops the run before anything has started. An entrypoint is admitted against what the script holds (below): one that requires more stops the run before anything is activated, and `jess package graph` shows what each requires.

## Capabilities

A script you run is yours, so it holds what you already have: its terminal (`process.stdio`, for `@dream/process`'s `write`) and the input devices in front of you (`input.actions`, `input.observe`, `input.configure`). Everything else is a flag away and denied without one, with the function and the capability named:

```text
$ jess run read.jsl
error: dream.fs.readFileString: needs the 'filesystem.read' capability, which this runtime does not grant
$ jess run --capability filesystem.read read.jsl
```

There's no script domain under `jess run`, so `@jess/worker`'s `spawn` and `@jess/context`'s `bind` refuse: `worker.spawn: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and command targets run scripts outside one`.

## Output

`jess run` doesn't print a structured document. What you see is whatever the script `print`s, as it prints it. A script's first returned number, an integer from 0 to 255, is the exit status; other return values are ignored.

## Errors

A script that raises an error stops the run, prints the error on stderr, and exits `1`:

```text
error: err.jsl:2: boom
```

Anything the script already wrote stays written.

Scripts aren't sandboxed. They can read and write any file you can, which is the point, but read one before you run it.

## Related

- [`plugin modify --script`](@/docs/cli/plugin.md#modify) and [`nif modify --script`](@/docs/cli/nif.md#modify) run a script on one file, called with `(handle, path)`, and write the file if it changed.
- [Modify Scripts](@/docs/scripting/modify-scripts.md) covers those.
