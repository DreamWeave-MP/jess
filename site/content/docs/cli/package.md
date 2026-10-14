---
title: 'jess pack and jess package'
description: 'Pack a Package Source into a JPK; inspect, resolve, and test packages without running them by accident.'
weight: 43
extra:
  kind: reference
---

{{ api_signature(value="jess pack <SOURCE> -o <FILE>") }}

{{ api_signature(value="jess package inspect <PACKAGE> | graph <PACKAGES>... [--layer ID]... | test <PACKAGE>") }}

Packages are covered in [Packages](@/docs/scripting/packages.md); these are the commands. `PACKAGE` is a Package Source folder (the one holding `jess.toml`) or a `.jpk` file, and every command except `test` treats the two the same. None of them runs package code except `test`, which runs the package's own tests and nothing else. To run a package's modules, activate it with [`jess run --package`](@/docs/cli/run.md#running-with-packages).

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
include = ["src"]
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

Writes the [canonical JPK](@/docs/api/jpk.md#the-canonical-writer): `jess.toml` and what its `include` names, sorted, each entry Zstandard or stored, every header field fixed. The same source gives the same bytes on every machine.

```text
$ jess pack greeter -o greeter.jpk
input: greeter
id: com.example.greeter
version: 1.0.0
digest: dab090658ab19adbb8fc0f6e125269cde47027ce918173b1bd9c9fad313c5971
files: 3
size: 703
written:
  path: greeter.jpk
```

Three files: `tests/run.jsl` isn't in `include`, so it stayed home.

## jess package inspect

Reads one package and prints what it is: id, version, [content digest](@/docs/api/package.md#the-content-digest), which kind of provider opened it, the package API generation it expects, how many files its payload holds, its named modules, its layers with their exports and entrypoints, its package dependencies, every capability its entrypoints require, and the tests its manifest names. It runs nothing, and checks everything [`package.open`](@/docs/api/package.md#package-open) checks.

```text
$ jess package inspect greeter.jpk
input: greeter.jpk
id: com.example.greeter
version: 1.0.0
digest: dab090658ab19adbb8fc0f6e125269cde47027ce918173b1bd9c9fad313c5971
kind: jpk
location: greeter.jpk
jess_api: '1'
files: 3
modules:
- name: '@greeter'
  path: src/init.jsl
layers:
- id: example.greetings
  requires: []
  exports:
  - '@greeter'
  entrypoints: []
dependencies: []
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
  digest: dab090658ab19adbb8fc0f6e125269cde47027ce918173b1bd9c9fad313c5971
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
  digest: dab090658ab19adbb8fc0f6e125269cde47027ce918173b1bd9c9fad313c5971
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
  digest: dab090658ab19adbb8fc0f6e125269cde47027ce918173b1bd9c9fad313c5971
  run:
    ...the same, from the JPK...
passed: 2
failed: 0
```

Tests are source-only: they and their fixtures stay out of the JPK, so a shipped package doesn't carry megabytes of goldens nobody runs. Given a `.jpk`, `jess package test` reports no providers, says so on stderr, and succeeds with nothing run:

```text
jess: greeter.jpk carries no tests; run `jess package test` on its Package Source
```
