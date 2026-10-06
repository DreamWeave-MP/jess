---
title: 'Records and Views'
description: 'Record handles, live field views, decode/set/update/edit, and what''s a copy.'
weight: 20
extra:
  kind: guide
---

Almost every script reads or changes records. jess gives you two ways at them: **views**, which read and write the plugin in place, and **decoded tables**, which are plain copies you change and write back. Views are the default and usually what you want. This page covers both, when each fits, and the few places where what looks like a view is really a copy.

## Handles follow their records

A record handle points at one record, not at a position. Insert records in front of it or delete others, and it still points at the same record:

```luau
--!strict
local plugin = jess.plugin.new { masters = { 'Morrowind.esm' } }

local torch = plugin:insert { type = 'Light', id = 'jess_torch', name = 'Torch' }
local rock = plugin:insert({ type = 'Static', id = 'jess_rock', mesh = 'jess/rock.nif' }, 1) :: StaticRecord
print(jess.meta(torch).index, jess.meta(rock).index)

rock:delete()
print(jess.meta(torch).index)

-- The deleted rock's handle throws on any use.
local ok = pcall(function() return rock.id end)
print(ok)
```

```
2	1
1
false
```

`insert`'s second argument is the record number to insert at; the rock went in at 1, in front of the torch, and the torch's record number moved to 2. Deleting the rock moved it back. The rock's own handle now raises `record was deleted` on any use.

That's the point of handles. A record number is a position, and positions move every time anything is inserted or deleted in front of them. Keep a number as a record's identity and the next insert quietly points it at a different record, which you then edit. Each handle is tied to a stable slot for its record instead, so its record number is recomputed on every read and always tells the truth. When the record is deleted, the handle doesn't drift to whichever record now sits at that number. It fails, loudly, because silently editing the wrong record is the worst possible outcome.

**Don't keep a record number as a record's identity.** Keep the handle.

## A handle's names are the record's

Every name on a handle is one of its record's fields, or one of the handle's methods (`decode`, `set`, `delete`, ...), and no record type has a field named like a method. Nothing else sits on the handle to shadow a field. That matters more than it sounds: an `LTEX` has a field called `index`, and if the handle kept its record number under that name, the texture's index would be unreachable through it.

What the handle knows about its record is `jess.meta(record)`'s instead:

| Property | What it is |
| --- | --- |
| `index` | The record's number, 0-based, as `jess plugin dump` prints it. Record 0 is the `TES3` header. |
| `tag` | The record's tag: `"LIGH"`, `"NPC_"`, `"CELL"`. |
| `typeName` | The record type's name, `"Light"`, as `decode()` spells `type`. |
| `id` | The id `plugin:find` looks it up by, or `nil` for records without one: usually the record's `id` field, a `CELL`'s name for a cell. |
| `size` | The record's size in bytes, as it would be written now. |
| `headerFlags` | The raw header flags: `value`, plus `deleted`, `persistent`, `ignored`, and `blocked` booleans. |
| `plugin` | The plugin it belongs to. |

```luau
--!strict
local plugin = jess.plugin.new()
local dirt = plugin:insert { type = 'LandscapeTexture', id = 'dirt', index = 4, fileName = 'dirt.dds' } :: LandscapeTextureRecord
local meta = jess.meta(dirt)
print(dirt.index, meta.index, meta.tag, meta.headerFlags.deleted)
```

```
4	1	LTEX	false
```

## Views: read and write in place

The record's fields on a handle are live. Reading `lantern.data.radius` reads the plugin; assigning to it changes the plugin, right then:

```luau
--!strict
local morrowind = jess.plugin.open 'Morrowind.esm'
local lantern = morrowind:find('LIGH', 'light_com_lantern_01') :: LightRecord?
assert(lantern, 'no lantern in Morrowind.esm')

lantern.data.radius *= 2
lantern.data.color[1] = 255
print(lantern.data.radius, lantern.data.color[1])

local copy = lantern:toTable()
copy.data.radius = 1
print(lantern.data.radius, copy.data.radius)
```

```
512	255
512	1
```

Nested structs (`lantern.data`) are views too, and so are lists (`lantern.data.color`). Nothing is copied until you ask: `toTable()` on a handle, a struct, or a list copies it into plain Luau tables, which you can then change without touching the plugin.

Cast the handle to its record type to reach these fields in typed code (`record :: LightRecord`); the [record types reference](@/docs/api/record-types.md) lists every type's fields.

### Lists

A list view takes `#`, indexing, and `for i, v in list`, like an array. It changes in place, so it has its own methods for changing its length:

```luau
--!strict
local morrowind = jess.plugin.open 'Morrowind.esm'
local list = morrowind:find('LEVI', 'random_de_weapon') :: LeveledItemRecord?
assert(list, 'no list')

print(#list.items, list.items[1][1])

list.items:push { 'iron dagger', 1 }
list.items:insert(1, { 'chitin dagger', 1 })
list.items:remove(#list.items)

for index, entry in list.items do
  if index <= 3 then print(index, entry[1], entry[2]) end
end
```

```
11	chitin shortsword
1	chitin dagger	1
2	chitin shortsword	1
3	chitin spear	1
```

`push` appends, `insert(position, value)` inserts, and `remove(position)` removes; positions are 1-based like any Luau array. `table.insert` and `table.remove` don't work on views: they aren't tables, so those raise an error.

## Decoded tables: `decode`, `set`, `update`, `edit`

`record:decode()` returns the whole record as a plain table, with a `type` field naming the record type (`"Book"`, `"Npc"`, `"Light"`). Change it, then write it back:

```luau
--!strict
local morrowind = jess.plugin.open 'Morrowind.esm'
local book = morrowind:find('BOOK', 'bk_BoethiahPillowBook')
assert(book, 'no pillow book')

-- update: merge a few fields over the record.
book:update { name = 'Boethiah\'s Pillow Book, Abridged' }

-- edit: change the decoded table in a function, and it's written back.
book:edit(function(value)
  if value.type == 'Book' then value.data.weight = 1 end
end)

-- decode and set: the long way round, for when you need the value in between.
local value = book:decode()
if value and value.type == 'Book' then
  value.data.value = 5000
  book:set(value)
end

local typed = book :: BookRecord
print(typed.name, typed.data.weight, typed.data.value)
```

```
Boethiah's Pillow Book, Abridged	1	5000
```

- `update(fields)` merges the fields you give over the record. Other names are errors, so typos don't vanish.
- `edit(function)` decodes, calls your function with the table, and writes back whatever the function left in it (or the table it returns).
- `set(value)` replaces the record with a decoded table.

The `value.type == "Book"` check matters in typed code: `decode` returns any record type, and checking `type` narrows it to one.

**Views or tables?** Views, for reading and for changing a few fields: they copy nothing and are faster. Decoded tables when you need the whole record as data: to copy it into another plugin (`insert` takes a decoded table, `type` and all), compare two records, or hand it to code that expects plain tables. `toTable()` on a handle gives the same table as `decode()`, `type` included, so either can be inserted.

## What gets re-encoded, and when

Changes don't turn into bytes right away, because re-encoding a record on every assignment would be a waste when you're about to assign ten more fields. The plugin's bytes catch up when something reads them: `save`, `bytes()`, `find`, selections, a handle's `size`. Only what changed is re-encoded then. Every subrecord you didn't touch, and every `CELL` reference you didn't change, keeps its exact original bytes, so edits to vanilla data stay minimal and diff cleanly.

That timing has a consequence. A value that can't be encoded doesn't fail when you assign it, but when the bytes are next needed:

```luau
--!strict
local plugin = jess.plugin.new()
local book = plugin:insert { type = 'Book', id = 'jess_note' } :: BookRecord

book.name = 'Заметки' -- fine so far: nothing's encoded yet

-- `bytes` encodes the pending edits, and throws on text the encoding can't hold.
local ok, problem = pcall(function() return plugin:bytes() end)
print(ok, problem)
```

```
false	record 1 (BOOK jess_note): encode error: Заметки
```

The error names the record. Plugins default to the code page OpenMW reads your content in, the config's `encoding=`, and to the Western `win1252` without one; open a Russian or Polish plugin from elsewhere with `jess.plugin.open(path, { encoding = "win1251" })` (or `"win1250"`) and its text reads and writes correctly.

## Things that are copies

Two fields look like views but aren't.

**A `CELL`'s `references`.** Reading it copies every reference in the cell into a table; changing that table changes nothing until you assign it back:

```luau
--!strict
local morrowind = jess.plugin.open 'Morrowind.esm'
local record = morrowind:find('CELL', 'Seyda Neen, Arrille\'s Tradehouse')
assert(record, 'no Arrille\'s')
local cell = record :: CellRecord

local references = cell.references -- a copy of all 207
for _, reference in references do
  if reference.id == 'arrille' then reference.scale = 1.25 end
end
print(cell.references[1].scale) -- still nil: the cell hasn't changed

cell.references = references -- written back, all at once
print(cell.references[1].scale)
```

Read it once, edit the copy, assign it back. Don't read `cell.references` inside a loop: every read copies the whole cell again, and big exterior cells have thousands of references. References stay a copy because the cell keys them by their `FRMR` index, which edits can change under a live view.

**A `Script`'s `bytecode`.** The Construction Set compiles a script's `text` into `bytecode`. Editing `text` leaves `bytecode` as it was. OpenMW doesn't care: it compiles every script from its text and never runs the stored bytecode. Morrowind.exe runs the bytecode, so if the plugin is for the original engine, recompile the script in the Construction Set after changing its text.

## Reading, inserting, saving

- `record:display()` returns the record in the readable shape `jess plugin dump` prints: its number, type name, and fields as strings, numbers, and tables. It's for printing and inspecting; `set` doesn't accept it.
- `plugin:insert({ type = "Static", id = "my_rock", mesh = "rock.nif" })` adds a record. Fields you leave out take the type's defaults, and `type` takes either the type name (`"Static"`) or the tag (`"STAT"`). It returns the new record's handle.
- `plugin:save()` writes the plugin back where it came from and keeps a `.bak` of the file it overwrote (then `.bak1`, `.bak2`, ...). `plugin:save(path)` writes to `path` instead and makes that the plugin's path from then on. Either returns `{ path, backup }`; `{ backup = false }` skips the backup.

```luau
--!strict
local plugin = jess.plugin.new { masters = { 'Morrowind.esm' } }
plugin:insert { type = 'Book', id = 'jess_notes', name = 'Notes' }

local first = plugin:save 'Notes.esp'
print(first.path, first.backup, plugin.path)

local notes = plugin:find('BOOK', 'jess_notes') :: BookRecord
notes.name = 'Notes, Revised'
local second = plugin:save()
print(second.path, second.backup)
```

```
Notes.esp	nil	Notes.esp
Notes.esp	Notes.esp.bak
```

## Related

- [Selections](@/docs/scripting/selections.md): picking the records to work on.
- [Creating Plugins](@/docs/scripting/creating-plugins.md): building plugins out of new and copied records.
- The [record handle](@/docs/api/record.md), [plugin](@/docs/api/plugin.md), and [record types](@/docs/api/record-types.md) references.
