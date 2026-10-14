---
title: 'Checking Assets'
description: 'Find every missing model, texture, icon, sound, and script your plugins refer to.'
weight: 100
extra:
  kind: guide
---

Goal: before you ship a mod (or after you install one), make sure every file its plugin names actually exists where OpenMW will look, and that every model it uses loads.

## Check a plugin

```sh
jess plugin assets BalmoraTweaks.esp
```

```yaml
summary:
  plugins:
  - BalmoraTweaks.esp
  unreadable_plugins: 0
  references: 6
  files: 5
  missing: 1
  meshes_checked: 2
  meshes_wont_load: 0
  by_code:
    missing_mesh: 2
  meshes_by_code: {}
findings:
- severity: missing_asset
  code: missing_mesh
  message: model "bt\balmora_bench.nif" not found at meshes/bt/balmora_bench.nif
  plugin: BalmoraTweaks.esp
  record_type: STAT
  id: bt_balmora_bench
  path: bt\balmora_bench.nif
- severity: missing_asset
  code: missing_mesh
  message: model "bt\balmora_bench.nif" not found at meshes/bt/balmora_bench.nif
  plugin: BalmoraTweaks.esp
  record_type: STAT
  id: bt_balmora_bench
  path: bt\balmora_bench.nif
```

```text
error: 1 referenced file is missing
```

Six references, five distinct files, one missing (named by two records). Each finding says which record wants the file, the path as the record spells it, and where OpenMW looked.

The plugin's own folder is searched first, as if the mod were installed last, then each `--data` folder or archive. So a mod checked in place in `Data Files` finds its own loose meshes; what it borrows from Morrowind's archives takes `--data "Data Files/Morrowind.bsa"`. The CLI doesn't read your load order, so anything you don't name isn't searched.

## What gets checked

Every file a record names, looked up the way OpenMW does it (a port of OpenMW's `resourcehelpers.cpp`):

| Kind | From | Looked up |
| --- | --- | --- |
| Models | statics, body parts, and every item (weapons, armor, clothing, books, potions, ingredients, tools, misc) | under `meshes/`, exactly as written |
| Animated models | activators, containers, doors, lights, creatures, NPCs | under `meshes/`, using the `x`-prefixed variant when its `.kf` exists, as OpenMW does |
| Distant models | objects OpenMW pages in the distance | their `_dist` versions too |
| Icons | items and lights | under `icons/`, `.dds` first |
| Magic effect icons | magic effects | the icon, and its `b_` big icon |
| Textures | magic effect particles, birthsigns, landscape textures | under `textures/`, `.dds` first |
| Sounds | sound records | under `sound/`, falling back to `.mp3` like OpenMW |
| Voice | dialogue voice lines, and `Say` in scripts and dialogue results | under `sound/` |
| Music, video | `StreamMusic` and `PlayBink` in scripts and dialogue results | under `music/` and `video/` |
| Book images | `<IMG>` tags in book text | under `bookart/` or `textures/`, `.dds` first, with the size-suffixed fallback |
| Lua scripts | OpenMW script lists | the script path |

Every model that's found then goes through the full [NIF check](@/docs/guides/nif-checking-and-fixing.md), textures included. A model OpenMW won't load counts as a failure, like a missing file. `--no-meshes` skips that when you only care about files existing.

Records whose text OpenMW can't use turn up too, like book markup it can't lay out (`book_markup_error`).

## Check the whole load order

The command line checks the plugins you name. "What's actually missing from my game" is a question about the load order, so it's a script: [`jess.plugin.assets`](@/docs/api/plugin.md) with no plugins and a config checks every active plugin, counting only the records that win. If a mod replaces a vanilla lamp's mesh, the vanilla mesh isn't checked, because OpenMW never loads it.

```luau
--!strict
local config: string = ...
local report = jess.plugin.assets(nil, { config = config, meshes = false })

print(`{report.summary.files} files, {report.summary.missing} missing`)
```

```sh
$ jess run missing.jsl ~/.config/openmw/openmw.cfg
14237 files, 25 missing
```

Everything is looked up through that config's VFS: every `data=` folder and archive. It's the script to run after installing a batch of mods; drop `meshes = false` to put every model through the NIF check too.

## Add folders

A mod that needs another mod's assets (OAAB, Tamriel Data) reports all of them missing when the dependency isn't installed. Point jess at it with `--data`:

```sh
jess plugin assets --data "Mods/OAAB_Data/00 Core" --data "Mods/TamrielData/00 Data Files" MyOAABHouse.esp
```

`--data` takes folders or archives, repeatable, later ones winning; the plugin's own folder still wins over all of them.

## Narrow or widen the output

`--show` works like `nif check`'s: the least severe finding to show, default `collision`.

```sh
jess plugin assets --show missing-asset MyMod.esp    # missing files and broken models only
jess plugin assets --verbose MyMod.esp               # everything: every reference, model, and file
jess plugin assets --show files MyMod.esp            # just the files the plugin loads
```

`--show files` prints `used_files`, every file the plugin pulls in, with where it is on disk:

```yaml
used_files:
- file: /home/you/Games/Morrowind/Data Files/Morrowind.bsa
  path: icons/m/misc_com_bottle_03.dds
  in_archive: true
- file: /home/you/Mods/MyMod/Data Files/meshes/bt/balmora_bench.nif
  path: meshes/bt/balmora_bench.nif
  in_archive: false
# ...
```

That's a packing list: `jq -r '.used_files[] | select(.in_archive | not) | .file'` gives the loose files a mod actually needs, which is handy for spotting the 300 MB of unused textures in someone's upload. Missing files have no path to list, so they're named on stderr.

## In CI

`plugin assets` exits `1` when a file is missing, a plugin can't be read, or a model won't load, and prints the full report first. Run it on every build:

```sh
jess plugin assets --data "Data Files" "Data Files/MyMod.esp"
```

Only your inputs and `--data` are searched, which is what you want in CI anyway: the build machine has no load order to lean on. Add the vanilla archives with `--data` too if your mod borrows from them.

## Related

- [`plugin assets`](@/docs/cli/plugin.md#assets): every flag and field.
- [Checking and Fixing NIFs](@/docs/guides/nif-checking-and-fixing.md): the model check on its own.
- In Luau, `jess.plugin.assets` returns the same report; see the [plugin API](@/docs/api/plugin.md).
