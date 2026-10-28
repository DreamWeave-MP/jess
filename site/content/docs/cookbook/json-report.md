---
title: 'Write a JSON Report'
description: 'Build an explicitly shaped value and encode it as JSON.'
weight: 10
extra:
  kind: guide
---

You have a small report to hand to another program. Build its shape explicitly, then encode it:

```luau
--!strict
local json = require '@jess/json'
local value = require '@jess/data/value'

local report = value.struct {
  { 'name', 'counter' },
  { 'samples', value.array { 2, 3 } },
}

print(json.encode(report))
```

Save it as `report.jsl` and run `jess check report.jsl`, then `jess run report.jsl`. It prints:

```json
{"name":"counter","samples":[2,3]}
```

The explicit `struct` supplies field order, and `array` says the samples are an array. This matters because an ordinary Luau table alone does not say whether an empty table is an array or an object, or what order object fields should take.

If you have an ordinary table and want a stable, human-readable representation, `json.encodeTable` uses its documented rules to infer a shape and can indent the result. It treats an empty table as an array. Neither encoder writes a file: printing uses stdout, and saving a report is a separate filesystem operation with its own capability.

For decoding, null handling, errors, and numeric limits, read [@jess/json](@/docs/api/json.md). [@jess/data/value](@/docs/api/value.md) describes the shapes the explicit encoder accepts.
