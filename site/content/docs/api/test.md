---
title: '@jess/test'
description: 'Test suites in Luau: describe, it, hooks, expectations with deep equality, error assertions, log capture, and a report.'
weight: 113
extra:
  kind: api
---

{{ api_signature(value="require('@jess/test'): { new, expect }") }}

A [built-in module](@/docs/scripting/built-in-modules.md), typed as `Module__jess_test`. A test module builds a suite, registers cases on it, and returns it; [`jess test`](@/docs/cli/test.md) runs the suite and prints the report, and a script can call `suite.run()` itself. Every case runs inside a [`log.capture`](@/docs/api/log.md#log-capture), so what it logged ends up in its result instead of on stderr.

```luau
--!strict
local test = require '@jess/test'
local expect = test.expect
local suite = test.new()

suite.describe('openmw.cfg', function()
  local config

  suite.beforeEach(function() config = openmwConfig.loadOptional 'openmw.cfg' end)

  suite.it('keeps the content files in order', function()
    expect(config:contentFiles():toTable()).toEqual { 'Morrowind.esm', 'Tribunal.esm' }
  end)

  suite.it('rejects a config that is not there', function()
    expect(function() openmwConfig.new 'nowhere/openmw.cfg' end).toThrow 'nowhere'
  end)
end)

return suite
```

### test.new

{{ api_signature(value="test.new() -> JessTestSuite") }}

An empty suite. Its functions are plain functions, not methods: `suite.it(...)`, not `suite:it(...)`.

### JessTestSuite.describe

{{ api_signature(value="suite.describe(name: string, body: () -> ()) -> ()") }}

A group. `body` runs at once and registers cases and hooks; a case's full name is its groups' names and its own, joined with ` / `. Groups nest.

### JessTestSuite.it

{{ api_signature(value="suite.it(name: string, body: () -> ()) -> ()") }}

One case. It passes when its `beforeEach` hooks, its body, and its `afterEach` hooks all return without an error.

### JessTestSuite.beforeEach, afterEach

{{ api_signature(value="suite.beforeEach(body: () -> ()) -> ()") }}

Hooks for every case in the current group and the groups inside it. `beforeEach` hooks run outermost first; `afterEach` hooks run innermost first, and they run even when setup or the body failed. Every failure is in the case's `error`, one traceback each.

### JessTestSuite.run

{{ api_signature(value="suite.run() -> JessTestReport") }}

Runs every case in registration order and returns `{ passed, failed, tests }`, each test a `{ name, passed, error, logs }`. Nothing is printed; that's the host's job. Cases can't be added while the suite runs.

### test.expect

{{ api_signature(value="test.expect(actual: unknown) -> JessTestExpectation") }}

An assertion on `actual`. Each check raises with what it saw and what it wanted, tables rendered sorted and shortened: `expected { 1, name = "a" } to equal { name = "b" }`.

| Check | Passes when |
| --- | --- |
| `toBe(expected)` | `rawequal(actual, expected)`. |
| `toEqual(expected)` | Deep equality: same keys, values compared the same way, cycles handled. Tables only compare with tables; anything else is `rawequal`. |
| `toBeTruthy()`, `toBeFalsy()`, `toBeNil()` | What they say. |
| `toContain(item)` | `actual` is a string containing `item`, or a list with an element that `toEqual`s it. |
| `toThrow(contains?)` | `actual` is a function that raises, and its message contains `contains` when given. The one place a test catches an error on purpose. |

## Running

[`jess test a.jsl b.jsl`](@/docs/cli/test.md) loads each module, runs the suite it returns, and prints one report; it exits non-zero when anything failed. From Rust, `jess_lua::testing::run` does the same and hands back the report.

## Types

```luau
type JessTestSuite = {
  describe: (name: string, body: () -> ()) -> (),
  it: (name: string, body: () -> ()) -> (),
  beforeEach: (body: () -> ()) -> (),
  afterEach: (body: () -> ()) -> (),
  run: () -> JessTestReport,
}
type JessTestResult = { name: string, passed: boolean, error: string?, logs: { JessLogRecord } }
type JessTestReport = { passed: number, failed: number, tests: { JessTestResult } }
```
