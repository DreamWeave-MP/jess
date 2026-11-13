---
title: 'jess test'
description: 'Run Luau test modules built with @jess/test and print one report.'
weight: 45
extra:
  kind: reference
---

{{ api_signature(value="jess test [--shared] [--capability NAME]... [--package PACKAGE]... [--package-path FOLDER]... [--layer ID]... <MODULES>...") }}

Runs Luau test modules, named one by one or a folder at a time. Each is a file that returns a suite made with [`@jess/test`](@/docs/api/test.md); jess loads it with the whole API, runs the suite, and prints every case. The exit status is non-zero when any case failed or any module didn't run.

| Argument / option | Does |
| --- | --- |
| `<MODULES>...` | The test modules, as filesystem paths, `.jsl` files. Each gets a fresh VM; one that isn't a `.jsl` file is reported as a module that didn't run. A folder stands for every `*.test.jsl` file in it and in the folders below it, so `jess test tests` runs a folder of loose test modules and leaves the helpers beside them alone; [a folder of tests](#a-folder-of-tests) has the rest. A file named outright needs no such name. |
| `--shared` | One VM for every module, so they share the module cache and whatever state a module keeps. Faster for many small modules; isolation is the default because a test that depends on another module having run first is a bug. |
| `--capability NAME` | Grant the modules a capability they don't hold by default (repeatable), as [`jess run`](@/docs/cli/run.md#capabilities) does: test modules hold the terminal and the input devices, and everything else, such as `filesystem.read` for a suite that reads files, is a flag away. |
| `--package PACKAGE`, `--package-path FOLDER`, `--layer ID` | Activate packages before each module runs, exactly as [`jess run`](@/docs/cli/run.md) takes them, so a module can `require` what they export. They are activated in every module's VM, so a layer's entrypoint runs once a module, and what it prints lands before the report; with `--shared` they are activated once, in the one VM. A package's own tests belong to it instead: list them under `tests` in its manifest and [`jess package test`](@/docs/cli/package.md) runs them from source and packed, with its dependencies. |

With none of the three, the modules run with what the `[scripts]` table of the workspace file above them names, as [`jess run`](@/docs/cli/run.md#the-folder-s-packages) does; the table is found from the first module's folder.

## A folder of tests

```text
$ jess test tests
```

runs every file under `tests` whose name ends in `.test.jsl`:

- **Folder by folder, each in order of name**, so the report is the same on every machine: `tests/a/inner.test.jsl`, then `tests/a.test.jsl`, then `tests/b/...`. Uppercase sorts before lowercase.
- **Only that name.** `tests/helper.jsl` is not run, and `tests/Helper.TEST.JSL` is not either. Named outright, any `.jsl` file is a module.
- **A link to a file is the file; a link to a folder is not entered.** A folder whose name starts with a dot is skipped with everything in it, so `jess test .` leaves `.git` and `.jess` alone.
- **A folder adds only what nothing earlier on the line has named**, so two folders that overlap run a module once. A file named outright runs as often as it is named.
- **A folder holding none is an error**, since a run that tested nothing would pass: `'tests' holds no test modules: a folder's are its *.test.jsl files, and a package's are the ones its manifest lists, which jess package test runs`. A folder that can't be read is an error too.

This is for loose test modules. A package names its tests in its manifest, whatever they are called, and [`jess package test`](@/docs/cli/package.md#jess-package-test) runs those.

Plus the [global options](@/docs/cli/global-options.md). Like `jess run`, a test module holds its terminal and the input devices and nothing else until a flag says otherwise.

```luau
-- numbers.jsl
local test = require '@jess/test'
local log = require '@jess/log'
local expect = test.expect
local suite = test.new()

suite.describe('numbers', function()
  suite.it('add', function()
    log.info('adding', { left = 1 })
    expect(1 + 1).toBe(2)
  end)

  suite.it('compare', function() expect({ 1 }).toEqual { 2 } end)
end)

return suite
```

```sh
jess test numbers.jsl
```

```yaml
passed: 1
failed: 1
modules:
- input: numbers.jsl
  report:
    passed: 1
    failed: 1
    tests:
    - name: numbers / add
      passed: true
      error: null
      logs:
      - level: info
        target: jess
        message: adding
        fields:
          left: 1
    - name: numbers / compare
      passed: false
      error: |-
        expected { 1 } to equal { 2 }
        numbers.jsl:13
      logs: []
  error: null
  logs: []
  seconds: 0.004
```

A case's `logs` are what it wrote through `@jess/log`, captured instead of printed. A module that doesn't return a suite, or fails while loading, gets `error` set and `report: null`, counts as one failure, and doesn't stop the modules after it; what it logged while loading is in its `logs`. Its `seconds` is how long it took by the wall clock, loading and every case.

`--format json` prints the same document on one line, for a script or a CI step to read.
