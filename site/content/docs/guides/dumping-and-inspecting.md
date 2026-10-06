---
title: 'Dumping and Inspecting'
description: 'Read any record, subrecord, or byte in a plugin.'
weight: 70
extra:
  kind: guide
---

Goal: see exactly what a plugin contains, from a count of its records down to the byte offset of a single subrecord, and make quick text edits without opening the Construction Set.

## Start with the shape

```sh
jess plugin stats "Siege at Firemoth.esp"
```

`stats` counts records by type and subrecords by `RECORD.SUBRECORD`. It's the fastest way to learn what a mod *is*: 300 `CELL`s and no `NPC_` is a landmass; 2,000 `INFO` is a quest mod. Give it several files and it sums them.

## Find records

`--list` shows what matches without the contents:

```sh
jess plugin dump --type book --id 'bk_BriefHistory' --list Morrowind.esm
```

```yaml
records:
- record: 11182
  type: BOOK
  id: bk_BriefHistoryEmpire1
- record: 11183
  type: BOOK
  id: bk_BriefHistoryEmpire2
# ...
- record: 15876
  type: BOOK
  id: bk_BriefHistoryofWood_01
```

`record` is the record's position in the file (the header is 0), under the same name as in the full dump. It's the same number Luau scripts see as `jess.meta(record).index`.

The selectors, which `dump`, `delete`, and `modify` all share:

| Want | Use |
| --- | --- |
| a record type | `--type book` (`-t`), repeatable; four-letter tags or type names (`Light`, `MiscItem`) in any case, `npc` for `NPC_` |
| IDs matching a pattern | `--id '^bk_'`, a case-insensitive regex |
| one exact ID | `--exact-id bk_BriefHistoryEmpire1` |
| records containing some bytes | `--match 'Dagoth'`, `--no-match 'Dagoth Ur'` |
| records with a header flag | `--flag deleted`, `persistent`, `ignored`, `blocked` |
| interior or exterior cells | `--interior`, `--exterior` |
| records with a matching subrecord | `--sub-match '^mesh:.*dwrv'` |
| cells with matching references | `--instance-match fargoth` (picks only `CELL`s) |

Everything you give must match. The full rules are in the [selector reference](@/docs/cli/plugin.md#selectors).

`--match` searches the record's raw bytes: IDs, names, script text, book text, everything, in one regex. It's a fast way to find "which records mention Vivec at all":

```sh
jess plugin dump --match vivec --list Morrowind.esm
```

## Read records

Drop `--list` to get the whole record, decoded:

```sh
jess plugin dump --type misc --exact-id Misc_SoulGem_Grand Morrowind.esm
```

```yaml
records:
- record: 3002
  type: MiscItem
  id: Misc_SoulGem_Grand
  name: Grand Soul Gem
  mesh: m\Misc_soulgem_grand.NIF
  icon: m\Tx_soulgem_Grand.tga
  data:
    weight: 2.0
    value: 200
```

Cells show their references. Instance filters keep just the ones you care about:

```sh
jess plugin dump --type cell --instance-match fargoth Morrowind.esm
```

```yaml
records:
- record: 19012
  type: Cell
  name: Seyda Neen
  data:
    flags: HAS_WATER | RESTING_IS_ILLEGAL | 0x40
    grid: [-2, -9]
  region: Bitter Coast Region
  map_color: [202, 165, 96, 0]
  references:
  - mast_index: 0
    refr_index: 113852
    id: ex_nord_door_01
    temporary: false
    translation: [-11958.173, -67956.75, 452.5]
    rotation: [0.0, 0.0, 3.8990655]
    owner: fargoth
    # ...
  - mast_index: 0
    refr_index: 128964
    id: fargoth
    temporary: false
    translation: [-10510.78, -71218.164, 196.34192]
    rotation: [0.0, 0.0, 1.9707965]
- record: 21358
  type: Cell
  name: 'Seyda Neen, Fargoth''s House'
  # ...
```

Fargoth himself, and every door and bottle he owns. Floats print in the shortest spelling that reads back as the same value (`3.8990655`, not a rounded `3.899`), so nothing in a dump is approximate.

The readable dump is for reading. Its shape follows the record types (snake_case field names, flags as `NAME | NAME`), and it doesn't round-trip back into a plugin; edit with `plugin modify`, or with a [Luau script](@/docs/scripting/records-and-views.md) when you need more.

## Read bytes

`--raw` shows the layout instead: every subrecord's tag, size, and byte range, with string fields decoded. It prints JSON unless you ask for YAML:

```sh
jess plugin dump --raw --format yaml --type stat --exact-id bt_balmora_bench BalmoraTweaks.esp
```

```yaml
records:
- index: 1
  type: STAT
  id: bt_balmora_bench
  flags: 0
  size: 54
  range:
    start: 362
    end: 432
  subrecords:
  - index: 0
    type: NAME
    size: 17
    range:
      start: 378
      end: 403
    field: id
    value: bt_balmora_bench
  - index: 1
    type: MODL
    size: 21
    range:
      start: 403
      end: 432
    field: mesh
    value: bt\balmora_bench.nif
```

`index` is the record's number, as `record` is in the typed dump (and a subrecord's position in its record), the name Luau's `record:layout()` uses too. `range` is in bytes from the start of the file and includes the header; `size` is the payload size. Open a hex editor at those offsets and you're looking at exactly that subrecord. `--raw --list` shows just the records' ranges.

## Pull records out

`--raw-output` writes the matching records to a file instead of printing them, byte for byte. Records are copied whole even when `--sub-match` picked them, while a cell picked with `--instance-match` keeps only the matching references, so `--instance-match fargoth --raw-output fargoth.esp` pulls out just Fargoth's placements. A file already at that path is backed up first (`histories.esp.bak`):

```sh
jess plugin dump --type book --id '^bk_BriefHistoryEmpire[1-4]$' --raw-output histories.esp --header Morrowind.esm
```

```yaml
input: Morrowind.esm
path: /home/you/Games/Morrowind/Data Files/Morrowind.esm
bytes: 22276
written:
  path: histories.esp
```

`--header` puts a `TES3` header in front (copied from the input, record count fixed) so the result is a plugin jess and other tools can open. Since it's Morrowind.esm's header, the new file says it's a master with no masters of its own. That's fine for poking at; if you're going to ship it, fix the header with a few lines of Luau (`setHeader { fileType = "plugin" }`, `setMasters { "Morrowind.esm" }`; see [Masters and Headers](@/docs/guides/masters-and-headers.md)).

## Quick text edits

`plugin modify` does regex search-and-replace inside string fields, for the jobs that don't deserve a script. Always narrow it to the field you mean with `--sub-match`, which matches the field's `name: value` text:

```sh
# prefix every static's ID
jess plugin modify --type stat --sub-match '^id:' --replace '/^/PC_/' BalmoraTweaks.esp

# point a model at a new mesh
jess plugin modify --type stat --sub-match '^mesh:' --replace '/balmora_bench/balmora_bench_02/' BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
fields: 1
records: 1
written:
  path: BalmoraTweaks.esp
  backup: BalmoraTweaks.esp.bak
```

`--replace` replaces every match in each field; `--replace-first` only the first. `--dry-run` reports the counts without writing anything. The pattern is case-insensitive, `$1` and `${name}` in the replacement are captures, and the first character is the delimiter (`|x/y|z|` when the pattern has slashes).

Without `--sub-match`, every string field of the selected records is searched, IDs and paths included. `--replace '/bench/seat/'` on a static named `bt_balmora_bench` with model `bt\balmora_bench.nif` changes both. Usually not what you meant.

The field names to match on are the ones `--raw` prints next to each subrecord (`id`, `mesh`, `name`, `icon`, `script`, `text`, ...), which are the typed dump's names for them. Try the selection with `dump` first; it takes the same flags.

## Non-English plugins

Text in plugins is in a Windows code page, not UTF-8. jess reads it as Windows-1252 unless told otherwise, so a Russian or Polish plugin needs `--encoding`, after the command it applies to:

```sh
jess plugin dump --encoding win1251 --type book RuMod.esp
jess plugin modify --encoding win1250 --type npc --sub-match '^name:' --replace '/Jan/Janek/' PlMod.esp
```

The default, `win1252`, covers English, French, German, and the other Western releases. A wrong encoding garbles the text jess shows you, and makes replacements with characters the encoding can't hold fail, but it never damages the file: bytes jess doesn't change stay as they were.

## Into other tools

```sh
# every book title in the game
jess plugin dump --type book --format json Morrowind.esm | jq -r '.records[].name'

# how many NPCs Tribunal adds or changes
jess plugin stats Tribunal.esm | yq '.record_types.NPC_'
```

## Related

- [`plugin dump`, `delete`, `modify`, `stats`](@/docs/cli/plugin.md#dump): every flag.
- [Records and Views](@/docs/scripting/records-and-views.md): the same data in Luau, editable.
