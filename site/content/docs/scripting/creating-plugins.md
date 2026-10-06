---
title: 'Creating Plugins'
description: 'Build a new plugin from scratch or from pieces of others.'
weight: 60
extra:
  kind: guide
---

Patchers write plugins. This page covers building one: starting empty, adding new records, copying records out of other plugins, getting the masters and header right, and saving.

## From scratch

A plugin that adds a lantern that never burns out, and puts one in Fargoth's inventory:

```luau
--!strict
local plugin = jess.plugin.new {
  masters = { 'Morrowind.esm' },
  author = 'Me',
  description = 'A lantern that never runs out.',
}

plugin:insert {
  type = 'Light',
  id = 'jess_eternal_lantern',
  name = 'Eternal Lantern',
  mesh = 'l/light_com_lantern_01.nif',
  icon = 'l/tx_lantern_01.tga',
  data = {
    weight = 1,
    value = 250,
    time = -1,
    radius = 256,
    color = { 255, 200, 120, 0 },
    flags = 'DYNAMIC | CAN_CARRY | FIRE',
  },
}

-- Put one in Fargoth's hands: copy his record from Morrowind.esm and edit the copy.
local fargoth = jess.plugin.open('Morrowind.esm'):find('NPC_', 'fargoth')
local value = fargoth and fargoth:decode()
if value and value.type == 'Npc' then
  table.insert(value.inventory, { 1, 'jess_eternal_lantern' })
  plugin:insert(value)
end

local written = plugin:save 'Eternal Lantern.esp'
print(written.path, plugin.header.recordCount, plugin.masters[1].name, plugin.masters[1].size)
```

```
Eternal Lantern.esp	2	Morrowind.esm	79837552
```

Step by step:

- **`jess.plugin.new { ... }`** starts an empty plugin. It takes `masters`, `author`, `description`, `fileType` (`"plugin"`, the default, or `"master"`), and `encoding` (your config's `encoding=` unless you say otherwise, and `"win1252"` without one). It has no path until you save it somewhere.
- **Masters by name** take their sizes from your load order: `"Morrowind.esm"` became `{ name = "Morrowind.esm", size = 79837552 }`. A name the load order doesn't have is an error, since jess can't know its size; pass `{ name = ..., size = ... }` tables for those.
- **`insert`** adds a record from a table. `type` is required, as a type name (`"Light"`) or a tag (`"LIGH"`), and every field you leave out takes the type's default. It returns the new record's handle.
- **Copying a record** from another plugin is `decode()` then `insert`. The decoded table is yours: change it before inserting (here, one more inventory entry) and the source plugin never knows. `insert` takes the decoded table as it is, `type` and all.
- **The record count** in the header follows every insert and delete, so the saved plugin says 2.
- **`save(path)`** writes the file and makes `path` the plugin's path, so a later `save()` writes there again.

## From pieces of other plugins

`extract` copies a selection's records into a new plugin in one go. Here, every Tribunal weapon, made lighter:

```luau
--!strict
local tribunal = jess.plugin.open 'Tribunal.esm'

local patch = tribunal:select({ type = 'WEAP' }):extract()
patch:setMasters { 'Morrowind.esm', 'Tribunal.esm' }
patch:setHeader { author = 'Me', description = 'Lighter Tribunal weapons.', fileType = 'plugin' }

for record in patch:select({ type = 'WEAP' }):records() do
  local weapon = record :: WeaponRecord
  weapon.data.weight *= 0.8
end

patch:save 'Lighter Tribunal Weapons.esp'
```

An extracted plugin starts with the *source's* header: Tribunal's author, description, file type (it's a master), and masters (just `Morrowind.esm`). That's rarely what a patch wants, so fix it:

- **`setMasters`** replaces the master list. A plugin that overrides Tribunal's records needs Tribunal as a master, after Morrowind. A name gets its file's size the way `syncMasters` finds the file: through the config you pass (`{ openmw = context }` or `{ config = path }`) or the default one, then beside the plugin's file. It returns whether the list changed.
- **`setHeader`** sets any of `author`, `description`, and `fileType`. `fileType = "plugin"` makes it an `.esp`-style plugin, whatever the file is called.

`extract` copies what its selection picks: whole records, and for a selection with `instance` or `notInstance`, cells with only their matching references. See [Selections](@/docs/scripting/selections.md#acting-on-everything-at-once) for the details.

## Copies of whole plugins

`plugin:clone()` returns an independent copy of a plugin, edits included. Change the clone freely; the original doesn't move:

```luau
--!strict
local patch = jess.plugin.open 'Lighter Tribunal Weapons.esp'
local noEbony = patch:clone()

noEbony:select({ type = 'WEAP', id = '^ebony' }):delete()
noEbony:save 'Lighter Tribunal Weapons, No Ebony.esp'
```

A clone has no path of its own until you save it with one, so it can't overwrite the original by accident: a bare `noEbony:save()` is an error.

## Masters and plugins on disk

`setHeader({ fileType = "master" })` changes a plugin you have open. For a file that's already written, `jess.plugin.convert` is `jess plugin esm` and `esp`:

```luau
--!strict
local converted = jess.plugin.convert('Eternal Lantern.esp', 'master')
print(converted.written.path, converted.fileType)
```

```
Eternal Lantern.esm	master
```

Only the header's file type changes; every record keeps its bytes. The result goes next to the input with the new extension, or to `output`. The input must have the other type's extension, and an existing file at the output is an error unless you pass `overwrite = true`, which backs it up first. It returns `{ input, fileType, written = { path, backup } }`.

## Keeping master sizes current

Plugins record each master's file size. When a master changes (a new version of a mod, a cleaned `Morrowind.esm`), the recorded sizes go stale. OpenMW doesn't check them, but the original engine and some tools compare them to the files. `syncMasters` sets every master's size from the file your load order would load:

```luau
--!strict
-- A plugin made against an older Morrowind.esm: its master size is stale.
local plugin = jess.plugin.new { masters = { { name = 'Morrowind.esm', size = 79764287 } } }

for _, change in plugin:syncMasters() do
  print(change.name, change.oldSize, change.newSize)
end
```

```
Morrowind.esm	79764287	79837552
```

It returns what it changed: each master's `name`, `oldSize`, `newSize`, and the `path` it measured. It's `jess plugin header --update-masters` from Luau.

## Pitfalls

- **Masters need sizes.** A master name jess can't find (a typo, a mod that isn't installed, or a master that's neither in the config you passed nor beside the plugin) is an error. Pass the load order's config, give such a master as `{ name = ..., size = ... }`, or run `syncMasters` once the masters exist.
- **Masters are in load order.** List them in the order your load order loads them. A patch whose masters are out of order confuses the tools that check it.
- **Where to save.** A patcher's output belongs in `data-local`, not in a mod's folder; see [Load-Order Scripts](@/docs/scripting/load-order-scripts.md#a-complete-patcher).
- **Proving a file survives.** `jess.plugin.checkRoundTrip(path)` reads a plugin (a path, a name, or a `buffer`) and writes it back in memory, and returns `{ input, ok, error }`: `ok` is whether the bytes came back identical. It's `jess plugin test-codec`, for when a plugin you're about to edit was made by a tool with its own ideas about the format.

## Related

- [Records and Views](@/docs/scripting/records-and-views.md), for what's in a record.
- The [plugin reference](@/docs/api/plugin.md), for `new`, `insert`, `setHeader`, `setMasters`, `syncMasters`, and `save`.
- [Masters and Headers](@/docs/guides/masters-and-headers.md), the same jobs from the command line.
