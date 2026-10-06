---
title: 'Selection and Selectors'
description: 'Selector spec keys and Selection methods.'
weight: 40
extra:
  kind: api
---

{{ api_signature(value="plugin:select(spec: SelectorSpec | Selector | nil) -> Selection") }}

A selector picks records the way `jess plugin dump`, `delete`, and `modify` do: by type, flags, id, raw bytes, subrecord text, or `CELL` reference. `plugin:select` pairs one with a plugin as a `Selection`, which you iterate, count, read, or edit in bulk.

A selection is live: every use sees the plugin as it is at that moment, so records inserted, deleted, or edited after `select` are picked or dropped accordingly.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
for book in plugin:select({ type = 'BOOK', sub = '^name: test' }):records() do
  book:update { name = 'Renamed' }
end
print(plugin:select({ type = 'CELL', instance = 'teleport_enabler' }):delete().references) -- 2
```

## Selector spec

{{ api_signature(value="type SelectorSpec = { type, flags, id, exactId, match, notMatch: (string | { string })?, sub, notSub, instance, notInstance: string?, interior, exterior: boolean? }") }}

A table of filters; a record must pass every one given. An empty spec (or `nil`) picks every record. Unknown keys are errors, so a typo never silently matches everything.

Every pattern is a regex in Rust's `regex` syntax, matched case-insensitively, except `exactId`. See [jess.regex](@/docs/api/regex.md) for the syntax.

| Key | Takes | Picks records that |
| --- | --- | --- |
| `type` | type or list | are of any of these types: four-letter tags (`"LIGH"`, `"book"`; `"npc"` means `NPC_`) or type names (`"Light"`, `"LeveledItem"`), in any case. |
| `flags` | name or list | have any of these header flags: `"deleted"`, `"persistent"`, `"ignored"`, `"blocked"`, or a number (`"1024"`, `"0x400"`). |
| `id` | pattern or list | have an id matching every pattern. |
| `exactId` | id or list | have an id equal to any of these, ignoring case. |
| `match` | pattern or list | have raw record bytes matching every pattern. |
| `notMatch` | pattern or list | have raw bytes matching none of the patterns. |
| `sub` | pattern | have a subrecord whose text matches. |
| `notSub` | pattern | have a subrecord whose text doesn't match. |
| `instance` | pattern | are `CELL`s with a reference whose text matches. With either instance key, only `CELL`s are picked. |
| `notInstance` | pattern | are `CELL`s with a reference whose text doesn't match. |
| `interior` | boolean | are interior `CELL`s (when `true`). |
| `exterior` | boolean | are exterior `CELL`s (when `true`). |

**Subrecord text.** `sub` and `notSub` match each subrecord as a line of text: `name: value` for the string fields jess knows (`mesh: editormarker.nif`, `name: Lamp`), and `TAG: payload` for the rest. A reference's text, for `instance` and `notInstance`, is those lines for each of its subrecords, joined with newlines.

**Partial picks.** With `instance` or `notInstance`, a `CELL` is picked through its matching references only, so `delete()` removes those references and keeps the cell, and `display()` and `extract()` show and copy only them. With `sub` or `notSub`, `delete()` removes the matching subrecords rather than whole records.

- **Errors:** `unknown selector option '<key>': expected type, flags, ...`, `invalid record type '<type>': expected a tag like LIGH or a type name like Light`, `invalid record flag '<flag>'`, `invalid regex '<pattern>': ...`, or a value of the wrong kind (`sub must be a string, not table`).

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
print(plugin:select({ type = { 'LIGH', 'STAT' }, id = '_p$' }):count())
print(plugin:select({ flags = 'persistent' }):count())
print(plugin:select({ exactId = 'TEST_LIGHT' }):count())
print(plugin:select({ type = 'CELL', interior = true }):count())
print(plugin:select({ type = 'LIGH', notSub = '^mesh: editormarker' }):count())
```

Related: [jess.select](@/docs/api/jess.md#jess-select) compiles a spec into a reusable `Selector`; [Selections](@/docs/scripting/selections.md) is the guide.

## Selection

A `Selection` is userdata; its methods use `:`.

### Length and iteration

{{ api_signature(value="#selection -> number; for record in selection do ... end") }}

`#selection` is the same as `selection:count()`. `for record in selection do` is the same as iterating `selection:records()`.

```luau
--!strict
local lights = jess.plugin.open('MyMod.esp'):select { type = 'LIGH' }
print(#lights)
for record in lights do
  print((record :: LightRecord).id)
end
```

### selection:records

{{ api_signature(value="selection:records() -> () -> Record?") }}

Returns an iterator over the handles of the picked records, in plugin order.

It walks the plugin's records as of the call, testing each against the selector as it goes, so editing a record during the loop is fine; records you delete are skipped and records you insert aren't visited.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
for record in plugin:select({ type = 'LIGH' }):records() do
  local light = record :: LightRecord
  light.data.radius *= 2
end
```

### selection:count

{{ api_signature(value="selection:count() -> number") }}

Returns how many records are picked.

The result is cached until the plugin changes.

```luau
--!strict
print(jess.plugin.open('MyMod.esp'):select({ type = 'CELL' }):count()) -- 3
```

### selection:first

{{ api_signature(value="selection:first() -> Record?") }}

Returns the first picked record's handle, or `nil`, without looking past it.

```luau
--!strict
local cell = jess.plugin.open('MyMod.esp'):select({ type = 'CELL', interior = true }):first()
print(cell and (cell :: CellRecord).name) -- Dagoth Ur, Facility Cavern
```

### selection:list

{{ api_signature(value="selection:list() -> { Record }") }}

Returns the picked records' handles as an array.

```luau
--!strict
local cells = jess.plugin.open('MyMod.esp'):select({ type = 'CELL' }):list()
print(#cells, (cells[1] :: CellRecord).name) -- 3  test_exterior
```

### selection:ids

{{ api_signature(value="selection:ids() -> { string }") }}

Returns the picked records' ids, skipping records without one.

```luau
--!strict
for _, id in jess.plugin.open('MyMod.esp'):select({ type = 'CELL' }):ids() do
  print(id)
end
```

### selection:decode

{{ api_signature(value="selection:decode() -> { RecordValue }") }}

Returns every picked record decoded into a table, as [record:decode](@/docs/api/record.md#record-decode) does, skipping types jess doesn't model.

The tables are copies; write changes back through the records' handles.

```luau
--!strict
for _, value in jess.plugin.open('MyMod.esp'):select({ type = 'LIGH' }):decode() do
  if value.type == 'Light' then print(value.id, value.data.radius) end
end
```

### selection:display

{{ api_signature(value="selection:display() -> { RecordDisplay }") }}

Returns every picked record in the readable shape `jess plugin dump` prints, as [record:display](@/docs/api/record.md#record-display) does.

With `instance` filters, a cell shows only the references that matched.

```luau
--!strict
local shown =
  jess.plugin.open('MyMod.esp'):select({ type = 'CELL', instance = 'teleport' }):display()
print(shown[1].record, shown[1].type, shown[1].name)
```

### selection:layout

{{ api_signature(value="selection:layout(options: { subrecords: boolean?, fields: boolean? }?) -> { RecordLayout }") }}

Returns the byte layout of the picked records, as [plugin:layout](@/docs/api/plugin.md#plugin-layout) does.

```luau
--!strict
for _, layout in
  jess.plugin.open('MyMod.esp'):select({ type = 'LIGH' }):layout { subrecords = false }

do
  print(layout.id, layout.size, layout.range.start)
end
```

### selection:delete

{{ api_signature(value="selection:delete() -> { records: number, subrecords: number, references: number }") }}

Deletes what the selection picks, like `jess plugin delete`.

- **Returns:** how many whole records, subrecords, and `CELL` references were removed.
- **Behavior:** without `sub` or `instance` filters, every picked record goes, except the `TES3` header. With `sub`/`notSub`, only the matching subrecords go. With `instance`/`notInstance`, only the matching references go and the cells stay.
- **Behavior:** the header's record count follows the deletion. Handles to deleted records raise `record was deleted`.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local gone = plugin:select({ type = 'GMST' }):delete()
print(gone.records) -- 3
local refs = plugin:select({ type = 'CELL', instance = 'teleport_enabler' }):delete()
print(refs.records, refs.references) -- 0  2
```

### selection:replace

{{ api_signature(value="selection:replace(pattern: string, replacement: string?, options: { first: boolean? }?) -> { records: number, fields: number }") }}

Rewrites the string fields of the picked records with a regex, like `jess plugin modify --replace`.

- **Parameters:**
  - One-argument form: `pattern` is a whole `/pattern/replacement/` expression. Any character can stand in for `/`: the first character is the delimiter.
  - Two-argument form: `pattern` and `replacement` separately.
  - Either way the pattern ignores case, like every selector pattern; start it with `(?-i)` to match case exactly.
  - `replacement` expands `$1`, `${name}`, and `$$`.
  - `options.first`: replace only the first match in each field (default: every match).
- **Returns:** how many records and fields changed.
- **Behavior:** only the string fields jess knows are rewritten (names, ids, models, scripts, text), and with `sub`/`notSub`, only those subrecords.
- **Errors:** `instance filters can't select fields to modify`, an invalid regex, or a malformed expression.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local changed = plugin:select({ type = 'LIGH' }):replace '/^test_/demo_/'
print(changed.records, changed.fields)
print(plugin:select({ type = 'LIGH' }):replace('light', 'lamp', { first = true }).fields)
```

### selection:extract

{{ api_signature(value="selection:extract(options: { header: boolean? }?) -> Plugin") }}

Copies the picked records into a new plugin in memory. Records picked through `sub` or `notSub` are copied whole; `CELL`s picked through `instance` or `notInstance` keep only their matching references.

- **Parameters:** `options.header` (default `true`) starts the copy with the original's `TES3` header, its record count updated. With `false`, the copy is just the picked records.
- **Returns:** a [Plugin](@/docs/api/plugin.md#plugin) with no `path`; save it with a path. Partly loaded `CELL`s stay partial in the copy.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local lights = plugin:select({ type = 'LIGH' }):extract()
print(#lights, lights.header.recordCount) -- 3  2
lights:save 'MyMod-lights.esp'
```
