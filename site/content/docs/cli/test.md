---
title: 'jess test'
description: 'Run Luau test modules built with @jess/test and print one report.'
weight: 45
extra:
  kind: reference
---

{{ api_signature(value="jess test [--shared] <MODULES>...") }}

Runs Luau test modules. Each is a file that returns a suite made with [`@jess/test`](@/docs/api/test.md); jess loads it with the whole API, runs the suite, and prints every case. The exit status is non-zero when any case failed or any module didn't run.

| Argument / option | Does |
| --- | --- |
| `<MODULES>...` | The test modules, as filesystem paths. Each gets a fresh VM. |
| `--shared` | One VM for every module, so they share the module cache and whatever state a module keeps. Faster for many small modules; isolation is the default because a test that depends on another module having run first is a bug. |

Plus the [global options](@/docs/cli/global-options.md). Like `jess run`, tests start with no OpenMW config; one that needs a load order loads it with `jess.openmw.load`.

```luau
-- numbers.luau
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
jess test numbers.luau
```

```yaml
passed: 1
failed: 1
modules:
- input: numbers.luau
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
      error: |
        expected { 1 } to equal { 2 }
        numbers.luau:12
      logs: []
  error: null
  logs: []
```

A case's `logs` are what it wrote through `@jess/log`, captured instead of printed. A module that doesn't return a suite, or fails while loading, gets `error` set and `report: null`, counts as one failure, and doesn't stop the modules after it; what it logged while loading is in its `logs`.

`--format json` prints the same document on one line, for a script or a CI step to read.
