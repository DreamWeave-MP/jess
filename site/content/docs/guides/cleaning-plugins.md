---
title: 'Cleaning Plugins'
description: 'Remove duplicate records, junk GMSTs, and other safe dirt from a plugin.'
weight: 20
extra:
  kind: guide
---

Goal: take a plugin the Construction Set left dirty and strip out the records it never meant to change, without touching the ones it did.

"Dirt" is what the CS saves when you so much as look at something: a vanilla record copied in unchanged, an interior cell you opened and closed, a game setting from an expansion. Dirty records override other mods for no reason. That's how a harmless house mod undoes your lighting overhaul.

## Clean it

```sh
jess plugin clean BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
cleanups:
- duplicate_records
- evil_gmsts
- cell_params
- junk_cells
removed_records: 4
removed_subrecords: 0
record_count_updated: true
written:
  path: BalmoraTweaks.esp
  backup: BalmoraTweaks.esp.bak
```

The original is now `BalmoraTweaks.esp.bak`, and `BalmoraTweaks.esp` is clean. Clean it again and nothing happens: `written: null`, file untouched.

With no cleanups named, `jess plugin clean` runs all of them, like tes3cmd's `clean`: each one only removes what's safe to remove. Name one or more to run only those; `cleanups` in the report says what ran.

| Flag | Removes |
| --- | --- |
| `--evil-gmsts` | Evil GMSTs: tes3cmd's list of the 11 Tribunal and 61 Bloodmoon GMSTs (`sMaxSale`, `fWerewolfHealth`, ...) the CS copies into plugins made with the expansions loaded. Only ones still at the expansion's value go; one you've actually changed is an edit, and stays. IDs match ignoring case. tes3cmd's `--gmsts` works too. |
| `--duplicate-records` | A second copy of the same record in one plugin (the later one goes), and records identical, byte for byte, to the copy in one of the plugin's masters. The second kind is the classic "identical to master" dirt. tes3cmd's `--dups` works too. |
| `--junk-cells` | Cells with nothing but their name and grid/flags: no references, no lighting, no region, nothing changed. |
| `--cell-params` | Repeated cell header subrecords before the references (a CS quirk). |

To find masters for `--duplicate-records`, jess looks at the master's name as a path, relative to where you ran it, then in the plugin's own folder. If a master can't be found, the identical-to-master check can't run for it, so clean the plugin where it's installed, beside its masters.

## Look before you leap

`plugin lint` reports the same things without changing anything:

```sh
jess plugin lint --evil-gmsts --duplicate-records --junk-cells --cell-params BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
checks:
- duplicate_records
- evil_gmsts
- cell_params
- junk_cells
summary:
  errors: 0
  warnings: 3
  info: 0
findings:
- check: duplicate_records
  severity: warning
  message: STAT bt_balmora_bench at record 3 duplicates record 2
- check: evil_gmsts
  severity: warning
  message: GMST sMaxSale at record 1 has Tribunal's default value
- check: junk_cells
  severity: warning
  message: CELL Balmora at record 6
```

Lint's `duplicate_records` only reports copies within the plugin; the identical-to-master records are what `--overrides` lists among every override. More in [Linting Plugins](@/docs/guides/linting.md).

Or clean into a new file and compare:

```sh
jess plugin clean --dry-run BalmoraTweaks.esp                          # what would go
jess plugin clean --output BalmoraTweaks-clean.esp BalmoraTweaks.esp
jess plugin diff BalmoraTweaks.esp BalmoraTweaks-clean.esp
```

```yaml
left_only:
- type: CELL
  id: Balmora
- type: GMST
  id: sMaxSale
- type: MISC
  id: Misc_SoulGem_Grand
right_only: []
changed: []
```

Three records gone: the junk exterior cell (a copy of one Balmora cell with nothing but its name and grid), the evil GMST, and a Grand Soul Gem that was a byte-for-byte copy of Morrowind's. The fourth removed record was the duplicate `bt_balmora_bench`, which `diff` doesn't show because the other copy is still there. The Common Soul Gem, which the plugin actually changed, stayed.

## Removing specific records

For anything clean doesn't know about, delete by selector. Always `dump --list` the selection first:

```sh
jess plugin dump --type stat --list BalmoraTweaks.esp
```

```yaml
records:
- record: 1
  type: STAT
  id: bt_balmora_bench
```

```sh
jess plugin delete --type stat --exact-id bt_balmora_bench BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
deleted: 1
unit: records
record_count_updated: true
written:
  path: BalmoraTweaks.esp
  backup: BalmoraTweaks.esp.bak1
```

Note the backup is `.bak1`: the `.bak` from the earlier clean is still there. jess never overwrites a backup.

`delete` takes the same [selectors](@/docs/cli/plugin.md#selectors) as `dump`. With subrecord filters it deletes matching subrecords; with instance filters, matching references inside cells:

```sh
# every reference to Fargoth, in whatever cells the plugin touches
jess plugin delete --type cell --instance-match fargoth --output NoFargoth.esp MyMod.esp
```

`delete` needs at least one selector: `jess plugin delete MyMod.esp` on its own is refused rather than deleting every record.

## What clean won't do

- **It won't judge edits.** A record that differs from its master by one byte is an edit, as far as jess is concerned. Records the CS "touched" but changed trivially (a float rounded differently) stay.
- **It isn't fully master-aware.** Identical-to-master records go; per-field comparisons against masters, and dirty references in cells, aren't done. tes3cmd's `clean` goes further there.
- **It won't undelete.** tes3cmd's `undelete` isn't ported: it marks references disabled with a `ZNAM` subrecord, which OpenMW doesn't understand. OpenMW stops loading the cell at that point and loses every reference after it. On OpenMW, deleted references are best left deleted, or restored by removing the deleting record with `plugin delete`.

## Coming from tes3cmd

The full command-by-command mapping is on [Coming from tes3cmd](@/docs/getting-started/coming-from-tes3cmd.md). For cleaning:


| tes3cmd | jess |
| --- | --- |
| `tes3cmd clean MyMod.esp` | `jess plugin clean MyMod.esp` (the record count is kept right) |
| `tes3cmd clean --gmsts MyMod.esp` | `jess plugin clean --evil-gmsts MyMod.esp` (`--gmsts` works too) |
| backups in a `backups` folder | backups beside the plugin: `MyMod.esp.bak`, `.bak1`, ... |
| cleans the file in place | the same, or `--output` for a new file |

## Related

- [`plugin clean`](@/docs/cli/plugin.md#clean), [`plugin delete`](@/docs/cli/plugin.md#delete): every flag.
- [Linting Plugins](@/docs/guides/linting.md): the checks behind the cleanups, and many more.
- In Luau: `plugin:clean { ... }` and `plugin:select(spec):delete()`; see the [plugin API](@/docs/api/plugin.md).
