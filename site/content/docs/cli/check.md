---
title: 'jess check'
description: 'Type-check scripts in strict mode from the command line, with jess''s types and any packages they require. Runs nothing.'
weight: 46
extra:
  kind: reference
---

{{ api_signature(value="jess check [--package PACKAGE]... [--package-path FOLDER]... [--layer ID]... <SCRIPTS>...") }}

Type-checks `.jsl` scripts the way an editor in a folder `jess init` set up checks them, and prints what it finds. Nothing runs: no script, no entrypoint, no target. [`jess package check`](@/docs/cli/package.md#jess-package-check) is the same checker over a whole Package Source; this is for the scripts that aren't in one.

| Argument / option | Does |
| --- | --- |
| `<SCRIPTS>...` | The scripts to check, by path, `.jsl` files. A file one of them reaches with `./` or `../` is checked with it and reported under the path the `require` resolved to. |
| `--package PACKAGE` | A package whose exports the scripts require (repeatable): a Package Source folder or a `.jpk`, resolved as [`jess run --package`](@/docs/cli/run.md#running-with-packages) resolves it, so `require '@greeter'` has a type. Its modules are mounted for the checker to read, and none of them run. |
| `--layer ID` | Resolve only this layer of the packages, and the layers it requires (repeatable). Without any, every layer they declare. Needs packages. |
| `--package-path FOLDER` | A folder of packages to find layers and dependencies in (repeatable), as `jess run --package-path` reads one. |

Plus the [global options](@/docs/cli/global-options.md). The report is one document in the `--format` you ask for, and the exit status is `1` when any error was found, with `error: N type errors` on stderr.

## What it resolves

`require` is typed the way the runtime resolves it: `@jess/` and `@dream/` names through the types folder `jess luau-types` writes, so the built-in modules, the built-in package's exports, and L3i's modules all have their real types; `./` and `../` names beside the script, trying `name.jsl` then `name/init.jsl`; and the exports of the packages named with `--package`. A bare name resolves to nothing and is reported, since it would be at run time too.

Every file is checked in strict mode, as the `.jessrc` from `jess init` sets, and a `--!nonstrict` or `--!nocheck` directive at the top of a file still wins for that file.

You can name a script and a file it requires in the same invocation. Each named entry is analyzed independently, with its required files treated as modules; duplicate diagnostics are reported once. The `files` count remains the number of paths given, not the number of files reached through `require`.

## Examples

A clean script:

```sh
jess check scripts/main.jsl
```

```yaml
files: 1
errors: []
```

A script with a mistake, and a mistake in a file it requires:

```luau
--!strict
local count: number = 'three'
local util = require './lib/util'

print(count, util.answer)
```

```luau
--!strict
local answer: string = 42

return { answer = answer }
```

```text
$ jess check scripts/bad.jsl
files: 1
errors:
- file: scripts/bad.jsl
  line: 2
  column: 23
  message: Expected this to be 'number', but got 'string'
- file: scripts/lib/util.jsl
  line: 2
  column: 24
  message: Expected this to be 'string', but got 'number'
error: 2 type errors
```

`files` counts the scripts given; the required file is reported because the script reached it, not because it was named.

A script that uses a package's export:

```luau
--!strict
local greeter = require '@greeter'
local count: number = greeter.greet 'world'

print(count)
```

```text
$ jess check --package greeter scripts/pkg.jsl
files: 1
errors:
- file: scripts/pkg.jsl
  line: 3
  column: 23
  message: Expected this to be 'number', but got 'string'
error: 1 type error
```

Without `--package greeter`, `@greeter` resolves to nothing, and the error is `Unknown require: unsupported path` on that line instead: the check knows exactly what `jess run` would know.

## Related

- [`jess run`](@/docs/cli/run.md) runs what passed; the two take the same package flags.
- [`jess package check`](@/docs/cli/package.md#jess-package-check) checks a Package Source, every `.jsl` under it, with each `require` resolved under the package's plan.
- [Editor Setup](@/docs/getting-started/editor-setup.md) gets the same checks into your editor as you type.
