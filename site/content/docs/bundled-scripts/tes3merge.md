---
title: 'TES3Merge'
description: 'TES3Merge''s object merging as a Dreamwind module, @dreamwind/tools/tes3merge: behavior, options, and differences from the native tool.'
weight: 10
extra:
  kind: reference
---

`@dreamwind/tools/tes3merge` is a Luau port of [TES3Merge](https://github.com/NullCascade/TES3Merge)'s object merging. When several mods change the same record, the game only sees the last one. One mod rebalances a sword's damage, a later one renames it, and the rebalance is gone without a word. This script writes `Merged Objects.esp`, with each such record carrying every mod's changes instead of just the winner's. Its output is the native tool's, byte for byte, except for [a few records native TES3Merge writes that change nothing, and a count it keeps that counts nothing](#differences-from-native-tes3merge), and it gets there {{ bench(workload="tes3merge", value="faster") }} times faster on a {{ bench(value="plugins") }}-plugin load order.

## Run it

```luau
--!strict
-- merge.jsl <openmw.cfg>
local config: string = ...
local openmw = jess.openmw.load(config)
local tes3merge = require '@dreamwind/tools/tes3merge'

tes3merge({}, openmw)
```

```sh
$ jess run --package jess/jpk/dreamwind merge.jsl ~/.config/openmw/openmw.cfg
tes3merge: wrote 22 merged records to /home/you/.local/share/openmw/data/Merged Objects.esp
```

Then activate `Merged Objects.esp` last in your load order, or set `activate = true` and let the tool do it. Rerun after every change to your mod list. A stale merge carries the old versions of records whose mods have since changed. [Merging and Fixing a Load Order](@/docs/guides/load-order-scripts.md) walks through it from the player's side.

## What it does

1. **Reads the load order.** Every active content file, in order, except the output file itself (`Merged Objects.esp`, or `fileName`), files `fileFilters` turns off, files that aren't in any data folder, and `.omwscripts` files. Only the record types being merged are read, in parallel, in the text encoding OpenMW reads them in: the `encoding=` of your `openmw.cfg` chain, Windows-1252 if none sets it, or the `encoding` option. A plugin that fails to parse is skipped and listed in the report's `failed`.
2. **Groups records.** Records are grouped by type and id, ignoring case. Skills are grouped by skill, magic effects by effect, and exterior cells by grid position, since those have no ids of their own. `objectFilters` can leave records out.
3. **Merges.** A group is a merge candidate when three or more plugins define the record: with only two, the later one is simply the intended change. The merge starts from the last definition and walks back through the middle ones. The first definition is the base: a field takes a middle plugin's value when the result still has the base's value there and the middle plugin changed it. So every plugin's changes survive, and the latest plugin wins where two change the same field.
4. **Patches.** Applies the `patches` to what the merge left alone: records one or two plugins define, and cells whose flags and name it didn't change. That's TES3Merge's rule (`isAnythingChanged || Patch(...)`), and its merge reports a change for every other record three or more plugins define, so those never get patched.
5. **Writes.** A merged record goes into the output only if it changed and differs from the last definition. Records go in grouped by type, types in the order they first appear in the load order and records in the order their ids first appear, which is the order TES3Merge writes them in. The output's masters, in load order, are the plugins behind each record written: its first and last definitions and every plugin in between, whether or not that plugin changed anything, because TES3Merge counts them all. For a cell, the plugins in between count from the latest one whose merge changed it, walking back.

### Record types

By default it merges the 32 types native TES3Merge does:

`ACTI` `ALCH` `APPA` `ARMO` `BODY` `BOOK` `BSGN` `CELL` `CLAS` `CLOT` `CONT` `CREA` `DOOR` `ENCH` `GMST` `INGR` `LEVC` `LEVI` `LIGH` `LOCK` `MGEF` `MISC` `NPC_` `PROB` `RACE` `REPA` `SKIL` `SNDG` `SOUN` `SPEL` `STAT` `WEAP`

### Fields with their own rules

Most fields merge as described above, recursing into nested structures. These have rules of their own, ported from TES3Merge:

| Record | Field | Merge |
| --- | --- | --- |
| Spell, enchantment, potion | `effects` | Per slot: a changed effect replaces the slot, the same effect merges its fields, and effects a later plugin adds are appended. |
| NPC | `data` | `level`, `disposition`, `reputation`, `rank`, and `gold` field by field; `health`, `magicka`, `fatigue`, attributes, and skills value by value. |
| NPC, creature | `aiPackages` | The Wander package's fields merge; other packages keep the last definition's. |
| Leveled item, leveled creature | `items`, `creatures` | Union of every definition's entries: each (id, level) pair appears as many times as the list with the most of it has it, sorted by level, then id. |
| Class | `data` | `specialization` and `flags` only. |
| Faction | `data` | `flags` only. |
| NPC, creature, container | `inventory` | With `inclusive`: union of every definition's items. Otherwise the last definition's. |
| Cell | everything | Only the record flags, and an exterior cell's name (the cellnames fix, below). Merged cells carry no references. |

A later plugin that clears a value doesn't clear it in the merge: the latest value is kept.

### Patches

| Patch | Fixes |
| --- | --- |
| `fogbug` | Interior cells (not behaving like exteriors) with a fog density of 0 get 0.01. A fog density of exactly 0 is the "fog bug": some graphics setups draw those interiors wrong. |
| `summons` | The 22 summoned creatures in the vanilla game and its expansions (`atronach_flame_summon`, `bonelord_summon`, `golden saint_summon`, ...) are flagged `PERSISTENT` if a plugin left the flag off. |

Both are on by default, and both skip a record the merge changed (step 4 above), as in native TES3Merge: a summoned creature three plugins define stays as the merge leaves it. The **cellnames** fix is part of how cells merge and always applies, as it does in native TES3Merge: when a plugin renames an exterior cell and a later plugin reverts it to the original name by accident, the rename wins.

`patches` takes native TES3Merge's names, ignoring case: `fogbug`, `summons`, `cellnames` (accepted, and always on anyway), `all` for every patch, and `none`, which adds nothing, so `{ "none" }` applies no patches. Any other name stops the run with `tes3merge: unknown patch "<name>"; expected fogbug, summons, cellnames, all, or none`, before anything is read or written. Earlier versions of the port ignored names they didn't know, which is how `{ "all" }` once applied nothing. A typo is an error now.

## Options

Options go in `state.tes3merge`, in the table the tool is called with:

```luau
local state = {
  tes3merge = {
    ignoreRecords = { 'STAT' },
    fileFilters = { ['S3LightFixes.omwaddon'] = false },
    objectFilters = { { '^ttm_', false } },
    activate = true,
  },
}

tes3merge(state, openmw)
```

| Option | Type | Default | Native equivalent | Does |
| --- | --- | --- | --- | --- |
| `recordTypes` | `{ string }` | the 32 types above | `[RecordTypes]` in `TES3Merge.ini` | The record types to consider, by tag. Replaces the default list. |
| `records` | `{ string }` | all of `recordTypes` | `--records` | Merges only these types. |
| `ignoreRecords` | `{ string }` | none | `--ignore-records` | Never merges these types. |
| `fileFilters` | `{ [string]: boolean }` | none | `[FileFilters]` | A file name set to `false` isn't read at all. Names ignore case. `true` does nothing. |
| `objectFilters` | list of `{ pattern, allow }` | none | `[ObjectFilters]` | `{ pattern, allow }` pairs, in order: a record whose id matches a pattern is kept (`true`) or left out (`false`), and the last matching pair decides. Patterns are case-insensitive regexes. |
| `inclusive` | `boolean` | `false` | `--inclusive` | Merges NPC, creature, and container inventories as a union of every definition's. |
| `noMasters` | `boolean` | `false` | `--no-masters` | Writes the output without a master list. |
| `patches` | `{ string }` | `{ "all" }` | `--patches` | The [patches](#patches) to apply, by name. `{ "none" }` (or `{}`) applies none. |
| `encoding` | `"win1250"`, `"win1251"`, `"win1252"`, or a code page number | `openmw.cfg`'s `encoding=`, else `"win1252"` | `TextEncodingCode` in `TES3Merge.ini` | The text encoding plugins are read and the output is written in. Code pages `1250`, `1251`, and `1252` name the same encodings; any other number stops the run. |
| `fileName` | `string` | `"Merged Objects.esp"` | the output name | The output's file name. |
| `outputDir` | `string` | `data-local`, else the first `data=` folder | `OutputPath` | The folder to write to, created if it's missing. |
| `activate` | `boolean` | `false` | none | Adds the output to the content list and saves `openmw.cfg`, backing it up first. When `outputDir` isn't a data folder, OpenMW couldn't load the output, so it prints why and leaves it out. |
| `verbose` | `boolean` | `false` | none | Prints each merged record (`Merged CREA record: ...`). |

Options not set keep their defaults; an empty `state.tes3merge` is the native tool's default run. If filtering leaves no record types, the tool stops with `tes3merge: no record types left to merge`. Without `outputDir`, and with neither a `data-local=` nor a `data=` folder in the config, it stops before writing: `tes3merge: openmw.cfg has no data-local= or data= folder to write to; add one or set outputDir`.

## Output

- The file is written to `outputDir/fileName`, replacing any earlier one, with no backup. It's a generated file: regenerate it, don't edit it. Hand edits last until the next run.
- Its header's author is `TES3Merge` and its description `Automatic merge generated at <UTC time>.` Its text is written in the encoding the plugins were read in.
- If nothing needs merging, it prints `tes3merge: no merges were deemed necessary`, writes nothing, and leaves any earlier output where it is.
- `openmw.cfg` is only touched with `activate = true`.

## Report

Afterwards `state.tes3merge.report` holds what happened, for whatever runs next:

| Field | Type | What |
| --- | --- | --- |
| `path` | `string?` | The file written, or `nil` if nothing was. |
| `masters` | `{ string }` | The output's masters, in load order. Empty with `noMasters`. |
| `files` | `number` | How many plugins were read. |
| `candidates` | `number` | How many records three or more plugins define. |
| `merged` | `{ [string]: number }` | Records written, by type tag. |
| `failed` | `{ string }` | Plugins that couldn't be read, and records that failed to merge, with why. |
| `activated` | `boolean?` | With `activate`, whether the output is in the content list now; `false` when it went outside the data folders. |

```luau
local config: string = ...
local openmw = jess.openmw.load(config)
local tes3merge = require '@dreamwind/tools/tes3merge'
local state = {}

tes3merge(state, openmw)

local report = state.tes3merge.report
print(`{report.files} files, {report.candidates} candidates, written to {report.path or 'nothing'}`)

for tag, count in report.merged do
  print(tag, count)
end

for _, problem in report.failed do
  print('failed:', problem)
end
```

A record that fails to merge is left out of the output and listed in `failed`; the rest of the merge carries on.

## Differences from native TES3Merge

On the [benchmark](@/docs/contributing/benchmarks.md) load order, the port writes what native TES3Merge writes, byte for byte, in the same order, with the same masters, except for these. The harness allows for each of them explicitly, and for nothing else:

- **AI data padding.** A creature's or NPC's AI data (`AIDT`) has unused padding bytes, which the Construction Set fills with whatever was in memory at the time. Native TES3Merge compares and merges those bytes as if they were data, so it writes creature records whose only difference is garbage in the padding (four on the benchmark load order); Tribunal.esm's is `96 231 119`. They change nothing in game. jess treats padding as unused, so it doesn't write them.
- **Magic effect subrecord order.** TES3Lib, the library under native TES3Merge, writes a magic effect's sounds after its description. The Construction Set, OpenMW, and every plugin in the load order put them before the visual effects, and so does the port. Native TES3Merge decides whether a merge changed anything by comparing the bytes it would write, and its reordered bytes never match, so it also writes magic effects that are the winning plugin's with the subrecords shuffled (three on the benchmark load order).
- **Cell reference counts.** A cell's `NAM0` counts the temporary references after it. Merged cells carry no references in either tool, but TES3Merge's parser stops at a cell's first reference, so when the count comes first (a cell without persistent references), it keeps it, and its merged cell claims references it doesn't have. The port's merged cells have no count. OpenMW's engine never reads it; only OpenMW-CS does.

And by design, beyond anything that load order exercises:

- **Ids match ignoring case.** Native TES3Merge groups records by their ids as spelled, so a mod that respells one (`Bk_Note` for `bk_note`) starts a group of its own there, and nothing merges across the two. The port matches ids ignoring case, as OpenMW does.
- **Merged records are encoded from their fields.** The port writes a merged record from its decoded fields, so it spells a few things the way jess's typed encoding does, where native TES3Merge copies the subrecords it didn't change as they were. Ids and names end in a NUL, which the Construction Set leaves off a few (the body parts of armor and clothing, a faction's reactions, a game setting's id). An empty subrecord, like an NPC's blank `ANAM`, is left out. Unused padding, which the Construction Set fills with garbage in NPC stats and AI packages too, is written as zeros. None of it is data the game reads. Free text (book text, descriptions) keeps its bytes either way.
- **Configured through `state`, not an INI.** Every option above is set from a script. `fileFilters` starts empty; native TES3Merge's shipped INI turns off `S3LightFixes.omwaddon`, so add that filter if you run both, as in the example above.
- **Object filter patterns ignore case.** Native TES3Merge lowercases ids and matches patterns as written, so a pattern with capitals never matches there. Here patterns match case-insensitively.
- **Output goes where native's does.** Native TES3Merge writes to `data-local`, else the first `data=` folder. The port reads your `openmw.cfg` and picks the same folder, unless `outputDir` says otherwise.
- **The output's timestamp isn't preserved.** Native TES3Merge keeps the output file's old modification time, because Morrowind.exe orders plugins by file date. OpenMW loads by `content=` order, so the port doesn't bother.
- **Encoding from `openmw.cfg`.** Native TES3Merge reads `TextEncodingCode` from its INI, Windows-1252 by default. The port uses your `openmw.cfg`'s `encoding=`, as OpenMW does, unless the `encoding` option says otherwise. Japanese (code page 932) isn't supported, since OpenMW doesn't read it either.
- **Merges only.** The `multipatch` and `verify` commands aren't ported. For `verify`'s job, use [asset checking](@/docs/guides/asset-checking.md). `multipatch` merges leveled lists and applies the patches without merging anything else; `records = { "LEVI", "LEVC", "CREA", "CELL" }` comes close, but also merges those creatures' and cells' other fields.
- **No INI-only settings.** `PauseOnCompletion`, `DumpMergedRecordsToLog`, and `InstallPath` don't apply: the script never pauses, reports through `state`, and finds everything through `openmw.cfg`.

## Performance

On a {{ bench(value="plugins") }}-plugin load order, median of {{ bench(value="runs") }} runs:

| | Time | Peak memory |
| --- | --- | --- |
| `@dreamwind/tools/tes3merge` | {{ bench(workload="tes3merge", value="jess_time") }} | {{ bench(workload="tes3merge", value="jess_memory") }} |
| TES3Merge (native) | {{ bench(workload="tes3merge", value="native_time") }} | {{ bench(workload="tes3merge", value="native_memory") }} |

{{ bench(workload="tes3merge", value="faster") }} times faster, with {{ bench(workload="tes3merge", value="less_memory") }} times less memory. [Performance](@/docs/scripting/performance.md) explains how a Luau script manages that.

## Source

The script is [`jpk/dreamwind/src/tools/tes3merge/init.jsl`](https://github.com/DreamWeave-MP/jess/blob/main/jpk/dreamwind/src/tools/tes3merge/init.jsl), the [Dreamwind](@/docs/scripting/packages.md#dreamwind) module `@dreamwind/tools/tes3merge`, ported from TES3Merge (MIT, © 2019 NullCascade). It's plain Luau against the public API, so it's also a worked example of a large load-order script.

To change what it does beyond its options, copy it beside your script and require the copy: [Changing a tool](@/docs/guides/load-order-scripts.md#changing-a-tool).
