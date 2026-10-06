---
title: 'Selections'
description: 'Pick records with selectors, then count, decode, delete, replace, or extract them.'
weight: 30
extra:
  kind: guide
---

A selection is "these records in this plugin": every daedric weapon, every interior cell, every crate reference in Tribunal. You describe what you want in a small table, the **spec**, and get back a selection you can loop over or act on all at once. It's the same filter language as the CLI's `--type`/`--id`/`--instance-match` flags, so anything you've done with `jess plugin delete` works here too.

## Pick and loop

```luau
--!strict
local morrowind = jess.plugin.open 'Morrowind.esm'

local daedric = morrowind:select { type = 'WEAP', id = '^daedric' }
print(daedric:count(), 'daedric weapons')

for record in daedric:records() do
  local weapon = record :: WeaponRecord
  if weapon.data.weight > 60 then print(weapon.id, weapon.data.weight) end
end
```

```
25	daedric weapons
daedric battle axe	90
daedric claymore	81
daedric war axe	72
daedric warhammer	96
daedric warhammer_ttgd	96
```

`records()` yields [record handles](@/docs/scripting/records-and-views.md), in plugin order. A selection can also be looped over directly (`for record in selection do`), and so can a plugin, which yields every record; `#selection` and `#plugin` count them.

## The spec

Every key is optional; a record has to pass all the keys you give. Patterns are case-insensitive regexes in Rust's [`regex` syntax](https://docs.rs/regex/latest/regex/#syntax) (not Luau patterns), except `exactId`.

| Key | Takes | Picks |
| --- | --- | --- |
| `type` | type or list | Records of any of these types, as tags or type names: `"WEAP"`, `{ "Armor", "Clothing" }`. Case doesn't matter, and `"npc"` means `NPC_`. |
| `flags` | name or list | Records with any of these header flags: `deleted`, `persistent`, `ignored`, `blocked`, or a number. |
| `id` | pattern or list | Records whose id matches every pattern. |
| `exactId` | id or list | Records whose id is any of these, ignoring case. |
| `match` | pattern or list | Records whose raw bytes match every pattern. |
| `notMatch` | pattern or list | Records whose raw bytes match none of them. |
| `sub` | pattern | Records with a subrecord whose text matches... |
| `notSub` | pattern | ...and doesn't match this. |
| `instance` | pattern | `CELL` references whose text matches... |
| `notInstance` | pattern | ...and doesn't match this. |
| `interior` | boolean | Interior `CELL`s only. |
| `exterior` | boolean | Exterior `CELL`s only. |

A subrecord's text is `name: value`: the field name and its text for string fields (`name: Boethiah's Pillow Book`), else the tag and the raw payload. A reference's text is its subrecords' texts, one per line: `instance = "crate"` matches "crate" anywhere in the reference, and `instance = "(?m)^id: crate"` only at the start of its object id. `plugin:select()` with no spec picks every record. An unknown key is an error that lists the real ones, so `{ ids = "x" }` fails loudly instead of quietly picking everything.

`instance` and `notInstance` work on the references inside cells: a selection with them picks those references, not whole cells. `delete()` then deletes just the references.

## Selections are live

A selection is a question, not a list. Every use asks it again of the plugin as it is right then:

```luau
--!strict
local plugin = jess.plugin.new()
local notes = plugin:select { type = 'BOOK', id = '^jess_' }
print(notes:count())

plugin:insert { type = 'Book', id = 'jess_note_1' }
plugin:insert { type = 'Book', id = 'jess_note_2' }
print(notes:count(), table.concat(notes:ids(), ', '))
```

```
0
2	jess_note_1, jess_note_2
```

Make the selection once and reuse it; you don't have to rebuild it after changing the plugin.

## Acting on everything at once

| Method | Does |
| --- | --- |
| `records()` | Iterates the picked records' handles. |
| `count()` | How many records it picks. |
| `first()` | The first picked record's handle, or `nil`. |
| `list()` | Every picked record's handle, as an array. |
| `ids()` | Every picked record's id. |
| `decode()` | Every picked record as a [decoded table](@/docs/scripting/records-and-views.md#decoded-tables-decode-set-update-edit). |
| `display()` | Every picked record as `jess plugin dump` shows it. |
| `layout(options)` | Every picked record's byte layout, like `jess plugin dump --raw`. |
| `delete()` | Deletes what it picks; returns how many `records`, `subrecords`, and `references` went. |
| `replace(pattern, replacement, options)` | Replaces text in the picked records' string fields; returns how many `records` and `fields` changed. |
| `extract(options)` | A new plugin holding copies of the picked records. |

Deleting and replacing:

```luau
--!strict
local tribunal = jess.plugin.open 'Tribunal.esm'

-- Every crate in every Tribunal cell, gone. Only the references go; the cells stay.
local crates = tribunal:select { type = 'CELL', instance = 'crate' }
local removed = crates:delete()
print(removed.records, removed.references)

-- Rename across every string field of the records picked.
local renamed =
  tribunal:select({ type = 'BOOK', sub = 'Mournhold' }):replace('Mournhold', 'Mournhold City')
print(renamed.records, renamed.fields)
```

```
0	282
3	3
```

`replace` takes the pattern and the replacement separately, or tes3cmd-style as one string: `replace("/Mournhold/Mournhold City/")`. Either way the pattern ignores case, like the spec's. The replacement expands `$1` and `${name}` from the pattern's groups. `{ first = true }` replaces only the first match in each field. Nothing here is saved until you `save()` the plugin.

`extract()` returns a new, unsaved plugin: the source's `TES3` header (masters included, record count updated) followed by copies of the picked records. `{ header = false }` leaves the header out. [Creating Plugins](@/docs/scripting/creating-plugins.md) shows how to turn that into a proper patch.

`extract` copies what the selection picks, the way `display()` shows it. A record picked through `sub` or `notSub` is copied whole. A cell picked through `instance` or `notInstance` is copied with only its matching references: `select({ instance = "crate" }):extract()` gives the cells that have crates, holding just the crates.

## Reusing a spec

`plugin:select(spec)` compiles the spec's patterns every time. To run the same spec over many plugins, compile it once with `jess.select` and pass that instead:

```luau
--!strict
local gold = jess.select { type = 'MISC', id = '^gold_' }

for _, name in { 'Morrowind.esm', 'Tribunal.esm', 'Bloodmoon.esm' } do
  print(name, jess.plugin.open(name):select(gold):count())
end
```

```
Morrowind.esm	7
Tribunal.esm	0
Bloodmoon.esm	0
```

`plugin:layout(spec)` takes a spec or compiled selector the same way.

## Pitfalls

- **`instance` picks cells.** Instance filters match references, which only cells have, so a spec with `instance` or `notInstance` picks nothing but cells, with or without `type = "CELL"`.
- **Ids are regexes.** `id = "gold_001"` also picks `gold_0010` and `my_gold_001`. Anchor it (`"^gold_001$"`) or use `exactId`.
- **Regex, not Luau patterns.** `%d` means nothing here; use `\\d` (the backslash doubled inside a Luau string) or a character class.

## From the CLI

Every key has a CLI flag, so a selection you've tried with `jess plugin dump` or `delete` translates directly:

| Spec key | CLI flag |
| --- | --- |
| `type` | `-t`, `--type` |
| `flags` | `--flag` |
| `id` | `--id` |
| `exactId` | `--exact-id` |
| `match` / `notMatch` | `--match` / `--no-match` |
| `sub` / `notSub` | `--sub-match` / `--sub-no-match` |
| `instance` / `notInstance` | `--instance-match` / `--instance-no-match` |
| `interior` / `exterior` | `--interior` / `--exterior` |

`jess plugin delete` is `select(...):delete()`, `jess plugin modify --replace` and `--replace-first` are `replace(...)` without and with `{ first = true }`, and `jess plugin dump` is `display()`. See the [`jess plugin` reference](@/docs/cli/plugin.md) and the [selection API reference](@/docs/api/selection.md).
