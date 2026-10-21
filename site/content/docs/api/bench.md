---
title: 'jess.bench'
description: 'What a measurement needs from the host: whether this build is the one releases ship, and a full collection between batches.'
weight: 60
extra:
  kind: api
---

{{ api_signature(value="jess.bench: { verified: boolean, collect: () -> () }") }}

Also `require('@jess/bench')`. Two things a benchmark needs that nothing else in Luau gives it, and nothing more: the harness, the batches, and the statistics are a package's business.

A number means something only on the build releases ship. jess is built with L3i's verified toolchain, cross-language link-time optimization included, and a development build (`tools/dev/no-lto.sh`, or the MSRV check) runs Luau slower than a release does. A harness that can't tell which build it is on will publish a number from the wrong one, so `verified` says, and `jess --version` marks an unverified build too.

## jess.bench.verified

{{ api_signature(value="jess.bench.verified: boolean") }}

Whether this jess was built with the verified toolchain. A harness refuses to produce a number when it isn't; a time from an unverified build measures a slower Luau, and the honest thing to do with it is nothing.

## jess.bench.collect

{{ api_signature(value="jess.bench.collect() -> ()") }}

Runs a full garbage collection, so one batch's garbage isn't collected inside the next one's timing. Call it between batches, not inside one.

```luau
--!strict
local bench = require '@jess/bench'

if not bench.verified then
  error('this jess was built without the verified toolchain; its numbers measure a slower Luau', 0)
end

local numbers = table.create(100000, 1)

local function sum(): number
  local total = 0
  for _, number in numbers do
    total += number
  end

  return total
end

bench.collect()
local started = os.clock()
for _ = 1, 50 do
  sum()
end

print(`{(os.clock() - started) * 1000 / 50} ms per sum`)
```

On a development build, which is what the example was run on:

```text
error: this jess was built without the verified toolchain; its numbers measure a slower Luau
```

That is the example working. `os.clock` is a wall clock; a bench checks its answer before and after timing, so a path that stops doing the work fails instead of posting a time, and it reports a median of several batches, never one run. [Benchmarks](@/docs/contributing/benchmarks.md) covers the harness jess's own measurements go through.

Related: [Performance](@/docs/scripting/performance.md).
