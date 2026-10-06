---
title: 'Record'
description: 'Record handles: fields, decode/set/update/edit, display, toTable.'
weight: 30
extra:
  kind: api
---

{{ api_signature(value="type Record = { <the record's fields>, decode, display, set, update, edit, toTable, subrecords, bytes, delete }") }}

A `Record` is a handle to one record in a [Plugin](@/docs/api/plugin.md). You get one from iterating a plugin or a selection, `plugin:record`, `plugin:find`, or `plugin:insert`.

A handle stays attached to its record as other records are inserted and deleted around it. A position can't do that: delete record 10 and every record after it moves up one, so "record 36" now means somebody else's record. Handles name a slot that belongs to one record for its whole life.

Once its own record is deleted, every read or write through the handle raises `record was deleted`. It doesn't quietly move on to whichever record took its place, because a handle that silently retargets would edit the wrong record and tell you it worked.

{% usage_note(title="Every name is the record's") %}
A record handle has two kinds of names: the record's own fields, and the methods below. `light.data.radius = 300` changes the record right away, with nothing copied into tables, and structs and lists inside it are [views](#views-and-lists) that read and write in place too.

There's nothing else on the handle. An `LTEX` has a field called `index`, so `ltex.index` is the texture's index, full stop; if the handle kept its record number under the same name, one of the two would have to lose, and you'd find out which by shipping a broken plugin. What the handle knows about its record, the record number, tag, plugin, and header flags, is [jess.meta](#metadata)'s answer instead.

Methods share the namespace, since `record:bytes()` is `record.bytes(record)` in Luau. No record type has a field named after one of them, and a test fails the build if one ever does.
{% end %}

The plugin's bytes don't change on every write; re-encoding a record per field assignment would be absurd. They catch up when something reads them (`bytes()`, `save()`, `find`, selections, `jess.meta(record).size`, and the like), and only what changed is re-encoded then. So a value the plugin can't encode raises its error there, not at the assignment, and the error names the record: `record 40 (NPC_ test_npc): encode error: ...`.

In typed code, cast a handle to its record type to reach the record's fields: `local light = record :: LightRecord`. An uncast `Record` has only the methods, since its type doesn't say which fields the record has. [Record and NIF Types](@/docs/api/record-types.md) lists the types.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
for record in plugin:select({ type = 'LIGH' }):records() do
  local light = record :: LightRecord
  light.data.radius *= 2
  light.data.color[1] = 255
end
plugin:save 'MyMod-bright.esp'
```

## Metadata

{{ api_signature(value="jess.meta(record: Record) -> RecordMeta") }}

[jess.meta](@/docs/api/jess.md#jess-meta) returns what the handle knows about its record besides the record's fields, as read-only properties. It reads each one when asked, so a `RecordMeta` kept around follows the record through edits, like the handle does, and raises `record was deleted` once the record is gone.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local ltex = plugin:select({ type = 'LTEX' }):first() :: LandscapeTextureRecord
local meta = jess.meta(ltex)
print(ltex.index, meta.index, meta.tag, meta.typeName) -- 0  17  LTEX  LandscapeTexture
```

### meta.index

{{ api_signature(value="meta.index: number") }}

The record's current 0-based record number, matching `jess plugin dump`.

It changes when records before it are inserted or deleted; the handle still points at the same record. Use it for display and for `plugin:record`, never as the record's identity: keep the handle instead.

```luau
--!strict
local record = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light')
print(record and jess.meta(record).index) -- 36
```

### meta.tag

{{ api_signature(value="meta.tag: string") }}

The record's four-letter tag, such as `"LIGH"` or `"NPC_"`.

```luau
--!strict
for record in jess.plugin.open('MyMod.esp'):records() do
  if jess.meta(record).tag == 'NPC_' then print((record :: NpcRecord).id) end
end
```

Filtering by type is what [selections](@/docs/api/selection.md) are for: `plugin:select { type = 'NPC_' }` does the loop above without making a `RecordMeta` per record.

### meta.typeName

{{ api_signature(value="meta.typeName: string?") }}

The record type's name, such as `"Light"`: what `decode()` and `toTable()` put in `type`. `nil` for a tag jess doesn't model, which in practice means a save's records.

```luau
--!strict
local record = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light') :: Record
print(jess.meta(record).typeName, record:toTable().type) -- Light  Light
```

### meta.id

{{ api_signature(value="meta.id: string?") }}

The id `plugin:find` looks the record up by, or `nil` for records without one, such as the `TES3` header.

It's the `INAM` subrecord for dialogue responses (`INFO`), the script name for `SCPT`, and the `NAME` subrecord otherwise, as `jess plugin dump` shows it. For most record types that's the record's `id` field, and reading the field is the natural way to get it. `meta.id` is for the records whose key isn't called `id`: a `CELL`'s or `PGRD`'s is its cell name, and a save's records have ids jess doesn't decode into fields. A `CELL` opened with `cell = { header = false }` has no id: its name was in the header.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local header = plugin:record(0)
local cell = plugin:select({ type = 'CELL' }):first()
print(header and jess.meta(header).id, cell and jess.meta(cell).id) -- nil  test_exterior
```

### meta.size

{{ api_signature(value="meta.size: number") }}

The size of the record's data in bytes, after any pending field edits are encoded.

```luau
--!strict
local light = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light') :: LightRecord
light.name = 'A much longer name than before'
print(jess.meta(light).size)
```

### meta.headerFlags

{{ api_signature(value="meta.headerFlags: { value: number, deleted: boolean, persistent: boolean, ignored: boolean, blocked: boolean }") }}

The record header's flags, as a number and as booleans.

`deleted` is bit `0x20`, `persistent` `0x400`, `ignored` `0x1000`, and `blocked` `0x2000`. Each read makes a new table: change flags through the record's own `flags` field, which spells them as names (`"PERSISTENT | BLOCKED"`).

```luau
--!strict
local light = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light') :: LightRecord
local flags = jess.meta(light).headerFlags
print(flags.value, flags.blocked, light.flags) -- 8192  true  BLOCKED
light.flags = 'PERSISTENT'
print(jess.meta(light).headerFlags.persistent) -- true
```

### meta.plugin

{{ api_signature(value="meta.plugin: Plugin") }}

The plugin the record belongs to.

Handles to the same plugin compare equal, so `jess.meta(record).plugin == plugin` tells whether a record belongs to `plugin`.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local owner = jess.meta(plugin:record(1) :: Record).plugin
print(owner.path, #owner, owner == plugin) -- MyMod.esp  80  true
```

## Operators

| Operation | Result |
| --- | --- |
| `record.field`, `record.field = value` | Reads or writes one of the record's own fields. Every name but the methods below is a field. Reading a name the record type doesn't have gives `nil`; writing one raises `no field '<name>'`. |
| `tostring(record)` | `Record(LIGH test_light #36)` |
| `a == b` | Whether two handles point at the same record of the same plugin. |

Record handles have no `#` and can't be iterated. Use `record:toTable()` or `record:decode()` to walk a record's fields.

## Methods

### record:decode

{{ api_signature(value="record:decode() -> RecordValue?") }}

Returns a copy of the record as a plain table, with `type` set to the type name (`"Light"`).

- **Returns:** a `RecordValue` (the union of every record type; see [Record and NIF Types](@/docs/api/record-types.md)), or `nil` for a record type jess doesn't model.
- **Errors:** a record whose bytes don't decode.
- **Behavior:** the table is a copy: changing it changes nothing until you pass it to `set`.

```luau
--!strict
local record = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light') :: Record
local value = record:decode()
if value and value.type == 'Light' then print(value.name, value.data.radius) end
```

### record:toTable

{{ api_signature(value="record:toTable() -> RecordValue (Light for a LightRecord; each record type returns its own table type)") }}

Copies the record into plain tables: the same table `decode()` gives, `type` included, so it can go straight to `insert`.

On a plain `Record` it returns a `RecordValue`, to narrow by `type` like `decode()`'s. On a handle cast to its record type it returns that type's table: `Light` for a `LightRecord`.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local light = plugin:find('LIGH', 'test_light') :: LightRecord
local copy = light:toTable()
print(copy.id, copy.data.radius)

local first = plugin:record(1) :: Record
local value = first:toTable()
if value.type == 'Light' then print(value.data.radius) end
```

### record:display

{{ api_signature(value="record:display() -> RecordDisplay") }}

Returns the record in the readable shape `jess plugin dump` prints: `record` (its number), `type` (its type name), and its fields.

Field names are camelCase, as everywhere in Luau (`waterHeight`, `atmosphereData`); values are spelled the dump's way, and the shape isn't accepted by `set`. Use it for reports; use `decode()` to edit.

```luau
--!strict
local record = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light') :: Record
local shown = record:display()
print(shown.record, shown.type, shown.name) -- 36  Light  test_name
```

### record:set

{{ api_signature(value="record:set(value: RecordValue) -> ()") }}

Replaces the record with a whole record table, as `decode()` returns it.

- **Parameters:** `value`, a complete record table. Every field must be there; for a partial change use [update](#record-update).
- **Errors:** ``missing field `name` `` (fields are named in camelCase), `unknown field 'data.colour'` for a name the record type doesn't have, or a wrong-typed field. A value the plugin's encoding can't hold.
- **Behavior:** only what changed is re-encoded: every other subrecord, and every untouched `CELL` reference, keeps its exact bytes.

```luau
--!strict
local record = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light') :: Record
local value = record:decode()
if value and value.type == 'Light' then
  value.data.radius = 512
  record:set(value)
end
```

### record:update

{{ api_signature(value="record:update(fields: RecordFields) -> ()") }}

Merges some fields over the record's current value.

- **Parameters:** `fields`, a table of the fields to change. A nested table merges into the struct it names the same way, so `{ data = { radius = 64 } }` changes only the radius; a list replaces the whole list.
- **Errors:** `unknown field '<name>'` for a name the record type doesn't have (nested ones as `data.radiuss`), or a field value of the wrong shape. Either way nothing changes.

```luau
--!strict
local record = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light') :: Record
record:update { name = 'Lamp' }
print((record :: LightRecord).name) -- Lamp
```

### record:edit

{{ api_signature(value="record:edit(edit: (value: RecordValue) -> ...RecordValue) -> ()") }}

Decodes the record, calls `edit` with the table, and writes the result back.

The function may change the table in place and return nothing, or return a new table to use instead.

- **Errors:** anything `edit` raises, and whatever `set` would raise for the result.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
for book in plugin:select({ type = 'BOOK' }):records() do
  book:edit(function(value)
    if value.type == 'Book' then
      value.name ..= ' (Annotated)'
    end
  end)
end
```

### record:subrecords

{{ api_signature(value="record:subrecords() -> { SubrecordLayout }") }}

Returns the record's subrecords in order: `index`, `type`, `size`, `range` (`start` and `end` byte offsets in the file), and, where jess knows it, the `field` it holds and its `value` as text.

```luau
--!strict
local record = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light') :: Record
for _, subrecord in record:subrecords() do
  print(subrecord.type, subrecord.size, subrecord.field, subrecord.value)
end
```

Related: [plugin:layout](@/docs/api/plugin.md#plugin-layout).

### record:bytes

{{ api_signature(value="record:bytes() -> buffer") }}

Returns the record's bytes as they'd be saved, header included, pending edits included.

```luau
--!strict
local record = jess.plugin.open('MyMod.esp'):find('LIGH', 'test_light') :: Record
print(buffer.len(record:bytes()), jess.meta(record).size) -- 174  158
```

### record:delete

{{ api_signature(value="record:delete() -> ()") }}

Removes the record from the plugin.

Every handle to it then raises `record was deleted`. Handles to other records keep working, and the header's record count follows the deletion.

- **Errors:** `the TES3 header can't be deleted`.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local record = plugin:find('GMST', 'sDefaultCellname') :: GameSettingRecord
record:delete()
-- A deleted record's handle throws on any use.
print(pcall(function() return record.id end)) -- false  record was deleted
```

## Views and lists

Reading a struct or list field of a record handle gives a view: userdata that reads and writes the record in place.

| Operation | On a struct view | On a list view (`List<T>`) |
| --- | --- | --- |
| `view.name` / `view[i]` | Reads a field (`nil` for a name it doesn't have) | Reads item `i` (1-based; `nil` past the end) |
| `view.name = v` / `view[i] = v` | Writes a field | Writes item `i`; `#list + 1` appends. Further out raises `index 99 is out of range for a list of 2` |
| `#view` | Number of fields | Number of items |
| `for k, v in view do` | Field names and values | Positions and items |
| `view:toTable()` | Copies the struct into a table | Copies the list into a table |

Assigning a table to a struct field replaces the whole struct, so it needs every field (``missing field `weight` `` otherwise), and a name the struct doesn't have is an error (`unknown field 'extra'`), leaving the field as it was.

### list:push

{{ api_signature(value="list:push(value: unknown) -> ()") }}

Appends an item.

- **Errors:** `this list has a fixed length` for fixed-size arrays such as a light's `color`. A value of the wrong shape.

```luau
--!strict
local npc = jess.plugin.open('MyMod.esp'):find('NPC_', 'test_npc') :: NpcRecord
npc.spells:push 'fireball'
npc.inventory:push { 5, 'gold_001' }
print(#npc.spells, npc.spells[#npc.spells])
```

### list:insert

{{ api_signature(value="list:insert(position: number, value: unknown) -> ()") }}

Inserts an item at a 1-based position, moving the rest along.

Use this instead of `table.insert`, which doesn't work on views.

```luau
--!strict
local npc = jess.plugin.open('MyMod.esp'):find('NPC_', 'test_npc') :: NpcRecord
npc.spells:insert(1, 'first_spell')
print(npc.spells[1])
```

### list:remove

{{ api_signature(value="list:remove(position: number) -> ()") }}

Removes the item at a 1-based position.

Use this instead of `table.remove`.

```luau
--!strict
local npc = jess.plugin.open('MyMod.esp'):find('NPC_', 'test_npc') :: NpcRecord
while #npc.spells > 0 do
  npc.spells:remove(#npc.spells)
end
```

### list:toTable

{{ api_signature(value="list:toTable() -> { T }") }}

Copies the list into a plain table; lists of structs copy each struct too.

```luau
--!strict
local npc = jess.plugin.open('MyMod.esp'):find('NPC_', 'test_npc') :: NpcRecord
local spells: { string } = npc.spells:toTable()
table.sort(spells)
print(table.concat(spells, ', '))
```

## Copied fields

A few fields are copied values rather than live views:

- A `CELL`'s `references`. Reading it copies every reference into a table (slow on big cells), and changing that table doesn't change the cell. Read it once, edit the copy, and assign it back. References stay a copy because the cell keys them by their `FRMR` index, which edits could change under a live view.
- A `Script`'s `bytecode` is compiled from its `text` by the Construction Set. Editing `text` leaves `bytecode` as it was. OpenMW compiles every script from its text, so it doesn't mind; Morrowind.exe runs the bytecode, so recompile in the Construction Set for it.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
for record in plugin:select({ type = 'CELL' }):records() do
  local cell = record :: CellRecord
  local references = cell.references
  for _, reference in references do
    reference.scale = 1
  end
  cell.references = references -- writes them all back
end
```

Related: [Records and Views](@/docs/scripting/records-and-views.md), [Performance](@/docs/scripting/performance.md).
