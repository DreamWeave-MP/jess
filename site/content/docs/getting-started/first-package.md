---
title: 'Your First Package'
description: 'A manifest, a module, a test, and a command target; inspected, tested, type-checked, run, packed, and run again from the JPK.'
weight: 20
extra:
  kind: guide
---

A script is a file. A package is a folder with a `jess.toml` at the top that says what the folder is: what it exports, what runs, and what that needs. This page builds the smallest one worth having, `greeter`, and puts it through every command that applies. Every output below is what the binary printed.

## The folder

Start with `jess init`, which sets up the types so your editor can see the API:

```sh
jess init greeter --editor none
```

```text
dir: greeter
editor: none
types:
  folder: greeter/types
  definitions: greeter/types/jess.d.jsl
  files: 44
  removed: []
  aliases:
    dream: types/dream
    jess: types/jess
written:
- path: greeter/.jessrc
- path: greeter/main.jsl
skipped: []
next_steps:
- 'run the example: jess run greeter/main.jsl'
- rerun jess init after updating jess to refresh types/
```

Pass `--editor vscode`, `neovim`, `zed`, or `helix` instead of `none` to get that editor's luau-lsp settings too; [Editor Setup](@/docs/getting-started/editor-setup.md) covers them. Delete `main.jsl`, since the package has its own entry, and add four files:

```text
greeter/
  jess.toml
  src/init.jsl
  tests/run.jsl
  tools/greet.jsl
  types/           # from jess init; not part of the package
  .jessrc
```

`jess.toml`, the whole description:

```toml
manifest_version = 1

[package]
id = "com.example.greeter"
version = "1.0.0"
jess_api = "1"
include = ["src", "tests", "tools"]
tests = ["tests/run.jsl"]
default_target = "greet"

[modules]
"@greeter" = "src/init.jsl"

[[layer]]
id = "example.greetings"
exports = ["@greeter"]

[targets.greet]
kind = "command"
command = "greet"
entry = "tools/greet.jsl"
capabilities = ["process.environment"]
```

Read it top to bottom. The package is `com.example.greeter` at version 1.0.0, and its payload is the three folders `include` names; `types/` and `.jessrc` are not in it, which is why they can sit there. One module, `@greeter`, lives at `src/init.jsl`. One layer, `example.greetings`, exports it: a host that activates the layer makes `require('@greeter')` work. One command target, `greet`, runs `tools/greet.jsl` and needs to read the environment, and it is the default, so `jess package run greeter` runs it without `--target`. [The Manifest](@/docs/reference/manifest.md) has every key.

`src/init.jsl`, the module. It returns a frozen table, because what comes back from `require` is frozen anyway and you may as well say so:

```luau
--!strict
local function greet(name: string): string
  return `Hello, {name}.`
end

return table.freeze { greet = greet }
```

`tests/run.jsl`, the test, a module returning an [`@jess/test`](@/docs/api/test.md) suite:

```luau
--!strict
local test = require '@jess/test'
local greeter = require '@greeter'

local suite = test.new()

suite.it('greets by name', function()
  test.expect(greeter.greet 'world').toBe 'Hello, world.'
end)

return suite
```

`tools/greet.jsl`, the target's entry. It is a script with the package's layers already activated, so `@greeter` resolves, and it reads the environment, which is why the manifest asked for `process.environment`:

```luau
--!strict
local process = require '@dream/process'
local greeter = require '@greeter'

print(greeter.greet(process.env 'USER' or 'stranger'))
```

## Look at it

`jess package inspect` reads the manifest and the payload and prints what the package is. It runs nothing:

```sh
jess package inspect greeter
```

```text
input: greeter
id: com.example.greeter
version: 1.0.0
digest: 3e450d1a907a8ce0e6b034a434cc5b04caed51c3ff816bfa9a856dd92b26b076
kind: source
location: greeter
jess_api: '1'
default_target: greet
files: 4
modules:
- name: '@greeter'
  path: src/init.jsl
layers:
- id: example.greetings
  requires: []
  exports:
  - '@greeter'
  entrypoints: []
managed: []
targets:
- id: greet
  kind: command
  entry: tools/greet.jsl
  capabilities:
  - process.environment
  command: greet
dependencies: []
dev_dependencies: []
capabilities: []
tests:
- tests/run.jsl
```

Four files: the manifest and the three under `include`. The `digest` is the content hash of exactly those, and it will come up again.

## Test it

```sh
jess package test greeter
```

```text
id: com.example.greeter
version: 1.0.0
tests:
- tests/run.jsl
providers:
- kind: source
  location: greeter
  digest: 3e450d1a907a8ce0e6b034a434cc5b04caed51c3ff816bfa9a856dd92b26b076
  run:
    passed: 1
    failed: 0
    ...
- kind: jpk
  location: /tmp/jess-package-test-37516/com.example.greeter-1.0.0.jpk
  digest: 3e450d1a907a8ce0e6b034a434cc5b04caed51c3ff816bfa9a856dd92b26b076
  run:
    passed: 1
    failed: 0
    ...
passed: 2
failed: 0
```

One test, two runs. `jess package test` runs the suite against the folder, then packs the folder into a temporary JPK and runs it again from there, and checks both providers report the same digest. That is the "Package Source and JPK are the same package" rule being enforced rather than asserted. A test that passes from the folder and fails from the archive has found a file the manifest forgot to include.

## Type-check it

```sh
jess package check greeter
```

```text
id: com.example.greeter
version: 1.0.0
files: 3
errors: []
```

Every `.jsl` under the package, in strict mode, with each `require` resolved the way the runtime will resolve it under the package's plan. Misspell `greet` as `gret` in the entry and the check says so, with the file, line, and column, before anyone runs it:

```text
errors:
- file: tools/greet.jsl
  line: 5
  column: 7
  message: 'Key ''gret'' not found in table ''{ read greet: (string) -> string }'''
error: 1 type error
```

## Run it

```sh
jess package run greeter
```

```text
error: cannot run target greet

required capability denied:
    process.environment

requested by:
    com.example.greeter 1.0.0
    target greet
```

This is the correct outcome. The manifest *declares* `process.environment`; it does not *grant* it, because a package never grants itself anything. You do:

```sh
jess package run --capability process.environment greeter
```

```text
Hello, dave.
```

`--trusted` grants everything the target declares, which `inspect` showed you a moment ago. Either way the target holds exactly what it declared and nothing else: no files, no network, not even the terminal beyond `print`. [Capabilities](@/docs/reference/capabilities.md) has the table.

A script of your own can use the package too. `jess run --package` activates its layers before the script's first line:

```luau
--!strict
-- hello.jsl
local greeter = require '@greeter'

print(greeter.greet 'world')
```

```sh
jess run --package greeter hello.jsl
```

```text
Hello, world.
```

## Pack it

```sh
jess pack greeter -o greeter.jpk
```

```text
input: greeter
id: com.example.greeter
version: 1.0.0
digest: 3e450d1a907a8ce0e6b034a434cc5b04caed51c3ff816bfa9a856dd92b26b076
files: 4
size: 1246
written:
  path: greeter.jpk
```

The same digest as the folder, because it is the same payload. The JPK is canonical: the same source packs to the same bytes on any machine, so two people building it get one file, not two files that happen to match.

Everything above works on the archive exactly as on the folder, because above the provider boundary they are one package:

```sh
jess package run --capability process.environment greeter.jpk
jess run --package greeter.jpk hello.jsl
```

```text
Hello, dave.
Hello, world.
```

The one exception is `test`, which runs from a Package Source only; on a `.jpk` it reports no providers and says why. Edit `src/init.jsl` and inspect both again, and the folder's digest moves while the archive's stays: a changed package is a new JPK, and you pack it again.

## Where next

- [Packages](@/docs/scripting/packages.md): layers, entrypoints, resolution, activation, dependencies, and what `require` may reach from inside a package.
- [The Manifest](@/docs/reference/manifest.md): every table and key, and the error each one gives when it is wrong.
- [Capabilities](@/docs/reference/capabilities.md): the fourteen names, who holds what, and how a grant is made.
- [`jess workspace`](@/docs/cli/workspace.md), once you have two packages that belong together.
