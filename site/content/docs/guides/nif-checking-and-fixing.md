---
title: 'Checking and Fixing NIFs'
description: 'Find the meshes that won''t load, draw wrong, or cost too much, and fix the ones with one clear fix.'
weight: 90
extra:
  kind: guide
---

Goal: find the mesh that puts a red error marker in your game (or makes you fall through a floor, or walk through a wall), and fix it if there's one right fix.

`jess nif check` reads NIFs the way OpenMW's loader does, following its code rule by rule, and says what OpenMW will do with each one.

## Check a mod

Point it at a mod's folder, an archive, or single files:

```sh
jess nif check "Mods/MyMod/Data Files"
jess nif check MyMod.bsa
jess nif check --data "Data Files/Morrowind.bsa" meshes/f/furn_de_firepit_01.nif
```

Everything your load order loads is a script's job, since the CLI doesn't read `openmw.cfg`. [`jess.nif.check`](@/docs/api/nif.md) with no inputs and a config checks every mesh the load order's VFS has, winners only:

```luau
--!strict
local config: string = ...
local report = jess.nif.check(nil, { config = config, show = 'wontLoad' })

print(`{report.summary.files} files, {report.summary.wontLoad} won't load`)
```

```sh
$ jess run all-meshes.luau ~/.config/openmw/openmw.cfg
7506 files, 2 won't load
```

Here's all of vanilla Morrowind, straight out of `Morrowind.bsa`, in about eight seconds:

```sh
jess nif check ~/Games/Morrowind/Data\ Files/Morrowind.bsa
```

```yaml
summary:
  textures_checked: true
  load_order_checked: true
  files: 7506
  deeply_checked: 7506
  wont_load: 2
  by_worst_severity:
    broken: 11
    collision: 46
    info: 220
    missing_asset: 58
    performance: 48
    wont_load: 2
  by_code:
    controller_without_data: 30
    invisible_shape_collides: 47
    missing_texture: 58
    no_root_nodes: 2
    root_transform_ignored: 137
    # ...
files:
- path: meshes/a/a_ice minions_shield2.nif
  source: /home/you/Games/Morrowind/Data Files/Morrowind.bsa
  version: 4.0.0.2
  game: Morrowind
  deeply_checked: true
  worst: collision
  findings:
  - severity: collision
    code: invisible_shape_collides
    message: '"Tri Shield_iron 0" (and 2 more shapes) is hidden and doesn''t draw, but still collides because there''s no RootCollisionNode'
    record: 4
  # ...
```

Yes, Bethesda shipped two meshes OpenMW won't load. Nobody's perfect.

## Read the results

`summary` counts everything. `files` lists each file with a finding at the `--show` level or worse, with its findings, its textures and where each was found, and some statistics.

Every finding has a severity:

| Severity | What you'll see in game |
| --- | --- |
| `wont_load` | The error marker, or nothing, instead of the mesh. |
| `broken` | Something in it is dropped or drawn differently than its author made it. |
| `missing_asset` | A texture that can't be found: purple, or the wrong texture. |
| `collision` | You walk through it, or bump into things you can't see. |
| `performance` | It works, but costs frame time or memory for nothing. |
| `info` | Worth knowing; not a problem. |

and a `code` naming the rule, plus a `message` in plain words and the `record` it's about. [Checking Rules](@/docs/nif/checking-rules.md) explains every code and the OpenMW code it comes from.

By default you see `collision` and worse. Change that with `--show`:

```sh
jess nif check --show wont-load MyMod.bsa        # only what won't load
jess nif check --show performance MyMod.bsa      # add performance smells
jess nif check --verbose MyMod.bsa               # every file, every finding
```

`jess nif check` exits `1` when anything won't load, so it drops straight into a mod's CI:

```sh
jess nif check --show wont-load --format json "Data Files" > nif-report.json
```

## Textures

Textures are looked up exactly as OpenMW does it: under `textures/`, `.dds` first whatever the NIF says, then the name as written. They're found in your inputs first, then the `--data` folders and archives. A missing texture looks like this:

```yaml
  - severity: missing_asset
    code: missing_texture
    message: texture "TX_A_Indoril_LA_mail.tga" not found (tried textures/tx_a_indoril_la_mail.dds, textures/tx_a_indoril_la_mail.tga)
```

When a mod depends on textures that aren't in its own folder (another mod's, or Morrowind's archives), add them with `--data` rather than get a wall of `missing_texture`:

```sh
jess nif check --data "Mods/OAAB_Data/00 Core" "Mods/MyOAABHouse/Data Files"
```

`--data` folders and archives are looked in, not checked. Repeat it for more; later ones win.

## Look closer

`nif info` describes a file: header, record types, collision setup, statistics, textures, and the scene graph:

```sh
jess nif info meshes/i/tx_crystal_02.nif
jess nif info --no-tree "Mods/MyMod/Data Files/meshes/x/rock.nif"
```

`nif dump` shows every field of every record, with links as record indices:

```sh
jess nif dump --list meshes/f/furn_de_firepit_01.nif
jess nif dump --type NiTriShape meshes/f/furn_de_firepit_01.nif
jess nif dump --index 0 --index 11 meshes/f/furn_de_firepit_01.nif
jess nif dump --name 'collision' meshes/x/ex_something.nif
```

A finding's `record` is the index to look at: `jess nif dump --index 4 ...` for the shield above.

Both read NIFs from every game OpenMW supports, the way OpenMW reads them: point `--data` at an Oblivion or Skyrim archive and dump away.

```sh
jess nif dump --data "Oblivion - Meshes.bsa" meshes/clutter/apple01.nif
```

## Fix what has one fix

Some problems have exactly one right answer. `nif fix` makes those repairs. Three of them add or change the string-data markers OpenMW looks for on the root node ([`RCN`, `NCC`, `MRK`](@/docs/reference/glossary.md#rcn)):

| Fix | Problem | Repair |
| --- | --- | --- |
| `rcn` | The `RootCollisionNode` is nested under a child, so OpenMW collides with the render geometry instead. | Adds `RCN` string data, so OpenMW searches for it. |
| `ncc` | Lowercase `ncc` string data turns off *all* collision. | Uppercases it to `NCC`, which keeps camera collision. |
| `markers` | `Tri EditorMarker` shapes that would draw in game. | Adds `MRK` string data to hide them. |
| `skin-weights` | Skin weights for vertices the shape doesn't have; OpenMW refuses the file. | Drops them. |
| `unreachable` | Records nothing links to. | Drops them. |

Dry run first, always:

```sh
jess nif fix --dry-run "Mods/MyMod/Data Files"
```

```yaml
summary:
  files: 1
  fixed: 1
  written: 0
  skipped: 0
  by_fix:
    ncc: 1
files:
- path: meshes/f/furn_de_firepit_01.nif
  source: Mods/MyMod/Data Files
  fixes:
  - fix: ncc
    message: '"ncc" is now "NCC", so the camera still collides'
    record: 14
  written: null
  skipped: null
```

Then for real. There are two ways to write the results.

**In place**, each fixed file overwritten after a backup beside it (`.bak`, `.bak1`, ...). Right for your own mod:

```sh
jess nif fix "Mods/MyMod/Data Files"
```

**Into an override folder**, leaving the originals alone. Each fixed file lands at its VFS path under the folder you name, which you then add as the last `data=` line in `openmw.cfg` so its copies win. Right for other people's mods, and required for anything inside an archive:

```sh
jess nif fix --output "Mods/NIF Fixes" MyMod.bsa
```

Several mods go into one override folder the same way: name them all.

```sh
jess nif fix --output "Mods/NIF Fixes" --dry-run "Mods/ModA/Data Files" "Mods/ModB/Data Files"
```

`--only` picks fixes (`--only ncc,markers`). Only Morrowind NIFs are fixed; other games' are skipped. `nif fix` exits `1` when a file needed fixing and couldn't be written, and says why under that file's `skipped`.

Everything else `check` finds needs a human decision (or a script: [NIF Optimization Cookbook](@/docs/nif/optimization-cookbook.md) has recipes).

## Related

- [`jess nif`](@/docs/cli/nif.md): every flag and field.
- [Checking Rules](@/docs/nif/checking-rules.md): every finding code.
- [Editing NIFs](@/docs/nif/editing.md): when you need to change a mesh yourself.
- [Checking Assets](@/docs/guides/asset-checking.md): the same check, run on the models your plugins actually use.
