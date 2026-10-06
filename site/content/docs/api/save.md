---
title: 'jess.save and Save'
description: 'Open, parse, check, and strip saves; every Save field and method.'
weight: 50
extra:
  kind: api
---

{{ api_signature(value="jess.save: { open, parse, check, strip }") }}

`jess.save` reads OpenMW saves (`.omwsave`) and Morrowind saves (`.ess`), and checks and strips OpenMW saves the way `jess save check` and `jess save strip` do, through the same `jess-save` code.

A save is a plugin-shaped file full of state that points into your load order by content file index. Removing a mod from one means renumbering every reference into the files after it, and deciding what to do with every reference into the mod itself. `strip` does that bookkeeping and refuses when it can't account for something, because a save that loads and is quietly wrong is worse than one that doesn't load.

{% usage_note(title="Paths are plain paths") %}
Unlike plugin names, save paths aren't looked up in the OpenMW VFS: pass the file's path. The OpenMW config still matters to `check` and `strip`, which compare the save against your load order.
{% end %}

## Functions

### jess.save.open

{{ api_signature(value="jess.save.open(path: string, options: SaveOptions?) -> Save") }}

Reads a save file.

- **Parameters:** `path`, the save's path. `options.game`, `"openmw"` or `"morrowind"` to insist on one; by default it's detected. `options.encoding`, the text encoding (by default the config's `encoding=`, else `win1252`, as for plugins).
- **Returns:** a [Save](#save).
- **Errors:** `'<path>' isn't an OpenMW or Morrowind save`, `'<path>' isn't a Morrowind save` (with `game`), an unknown `game` or `encoding`, or a read failure.

```luau
--!strict
local save = jess.save.open 'CaiusMet.omwsave'
print(save.game, save.header.player.name, save.header.cell)
```

### jess.save.parse

{{ api_signature(value="jess.save.parse(bytes: buffer | string, options: (SaveOptions & { name: string? })?) -> Save") }}

Reads a save from bytes in memory.

- **Parameters:** `bytes`, the whole file. `options.name` names it in errors and reports (default `"save"`). `options.game` and `options.encoding` as for `open`.
- **Returns:** a [Save](#save) with no `path`.

```luau
--!strict
local bytes = jess.fs.read 'CaiusMet.omwsave'
if bytes then
  local save = jess.save.parse(bytes, { name = 'CaiusMet' })
  print(save.path, save.game) -- nil  openmw
end
```

### jess.save.check

{{ api_signature(value="jess.save.check(save: Save | string, options: (ConfigOptions & SaveOptions)?) -> SaveCheck") }}

Checks an OpenMW save for damage and against the load order, like `jess save check`.

- **Parameters:** `save`, a handle (checked as it is in memory, edits included) or a path (read with `options.game` and `options.encoding`). `options.openmw = false` skips the load-order comparison; an [OpenMwContext](@/docs/api/openmw.md#openmwcontext) there, or `options.config`, compares against that config's load order, as its `openmw.cfg` has it on disk.
- **Returns:** a [SaveCheck](#savecheck).
- **Errors:** `save check isn't supported for Morrowind saves`, or a config that can't be loaded.

```luau
--!strict
local check = jess.save.check 'CaiusMet.omwsave'
if not check.loads then
  print(check.verdict)
  for _, finding in check.findings do
    print(finding.severity, finding.problem, finding.recordType, finding.id)
  end
end
```

Related: [Saves guide](@/docs/guides/saves.md), [jess save](@/docs/cli/save.md).

### jess.save.strip

{{ api_signature(value="jess.save.strip(save: Save | string, dependencies: { string }, options: (ConfigOptions & SaveOptions & { dryRun: boolean?, output: string?, inMemory: boolean?, sampleLimit: number? })?) -> StripReport") }}

Removes content files from an OpenMW save, fixing or refusing the state that depends on them, like `jess save strip`.

- **Parameters:**
  - `save`: a handle (stripped as it is in memory, edits included) or a path.
  - `dependencies`: the content files to remove, as the save names them (`"Bloodmoon.esm"`), in any order. jess removes the latest in the load order first, so dependents go before their masters.
  - `options.dryRun`: plan and verify without changing anything.
  - `options.output`: write the stripped save to this path (used as given, not resolved) and leave the source alone.
  - `options.inMemory`: change only the handle. It's the default for a handle without a path.
  - `options.sampleLimit`: how many example records each scan keeps, a non-negative integer (default 5).
  - `options.openmw`, `options.config`: the config the dependencies are looked up in.
- **Where it writes:** without `dryRun`, `output`, or `inMemory`, a path or a handle with a path is edited in place, with a backup.
- **All or nothing:** every removal is applied and verified in memory before anything is written. If any is blocked, nothing is written and the handle is left alone. There is no half-stripped save.
- **What happens to a handle:** unless it's a dry run, went to `output`, or was blocked, the handle becomes the stripped save, and record handles into its old records read as deleted.
- **Returns:** a [StripReport](#stripreport). Check `blocked` before trusting the result.
- **Errors:** a dependency the save doesn't list, an `output` that is the input, `dryRun and output can't be used together`, `inMemory and output can't be used together`, `dryRun and inMemory can't be used together`, an invalid `sampleLimit`, a Morrowind save, or a config that can't be loaded.

```luau
--!strict
local report = jess.save.strip('CaiusMet.omwsave', { 'Bloodmoon.esm' }, { dryRun = true })
print(report.blocked)
for _, step in report.steps do
  print(step.dependency, step.mode, step.blockers.status, step.dryRun.status)
end
```

Related: [Stripping a mod from a save](@/docs/guides/saves.md), [Saves in scripts](@/docs/scripting/saves.md).

## SaveOptions

{{ api_signature(value="type SaveOptions = { encoding: Encoding?, game: (\"openmw\" | \"morrowind\")? }") }}

| Key | Default | Effect |
| --- | --- | --- |
| `game` | detected | `"openmw"` or `"morrowind"`: insist on one. OpenMW saves have a `SAVE` record; Morrowind saves have game data in their header. |
| `encoding` | the config's `encoding=`, else `"win1252"` | Text encoding of the save's strings. `check` and `strip` given a path take it from their `config` or `openmw` option's config. |

`check` and `strip` also take [ConfigOptions](@/docs/api/nif.md#configoptions) (`openmw`, `config`).

## Save

A save handle. Its fields are read-only properties.

### save.path

{{ api_signature(value="save.path: string?") }}

The file the save was read from, or `nil` for one from `parse`.

```luau
--!strict
print(jess.save.open('CaiusMet.omwsave').path) -- CaiusMet.omwsave
```

### save.game

{{ api_signature(value="save.game: \"openmw\" | \"morrowind\"") }}

Which game wrote the save (type `SaveGame`).

```luau
--!strict
print(jess.save.open('Nerevar.ess').game) -- morrowind
```

### save.header

{{ api_signature(value="save.header: SaveHeader") }}

The save's metadata, the shape `jess save info` prints, read fresh from the records each time.

| Field | Type | Meaning |
| --- | --- | --- |
| `format` | `"openmw" \| "morrowind"` | Which game wrote it. |
| `formatVersion` | `number?` | OpenMW's save format version; `nil` for Morrowind saves. |
| `description` | `string` | The save's name as the game shows it. |
| `player` | `{ name, level?, classId?, className? }` | `classId` when the player picked a class, `className` when they made their own. |
| `cell` | `string` | Where the player is. |
| `inGameTime` | `{ gameHour, day, month, year }` | The in-game date and time. |
| `timePlayed` | `number?` | Seconds played; Morrowind saves don't record it. |
| `dependencies` | `{ string }` | The content files the save was made with, in order. |
| `screenshotBytes` | `number` | The size of the stored screenshot. |
| `currentDay`, `currentHealth`, `maximumHealth` | `number?` | Where the save records them. |

```luau
--!strict
local header = jess.save.open('CaiusMet.omwsave').header
print(header.player.name, header.player.level, header.player.classId, header.cell)
print(header.inGameTime.day, header.inGameTime.month, header.timePlayed)
for _, dependency in header.dependencies do
  print(dependency)
end
```

### save.plugin

{{ api_signature(value="save.plugin: Plugin") }}

The save's raw records as a [Plugin](@/docs/api/plugin.md) handle.

Edits through it are what `check` and `strip` see when you pass the save handle. A strip that replaces the save replaces its records here too. Record ids read as OpenMW spells them (`#-2 -9` for an exterior cell).

```luau
--!strict
local save = jess.save.open 'CaiusMet.omwsave'
print(#save.plugin, save.plugin:select({ type = 'PLAY' }):count())
```

### save:screenshot

{{ api_signature(value="save:screenshot() -> (buffer, \"jpeg\" | \"tga\")") }}

Returns the save's screenshot as an image file, and its format.

OpenMW saves store a JPEG. Morrowind saves store raw pixels, which jess wraps in a TGA.

```luau
--!strict
local image, format = jess.save.open('CaiusMet.omwsave'):screenshot()
jess.fs.write('CaiusMet.' .. (if format == 'jpeg' then 'jpg' else 'tga'), image)
```

### save:values

{{ api_signature(value="save:values() -> { RecordDisplay }") }}

Returns every record in the readable shape `jess save dump` prints: `record` (its number), `type` (its tag), and its fields, with their names camelCase like every table the API hands out (`formatVersion`, not `format_version`). A save-only record's `subrecords` are keyed by tag, and its `LUAD` Lua data keeps the script's own keys.

```luau
--!strict
local values = jess.save.open('CaiusMet.omwsave'):values()
print(#values, values[1].type) -- 500  TES3
```

### save:luaData

{{ api_signature(value="save:luaData() -> { { record: number, type: string, value: LuaDataValue?, error: string? } }") }}

Decodes every `LUAD` payload (the storage of OpenMW Lua scripts) in the save.

Each entry names the record it's in (`record`, its 0-based number, and `type`, its tag) and holds either the decoded `value` (see [jess.luadata](@/docs/api/luadata.md)) or the `error` that stopped it.

```luau
--!strict
for _, entry in jess.save.open('CaiusMet.omwsave'):luaData() do
  print(entry.record, entry.type, entry.error or typeof(entry.value))
end
```

### tostring(save)

{{ api_signature(value="tostring(save) -> string") }}

`Save(<player name>, level <n>)`, or `Save(<player name>)` when the save has no level.

## Reports

### SaveCheck

{{ api_signature(value="type SaveCheck = { input, verdict, loads, formatVersion, strippableFormat, structureProblems, loadOrderChecked, missingContent, addedContent, orderChanged, findings, droppedOnLoad, warnings }") }}

What `jess.save.check` returns, the camelCase form of `jess save check --format json`.

| Field | Meaning |
| --- | --- |
| `input` | The save's path or name. |
| `verdict` | `invalidStructure`, `willNotLoad`, `brokenState`, `loadsWithWarnings`, `loadsCleanly`, or `structureValid` (the structure is fine and the load order wasn't checked). |
| `loads` | Whether OpenMW will load it. |
| `formatVersion`, `strippableFormat` | The save format version, and whether jess can strip saves of that version. |
| `structureProblems` | Framing problems found. |
| `loadOrderChecked` | Whether the save was compared against a load order. |
| `missingContent` | Content files the save needs that aren't active: `{ name, strippable }`. `strippable` is whether `jess.save.strip` can remove it on its own: it's installed, or everything else the save needs is. |
| `addedContent`, `orderChanged` | Active content the save wasn't made with, and whether the order differs. |
| `findings` | `CheckFinding`s: `severity`, `problem`, `id`, `recordIndex`, `recordType`, `subrecordIndex`, `subrecordType`. |
| `droppedOnLoad` | Counts, by kind, of state OpenMW will discard on load. |
| `warnings` | Anything else worth knowing. |

### StripReport

{{ api_signature(value="type StripReport = { input: string, dependencies: { string }, steps: { SaveStripPlan }, written: { path: string, backup: string? }?, blocked: boolean }") }}

What `jess.save.strip` returns, the camelCase form of `jess save strip --format json`: one `SaveStripPlan` per removed content file, in removal order; `written`, the file written and the backup of what it replaced (`nil` when nothing was written); and `blocked`, whether any step refused.

The main fields of a `SaveStripPlan`:

| Field | Meaning |
| --- | --- |
| `dependency` | The content file this step removes. |
| `mode` | `dryRun`, `inMemory`, `inPlace`, or `output`. |
| `dependencyIndex`, `dependenciesBefore`, `dependenciesAfter` | Where the dependency sits in the save's list, and the list's length before and after. |
| `requiresContentFileIndexRemapping` | Whether references to later content files must be renumbered. |
| `saveFormat` | `{ formatVersion, engineMinSupported, engineCurrent, mutationStatus, reason }`: whether jess can edit this save version. |
| `targetResolution` | `{ status, path? }`: `resolved`, `missing`, `disabled`, or `unavailable: <why>`. |
| `targetRecords` | `{ total, byType, samples }`: the records the dependency defines, when it was read. |
| `dependents` | Other content files that list the dependency as a master. |
| `saveReferences` | Save records that refer to the dependency's records: `confidence`, `total`, `byProvenanceRole`, `byMeaning`, `byRecordType`, and `samples` (`recordIndex`, `recordType`, `subrecordIndex`, `subrecordType`, `refId`, `meaning`, `provenanceRole`, `firstDefinedBy`, `lastDefinedBy`, `targetTypes`). |
| `contentFileReferences` | Object references numbered by content file: `confidence`, `total`, `byMeaning`, `byRecordType`, and `samples` (`recordIndex`, `recordType`, `subrecordIndex`, `subrecordType`, `meaning`, `contentFileIndex`, `localIndex`, `dependency`). |
| `referenceCoverage` | `{ status, knownScans, unclassifiedRiskyRecords }`: how much of the save the scans understood. |
| `blockers` | `{ status, hard, informational, protected }`; each concern is a `SaveStripConcern`, `{ kind, count, action }`. |
| `plannedOperations` | `{ name, status, count, reason }` per kind of change. |
| `plannedEdits` | Each edit: `operation`, `status`, `reason`, and where it applies (`recordIndex`, `recordType`, `subrecordIndex`, `subrecordEndIndex`, `subrecordType`, `meaning`, `localIndex`, `oldContentFileIndex`, `newContentFileIndex`). |
| `mutationReadiness` | `{ status, canAttemptDryRun, reason, referenceCoverage }`. |
| `dryRun` | `{ requested, status, reason, output, appliedEdits?, bytesBefore?, bytesAfter?, verification?, postMutationVerification? }`; the last is `{ status, reason, knownRemainingTargetRefids, invalidContentFileRefnums }`. |
| `mutation` | `written`, `dryRunOnly`, or `notRequested`. |
| `warnings` | Anything else worth knowing. |

Codes are camelCase strings. The full shape is `SaveStripPlan` in `jess luau-types`, and [Saves](@/docs/guides/saves.md) explains what a strip changes and why it might refuse.

```luau
--!strict
local report = jess.save.strip(
  'CaiusMet.omwsave',
  { 'Bloodmoon.esm' },
  { dryRun = true, sampleLimit = 1 }
)
local step = report.steps[1]
print(step.targetResolution.status, step.mutation)
for _, concern in step.blockers.hard do
  print(concern.kind, concern.count, concern.action)
end
```
