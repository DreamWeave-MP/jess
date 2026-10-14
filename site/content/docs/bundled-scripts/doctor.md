---
title: 'Doctor'
description: 'A load-order health check as a Dreamwind module, @dreamwind/tools/doctor: missing masters and content, missing assets, models OpenMW can''t load, dirty plugins, and changes a later plugin throws away, each with a fix.'
weight: 30
extra:
  kind: reference
---

`@dreamwind/tools/doctor` answers the question every OpenMW player eventually asks, usually at 2 AM: *why is my game broken?* It reads your load order, checks it the ways jess knows how, and writes down everything it finds, worst first, with the command or edit that fixes each one.

It doesn't fix anything itself. It never writes a plugin or `openmw.cfg`. The only file it writes is its report.

## Run it

Like the other [load-order tools](@/docs/bundled-scripts/_index.md), it's a function of `(state, openmw)`, called from a script that loaded your config:

```luau
--!strict
-- doctor.jsl <openmw.cfg>: check a load order, change nothing.
local config: string = ...
local openmw = jess.openmw.load(config)
local doctor = require '@dreamwind/tools/doctor'

doctor({}, openmw)
```

On the three vanilla masters:

```text
$ jess run --package jess/jpk/dreamwind doctor.jsl ~/.config/openmw/openmw.cfg
doctor: 0 errors, 14 warnings, 15 info in 3 content files
  by check: conflict 1, missingAsset 10, mesh 16, masterSize 2
  warning conflict: Bloodmoon.esm replaces 4 records Tribunal.esm changed, dropping its changes (2 of them @dreamwind/tools/tes3merge can merge)
    fix: merge them with @dreamwind/tools/tes3merge, and load Merged Objects.esp last; patch the 2 records it can't merge in a plugin loading after Bloodmoon.esm, or drop one of the two
  warning missingAsset: Bloodmoon.esm refers to 1 file nothing provides (missingSound)
    fix: install the mod that provides them, or fix the paths in Bloodmoon.esm
  warning missingAsset: Bloodmoon.esm refers to 2 files nothing provides (missingMesh)
    fix: install the mod that provides them, or fix the paths in Bloodmoon.esm
  ...
  ... and 19 more in the report
report: /home/you/.config/openmw/jess-doctor.yaml (the last one is in /home/you/.config/openmw/jess-doctor.yaml.bak)
```

Yes, Bethesda's own masters show up. Bloodmoon really does overwrite Tribunal's `SignRotate` script, and Morrowind.esm really does name meshes that never shipped. The doctor doesn't grade on a curve.

A load order with real problems gets errors, and they come first:

```text
doctor: 3 errors, 15 warnings, 15 info in 6 content files
  error missingContent: Gone.esp is in the load order, but no data folder has it
    fix: install the mod that provides Gone.esp, or remove content=Gone.esp from openmw.cfg
  error missingMaster: Orphan.esp needs Tamriel_Data.esm, which isn't in the load order
    fix: install Tamriel_Data.esm, or remove content=Orphan.esp from openmw.cfg
  error masterOrder: Early Patch.esp loads before its master Bloodmoon.esm
    fix: move content=Bloodmoon.esm above content=Early Patch.esp in openmw.cfg
  ...
```

It prints the first ten errors and warnings. Everything, info included, goes in the report.

Run it after the tools that write plugins, with the same `state`, and it knows what they did:

```luau
local state = {}

tes3merge(state, openmw)
doctor(state, openmw)
```

## The report

The report goes to `jess-doctor.yaml` in your user OpenMW config folder: the folder of the last `openmw.cfg` in the chain, the user's own, where the launcher and OpenMW's own logs live. A report already there is backed up to `jess-doctor.yaml.bak` (`.bak1`, ...) first, so the last run is always around to compare against.

Keys come out in a fixed order and findings are sorted, so two runs' reports diff cleanly: fix something, run it again, and `diff` shows exactly what went away.

```yaml
config: /home/you/.config/openmw/openmw.cfg
contentFiles: 3
plugins: 3
checks:
- masters
- content
- assets
- meshes
- dirt
- conflicts
summary:
  errors: 0
  warnings: 14
  info: 15
  byCheck:
    conflict: 1
    missingAsset: 10
    mesh: 16
    masterSize: 2
findings:
- severity: warning
  check: conflict
  plugins:
  - Tribunal.esm
  - Bloodmoon.esm
  message: "Bloodmoon.esm replaces 4 records Tribunal.esm changed, dropping its changes (2 of them @dreamwind/tools/tes3merge can merge)"
  fix: "merge them with @dreamwind/tools/tes3merge, and load Merged Objects.esp last; patch the 2 records it can't merge in a plugin loading after Bloodmoon.esm, or drop one of the two"
  records:
  - GMST sLoadingMessage5
  - SCPT Float
  - SCPT SignRotate
  - ARMO steel_cuirass
# ...
- severity: warning
  check: mesh
  plugins: []
  message: "meshes/c/c_m_shirt_c_common03.nif: texture \"TX_C_C_shirtcom03.bmp\" not found (tried textures/tx_c_c_shirtcom03.dds, textures/tx_c_c_shirtcom03.bmp)"
  fix: "`jess nif check` on the file for the details; `jess nif fix` repairs some"
  records:
  - BODY c_m_shirt_c_common03
  files:
  - meshes/c/c_m_shirt_c_common03.nif
  details:
  - "missingTexture: texture \"TX_C_C_shirtcom03.bmp\" not found (tried textures/tx_c_c_shirtcom03.dds, textures/tx_c_c_shirtcom03.bmp)"
```

| Key | What |
| --- | --- |
| `config` | The user `openmw.cfg`. |
| `contentFiles` | Every `content=` line, found or not. |
| `plugins` | Plugins read: content files that exist and parse, `.omwscripts` left out. |
| `checks` | The checks that ran. |
| `summary` | `errors`, `warnings`, and `info` counts, and `byCheck`, findings per check. |
| `findings` | Every finding, errors first, then warnings, then info. |

Each finding has:

| Key | What |
| --- | --- |
| `severity` | `error`: the game won't load this, or will load it wrong in ways you'll notice. `warning`: something is lost or missing and you probably want it back. `info`: worth knowing, often on purpose. |
| `check` | What kind of finding it is; the table below lists them. |
| `plugins` | The plugins involved: the one with the problem first, and for a conflict, the loser and then the winner. Empty for a model. |
| `message` | What's wrong, in words. |
| `fix` | What to do about it: a command to run, or the `openmw.cfg` edit to make. |
| `records` | The records involved, as `TYPE id`, when there are any. |
| `files` | The files involved (VFS paths as the record spells them, or the plugin that won't parse), when there are any. |
| `details` | The individual findings behind a grouped one: lint messages, or a model's NIF check findings. |

`format = 'json'` writes the same thing as JSON, to `jess-doctor.json`.

## What it checks

Six checks, all on by default. `checks` picks some.

| Check | Finding | Severity | Fix it suggests |
| --- | --- | --- | --- |
| `content` | `missingContent`: a `content=` file no data folder has. | error | Install the mod that provides it, or remove its `content=` line. |
| `content` | `unreadablePlugin`: a content file that doesn't parse. | error | Reinstall the mod that provides it, or remove its `content=` line. |
| `masters` | `missingMaster`: a plugin needs a master that isn't in the load order. | error | Add `content=<master>` above the plugin if it's installed; otherwise install it, or remove the plugin's `content=` line. |
| `masters` | `masterOrder`: a plugin loads before one of its masters. | error | Move the master's `content=` line above the plugin's. |
| `masters` | `masterSize`: a plugin records a master's size wrong. OpenMW doesn't care; Morrowind.exe and the Construction Set do. | info | `` `jess plugin header "<plugin path>" --update-masters` ``. |
| `assets` | `missingAsset`: models, textures, icons, sounds, voices, or music a plugin names that nothing provides, one finding per plugin and kind. | warning | Install what provides them, or fix the paths. |
| `assets` | `badRecord`: records OpenMW reads wrong, such as broken book image tags. | warning | Fix the records. |
| `meshes` | `mesh`: a model a plugin uses that OpenMW won't load (error), that's missing textures or loses its collision (warning), or that collides or draws differently than you'd guess (info). | varies | `jess nif check` on the file; `jess nif fix` repairs some. |
| `dirt` | `evilGmsts`, `junkCells`, `duplicateRecords`: the dirt `jess plugin clean` removes. | warning | `` `jess plugin clean "<plugin path>" --evil-gmsts` `` (or `--junk-cells`, `--duplicate-records`). |
| `dirt` | `cellParams`: cell settings repeated in one cell. | info | `` `jess plugin clean "<plugin path>" --cell-params` ``. |
| `dirt` | `identicalToMaster`: records copied unchanged from the plugin's own masters. | info | `` `jess plugin clean "<plugin path>" --duplicate-records` ``. |
| `conflicts` | `conflict`: a later plugin's copy of records throws away an earlier plugin's changes. | warning, or info | [Below](#conflicts). |
| `conflicts` | `tes3merge`: TES3Merge ran earlier in the same `state`, but its output isn't in the load order. | warning | Add `content=Merged Objects.esp` last, or run the merge with `activate = true`. |

### Missing content and masters

Straight from `openmw.cfg` and each plugin's header. A master counts as present when a `content=` line names it, ignoring case, as OpenMW matches names. When it's missing, the doctor looks for it in your data folders to decide between "add its `content=` line" and "install it".

### Assets and models

This is [`jess.plugin.assets`](@/docs/api/plugin.md#jess-plugin-assets) over the load order: every file the winning copy of each record names, looked up the way OpenMW looks it up, and the [NIF check](@/docs/guides/nif-checking-and-fixing.md) of every model those records use. Only models the load order actually uses are checked, not every NIF in every data folder.

The NIF check reports a lot that isn't a problem. Most models in most load orders have an invisible shadow box that collides, or a controller OpenMW ignores, and they looked the same in Morrowind. So the doctor sorts a model's findings: a model that won't load is an error; missing textures and lost collision (a lowercase `ncc`, a shape OpenMW gives no collision) are warnings; the rest is info. Performance findings are left out. `jess nif check` on the model has all of them.

### Dirt

`plugin:lint` with the checks `jess plugin clean` has cleanups for, on every plugin but Morrowind.esm, Tribunal.esm, and Bloodmoon.esm. Bethesda's masters carry what looks like dirt on purpose: Bloodmoon ships Tribunal's settings at Tribunal's values, which is exactly what an evil GMST is anywhere else. Nobody should clean them, so they're not reported.

Records identical to a master's come from the conflict check's comparison: a plugin's copy of a record, byte for byte the same as the copy in one of its own masters.

### Conflicts

When several plugins define the same record, the game takes the last one's copy, whole. Everything an earlier plugin changed in that record is gone, and nothing tells you.

The doctor compares every copy of every record across the load order with [`jess.compare.overrides`](@/docs/api/compare.md#jess-compare-overrides). A plugin's changes count as lost when its copy differs from every earlier copy (so it changed something) and from the last copy (so the winner didn't keep it). Two cases don't count:

- **Patches.** When the winner lists the losing plugin among its masters, its author loaded both and chose. A patch doesn't get reported for doing its job.
- **Records that don't replace.** Cells, dialogue, and path grids merge in the engine or aren't identified by id alone, landscape has no id, and each plugin keeps its own land textures. They're not compared.

Findings are grouped per pair of plugins: "B replaces 21 records A changed". The records are listed in the report.

What to do depends on the record types:

- [TES3Merge](@/docs/bundled-scripts/tes3merge.md) merges the 32 types it knows field by field, keeping every plugin's changes. The doctor counts it as in use when `Merged Objects.esp` (or its `fileName`) is in the load order, or when it ran earlier with the same `state`. When it isn't in use, the fix says to run it. When it is, and it can merge every record in the finding, the finding is info: handled, as long as you rerun it after changing the load order.
- Everything else, scripts, factions, globals, regions, needs a patch plugin that loads after both and carries the changes you want, or one of the two plugins gone.

Only bytes are compared. A winner that carries a loser's changes without listing it as a master (a merged plugin from another tool, say) still shows up, and a record that differs only in a field nobody cares about still counts as a change. The doctor tells you where to look; it doesn't read the mods' minds.

## Options

Set them in `state.doctor`, in the table the doctor is called with:

```luau
local state = {}

state.doctor = {
  checks = { 'masters', 'content', 'conflicts' },
  minSeverity = 'warning',
  format = 'json',
}
```

| Option | Type | Default | What |
| --- | --- | --- | --- |
| `checks` | `{ string }` | all six | Which checks run: `masters`, `content`, `assets`, `meshes`, `dirt`, `conflicts`. An unknown name stops the run. |
| `output` | `string` | `jess-doctor.<format>` in the user config folder | Where the report goes. |
| `format` | `'yaml' \| 'json'` | `'yaml'` | The report's format. |
| `minSeverity` | `'error' \| 'warning' \| 'info'` | `'info'` | Leave out findings milder than this, from the report and the summary. |
| `verbose` | `boolean` | `false` | Print every finding, info included, instead of the first ten errors and warnings. |

When TES3Merge runs with a different `fileName`, the doctor reads it from `state.tes3merge`, so call both with the same `state`.

## After it runs

`state.doctor.report` holds the report, as the file has it, plus `output`, the path it was written to, and `backup`, where the previous one went (`nil` when there wasn't one). The driver can act on the findings:

```luau
local config: string = ...
local openmw = jess.openmw.load(config)
local doctor = require '@dreamwind/tools/doctor'
local state = { doctor = { minSeverity = 'error' } }

doctor(state, openmw)

local report = state.doctor.report
if report.summary.errors > 0 then
  error(`the load order has {report.summary.errors} errors; see {report.output}`, 0)
end
```

## Performance

On a {{ bench(value="plugins") }}-plugin, {{ bench(value="data_dirs") }}-data-folder load order, median of {{ bench(value="runs") }} runs: {{ bench(workload="doctor", value="jess_time") }}, {{ bench(workload="doctor", value="jess_memory") }} peak memory. The asset and model check is most of the time: {{ bench(workload="doctor-assets", value="jess_time") }} and {{ bench(workload="doctor-assets", value="jess_memory") }} on its own, for {{ bench(value="plugin_size") }} of plugins and every model they use, read and checked on one thread. Without `assets` and `meshes`, the rest takes {{ bench(workload="doctor-rest", value="jess_time") }} and {{ bench(workload="doctor-rest", value="jess_memory") }}. Most of the memory is the plugins themselves: the dirt and conflict checks keep every plugin open, and the dirt check reads cells whole, since it needs their references to tell a junk cell from a real one.

## Source

The tool is [`jpk/dreamwind/src/tools/doctor/init.jsl`](https://github.com/DreamWeave-MP/jess/blob/main/jpk/dreamwind/src/tools/doctor/init.jsl), the [Dreamwind](@/docs/scripting/packages.md#dreamwind) module `@dreamwind/tools/doctor`: plain Luau against the public API, like the other tools. The comparison it rests on, `jess.compare.overrides`, is in Rust, in `omwaddon::compare`. To change what it does beyond its options, copy it beside your script and require the copy: [Changing a tool](@/docs/guides/load-order-scripts.md#changing-a-tool).
