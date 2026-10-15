---
title: 'Performance'
description: 'Make scripts over big load orders fast.'
weight: 110
extra:
  kind: guide
---

A script over the three vanilla masters finishes quickly however you write it. A script over a 500-plugin load order finishes quickly only if you let jess skip the work you don't need. Almost all of that is in *how you open plugins* and *what you copy*; the Luau in between rarely matters.

The [load-order tools](@/docs/bundled-scripts/_index.md) are plain Luau, written the way this page describes, measured whole against the native tools they port: a fresh process each, activating the package from its source folder and reading the same load order. Wall time and peak memory on a {{ bench(value="plugins") }}-plugin load order, median of {{ bench(value="runs") }} runs:

| Module | jess | Native | native / jess |
| --- | --- | --- | --- |
| `@dreamwind/tools/tes3merge` | {{ bench(workload="tes3merge", value="jess_time") }}, {{ bench(workload="tes3merge", value="jess_memory") }} | TES3Merge: {{ bench(workload="tes3merge", value="native_time") }}, {{ bench(workload="tes3merge", value="native_memory") }} | {{ bench(workload="tes3merge", value="faster") }}x wall, {{ bench(workload="tes3merge", value="less_memory") }}x memory |
| `@dreamwind/tools/s3lightfixes` | {{ bench(workload="s3lightfixes", value="jess_time") }}, {{ bench(workload="s3lightfixes", value="jess_memory") }} | S3LightFixes: {{ bench(workload="s3lightfixes", value="native_time") }}, {{ bench(workload="s3lightfixes", value="native_memory") }} | {{ bench(workload="s3lightfixes", value="faster") }}x wall, {{ bench(workload="s3lightfixes", value="less_memory") }}x memory |
| `@dreamwind/tools/merged-lands` | {{ bench(workload="mergedlands", value="jess_time") }}, {{ bench(workload="mergedlands", value="jess_memory") }} | Merged Lands: {{ bench(workload="mergedlands", value="native_time") }}, {{ bench(workload="mergedlands", value="native_memory") }} | {{ bench(workload="mergedlands", value="faster") }}x wall, {{ bench(workload="mergedlands", value="less_memory") }}x memory |

The [benchmark harness](@/docs/contributing/benchmarks.md) measured them on {{ bench(value="cpu") }} with {{ bench(value="threads") }} threads, jess {{ bench(value="commit") }} on {{ bench(value="runtime") }} and {{ bench(value="luau") }}. Merged Lands reads {{ bench(workload="mergedlands", value="lands") }} LAND records. Unlike the other two, it spends its time in Luau, walking every vertex of every cell, which makes it the one that shows what the [safe environment](#native-code) is worth.

Everything below is how they do it.

## Measure first

`os.clock()` is a monotonic wall clock with sub-millisecond resolution, so time spent waiting on a file or a child process counts too. Wrap the part you care about:

```luau
--!strict
--!native
local openmw = jess.openmw.default() or error 'needs an OpenMW config'

local paths: { string } = {}
for _, content in openmw:content() do
  if content.path then table.insert(paths, content.path) end
end

local function time(label: string, run: () -> ())
  local started = os.clock()
  run()
  print(string.format('%-40s %6.3fs', label, os.clock() - started))
end

time('open, everything', function()
  for _, path in paths do
    jess.plugin.open(path, { openmw = false })
  end
end)

time('openAll, everything', function() jess.plugin.openAll(paths, { openmw = false }) end)

time(
  'openAll, CELL headers only',
  function()
    jess.plugin.openAll(
      paths,
      { types = { 'CELL' }, cell = { references = false }, openmw = false }
    )
  end
)

time(
  'openAll, LIGH',
  function() jess.plugin.openAll(paths, { types = { 'LIGH' }, openmw = false }) end
)

time(
  'openAll, LIGH, decode = true',
  function() jess.plugin.openAll(paths, { types = { 'LIGH' }, decode = true, openmw = false }) end
)
```

Run it on your own load order. Opening only the record types you need takes a small fraction of the time of opening everything, and the gap grows with the load order.

## Open less

**`types`** keeps only the record types you name (tags or type names), and skips everything else *while reading*, so the cost of the rest is close to zero. A patcher that looks at lights has no business parsing every NPC and dialogue line in the load order.

**`cell`** trims `CELL` records, which are most of a big plugin's bytes because of their references:

- `cell = { references = false }` reads each cell's header only: `name`, `data`, `region`, `mapColor`, `waterHeight`, and `atmosphereData`. Right for anything that looks at cells, not at what's in them.
- `cell = { header = false }` reads only the references.
- Both `false` keeps bare cell records.

Parts you left out aren't empty; they're missing. Reading or writing one raises an error, so a script can't mistake "not loaded" for "no references".

A plugin opened with `types` or `cell` options is partial, and **can't be saved**: saving it would drop everything it didn't read. That's fine for a patcher, which reads plugins and writes a new one. To edit a plugin in place, open it whole.

**`jess.plugin.openAll(paths, options)`** opens many plugins in parallel, across all your cores. It returns two things: the plugins that opened, in the order of `paths`, and a table of why each other one didn't, keyed by path. A plugin that fails to parse doesn't stop the others:

```luau
--!strict
local openmw = jess.openmw.default() or error 'needs an OpenMW config'

local names: { string }, paths: { string } = {}, {}
for _, content in openmw:content() do
  if content.path then
    table.insert(names, content.name)
    table.insert(paths, content.path)
  end
end

local plugins, failures = jess.plugin.openAll(paths, { types = { 'LIGH' }, openmw = false })

-- The opened plugins skip the failed ones, so walk both lists together.
local opened = 1
for index, path in paths do
  if failures[path] then
    print(`skipping {names[index]}: {failures[path]}`)
    continue
  end

  local plugin = plugins[opened]
  opened += 1
  print(names[index], plugin:select({ type = 'LIGH' }):count())
end
```

**`decode = true`** decodes every kept record up front, in parallel when used with `openAll`. Without it, each record is decoded the first time you read one of its fields, one at a time on one core. Turn it on when you'll read most of the records you kept, which is usually true once `types` has narrowed them.

Pass `openmw = false` when you already have paths, as above: there's nothing to look up.

## Copy less

[Views](@/docs/scripting/records-and-views.md) read fields straight from the record; `toTable()` and `decode()` copy the record into Luau tables first. For reading a field or two across thousands of records, views are faster and allocate nothing.

The expensive copy is a cell's `references`, because every read copies every reference in the cell. Reading `references` on every cell of Morrowind.esm copies 316,116 references; reading `name` on every cell copies nothing, and the difference in run time is orders of magnitude. If you need references, read each cell's once, work on the copy, and assign it back once. If you don't, open with `cell = { references = false }` and they're never read at all.

Copy selectively. `@dreamwind/tools/tes3merge` groups records by id using only views and `jess.meta(record)` (the tag and id), and copies a record into a table with `decode()` only when three or more plugins define it, since fewer than three can't need merging, or when one of its patches applies. Most records are never copied.

## Re-encode less

Edits are held as values and turned back into bytes when something needs the bytes: `save`, `bytes()`, `find`, a selection, a handle's `size`. Only changed records are re-encoded, so this is cheap, but an edit-query-edit-query loop re-encodes each time it queries. Do your edits, then query; or select once, up front, and edit through the handles you already have.

## Native code

`--!native` on the line after `--!strict` asks Luau to compile the module to native code, and `@native` before one function compiles just that function. It pays off for code that spends its time in Luau: tight loops over numbers and strings, like s3lightfixes' color math. For code that spends its time in jess's bindings (reading, hashing, decompressing), it changes little. It never changes what code does, but it isn't free: a marked module is compiled to machine code every time it loads, which for a large module costs more than the module will ever save if it runs once. So mark what a measurement shows is hot and nothing else; the [Luau style page](@/docs/contributing/luau-style.md#types) has the rule jess's own Luau and its packages follow.

jess also marks the global environment safe, the way Luau's own sandbox does, so builtins like `math.floor` and `buffer.readi32` are looked up once, when the script loads, and called on Luau's fast path. Without that, every call is a table lookup and a full function call, many times slower in a loop over a LAND record's 4,225 heights; the harness's `safeenv` canary fails the test suite if that ever comes back. The price is that a builtin replaced after a script loaded (`math.floor = myFloor`) doesn't reach the code already loaded. Don't monkey-patch builtins.

## How the bundled scripts are built

Both follow the same shape, and it's a good one to copy:

1. List the load order with `openmw:content()`, dropping what doesn't apply (their own output, `.omwscripts`, excluded files).
2. Open everything in one `openAll`, with `types` narrowed to what they read, `decode = true`, and `cell = { references = false }`. S3LightFixes asks for `CELL` at all only when a setting could change a cell.
3. Walk the records through views, doing as little as possible per record.
4. Build the output from plain tables and `insert` them into one `jess.plugin.new`, saved once.

Read [`tools/tes3merge`](https://github.com/DreamWeave-MP/jess/blob/main/jpk/dreamwind/src/tools/tes3merge/init.jsl) and [`tools/s3lightfixes`](https://github.com/DreamWeave-MP/jess/blob/main/jpk/dreamwind/src/tools/s3lightfixes/init.jsl) in Dreamwind for the full versions.

## Related

- [Load-Order Scripts](@/docs/scripting/load-order-scripts.md): a complete patcher built this way.
- The [plugin reference](@/docs/api/plugin.md), for every `open` option.
