---
title: 'Packages'
description: 'Package Sources, JPKs, layers, and activation: how Luau software ships for jess.'
weight: 40
extra:
  kind: guide
---

Everything jess runs that isn't a one-off script is a package. A package has two forms and one meaning:

- a **Package Source** is a folder with a `jess.toml` at its top: what you edit, test, and keep in git;
- a **JPK** is that folder packed into one file by `jess pack`: what you ship. It is a ZIP, and it is immutable. A changed package is a new JPK;
- a **package** is what jess sees after opening either. The two are the same package, down to their [content digest](@/docs/api/package.md#the-content-digest), and nothing a package's code does may tell them apart.

A package has **layers**, named semantic systems like `example.greetings` that a host asks for. A layer **exports** modules, names like `@greeter` that `require` resolves: inert API that does nothing until something calls it. A layer can also have **entrypoints**, modules jess runs because the layer was activated: registering a command, starting a system. Asking for layers and getting an **activation plan** back is resolution, and it runs no code. Mounting the plan, so its exports resolve, and running its entrypoints is **activation**, and that is the host's decision, never a side effect of opening something.

```text
Package Source ──jess pack──▶ JPK
        │                      │
        └──── both open as ────┘
                   ▼
                package
         (modules, layers, payload)
                   │  resolve the requested layers: no code runs
                   ▼
             activation plan
                   │  the host activates it
                   ▼
         require('@greeter') works
```

The whole package layer is Luau: [`@jess/jpk`](@/docs/api/jpk.md) reads and writes the container, and [`@jess/package`](@/docs/api/package.md) reads manifests, computes digests, packs, and resolves. Rust's part is one seam: the mount that hands an accepted plan to `require`.

## The manifest

`jess.toml` is read and checked without running anything. An unknown key is an error, so a typo can't become a silently ignored setting.

```toml
manifest_version = 1

[package]
id = "com.example.greeter"
version = "1.0.0"
jess_api = "1"
include = ["src", "tests", "tools"]
tests = ["tests/run.jsl"]
default_target = "greet"

[dependencies]
"com.example.words" = "^1.2"

[modules]
"@greeter" = "src/init.jsl"
"@greeter/register" = "src/register.jsl"

[[layer]]
id = "example.greetings"
exports = ["@greeter"]

[[layer]]
id = "example.registration"
requires = ["example.greetings"]
exports = []

[[layer.entrypoint]]
module = "@greeter/register"
capabilities = ["process.environment"]

[targets.greet]
kind = "command"
command = "greet"
entry = "tools/greet.jsl"
capabilities = ["process.environment"]
```

| Key | Is |
| --- | --- |
| `manifest_version` | `1`. |
| `package.id` | The package's identity: two or more lowercase words joined by dots, like a reversed domain. Never inferred from a file name. |
| `package.version` | `major.minor.patch`, or a pre-release of one, `1.2.1-dev.57`: see [version requirements](@/docs/api/package.md#package-satisfies). |
| `package.jess_api` | `"1"`, the package API generation this jess provides. It moves when the package contract breaks, not when jess's version does. |
| `package.include` | The payload: folders and files, relative to the manifest, that ship. `jess.toml` always does, and the tests the manifest names must be among them. Everything else (benchmarks, `.git`, `target/`) doesn't, without an ignore list, and neither do an editor's or a merge tool's leftovers (`*.bak*`, `*~`, `*.orig`) under an included folder. |
| `package.tests` | Test modules for [`jess package test`](@/docs/cli/package.md#jess-package-test). Each must be in the payload, under a folder `include` names: tests and their fixtures ship in the JPK. |
| `package.default_target` | The target `jess package run` runs when none is named. |
| `dependencies` | Other packages this one's modules use, by id, each with a [version requirement](@/docs/api/package.md#package-satisfies). A dependency makes that package's exports resolvable; it activates none of its layers. |
| `modules` | The package's named modules, each mapped to a `.jsl` file in the payload. Every name is some layer's export or entrypoint; a name nothing uses is an error. Everything else in the payload is private, reachable only by relative `require`. |
| `[[layer]]` | Each layer: its `id`, the layers it `requires`, and the module names it `exports`. |
| `[[layer.entrypoint]]` | Each of the layer's entrypoints: its `module`, and the `capabilities` it needs. |
| `[managed.<id>]` | A managed module an [application](@/docs/scripting/applications.md) loads into its domain: its private `module` file, and the `interface` id it provides, if any. |
| `[targets.<id>]` | Something runnable: its `kind`, `command` (a script run to its end) or `application` (a host of its own, see [Applications](@/docs/scripting/applications.md)), its private `entry`, the `capabilities` it declares, and for a command an optional `command` name. |

Every key, with its rules, is in the [manifest reference](@/docs/reference/manifest.md); every capability name a target or entrypoint can declare is in [Capabilities](@/docs/reference/capabilities.md).

Paths are canonical: forward slashes, relative, no `..`. A Package Source can't hold symlinks, or two files whose names differ only in case.

## Modules and require

A package's modules are reachable two ways, and only two.

**By exported name.** `require('@greeter')` loads the file the manifest maps that name to. A plan's namespace is every export of every layer of every package it selected, activated or not: an export is inert, so making it resolvable starts nothing. Export names are `@scope/...`, lowercase. `@jess/` is jess's, and the only package that may export into it is jess's own [built-in package](#the-built-in-package). A name the host already has (`@dream/fs`, `@dream/bytes`) is a resolution error, and so is a name two packages export: nothing is chosen by load order, because "whichever loaded last" is not a design. What a package module returns is shared by every module that requires it, so a table it returns is frozen as it returns (shallowly, like `table.freeze`): no package patches another's API for the whole plan, and state a module keeps belongs in a table of its own, not on its export.

**Relatively, from inside the package.** `./`, `../`, and `@self/` work exactly as they do for files on disk: relative to the requiring module, an `init.jsl` standing for its folder. The result has to stay inside the package. A path that climbs out is an error, not a lookup somewhere else:

```text
error: module not found: ../../outside (required by @com.example.greeter@1.0.0/src/escape.jsl): it leaves the package root of com.example.greeter
```

That chunk name, `@<id>@<version>/<path>`, is what every error and traceback shows for package code, so `init.jsl:42` never has to mean "one of eleven `init.jsl`s." Underneath, a loaded module is cached by its package's id and digest and its path, so a different build of the same version is never served from a stale cache.

A package can't list other packages or open their files by path. If it wants another package's code, it declares the dependency and requires an export.

## Layers

A layer names a system, not a file or a package: `example.greetings`, `example.registration`. One package can provide several, and the package providing a layer can change without anyone who asks for it noticing. That's why layer ids aren't package ids.

A layer's `requires` are other layers, resolved recursively, in a deterministic order, with unknown layers and cycles as errors. Package `dependencies` are a different graph: what has to be available so modules resolve. Keeping the two apart is deliberate.

Exports and entrypoints are different things on purpose. `@greeter` is a service: hand it a name, get a greeting. Requiring it registers nothing, opens nothing, and needs no authority beyond the caller's. `@greeter/register` is behavior: it runs because `example.registration` was activated, and a host that only asks for `example.greetings` never sees it run.

So capabilities belong to entrypoints, never to exports. A library module runs with the authority of whatever calls it; an entrypoint is code jess itself runs, and says what it needs. A package can't grant itself anything: the host grants, and when it won't, activation fails before any of the package's code runs, naming the entrypoint, layer, and package that asked:

```text
$ jess run --package greeter hello.jsl
error: package: cannot activate example.registration

required capability denied:
    process.environment

requested by:
    com.example.greeter 1.0.0
    layer example.registration
    entrypoint @greeter/register
```

`jess package graph` shows every requirement, and every entrypoint, before anything executes. Activation runs the entrypoints after mounting the plan, each once, in layer order. A JPK is a package format, not a sandbox: the capabilities say what a package may reach, and a package that declares `filesystem.write` can write files once you grant it. Read the manifest before you grant it.

## Packing, testing, running

```sh
jess pack greeter -o greeter.jpk          # the canonical JPK
jess package inspect greeter.jpk          # what it is, without running it
jess package graph greeter                # its layers resolved into a plan
jess package test greeter                 # its tests, from the source and packed
jess package check greeter                # every .jsl type-checked in strict mode
jess run --package greeter.jpk hello.jsl  # a script with its modules activated
```

[`jess pack` and `jess package`](@/docs/cli/package.md) has every option and what each prints. `jess pack` writes the same bytes from the same source on every machine. `jess package test` is the parity check: the package's tests against the source, then against it packed. If the two disagree, the package is broken, whichever side passed.

## Workspaces

Packages developed together, a tool and the library it uses, are a **workspace**: a `jess-workspace.toml` naming its members, folders that each hold a Package Source. From anywhere inside one, `jess workspace run <target>` resolves the target's package over the other members, with no `--package-path` to assemble, and `jess workspace test` runs the members' tests the same way. What the members depend on beyond each other is pinned in `jess.lock`, by version and digest, and kept as JPKs in `.jess/packages`, so the workspace runs the same packages on every machine and offline. A workspace changes nothing about a package: its digest, its JPK, and what its modules may require are its manifest's alone.

```toml
manifest_version = 1

[workspace]
members = [".", "tools/greeter"]
```

```sh
jess workspace sync --package-path ../vendor   # pin and keep what the members need
jess workspace run greet --trusted -- you      # from anywhere in the workspace
jess workspace test                            # every member's tests
jess workspace pack --output app.jsw           # the whole thing, in one file
jess app.jsw                                   # anywhere, offline, no workspace
```

[`jess workspace`](@/docs/cli/workspace.md) has the rules: which workspace a folder is in, how a target is named, how `sync` chooses, and how `pack` freezes a workspace into one file, or one executable, to ship.

## The built-in package

`jpk/builtin` is jess's own package, `org.dreamweave.jess`: the Luau libraries and tools every runtime can require, shipped the way any Luau ships. Its layers are `jess.style`, exporting [`@jess/style`](@/docs/api/style.md), the [style checker](@/docs/contributing/luau-style.md#the-checker), and [`@jess/analysis`](@/docs/api/analysis.md); `jess.data`, the codecs ([`@jess/json`](@/docs/api/json.md), [`@jess/data/toml`](@/docs/api/toml.md), [`@jess/data/value`](@/docs/api/value.md), [`@jess/yaml`](@/docs/api/yaml.md)); `jess.l10n`, [`@jess/l10n`](@/docs/api/l10n.md); `jess.input`, [`@jess/input`](@/docs/api/input.md); `jess.backup`, [`@jess/backup`](@/docs/api/backup.md); `jess.http`, [`@jess/http`](@/docs/api/http.md); and `jess.net`, [`@jess/net`](@/docs/api/net.md) and [`@jess/websocket`](@/docs/api/websocket.md). Its tests ship in it.

jess carries it. The binary embeds the package's manifest and payload, taken from `jpk/builtin` when jess is built, and its exports resolve in every runtime with nothing activated: `require '@jess/style'` just works, in `jess run`, in a package, in a Rust host's VM.

It's the one package whose modules are `@jess/` names, and the privilege belongs to the copy jess carries, not to the id. A package on disk that calls itself `org.dreamweave.jess` opens only if its manifest and payload are byte for byte the embedded ones; anything else is refused, naming the first file that differs:

```text
error: package: 'mine' says it is jess's built-in package (org.dreamweave.jess), but its 'src/style/rules.jsl' isn't jess's; rebuild jess from this source, or give the package an id of its own
```

A manifest from any other package that names a `@jess/` module is refused as before, and names jess provides itself (`@jess/test`, `@jess/log`) stay reserved for jess. The alternative was a scope of its own, `@builtin/style`, which every other package could squat on until it too was reserved.

For working on it, the source is still an ordinary Package Source:

```sh
jess package test jpk/builtin
jess pack jpk/builtin -o builtin.jpk
jess run --package builtin.jpk check-style.jsl src
```

Testing or activating `jpk/builtin` works while it's the copy the binary embeds. Edit a file and the jess you have refuses it until you rebuild: the embedded copy is made from the source at build time (`include_str!`, so cargo rebuilds when a file changes), and the package tests fail if a payload file isn't in the embedded list, `FILES` in `src/lua/builtin.rs`.

## A worked example

The greeter, start to finish. The folder:

```text
greeter/
  jess.toml
  src/init.jsl        @greeter
  tests/run.jsl       the suite jess.toml names
  tools/greet.jsl     the greet target's entry
```

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

The export returns a frozen table, because that is what every requirer will share:

```luau
--!strict
-- src/init.jsl
local function greet(name: string): string
  return `Hello, {name}.`
end

return table.freeze { greet = greet }
```

The test is an [`@jess/test`](@/docs/api/test.md) suite, and `@greeter` resolves in it because `jess package test` activates the package's layers before it runs:

```luau
--!strict
-- tests/run.jsl
local test = require '@jess/test'
local greeter = require '@greeter'

local suite = test.new()

suite.it('greets by name', function()
  test.expect(greeter.greet 'world').toBe 'Hello, world.'
end)

return suite
```

The target's entry reads the environment, which is why the manifest declares `process.environment`:

```luau
--!strict
-- tools/greet.jsl
local process = require '@dream/process'
local greeter = require '@greeter'

print(greeter.greet(process.env 'USER' or 'stranger'))
```

Now the commands, in the order you'd run them. The check first, since it costs nothing:

```text
$ jess package check greeter
id: com.example.greeter
version: 1.0.0
files: 3
errors: []
```

Misspell `greet` in the tool and the same command says where:

```text
errors:
- file: tools/greet.jsl
  line: 5
  column: 7
  message: 'Key ''gret'' not found in table ''{ read greet: (string) -> string }'''
error: 1 type error
```

The tests, from the folder and then from a JPK packed into a temporary place, with both digests compared:

```text
$ jess package test greeter
...
passed: 2
failed: 0
```

Running the target without granting what it declares stops before anything is activated. Granting it runs the entry with that capability and nothing more:

```text
$ jess package run greeter
error: cannot run target greet

required capability denied:
    process.environment

requested by:
    com.example.greeter 1.0.0
    target greet

$ jess package run --capability process.environment greeter
Hello, dave.
```

Packing writes the canonical JPK, and the digest it prints is the folder's:

```text
$ jess pack greeter -o greeter.jpk
input: greeter
id: com.example.greeter
version: 1.0.0
digest: 3e450d1a907a8ce0e6b034a434cc5b04caed51c3ff816bfa9a856dd92b26b076
files: 4
size: 1246
written:
  path: greeter.jpk
```

From here the JPK is the package. `jess package inspect greeter.jpk` shows the same digest, `jess package run greeter.jpk` runs the same target under the same rule, and `jess run --package greeter.jpk hello.jsl` makes `@greeter` resolvable in a script. [Your first package](@/docs/getting-started/first-package.md) walks the same ground more slowly.
