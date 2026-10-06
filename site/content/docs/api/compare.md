---
title: 'jess.compare'
description: 'bytes, common, diff, and overlaps: the CLI''s comparisons as functions, and overrides across a load order.'
weight: 90
extra:
  kind: api
---

{{ api_signature(value="jess.compare: { bytes, common, diff, overrides, overlaps }") }}

The comparisons `jess plugin compare`, `common`, `diff`, and `overdial` make, as functions over plugin handles, and one over a whole load order. Handles are compared as they are in memory, unsaved edits included.

| Function | CLI equivalent |
| --- | --- |
| [bytes](#jess-compare-bytes) | `jess plugin compare` |
| [common](#jess-compare-common) | `jess plugin common` |
| [diff](#jess-compare-diff) | `jess plugin diff` |
| [overrides](#jess-compare-overrides) | none; the [doctor](@/docs/bundled-scripts/doctor.md) reports it |
| [overlaps](#jess-compare-overlaps) | `jess plugin overdial` |

## jess.compare.bytes

{{ api_signature(value="jess.compare.bytes(left: Plugin | buffer | string, right: Plugin | buffer | string, options: ConfigOptions?) -> { identical: boolean, leftBytes: number, rightBytes: number, firstDifference: number? }") }}

Compares two files byte for byte.

- **Parameters:** each side is a plugin handle (its current bytes), a `buffer`, or a path or name (resolved like [jess.plugin.open](@/docs/api/plugin.md#jess-plugin-open), so any file works, not just plugins). `options.openmw` and `options.config` pick the config names resolve through ([ConfigOptions](@/docs/api/nif.md#configoptions)): `openmw = false` for plain paths.
- **Returns:** whether they're `identical`, each side's size, and the 0-based byte offset of the first difference (`nil` when identical).
- **Errors:** a path that can't be resolved or read, or a value that's none of the three.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
print(jess.compare.bytes(plugin, 'MyMod.esp').identical) -- true: nothing edited yet
plugin:setHeader { author = 'Me' }
local result = jess.compare.bytes(plugin, 'MyMod.esp')
print(result.identical, result.leftBytes - result.rightBytes, result.firstDifference)
```

## jess.compare.common

{{ api_signature(value="jess.compare.common(left: Plugin, right: Plugin) -> { RecordKey }") }}

Returns the records both plugins have, by type and id, as `{ type, id }` pairs (`type` is the tag).

```luau
--!strict
local mine = jess.plugin.open 'MyMod.esp'
local copy = mine:clone()
copy:select({ type = 'GMST' }):delete()
local shared = jess.compare.common(mine, copy)
print(#shared, shared[1].type, shared[1].id)
```

## jess.compare.diff

{{ api_signature(value="jess.compare.diff(left: Plugin, right: Plugin, options: { ignore: { string }?, equal: boolean? }?) -> PluginDiff") }}

A semantic diff by record type and id, like `jess plugin diff`. Records are paired by what they are, not where they sit, so moving a record doesn't make it "changed". It's deliberately shallow: bytes decide whether a record changed, and only string fields are compared one by one.

- **Parameters:** `options.ignore`, record types to leave out, as tags or type names (`{ "CELL" }`). `options.equal = true` also lists the records whose bytes are identical (off by default, like `--equal`).
- **Returns:**
  - `leftOnly`, `rightOnly`: `{ type, id }` of records only one side has.
  - `equal`: identical records, only with `equal = true` (otherwise `nil`).
  - `changed`: records whose bytes differ. Each has `type`, `id`, `rawChanged` (always `true`), and `fields`: the string fields that differ, as `{ field, left, right }`, where `field` is the field's name and its 0-based subrecord position (`name[2]`). Numbers and flags aren't compared field by field, so a record can differ with an empty `fields`.
- **Errors:** an unknown record type in `ignore`.

```luau
--!strict
local before = jess.plugin.open 'MyMod.esp'
local after = before:clone()
local light = after:find('LIGH', 'test_light') :: LightRecord
light.name = 'Lamp'
after:insert { type = 'Static', id = 'new_rock', mesh = 'rock.nif' }
local diff = jess.compare.diff(before, after, { ignore = { 'CELL' } })
print(#diff.leftOnly, #diff.rightOnly, diff.equal) -- 0  1  nil
for _, changed in diff.changed do
  for _, field in changed.fields do
    print(changed.type, changed.id, field.field, field.left, field.right)
  end
end
```

Related: [Comparing Plugins](@/docs/guides/comparing-plugins.md).

## jess.compare.overrides

{{ api_signature(value="jess.compare.overrides(plugins: { Plugin }, options: { ignore: { string }? }?) -> { { type: string, id: string, plugins: { number }, identical: { { plugin: number, sameAs: number } }, overridden: { number } } }") }}

Walks a load order and reports the records several plugins define that are worth a look. The game uses the last plugin's copy of a record, whole. Everything an earlier plugin changed in it is gone, unless the last one carried the change along.

- **Parameters:** `plugins`, in load order. `options.ignore` leaves out record types (tags or type names).
- **Returns:** one entry per record, in the order the records first appear. Plugins are positions in `plugins` (1-based).
  - `type`, `id`: the record's tag, and its id as the first plugin to define it spells it. Ids match ignoring ASCII case, as the game matches them.
  - `plugins`: every plugin that defines it, first to last. The last one wins.
  - `identical`: copies byte for byte the same as an earlier plugin's, each `{ plugin, sameAs }` with the first earlier plugin that has the same bytes. When `sameAs` is one of the plugin's own masters, that's what `jess plugin clean --duplicate-records` removes.
  - `overridden`: the plugins whose changes the last copy throws away: each copy between the first and the last that differs from every earlier copy and from the last one.
- Only records that have an `identical` or an `overridden` entry are listed. A plugin that defines a record twice counts once, with its later copy. Records without an id (`LAND`, `SKIL`, `MGEF`) and `TES3` headers never appear.

Only bytes are compared. A patch that loads last and carries two mods' changes still "overrides" both, so whether that's a problem is yours to judge; a winner that lists the overridden plugin among its masters is usually a patch. [TES3Merge](@/docs/bundled-scripts/tes3merge.md) merges the record types it knows field by field.

```luau
--!strict
local names = { 'Base.esm', 'A.esp', 'B.esp' }
local plugins = jess.plugin.openAll(names, { cell = { references = false } })

for _, record in jess.compare.overrides(plugins, { ignore = { 'CELL', 'DIAL', 'INFO' } }) do
  local winner = names[record.plugins[#record.plugins]]
  for _, position in record.overridden do
    print(`{record.type} {record.id}: {winner} drops {names[position]}'s changes`)
  end
end
-- MISC doctor_gem: B.esp drops A.esp's changes
```

## jess.compare.overlaps

{{ api_signature(value="jess.compare.overlaps(plugins: { Plugin }, options: { single: boolean? }?) -> { { earlierPlugin: string, laterPlugin: string, outerTopic: string, innerTopic: string } }") }}

Finds overlapping dialogue topics between plugins, like `jess plugin overdial`: pairs where one plugin's topic contains a different topic from a later plugin, ignoring case.

- **Parameters:** `plugins`, in load order. `options.single = true` checks only whether the first plugin's topics contain topics from the plugins after it, like `--single`.
- **Returns:** one entry per overlap: the two plugins (by path, or `plugin <n>` for a handle without one), the containing `outerTopic`, and the contained `innerTopic`. Topics within the same plugin aren't compared.

```luau
--!strict
local base = jess.plugin.new()
base:insert { type = 'Dialogue', id = 'latest rumors' }
local addon = jess.plugin.new()
addon:insert { type = 'Dialogue', id = 'rumors' }
for _, overlap in jess.compare.overlaps { base, addon } do
  print(overlap.earlierPlugin, overlap.laterPlugin, overlap.outerTopic, overlap.innerTopic)
end
-- plugin 1  plugin 2  latest rumors  rumors
```
