---
title: 'Bundled Scripts'
description: 'jess''s load-order tools in Dreamwind: TES3Merge, S3LightFixes, Merged Lands, and the doctor.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 70

extra:
  kind: reference
---

jess has Luau ports of tools most OpenMW load orders already run. They ship in [Dreamwind](@/docs/scripting/packages.md#dreamwind), the package at `jpk/dreamwind` in the jess repository, as modules that each return a function taking `(state, openmw)`: an options table, and a loaded OpenMW config. A script loads your config and calls them; `jess run --package` makes them `require`-able. These pages cover every option and every way the ports differ from the native tools.

| Module | Ports | Writes | native / jess, {{ bench(value="plugins") }}-plugin load order |
| --- | --- | --- | --- |
| [`@dreamwind/tools/tes3merge`](@/docs/bundled-scripts/tes3merge.md) | [TES3Merge](https://github.com/NullCascade/TES3Merge) | `Merged Objects.esp` | {{ bench(workload="tes3merge", value="faster") }}x wall, {{ bench(workload="tes3merge", value="less_memory") }}x memory |
| [`@dreamwind/tools/s3lightfixes`](@/docs/bundled-scripts/s3lightfixes.md) | [S3LightFixes](https://github.com/DreamWeave-MP/S3LightFixes) 0.4.59 | `S3LightFixes.omwaddon` | {{ bench(workload="s3lightfixes", value="faster") }}x wall, {{ bench(workload="s3lightfixes", value="less_memory") }}x memory |
| [`@dreamwind/tools/merged-lands`](@/docs/bundled-scripts/mergedlands.md) | [Merged Lands](https://github.com/Sophialovesmorrowind/OpenMWMergedLands) 0.2.0 | `Merged Lands.omwaddon` | {{ bench(workload="mergedlands", value="faster") }}x wall, {{ bench(workload="mergedlands", value="less_memory") }}x memory |

[`@dreamwind/tools/doctor`](@/docs/bundled-scripts/doctor.md) ports nothing and changes nothing. It checks a load order for what breaks games and writes `jess-doctor.yaml` beside your `openmw.cfg`, with a fix for each finding. Call it the same way, last, with the same `state`, and it knows what the tools before it wrote.

## Running them

```luau
--!strict
-- load-order.luau <openmw.cfg>: merge objects, fix lights, merge terrain.
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

```sh
jess run --package jess/jpk/dreamwind load-order.luau ~/.config/openmw/openmw.cfg
```

Merge first, then fix lights, so S3LightFixes sees the merged lights. Native TES3Merge's shipped configuration leaves `S3LightFixes.omwaddon` out of the merge; the `fileFilters` line does the same. Merged Lands only reads terrain, so it can run anywhere in the list.

All three write their plugins to `data-local` (else the first `data=` folder), and none touches your content list unless you ask (`activate`, `autoEnable`). Enable the outputs at the end of your load order, in the order the script runs them: `Merged Objects.esp`, then `S3LightFixes.omwaddon`, then `Merged Lands.omwaddon`, which goes after everything else. [Merging and Fixing a Load Order](@/docs/guides/load-order-scripts.md) walks through it.

## Configuring

Options go in a table in `state`, named after the tool: `state.tes3merge`, `state.s3lightfixes`, and `state.mergedlands`. S3LightFixes also reads `lightconfig.toml`, and Merged Lands `merged_lands.toml` and its patch files, as the originals do. After a run, each leaves a `report` in its table for whatever runs next.

The tools are plain Luau against the public API, with no private hooks into jess: anything they do, your own script can do. Their source is in [`jpk/dreamwind/src/tools`](https://github.com/DreamWeave-MP/jess/tree/main/jpk/dreamwind/src/tools).

## Changing them

When the options don't reach, change the tool. Each is one file: copy it beside your script, `require('./tes3merge')` the copy instead of the package name, and edit it. The copy doesn't get the next Dreamwind's fixes until you merge them in, so diff it against the new file when you update. [Changing a tool](@/docs/guides/load-order-scripts.md#changing-a-tool) has the details.
