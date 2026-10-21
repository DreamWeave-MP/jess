---
title: 'jess run'
description: 'Run a Luau script with the jess API, optionally with packages activated.'
weight: 40
extra:
  kind: reference
---

{{ api_signature(value="jess run [--package PACKAGE]... [--package-path FOLDER]... [--layer ID]... [--capability NAME]... [--profile | --emit-asm FILE | --disassemble FILE] <SCRIPT> [ARGS]...") }}

Runs a Luau script with the whole jess API loaded.

| Argument / option | Does |
| --- | --- |
| `<SCRIPT>` | The script to run, by its path, a `.jsl` file. A missing one is `error: 'nope.jsl' isn't a file`; any other extension is `error: 'script.lua' isn't a .jsl file: jess runs .jsl sources, and nothing else`. |
| `[ARGS]...` | Everything after the script, passed to it as strings. Flags here belong to the script, not to jess: `jess run fix.jsl --verbose` hands `--verbose` to `fix.jsl`. |
| `--package PACKAGE` | Activate a [package](@/docs/scripting/packages.md), a Package Source folder or a `.jpk`, before the script runs (repeatable). Its exports are then `require`-able by the script, and the activated layers' entrypoints have run. |
| `--package-path FOLDER` | A folder of packages to find layers and dependencies in (repeatable): each Package Source folder and `.jpk` directly inside it, opened only when one is needed. A package named with `--package` is chosen over one found here; two found for one need are an error. |
| `--layer ID` | Activate only this layer of the packages, and the layers it requires (repeatable). Without any, every layer they declare. Needs `--package`. |
| `--capability NAME` | Grant the script a capability it doesn't hold by default (repeatable): `filesystem.read`, `network.transport`, and the rest of the [list](@/docs/reference/capabilities.md). A name no part of jess declares is an error, not a grant of nothing. |
| `--profile` | After the script, print its call, sampling, and native-block statistics. |
| `--emit-asm FILE` | Write the native assembly generated for the script to `FILE`, without running it. |
| `--disassemble FILE` | Write the script's Luau bytecode, as text, to `FILE`, without running it. |

Plus the [global options](@/docs/cli/global-options.md).

## Running one script

```sh
jess run args.jsl dave --loud
```

The script's arguments arrive two ways:

- as `...`, the script's varargs: `local name, flag = ...`
- in the global `arg` table: `arg[1]`, `arg[2]`, ..., with the script's own path in `arg[0]`

```luau
--!strict
-- args.jsl
local name, flag = ...

print('greeting', name, 'with', #arg, 'args; I am', arg[0], flag)
```

```text
greeting	dave	with	2	args; I am	args.jsl	--loud
```

The globals are Luau's libraries, `jess`, `arg`, `print`, and `require`. Under `jess run`, `require` finds files relative to the script (`require('./lib/util')`, trying `util.jsl` then `util/init.jsl`), jess's own modules (`@jess/log`), L3i's (`@dream/fs`), and the exports of `--package` packages. Nothing else: there is no search path, and a name nothing provides is `module not found`. [Your First Script](@/docs/scripting/intro.md) starts from here.

## Running with packages

`--package` resolves the packages' layers into a plan and activates it before the first line of the script runs: their exports resolve through `require`, and the activated layers' entrypoints run, in order:

```luau
--!strict
-- hello.jsl
local greeter = require '@greeter'

print(greeter.greet 'world')
```

```text
$ jess run --package greeter.jpk hello.jsl
Hello, world.
```

A plan that doesn't resolve stops the run before anything has started. An entrypoint is admitted against what the script holds (below): one that requires more stops the run before anything is activated, naming the entrypoint, its layer, and its package. `jess package graph` shows what each requires without running anything, and `--layer` picks the layers you want when a package declares more than you need:

```sh
jess run --package greeter --layer example.greetings hello.jsl
```

## Capabilities

A script you run is yours, so it holds what you already have: its terminal (`process.stdio`, for `@dream/process`'s `write`) and the input devices in front of you (`input.actions`, `input.observe`, `input.configure`). Everything else is a flag away and denied without one, with the function and the capability named:

```text
$ jess run read.jsl
error: dream.fs.readFileString: needs the 'filesystem.read' capability, which this runtime does not grant
$ jess run --capability filesystem.read read.jsl
```

A script can ask instead of trying: [`jess.granted`](@/docs/api/jess.md#jess-granted) says whether it holds a capability, so a tool can explain which flag it wants rather than fail halfway. [Capabilities](@/docs/reference/capabilities.md) lists every name, what asks for it, and what grants it.

There's no script domain under `jess run`, so `jess.worker.spawn` and `jess.context.bind` refuse: `worker.spawn: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and command targets run scripts outside one`. Those belong to [applications](@/docs/scripting/applications.md) and embedding hosts.

## Looking at the code

Three flags for finding out what Luau is doing, none of which needs another tool:

- `--profile` runs the script and then prints how it spent its time: calls, samples, and which native blocks ran. It is the first thing to reach for before writing `--!native` on anything; [Performance](@/docs/scripting/performance.md) says how to read it.
- `--emit-asm FILE` writes the native code the script would compile to, and doesn't run it.
- `--disassemble FILE` writes the bytecode as text, and doesn't run it.

## Output

`jess run` doesn't print a structured document. What you see is whatever the script `print`s, as it prints it, and what [`@jess/log`](@/docs/api/log.md) writes to stderr as JSON lines. A script's first returned number, an integer from 0 to 255, is the exit status; other return values are ignored.

## Errors

A script that raises an error stops the run, prints the error on stderr, and exits `1`:

```text
error: err.jsl:1: boom
```

Anything the script already wrote stays written.

A script isn't sandboxed beyond its capabilities: grant it `filesystem.write` and it can write any file you can, which is the point. Read a script before you grant it anything.

## Related

- [`jess test`](@/docs/cli/test.md) runs modules that return a test suite, with the same capability rules.
- [`jess package run`](@/docs/cli/package.md#jess-package-run) runs a package's command target: the same thing as `jess run` with the package's layers activated, except that the target's manifest, not the command line, says what it needs.
