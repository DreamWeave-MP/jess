---
title: 'jess pack and jess package'
description: 'Pack a Package Source into a JPK; inspect, resolve, and test packages without running them by accident.'
weight: 43
extra:
  kind: reference
---

{{ api_signature(value="jess pack <SOURCE> -o <FILE> [--as-version VERSION]") }}

{{ api_signature(value="jess package inspect <PACKAGE> | graph <PACKAGES>... [--layer ID]... | test <PACKAGE> | run <PACKAGE> [--target ID] [-- ARGS...]") }}

Packages are covered in [Packages](@/docs/scripting/packages.md); these are the commands. `PACKAGE` is a Package Source folder (the one holding `jess.toml`) or a `.jpk` file, and every command except `test` treats the two the same. None of them runs package code except `test`, which runs the package's own tests and nothing else, and `run`, which runs one of its targets. To run a package's modules from a script, activate it with [`jess run --package`](@/docs/cli/run.md#running-with-packages). Several packages developed together are a [workspace](@/docs/cli/workspace.md), which assembles all of this for you.

Each prints one document in the [`--format`](@/docs/cli/global-options.md) you ask for, YAML by default.

The examples use this package, `greeter/`:

```text
greeter/
  jess.toml
  src/init.jsl
  src/words.jsl
  tests/run.jsl
```

```toml
manifest_version = 1

[package]
id = "com.example.greeter"
version = "1.0.0"
jess_api = "1"
include = ["src", "tests"]
tests = ["tests/run.jsl"]

[modules]
"@greeter" = "src/init.jsl"

[[layer]]
id = "example.greetings"
exports = ["@greeter"]
```

## jess pack

| Argument / option | Does |
| --- | --- |
| `<SOURCE>` | The Package Source folder. |
| `-o, --output <FILE>` | The JPK to write, replaced atomically. It can't be inside `SOURCE`: a package that contains its own previous build is a package nobody meant to ship. |
| `--as-version <VERSION>` | Pack at this version instead of the one `jess.toml` says: the same payload with the manifest's `version` line revised, which is what a development build (`1.2.1-dev.57`) is. The source is untouched, and the JPK's digest is its own, since its manifest differs. |

Writes the [canonical JPK](@/docs/api/jpk.md#the-canonical-writer): `jess.toml` and what its `include` names, sorted, each entry Zstandard or stored, every header field fixed. The same source gives the same bytes on every machine.

```text
$ jess pack greeter -o greeter.jpk
input: greeter
id: com.example.greeter
version: 1.0.0
digest: 6fdd1a56f49cf9b23f2f9765b0cfce84836ba001cf57a602aa5ea47f7768e35b
files: 4
size: 1074
written:
  path: greeter.jpk
```

Four files: `include` names `tests`, so the test ships with the code it tests.

## jess package inspect

Reads one package and prints what it is: id, version, [content digest](@/docs/api/package.md#the-content-digest), which kind of provider opened it, the package API generation it expects, how many files its payload holds, its named modules, its layers with their exports and entrypoints, its managed implementations, its targets and which one is the default, its package dependencies and the ones only its tests need, every capability its entrypoints require, and the tests its manifest names. It runs nothing, and checks everything [`package.open`](@/docs/api/package.md#package-open) checks.

```text
$ jess package inspect greeter.jpk
input: greeter.jpk
id: com.example.greeter
version: 1.0.0
digest: 6fdd1a56f49cf9b23f2f9765b0cfce84836ba001cf57a602aa5ea47f7768e35b
kind: jpk
location: greeter.jpk
jess_api: '1'
default_target: null
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
targets: []
dependencies: []
dev_dependencies: []
capabilities: []
tests:
- tests/run.jsl
```

The digest is the source's: a JPK and the Package Source it was packed from are one package.

## jess package graph

| Argument / option | Does |
| --- | --- |
| `<PACKAGES>...` | The packages to resolve over. |
| `--layer <ID>` | A layer to resolve (repeatable). Without any, every layer the packages declare. |

Resolves the layers into an [activation plan](@/docs/api/package.md#package-resolve) and prints it: each layer after the ones it requires, the packages selected, the module namespace, the entrypoints activation would run, in order, and the capabilities they require, with who requires them. Nothing runs. When the graph doesn't resolve, the error says exactly where:

```text
$ jess package graph greeter --layer example.farewells
error: package: example.farewells: no provider found for example.farewells
```

```text
$ jess package graph greeter
requested:
- example.greetings
layers:
- id: example.greetings
  package: com.example.greeter
  version: 1.0.0
  requires: []
  exports:
  - '@greeter'
  entrypoints: []
packages:
- id: com.example.greeter
  version: 1.0.0
  digest: 6fdd1a56f49cf9b23f2f9765b0cfce84836ba001cf57a602aa5ea47f7768e35b
  kind: source
  location: greeter
modules:
- name: '@greeter'
  package: com.example.greeter
  path: src/init.jsl
entrypoints: []
capabilities: []
```

## jess package test

Runs the test modules a Package Source's manifest names, twice: against the source, then against the source packed into a temporary JPK, which is deleted afterwards. Each run activates every layer the package declares in a fresh VM, and each test module returns an [`@jess/test`](@/docs/api/test.md) suite, as for [`jess test`](@/docs/cli/test.md). The report has both runs and both digests; a failed case in either, or digests that disagree, fails the command.

That second run is the reason this command exists. A package that passes from its folder and fails from its JPK is broken, and this is how you find out before anybody else does.

```text
$ jess package test greeter
id: com.example.greeter
version: 1.0.0
tests:
- tests/run.jsl
providers:
- kind: source
  location: greeter
  digest: 6fdd1a56f49cf9b23f2f9765b0cfce84836ba001cf57a602aa5ea47f7768e35b
  run:
    passed: 1
    failed: 0
    modules:
    - input: greeter/tests/run.jsl
      report:
        passed: 1
        failed: 0
        tests:
        - name: greets
          passed: true
          error: null
          logs: []
      error: null
      logs: []
- kind: jpk
  location: /tmp/jess-package-test-737072/com.example.greeter-1.0.0.jpk
  digest: 6fdd1a56f49cf9b23f2f9765b0cfce84836ba001cf57a602aa5ea47f7768e35b
  run:
    ...the same, from the JPK...
passed: 2
failed: 0
```

Tests ship in the payload: a Package Source refuses to open when its manifest names a test its `include` doesn't cover, so every JPK carries its tests and their fixtures. They run from the source, though: given a `.jpk`, `jess package test` reports no providers, says so on stderr, and succeeds with nothing run:

```text
jess: greeter.jpk: tests run from a Package Source; run `jess package test` on its source
```

## jess package check

```text
jess package check <PACKAGE> [--package <PACKAGE>]... [--package-path <FOLDER>]...
```

| Option | Meaning |
| --- | --- |
| `PACKAGE` | A Package Source folder. |
| `--package <PACKAGE>` | A package its dependencies resolve over (repeatable), handed over outright. |
| `--package-path <FOLDER>` | A folder of packages its dependencies resolve over (repeatable), each opened only when needed. |

Type-checks every `.jsl` file under a Package Source in strict mode, tests and benches included, the way an editor checks a folder [`jess init`](@/docs/cli/init.md) set up. Each `require` resolves the way the runtime would resolve it under the package's plan: its own modules, the exports of its `[dependencies]` and `[dev-dependencies]` (from a folder or a JPK alike), the built-in package's exports, and jess's own modules as `jess luau-types` declares them. The plan is resolved and mounted, and nothing in it runs. Only the package's own files are reported, since a dependency's errors are the dependency's to fix. Any error fails the command.

```text
$ jess package check greeter
id: com.example.greeter
version: 1.0.0
files: 2
errors:
- file: tests/run.jsl
  line: 7
  column: 29
  message: Expected this to be 'string', but got 'number'
error: 1 type error
```

A package's gates are `jess package test` and `jess package check`, and `@jess/style` over its sources: tests say it works, the check says it's typed the way `--!strict` claims.

## jess package run

| Argument / option | Does |
| --- | --- |
| `<PACKAGE>` | The package whose target runs. |
| `--target <ID>` | The target to run. Without one, the package's `default_target`. |
| `--package <PACKAGE>` | A package its dependencies resolve over (repeatable), handed over outright. |
| `--package-path <FOLDER>` | A folder of packages its dependencies resolve over (repeatable), each opened only when needed. |
| `--capability <NAME>` | Grant a capability the target declares (repeatable). |
| `--trusted` | Grant everything the target declares. |
| `--frames <N>` | For an application target: stop after N frames, shutdown handlers and all, unless it stops itself first. Refused for a command target. |
| `ARGS...` | Arguments for the target, which gets them as `...` and in the global `arg` table. |

Runs one of a package's targets. A `command` target's package resolves with its dependencies, every layer it declares is activated, and the target's entry runs as a script in a fresh runtime holding exactly the capabilities the target declares. Not one more, and not one fewer: a capability it declares and you don't grant stops the run before anything is activated. The exceptions are what you already have when you run a tool, as with [`jess run`](@/docs/cli/run.md): your terminal (`process.stdio`) and the input devices in front of you (`input.actions`, `input.observe`, `input.configure`). A target that declares one gets it without asking; one that doesn't declare it still doesn't get it.

```text
$ jess package run tools/greeter --package-path .. -- one
error: cannot run target greet

required capability denied:
    process.environment

requested by:
    org.example.greeter 0.1.0
    target greet
```

`--trusted` grants what the target declares, which `package inspect` shows. A command target's first returned number is the exit status.

An `application` target resolves and is granted the same way, then runs in a host of its own: every selected package's [managed modules](@/docs/scripting/applications.md) load into one domain, the interfaces are composed, the entry runs, and frames run at 60 a second until the application calls [`application.exit`](@/docs/api/application.md), `--frames` runs out (status 0), or you press Ctrl+C (status 130; shutdown handlers still run). It runs headless, with no input source, so its `input` handlers don't run here; a host that embeds jess attaches one (see [Applications](@/docs/scripting/applications.md#embedding-it)). Its diagnostics go to stderr as they happen, and its exit status is the one it asked for.

An application whose `update` raises on its second frame and calls `application.exit(3)` on its third:

```text
$ jess package run app
jess: error in org.example.app/main during update (org.example.app@1.0.0/src/main.jsl:7): org.example.app@1.0.0/src/main.jsl:9: frame two broke
$ echo $?
3
```

In a [workspace](@/docs/cli/workspace.md), `jess workspace run` does the resolving part for you.
