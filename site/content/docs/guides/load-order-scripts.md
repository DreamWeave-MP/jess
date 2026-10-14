---
title: 'Merging and Fixing a Load Order'
description: 'Run TES3Merge, S3LightFixes, and Merged Lands over your load order from one Luau script.'
weight: 10
extra:
  kind: guide
---

Goal: one command that merges your load order's conflicting objects and leveled lists, fixes its lights, merges its terrain, and leaves the results enabled in OpenMW. Rerun it whenever the load order changes.

jess has Luau ports of [TES3Merge](https://github.com/NullCascade/TES3Merge), [S3LightFixes](https://github.com/DreamWeave-MP/S3LightFixes), and [Merged Lands](https://github.com/Sophialovesmorrowind/OpenMWMergedLands). They live in [Dreamwind](@/docs/scripting/packages.md#dreamwind), the package at `jpk/dreamwind` in the jess repository, as three modules: `@dreamwind/tools/tes3merge`, `@dreamwind/tools/s3lightfixes`, and `@dreamwind/tools/merged-lands`. Each returns a function that takes `(state, openmw)`: a table of options (and, afterwards, a report), and a loaded OpenMW config. You write the five lines that hand them your config.

## Set it up once

Save this as `load-order.jsl`, anywhere you like:

```luau
--!strict
-- load-order.jsl <openmw.cfg>: merge objects, fix lights, merge terrain.
local config: string = ...
local openmw = jess.openmw.load(config)
local mergedLands = require '@dreamwind/tools/merged-lands'
local s3lightfixes = require '@dreamwind/tools/s3lightfixes'
local tes3merge = require '@dreamwind/tools/tes3merge'

local state = {
  tes3merge = { fileFilters = { ['S3LightFixes.omwaddon'] = false } },
}

tes3merge(state, openmw)
s3lightfixes(state, openmw)
mergedLands(state, openmw)
```

Order matters. Merge first, then fix lights: S3LightFixes reads the active content, so once `Merged Objects.esp` is enabled, it fixes the merged lights rather than the ones they replaced. The `fileFilters` line keeps TES3Merge from reading S3LightFixes' own output, as native TES3Merge's shipped configuration does. Merged Lands only reads terrain, so it can go anywhere; last is as good as anywhere.

`--package` takes Dreamwind as the folder in a jess checkout, or packed into one file:

```sh
jess pack jess/jpk/dreamwind -o dreamwind.jpk
```

## Run it

```sh
jess run --package dreamwind.jpk load-order.jsl ~/.config/openmw/openmw.cfg
```

```text
tes3merge: wrote 22 merged records to /home/you/.local/share/openmw/data/Merged Objects.esp
# S3LightFixes 0.4.59
# config: /home/you/.config/openmw/openmw.cfg
# output: /home/you/.local/share/openmw/data/S3LightFixes.omwaddon
# content files: 3
# loaded plugins: 3
# masters: 3
# changed cells: 0
# changed lights: 676
LIGH "Light_Com_Candle_01" from "Bloodmoon.esm": color [245, 140, 40, 0] -> [140, 74, 46, 0], radius 128 -> 153, ...
...
merged-lands: 592 LAND and 123 LTEX records from 3 plugins in /home/you/.local/share/openmw/data/Merged Lands.omwaddon
```

S3LightFixes prints its whole log (every light it changed), so expect a wall of text; it also writes the same log to `lightconfig.log` beside `openmw.cfg`.

Three new plugins now sit in your [`data-local`](@/docs/reference/glossary.md#data-local) folder: `Merged Objects.esp`, `S3LightFixes.omwaddon`, and `Merged Lands.omwaddon`. `data-local` loads after every `data=` folder, and writing there means no mod's own folder gets touched. Without a `data-local=` line, they go in the first `data=` folder, which is TES3Merge's rule.

## Enable the results

Out of the box, none of the three touches your content list. Tick the plugins in the launcher, at the end of your load order: `Merged Objects.esp`, then `S3LightFixes.omwaddon`, then `Merged Lands.omwaddon` last of all. The merge loads after everything it merged, the light fixes after that, and the terrain after everything, since it carries whole LAND records.

Or have the tools do it, through their options: `activate = true` for TES3Merge and Merged Lands, `autoEnable = true` for S3LightFixes. Each appends its plugin to the content list when it isn't there yet, in the order they run, and saves `openmw.cfg` after backing it up (`openmw.cfg.bak`, then `.bak1`, ...). One pointed at a folder that isn't a data folder leaves its output disabled, with a message, since OpenMW wouldn't find it there.

## Options

Each tool reads its options from a table in `state` named after it: `state.tes3merge`, `state.s3lightfixes`, `state.mergedlands`. With the results enabled automatically:

```luau
local state = {
  tes3merge = {
    activate = true,
    fileFilters = { ['S3LightFixes.omwaddon'] = false },
  },
  s3lightfixes = { autoEnable = true },
  mergedlands = { activate = true },
}
```

S3LightFixes also reads `lightconfig.toml` next to your `openmw.cfg`, like the original, and writes a default one the first time; Merged Lands reads `merged_lands.toml` and its patch files. After each run, its table holds a `report` for whatever runs next. Every option, with its defaults and how it differs from the native tool, is on the [TES3Merge](@/docs/bundled-scripts/tes3merge.md), [S3LightFixes](@/docs/bundled-scripts/s3lightfixes.md), and [Merged Lands](@/docs/bundled-scripts/mergedlands.md) pages. When two mods reshape the same hill and you want both, a Merged Lands patch file says how they combine.

## Rerun after every load order change

The merged plugins and the light fixes are snapshots of the load order they were made from. Add, remove, or reorder mods, and run the script again. Each run replaces the previous output: `Merged Objects.esp` and `Merged Lands.omwaddon` are overwritten, and the old `S3LightFixes.omwaddon` is deleted before the new one is written.

Nothing else is edited. Your mods' plugins are only read.

## Changing a tool

Options are the better tool when they cover what you want: they survive every Dreamwind update. When they don't reach, change the tool. Each one is a single file, plain Luau against the public API: [`tes3merge/init.jsl`](https://github.com/DreamWeave-MP/jess/blob/main/jpk/dreamwind/src/tools/tes3merge/init.jsl) and its siblings under `jpk/dreamwind/src/tools`. Copy it beside your script and require the copy by its relative path:

```luau
local tes3merge = require './tes3merge'
```

From then on it's yours. It doesn't get the next Dreamwind's fixes until you merge them in; diff your copy against the new file when you update.

## Scripts that come with mods

A mod can ship its own load-order module, a file that returns a `function(state, openmw)`, in its data folder. `require` finds game files through the VFS once a script has a default config, so after `jess.openmw.configPath(config)`, `require('scripts.coolmod.patch')` loads `scripts/coolmod/patch.jsl` from whichever data folder or archive has it, and you call what it returns like the tools above.

Scripts aren't sandboxed: a load-order script can read and write anything you can. Only run ones from mods you trust, the same as you would an MWSE mod or a DLL.

## When something's wrong

When a script errors, `jess run` stops, prints the Luau error, and exits `1`; nothing after the failing call runs. Anything earlier calls already wrote stays.

Checking a load order instead of changing it is a script too. Dreamwind's [doctor](@/docs/bundled-scripts/doctor.md), `@dreamwind/tools/doctor`, checks for missing masters and content, files nothing provides, models OpenMW won't load, dirty plugins, and changes a later plugin throws away, and writes a report with a fix for each. Call it last, with the same `state`, and it knows what the tools before it did:

```luau
local doctor = require '@dreamwind/tools/doctor'

doctor(state, openmw)
```

[Asset Checking](@/docs/guides/asset-checking.md) and [Checking and Fixing NIFs](@/docs/guides/nif-checking-and-fixing.md) cover the CLI's half: plugins and folders you name.

## Related

- [`jess run`](@/docs/cli/run.md): every flag.
- [Load-Order Scripts](@/docs/scripting/load-order-scripts.md): writing your own, and what `state` and `openmw` give you.
- [Bundled Scripts](@/docs/bundled-scripts/_index.md): the tools, every option.
- [Merged Lands](@/docs/bundled-scripts/mergedlands.md): terrain, and the patch files that decide how it merges.
- [Doctor](@/docs/bundled-scripts/doctor.md): the checkup, check by check.
