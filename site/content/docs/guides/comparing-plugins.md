---
title: 'Comparing Plugins'
description: 'Diff two versions of a plugin, find shared records, and spot dialogue overlaps.'
weight: 50
extra:
  kind: guide
---

Four commands, four questions:

| Question | Command |
| --- | --- |
| What changed between version 1 and version 2 of this mod? | `plugin diff` |
| Are these two files the same, byte for byte? | `plugin compare` |
| Which records do these two plugins both touch? | `plugin common` |
| Will this mod's dialogue topics swallow another's? | `plugin overdial` |

## What changed in the update

A mod updated and the changelog says "fixes". Find out what that means:

```sh
jess plugin diff BalmoraTweaks-1.0.esp BalmoraTweaks-1.1.esp
```

```yaml
left_only:
- type: GMST
  id: sMaxSale
right_only:
- type: STAT
  id: bt_balmora_lamp
changed:
- type: MISC
  id: Misc_SoulGem_Common
  raw_changed: true
  fields: []
- type: STAT
  id: bt_balmora_bench
  raw_changed: true
  fields:
  - field: mesh[1]
    left: bt\balmora_bench.nif
    right: bt\balmora_bench_02.nif
```

Records are matched by type and ID, wherever they sit in the file:

- `left_only`: in the old version, gone from the new (here, an evil GMST got cleaned out).
- `right_only`: new in the new version.
- `changed`: in both, with different bytes. `fields` lists the string fields that changed (the field, then which subrecord). Numbers, flags, and other non-text data show up as `raw_changed: true` with no fields; here the soul gem's value changed.

To see a numeric change, dump the record from each file and diff the dumps:

```sh
diff <(jess plugin dump -t misc --exact-id Misc_SoulGem_Common BalmoraTweaks-1.0.esp) \
     <(jess plugin dump -t misc --exact-id Misc_SoulGem_Common BalmoraTweaks-1.1.esp)
```

```diff
2c2
< - record: 3
---
> - record: 2
10c10
<     value: 25
---
>     value: 30
```

The record moved (the GMST before it is gone) and its value went from 25 to 30.

Two flags narrow or widen the diff:

- `--ignore-type <TYPE>` leaves a record type out. `--ignore-type cell --ignore-type pgrd` skips the noisy world data.
- `--equal` also lists the records that are identical in both, under `equal`.

Text comparisons decode with `--encoding` (`jess plugin diff --encoding win1251 ...` for Russian plugins); the `raw_changed` test is always on bytes.

## Byte for byte

```sh
jess plugin compare MyMod.esp MyMod-from-nexus.esp
```

```yaml
left: MyMod.esp
right: MyMod-from-nexus.esp
identical: true
left_bytes: 940
right_bytes: 940
first_difference: null
```

`compare` doesn't parse anything; it's for "is this the same file?" (did the download corrupt, did the tool I ran change anything). When they differ, `first_difference` is the offset of the first differing byte, and jess exits `1`, which makes it easy to use in a script:

```sh
jess plugin compare A.esp B.esp > /dev/null && echo same || echo different
```

`jess plugin comp` works too, for tes3cmd fingers.

## Records two plugins share

```sh
jess plugin common BalmoraTweaks.esp Morrowind.esm
```

```yaml
left: BalmoraTweaks.esp
right: /home/you/Games/Morrowind/Data Files/Morrowind.esm
records:
- type: CELL
  id: Balmora
- type: MISC
  id: Misc_SoulGem_Common
- type: MISC
  id: Misc_SoulGem_Grand
```

Against a master, that's everything the plugin overrides. Against another mod, it's where the two will fight: whichever loads later wins each of those records. When the list is long and full of objects, that's what [TES3Merge](@/docs/bundled-scripts/tes3merge.md) is for.

## Dialogue topics that overlap

In Morrowind dialogue, a topic whose name contains another topic's name hides it: when both appear in a line, the longer one is highlighted, and the shorter one can't be clicked there. Mods adding topics can break each other's (or vanilla's) this way.

```sh
jess plugin overdial Morrowind.esm Tribunal.esm Bloodmoon.esm MyQuest.esp
```

```yaml
overlaps:
- earlier_plugin: /home/you/Games/Morrowind/Data Files/Morrowind.esm
  later_plugin: /home/you/Games/Morrowind/Data Files/Tribunal.esm
  outer_topic: B8_Failed_Nerevarine
  inner_topic: Nerevarine
# ...
```

Each plugin is compared with every plugin after it. `outer_topic` contains `inner_topic`, ignoring case. The official masters have plenty of these already (many of them are internal topics players never see), so compare *your* plugin against the rest with `--single`:

```sh
jess plugin overdial --single MyQuest.esp Morrowind.esm Tribunal.esm Bloodmoon.esm
```

With `--single`, only the first plugin's topics are checked, and only for containing the others' topics.

## Related

- [`plugin diff`, `compare`, `common`, `overdial`](@/docs/cli/plugin.md#diff): the flags.
- In Luau: `jess.compare.diff`, `bytes`, `common`, and `overlaps`; see [compare](@/docs/api/compare.md).
