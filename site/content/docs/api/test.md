---
title: '@jess/test'
description: 'Test suites in Luau: describe, it, hooks, expectations with deep equality, error assertions, log capture, and a report.'
weight: 105
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

local greeter = require '@greeter'

suite.describe('greet', function()
  local names: { string }

  suite.beforeEach(function()
    names = { 'ada', 'grace' }
  end)

  suite.it('greets by name', function()
    expect(greeter.greet(names[1])).toBe 'Hello, ada.'
  end)

  suite.it('greets everyone in order', function()
    expect(greeter.greetAll(names)).toEqual { 'Hello, ada.', 'Hello, grace.' }
  end)

  suite.it('refuses an empty name', function()
    expect(function() greeter.greet '' end).toThrow 'empty'
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

Within one group, hooks run in registration order. The first failing `beforeEach` stops the rest of setup and skips the case body; every registered `afterEach` still runs, including hooks in groups whose setup did not finish. A failing `afterEach` does not stop the remaining cleanup hooks.

### JessTestSuite.run

{{ api_signature(value="suite.run() -> JessTestReport") }}

Runs every case in registration order and returns `{ passed, failed, tests }`, each test a `{ name, passed, error, logs }`. Nothing is printed; that's the host's job. Cases and hooks can't be added while the suite runs; calling `run` recursively or inside `describe` raises.

### test.expect

{{ api_signature(value="test.expect(actual: unknown) -> JessTestExpectation") }}

An assertion on `actual`. Each check raises with what it saw and what it wanted, tables rendered sorted and shortened: `expected { 1, name = "a" } to equal { name = "b" }`.

| Check | Passes when |
| --- | --- |
| `toBe(expected)` | `rawequal(actual, expected)`. |
| `toEqual(expected)` | Deep equality: keys match by Luau table lookup and values compare recursively; cycles are handled. Tables only compare with tables; anything else is `rawequal`. Shared-reference identity is not required to match. |
| `toBeTruthy()`, `toBeFalsy()`, `toBeNil()` | What they say. |
| `toContain(item)` | `actual` is a string containing the literal string `item`, or a table with a value that `toEqual`s it. |
| `toBeCloseTo(expected, tolerance?)` | `actual` is a number whose absolute difference from `expected` is at most `tolerance` (default `1e-9`). This is an absolute tolerance, not a relative one. |
| `toThrow(contains?)` | `actual` is a function that raises, and its message contains `contains` when given. The one place a test catches an error on purpose. |

## Running

[`jess test a.jsl b.jsl`](@/docs/cli/test.md) loads each module, runs the suite it returns, and prints one report; it exits non-zero when anything failed. From Rust, `jess::lua::testing::run` does the same and hands back the report.

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
