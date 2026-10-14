---
title: 'Scripting Saves'
description: 'Read, check, and strip saves from Luau.'
weight: 70
extra:
  kind: guide
---

Everything `jess save` does is in `jess.save`: read a save's header and records, check it against your load order, and strip content files out of it. Scripting it pays off when you have more than one save to deal with, or want to decide what to strip rather than typing it.

## Fix a save in one go

You turned a mod off and your save still wants it. This script finds every content file the save needs that the load order lacks, makes sure removing them is safe, and strips them:

```luau
--!strict
-- Strips every content file a save needs but the load order no longer has.
local path: string = ...

local check = jess.save.check(path)
print(`{path}: {check.verdict}`)

-- Stripped together, missing files can be removed whether or not they're still installed.
local missing: { string } = {}
for _, content in check.missingContent do
  table.insert(missing, content.name)
end

if #missing == 0 then return end

-- Look before leaping: a dry run says whether anything blocks the strip.
local plan = jess.save.strip(path, missing, { dryRun = true })
if plan.blocked then
  for _, step in plan.steps do
    for _, concern in step.blockers.hard do
      print(`{step.dependency}: {concern.kind} x{concern.count}: {concern.action}`)
    end
  end
  return
end

local report = jess.save.strip(path, missing)
local written = assert(report.written)
print(`stripped {table.concat(missing, ', ')}; backup at {written.backup}`)
print(jess.save.check(path).verdict)
```

With Bloodmoon taken out of the load order:

```
$ jess run fix-save.jsl Caius_Met.omwsave
Caius_Met.omwsave: brokenState
stripped Bloodmoon.esm; backup at Caius_Met.omwsave.bak
loadsCleanly
```

`check` says whether the save will load and what's wrong with it; `strip` removes content files and fixes up what referred to them. The rest of this page takes them one at a time. For what a strip actually changes in a save and why, see the [saves guide](@/docs/guides/saves.md).

## Reading a save

`jess.save.open(path)` reads an OpenMW save (`.omwsave`) or a Morrowind one (`.ess`) and tells them apart by itself:

```luau
--!strict
local path: string = ...
local save = jess.save.open(path)
local header = save.header

print(`{header.player.name}, level {header.player.level or '?'}, in {header.cell}`)
print(`{save.game} save needing: {table.concat(header.dependencies, ', ')}`)

-- The screenshot, as an image file: a JPEG from OpenMW, a TGA from Morrowind.
local image, format = save:screenshot()
jess.fs.write(`{path}.{if format == 'jpeg' then 'jpg' else 'tga'}`, image)

-- The save's records are a plugin handle: select, count, and read them like any plugin's.
print(`{save.plugin:select({ type = 'NPC_' }):count()} NPC records`)
```

```
$ jess run read-save.jsl Caius_Met.omwsave
Ashur, level 1, in Balmora, Caius Cosades' House
openmw save needing: builtin.omwscripts, Morrowind.esm, Tribunal.esm, Bloodmoon.esm
5 NPC records
```

- `save.game` is `"openmw"` or `"morrowind"`. `jess.save.open(path, { game = "morrowind" })` insists on one, and `encoding` sets the text encoding, as for plugins.
- `save.header` is what `jess save info` prints: the player, cell, in-game time, time played, dependencies, and more.
- `save:values()` returns every record as `jess save dump` prints it, with camelCase field names.
- `save:screenshot()` returns the image file and its format.
- `save.plugin` is the save's records as a [plugin handle](@/docs/scripting/records-and-views.md). Selections, views, and `decode` all work on it.
- `jess.save.parse(bytes)` reads a save from a buffer or string instead of a file; its `path` is `nil`.

### Lua data

OpenMW stores Lua scripts' saved state in `LUAD` subrecords. `save:luaData()` decodes every one of them into Luau values:

```luau
--!strict
local save = jess.save.open(...)

for _, entry in save:luaData() do
  if entry.error then
    print(`record {entry.record} ({entry.type}): {entry.error}`)
  else
    print(`record {entry.record} ({entry.type}): {typeof(entry.value)}`)
  end
end
```

Each entry has the `record` number and `type` it came from, and either the decoded `value` or the `error` that stopped it. Engine types keep their identity: object references, vectors, colors, and the rest; the [Lua data reference](@/docs/api/luadata.md) covers them and how to encode values back.

## Checking

`jess.save.check(save)` is `jess save check`. It takes a path or a save handle; a handle is checked as it is in memory, unsaved edits included. The report's fields:

- `verdict`: `loadsCleanly`, `loadsWithWarnings`, `brokenState`, `willNotLoad`, `invalidStructure`, or `structureValid` (the load order wasn't checked).
- `loads`: whether the save loads at all.
- `missingContent`: content the save needs that the load order lacks, each with whether it's `strippable` on its own: true when the file is still installed, or when everything else the save needs is, so what the file defined can be told apart. Several uninstalled files are strippable together.
- `addedContent` and `orderChanged`: content that's new since the save, and whether the order moved.
- `findings`: each problem, with its `severity`, `problem`, and the record and subrecord it's in.

A save that won't load is a verdict, not an error. `check` only raises when it can't do the check at all (no such file, not an OpenMW save). Checking and stripping work on OpenMW saves only; `.ess` saves can be read, not checked.

## Stripping

`jess.save.strip(save, contentFiles, options)` is `jess save strip`. It removes the named content files from the save, dependents before their masters, in whatever order you list them. A file that's still installed (even if not active) is read to learn which records it defined; one that's been uninstalled is taken to have defined whatever the save refers to that no remaining content file defines, which needs the rest of the save's content installed and a config to find it. Otherwise the strip comes back blocked, with the reason under the step's `blockers.hard`.

Where the result goes depends on what you pass:

| Call | Writes |
| --- | --- |
| `strip(path, files)` | Over the file, keeping a `.bak` (then `.bak1`, ...). |
| `strip(path, files, { output = "New.omwsave" })` | To `output`, as given; the input is untouched. |
| `strip(path, files, { dryRun = true })` | Nothing: the report says what would happen. |
| `strip(handle, files)` | Over the handle's file, like a path. |
| `strip(handle, files, { inMemory = true })` | Nothing: only the handle changes. The default for a handle from `parse`, which has no file. |

The report has `blocked` (whether something stopped the strip; a blocked strip writes nothing), `written` (`{ path, backup }`: the file written and the backup of what it replaced, `nil` when nothing was), and a `steps` entry per content file removed, with its `blockers`, planned edits, and warnings. `dryRun` with `output` is an error.

**Handles become the stripped save.** Strip a handle, unless it's a dry run or went to `output`, and the handle now *is* the stripped save: `save.header.dependencies` no longer lists what you removed, and record handles into its old records read as deleted. Get fresh ones from `save.plugin`.

```luau
--!strict
local save = jess.save.open 'Caius_Met.omwsave'
jess.save.strip(save, { 'Bloodmoon.esm' }, { inMemory = true })
print(table.concat(save.header.dependencies, ', '))
```

```
builtin.omwscripts, Morrowind.esm, Tribunal.esm
```

## Related

- [Saves guide](@/docs/guides/saves.md), for the command-line side and what a strip changes.
- The [save API reference](@/docs/api/save.md), for every report field.
- The [`jess save` reference](@/docs/cli/save.md).
