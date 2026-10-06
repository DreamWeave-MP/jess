---
title: 'Merged Lands'
description: 'Merged Lands as a Dreamwind module, @dreamwind/tools/merged-lands: LAND and LTEX merged, seams closed, with patch files.'
weight: 25
extra:
  kind: reference
---

`@dreamwind/tools/merged-lands` is a Luau port of [Merged Lands](https://github.com/Sophialovesmorrowind/OpenMWMergedLands) 0.2.0, David Von Derau's terrain merger, in its OpenMW mode. It walks every LAND record in your load order, merges each cell's heights, normals, vertex colors, textures, and world map, closes the seams between cells, and writes whatever differs from what OpenMW would load anyway into one plugin, `Merged Lands.omwaddon`, with the LTEX records those cells use.

The plugin is the original's byte for byte, apart from the time of the run in its header. On a {{ bench(workload="mergedlands", value="plugins") }}-plugin load order with {{ bench(workload="mergedlands", value="lands") }} LAND records it's {{ bench(workload="mergedlands", value="faster") }} times faster and uses {{ bench(workload="mergedlands", value="less_memory") }} times less memory.

## Run it

```luau
--!strict
-- lands.luau <openmw.cfg>
local config: string = ...
local openmw = jess.openmw.load(config)
local mergedLands = require '@dreamwind/tools/merged-lands'

mergedLands({}, openmw)
```

```sh
$ jess run --package jess/jpk/dreamwind lands.luau ~/.config/openmw/openmw.cfg
merged-lands: 2144 LAND and 259 LTEX records from 51 plugins in /home/you/.local/share/openmw/data/Merged Lands.omwaddon
```

Then enable `Merged Lands.omwaddon` at the very end of your load order, after `Merged Objects.esp` if you run [TES3Merge](@/docs/bundled-scripts/tes3merge.md), or set `activate` and let the tool do it. Rerun after every change to your mod list. The plugin carries whole LAND records, so a stale one quietly puts old terrain back over whatever you installed since.

## What it does

**Which plugins.** Every `.esm`, `.esp`, `.omwaddon`, and `.omwgame` in `content=` order, except earlier outputs (`Merged Lands.omwaddon`, `Merged Lands.esp`, and `outputFile`) and anything `ignorePlugins` or `ignorePluginsFromPath` names. Only LAND and LTEX records are read. A plugin that won't open is skipped with a message. A plugin whose master is missing or loads after it stops the run, as it stops the original: OpenMW won't load that content list either.

**Textures.** LAND records don't name textures. They number them, by the LTEX records of their own plugin, so texture 3 in one plugin and texture 3 in another can be different things. Every LTEX is collected by id, in the order the load order first defines each one, and every LAND's texture indices are renumbered to that list as it's read. An index its plugin doesn't define becomes one of that plugin's own textures, the one the load order defined first, rather than no texture at all.

**Each cell**, in load order, one LAND at a time. Each LAND is measured against the terrain loaded before it, part by part: the vertices whose height, normal, or color differs, the texture squares that differ, the world map values that differ. Those win. A part a LAND doesn't have, or doesn't flag, leaves the earlier one in place, where OpenMW would drop it along with the rest of the record. A normal only counts where the height under it changed too.

This is load-order winner semantics, vertex by vertex. A later plugin made against the masters alone still undoes an earlier plugin's edits to the same cell, because measured against that earlier plugin, the master's heights *are* a change. Merging two mods' hills into one cell takes a patch file that says how.

**Seams.** Two cells share every vertex along the edge between them, and four share each corner. When different plugins win on either side, those vertices disagree and the ground tears. Every corner becomes the average of the cells there, then every other edge vertex the midpoint of its two cells, both rounded toward zero. This happens after every plugin has been merged, since a later plugin often fixes a tear an earlier one made.

**The output.** A merged cell is written only if it now differs from what OpenMW would load without it. Heights are re-encoded, with any step too steep for a byte clamped, and normals are recomputed from the heights wherever the height changed. Only the LTEX records written cells use are kept, renumbered in order, each with the last file name the load order gives it. The masters are the plugins those textures and cells come from, `.esm` files first, then oldest first by modification date, as the original writes them. The author is `Merged Lands by DVD`.

The old plugin in the output folder is deleted before the new one is written, along with a stale `Merged Lands.mergedlands.toml` beside it.

{% usage_note(title="Ground at exactly 0") %}
The first LAND in each cell is measured against nothing, which is to say against zero. A vertex whose height is exactly 0 therefore reads as unchanged, its normal is dropped, and the cell no longer matches what OpenMW loads. So a cell only one plugin touches is still written when it has ground at height 0 anywhere, with normals recomputed from its heights. The original does this, so the port does too.
{% end %}

## Patch files

A plugin's `<name>.mergedlands.toml`, found in the data folders by the plugin's name without its extension (`Cantons_on_the_Global_Map_v1.1.mergedlands.toml`), controls how its terrain merges. Nothing needs one until it's known to be necessary.

```toml
version = "0"
meta_type = "Patch"

[height_map]
conflict_strategy = "Overwrite"

[vertex_colors]
included = false

[texture_indices]
conflict_strategy = "Ignore"
```

| Key | Values |
| --- | --- |
| `version` | `"0"`. Any other string is reported as unsupported, and the file is ignored. |
| `meta_type` | `"Patch"` or `"Auto"`; `"MergedLands"` marks a plugin as one of the original's outputs, which is skipped entirely. |
| `[height_map]`, `[vertex_colors]`, `[texture_indices]`, `[world_map_data]` | Each takes `included` (default `true`) and `conflict_strategy` (default `"Auto"`). `height_map` covers normals too. |

`included = false` leaves that part of the plugin out of the merge, and out of what later plugins are measured against. The strategies decide what happens where this plugin and the merge so far both changed a vertex:

| Strategy | Where both changed it |
| --- | --- |
| `Auto` | This plugin wins. The same as `Overwrite`. |
| `Overwrite` | This plugin wins. |
| `Ignore` | The merge so far wins; this plugin only fills in what nothing changed yet. |
| `Resolve` | A weighted average of the two values, leaning toward whichever is farther from zero. For textures that averages two texture numbers, which is exactly as meaningful as it sounds. |

A file the original couldn't read (a missing `meta_type`, a strategy it doesn't know, TOML that doesn't parse) is ignored, as it is by the original.

## merged_lands.toml

The original keeps its settings in `merged_lands.toml`, in the OpenMW config folder. The port reads `merged_lands.toml` beside your `openmw.cfg`, the last one in the chain, when there is one:

| Key | Meaning |
| --- | --- |
| `output_file_dir` | Where to write the plugin, relative to the file's folder. |
| `ignore_plugins` | Plugin names to leave out, ignoring case. |
| `ignore_plugins_from_path` | Folders whose plugins to leave out, relative to the file's folder. |
| `generated_output_dir`, `generated_output_files` | Earlier outputs the original recorded; any still in the output folder are left out. |

Without the file, `ignore_plugins` is the original's first-run list: `delta-merged.omwaddon`, `deleted_groundcover.omwaddon`, `S3LightFixes.omwaddon`, `OMWLLFMod.omwaddon`, `merged.omwaddon`, and `Merged Objects.esp`. The port never writes the file.

## Options from Luau

Options go in `state.mergedlands`, in the table the tool is called with:

```luau
local state = {}

state.mergedlands = {
  activate = true,
  ignorePlugins = { 'Merged Objects.esp', 'S3LightFixes.omwaddon', 'Some Terrain Test.esp' },
}
```

| Option | Type | Original's flag or setting |
| --- | --- | --- |
| `outputFile` | `string` | `--output-file`. Default `Merged Lands.omwaddon`. |
| `outputDir` | `string` | `--output-file-dir`, else `output_file_dir`, else `data-local`, else the first `data=` folder. Created if it's missing. |
| `plugins` | `{ string }` | The plugin list on the command line: these, by name, instead of `content=`. |
| `sortOrder` | `"default"` or `"none"` | `--sort-order`. `"default"` sorts `plugins` and the masters `.esm` first, then by date; `"none"` keeps `plugins` as listed and the masters in load order. |
| `ignorePlugins` | `{ string }` | `ignore_plugins`, replacing the file's. |
| `ignorePluginsFromPath` | `{ string }` | `ignore_plugins_from_path`, replacing the file's. |
| `activate` | `boolean` | None. Enables the plugin in `openmw.cfg`, backing the config up first, when it's in a data folder; elsewhere OpenMW couldn't load it, so it's left disabled with a message. |

The original's `--openmw-cfg` is the config your script hands `jess.openmw.load`. `--vanilla`, `--data-files-dir`, and Morrowind.ini have no equivalent: this is the OpenMW mode, and the only one.

## Report

Afterwards `state.mergedlands.report` holds what happened, for whatever runs next:

| Field | Type | What |
| --- | --- | --- |
| `output` | `string` | The plugin written. |
| `plugins` | `number` | Plugins read. |
| `lands` | `number` | LAND records read, one per plugin and cell. |
| `merged` | `number` | Cells with terrain anyone changed. |
| `textures` | `number` | LTEX records written. |
| `seams` | `number` | Edge vertices seam repair moved. |
| `masters` | `{ string }` | The plugin's masters, in order. |
| `activated` | `boolean` | Whether `activate` enabled the plugin this run. |

## Differences from the original

The plugin matches Merged Lands 0.2.0's byte for byte, apart from the timestamp in its description. The run around it differs:

- **No conflict images.** The original draws PNGs of every merge into a `Conflicts` folder and writes `merged_lands.log`. The port does neither, and `--add-debug-vertex-colors` isn't ported.
- **`merged_lands.toml` is read, not written.** The original creates it on the first run, asks where `openmw.cfg` is if it can't tell, and records its outputs in it. The port reads it from beside your `openmw.cfg` and leaves it alone.
- **Errors instead of panics.** Where the original crashes (a deleted LAND after another in the same cell, two LANDs for a cell with different record flags, a deleted LTEX with a new id), the port stops with a message naming the plugin and cell.

## Performance

On a {{ bench(value="content_files") }}-entry content list: {{ bench(workload="mergedlands", value="plugins") }} plugins, {{ bench(workload="mergedlands", value="lands") }} LAND records, and {{ bench(workload="mergedlands", value="merged") }} cells with terrain some plugin changed, producing {{ bench(workload="mergedlands", value="written_LAND") }} LAND and {{ bench(workload="mergedlands", value="written_LTEX") }} LTEX records. Median of {{ bench(value="runs") }} runs:

| | Time | Peak memory |
| --- | --- | --- |
| `@dreamwind/tools/merged-lands` | {{ bench(workload="mergedlands", value="jess_time") }} | {{ bench(workload="mergedlands", value="jess_memory") }} |
| Merged Lands (native) | {{ bench(workload="mergedlands", value="native_time") }} | {{ bench(workload="mergedlands", value="native_memory") }} |

{{ bench(workload="mergedlands", value="faster") }} times faster, with {{ bench(workload="mergedlands", value="less_memory") }} times less memory. Most of the port's memory is the plugins themselves: opened with `types = { 'LAND', 'LTEX' }`, nothing else is read, and each LAND's parts are copied out into buffers with [`jess.land.read`](@/docs/api/land.md) only while its cell is being merged. Cells are merged one at a time and forgotten, keeping only their edges for seam repair, and the ones that get written are merged again at the end.

## Source

The script is [`jpk/dreamwind/src/tools/merged-lands/init.luau`](https://github.com/DreamWeave-MP/jess/blob/main/jpk/dreamwind/src/tools/merged-lands/init.luau), the [Dreamwind](@/docs/scripting/packages.md#dreamwind) module `@dreamwind/tools/merged-lands`, ported from Merged Lands (MIT, © 2022 David Von Derau). Its normals are computed in the original's single-precision arithmetic, rounded where it rounds, which is why the outputs match exactly.

To change what it does beyond its options, copy it beside your script and require the copy: [Changing a tool](@/docs/guides/load-order-scripts.md#changing-a-tool).
