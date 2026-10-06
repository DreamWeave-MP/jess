---
title: 'jess save'
description: 'info, check, strip, screenshot, dump.'
weight: 20
extra:
  kind: reference
---

{{ api_signature(value="jess save [--game <GAME>] [--encoding <ENCODING>] <COMMAND>") }}

Inspect, check, and strip saves, by their paths.

| Group option | Values | Default | Does |
| --- | --- | --- | --- |
| `--game <GAME>` | `auto`, `openmw`, `morrowind` | `auto` | Which game wrote the save. `auto` detects it: OpenMW saves have a `SAVE` record, Morrowind saves (`.ess`) have game data in their header. Naming one makes a save of the other kind an error. |
| `--encoding <ENCODING>` | `win1250`, `win1251`, `win1252` | `win1252` | Code page for strings. |

Both go before or after the subcommand: `jess save --game morrowind info Quicksave.ess` and `jess save info --game morrowind Quicksave.ess` are the same. Plus the [global options](@/docs/cli/global-options.md).

| Command | OpenMW `.omwsave` | Morrowind `.ess` |
| --- | --- | --- |
| [`info`](#info) | yes | yes |
| [`check`](#check) | yes | no |
| [`strip`](#strip) | yes | no |
| [`screenshot`](#screenshot) | yes (JPEG) | yes (TGA) |
| [`dump`](#dump) | yes | yes |

`check` and `strip` on a Morrowind save fail with `error: save check isn't supported for Morrowind saves`.

The guide is [Working with Saves](@/docs/guides/saves.md).

## info

{{ api_signature(value="jess save info <INPUT>") }}

The save's metadata: who, where, when, and what it needs.

```sh
jess save info Caius_Met.omwsave
```

```yaml
format: openmw
format_version: 37
description: Caius Met
player:
  name: Ashur
  level: 1
  class_id: Archer
cell: Balmora, Caius Cosades' House
in_game_time:
  game_hour: 8.372071
  day: 18
  month: 7
  year: 427
time_played: 4254.171185921878
dependencies:
- builtin.omwscripts
- Morrowind.esm
- Tribunal.esm
- Bloodmoon.esm
screenshot_bytes: 87035
current_day: 3
current_health: 55.0
maximum_health: 55.0
```

| Field | Meaning |
| --- | --- |
| `format` | `openmw` or `morrowind` |
| `format_version` | OpenMW's save format version. Absent for Morrowind saves. |
| `description` | The save's name, as the load menu shows it. |
| `player` | `name`, `level`, `class_id` |
| `cell` | Where the player is. |
| `in_game_time` | `game_hour`, `day`, `month`, `year` |
| `time_played` | Seconds. OpenMW only. |
| `dependencies` | The content files the save was made with, in order. |
| `screenshot_bytes` | Size of the embedded screenshot. |
| `current_day` | OpenMW only. |
| `current_health`, `maximum_health` | The player's health. |

## check

{{ api_signature(value="jess save check <INPUT>") }}

| Argument | Does |
| --- | --- |
| `<INPUT>` | The OpenMW save to check. |

Checks the save itself: a readable `SAVE` header, a format version OpenMW loads, and saved cells laid out the way the engine reads them. The CLI reads no OpenMW config, so it can't say what the save is missing from your load order; `load_order_checked` is always `false`, and a sound save's verdict is `structure_valid`.

```sh
jess save check Caius_Met.omwsave
```

```yaml
input: Caius_Met.omwsave
verdict: structure_valid
format_version: 40
strippable_format: true
structure_problems: []
load_order_checked: false
missing_content: []
added_content: []
order_changed: false
findings: []
dropped_on_load: {}
warnings: []
```

The comparison against a load order (content it needs that's gone, and records it names that no longer exist and that OpenMW *won't* quietly drop) is [`jess.save.check`](@/docs/api/save.md#jess-save-check) in a script, given the config:

```luau
--!strict
local savePath: string, config: string = ...
local report = jess.save.check(savePath, { config = config })

print(report.verdict)

for _, content in report.missingContent do
  print('missing:', content.name)
end
```

Its report has the fields above, in camelCase, with the load-order half filled in. Here's one from a real save with the expansions uninstalled, spelled the CLI's way:

```yaml
input: Caius_Met.omwsave
verdict: broken_state
format_version: 37
strippable_format: true
structure_problems: []
load_order_checked: true
missing_content:
- name: Tribunal.esm
  strippable: true
- name: Bloodmoon.esm
  strippable: true
added_content: []
order_changed: false
findings:
- severity: broken
  problem: npc_follows_or_escorts_missing_target
  id: BM_werewolf_skaal1A3
  record_index: 234
  record_type: CSTA
  subrecord_index: 522
  subrecord_type: TARG
dropped_on_load:
  cell_ref_base_object: 90
  global_script: 8
  global_variable: 70
  inventory_item_base_object: 170
  saved_cell_state: 6
  spell_state_spell: 16
warnings: []
```

| Field | Meaning |
| --- | --- |
| `verdict` | The overall result; see below. |
| `format_version` | The save's format version. |
| `strippable_format` | Whether `save strip` can edit this format version. |
| `structure_problems` | Damage to the file itself: layout, format version, cell state. |
| `load_order_checked` | Whether the save was compared against a load order: always `false` from the CLI. |
| `missing_content` | Content files the save needs that aren't active. `strippable` is `true` when a strip can remove the file on its own: it's still in your data folders, or everything else the save needs is, so what the file defined can be told apart. It's `false` only when several files are uninstalled; strip them together. |
| `added_content` | Active content files the save wasn't made with. Harmless. |
| `order_changed` | Whether the shared content files are in a different relative order. |
| `findings` | Missing records OpenMW keeps and trips over: `severity` (`will_not_load`, `broken`, `warning`), a `problem` code, the missing `id`, and where in the save it is. |
| `dropped_on_load` | Counts of missing references OpenMW drops on its own when loading, by kind. Not problems, just what you'll lose. |
| `warnings` | Anything else, such as a config that failed to load. |

Verdicts, worst first:

| Verdict | Means | Exit code |
| --- | --- | --- |
| `invalid_structure` | The file is damaged, or its format is one OpenMW won't read. | 1 |
| `will_not_load` | OpenMW will refuse it: a missing birthsign, race, or class. | 1 |
| `broken_state` | Loads, but something is broken: a door that goes nowhere, a follower with no one to follow, a missing head. | 0 |
| `loads_with_warnings` | Loads; content is missing, or something minor is off. | 0 |
| `loads_cleanly` | Nothing missing, nothing broken. | 0 |
| `structure_valid` | The file is fine, and there was no load order to check it against: what the CLI says about a sound save. | 0 |

What each finding means, and the engine code behind it, is in the repository's [`docs/save-strip.md`](https://github.com/DreamWeave-MP/jess/blob/main/docs/save-strip.md).

## strip

{{ api_signature(value="jess save strip [--dry-run] [--output <OUTPUT>] [--no-backup] [--sample-limit <N>] <INPUT> <DEPENDENCIES>...") }}

Removes content files from an OpenMW save: rewrites its dependency list, renumbers references to the content after them, fixes or deletes state that points at the removed content, and refuses when it can't do that safely.

| Argument / option | Default | Does |
| --- | --- | --- |
| `<INPUT>` | | The save to strip. |
| `<DEPENDENCIES>...` | | Content files to remove, by name as the save lists them. At least one. Any order: dependents are stripped before their masters. |
| `--dry-run` | off | Plan and verify the strip in memory; write nothing. Can't be combined with `--output`. |
| `--output <OUTPUT>` | | Write the stripped save here (the path as given) instead of editing `INPUT` in place. |
| `--no-backup` | off | Don't back up the save, or a file already at `--output`, before writing over it. |
| `--sample-limit <N>` | `5` | How many example record ids each part of the plan lists. |

Without `--output` or `--dry-run`, the save is edited in place after a backup (`.bak`, `.bak1`, ...). An existing `--output` file is backed up the same way before it's replaced. `--no-backup` skips both.

To strip a file, jess has to know which of the save's records came from it, and that means reading content files. The CLI reads no OpenMW config, so it can't: every dependency's `target_resolution` is `status: disabled`, and when the save refers to any record that might have come from a removed file, the step is blocked with `target_ref_ids_with_incomplete_prior_provenance`. A strip of a real save almost always is. A save that refers to nothing from the removed files strips fine.

[`jess.save.strip`](@/docs/api/save.md#jess-save-strip) in a script takes the config (`{ config = path }`) and does the whole job. With it, the removed files don't have to be installed. When one is still in your data folders (active or not), strip reads it to learn which records came from it. When it's gone, which is the usual case after uninstalling a mod mid-playthrough, its records are the ones the save refers to that nothing else defines: no remaining content file, and none of the save's own records. That takes every other content file the save needs installed (or being stripped in the same run); otherwise the step is blocked the same way, and the step's `warnings` say the file isn't installed. [Working with Saves](@/docs/guides/saves.md#removing-a-mod-mid-playthrough) has the script.

```sh
jess save strip --dry-run Caius_Met.omwsave Bloodmoon.esm Tribunal.esm
jess save strip --output Vanilla.omwsave Caius_Met.omwsave Bloodmoon.esm Tribunal.esm
```

The report is long (thousands of lines for a real save): one entry in `steps` per removed file, in the order they're stripped, then where it went.

```yaml
input: Caius_Met.omwsave
dependencies:
- Bloodmoon.esm
- Tribunal.esm
steps:
- dependency: Bloodmoon.esm
  mode: output
  dependency_index: 3
  dependencies_before: 4
  dependencies_after: 3
  save_format: { ... }
  target_resolution:
    status: resolved
    path: /home/you/Games/Morrowind/Data Files/Bloodmoon.esm
  target_records: { total: 10449, by_type: { ... }, samples: [ ... ] }
  dependents: []
  save_references: { total: 143, ... }
  blockers:
    status: no_known_blockers
  planned_operations: [ ... ]
  mutation_readiness: { ... }
  dry_run: { ... }
  mutation: written
  warnings: []
- dependency: Tribunal.esm
  # ...
written:
  path: Vanilla.omwsave
```

The parts worth reading:

| Field | Meaning |
| --- | --- |
| `steps[].blockers.status` | `no_known_blockers`, or `blocked` with the reasons under `hard` (each with a `kind`, `count`, and suggested `action`). Every reason a step can't go ahead is listed there, including `save_format_unsupported` and `unclassified_lua_data`. |
| `steps[].mutation_readiness` | Whether the step can go ahead, and if not, the `reason` (`hard_blockers_present`, `blocked_planned_edits_present`, `save_format_unsupported_for_mutation`, ...). |
| `steps[].dependents` | Other content in the save that needs this file. Stripping a master without its dependents is blocked. |
| `steps[].planned_operations` | Every edit, with the record and field it touches and why. |
| `steps[].dry_run` | The in-memory result: `status: verified` means the edited save was re-read and nothing pointing at the removed content remains. |
| `steps[].mutation` | `written`, or `dry_run_only`. |
| `written` | `{path, backup}`: where the save was written, and the backup of the file it replaced. `null` on a dry run or a blocked strip. |

A blocked strip writes nothing, prints its plan (so you can see why), and exits `1` with `error: nothing was written`. With `--dry-run` a blocked plan still exits `0`: the dry run did its job.

```sh
jess save strip --dry-run Caius_Met.omwsave Morrowind.esm
```

```yaml
  blockers:
    status: blocked
    hard:
    - kind: dependents_require_removed_master
      count: 2
      action: remove_the_dependent_content_too_or_keep_this_dependency
    # ...
```

Strip edits save formats 37 through 40 (check's `strippable_format` tells you up front). What it changes and why: [`docs/save-strip.md`](https://github.com/DreamWeave-MP/jess/blob/main/docs/save-strip.md).

## screenshot

{{ api_signature(value="jess save screenshot [-o, --output <OUTPUT>] [--no-backup] <INPUT>") }}

| Argument / option | Does |
| --- | --- |
| `<INPUT>` | The save. |
| `-o`, `--output <OUTPUT>` | Where to write the image. Without it, next to the save, named after it: `Caius_Met.jpg`, `Quicksave.tga`. An existing file there is backed up first (`.bak`, `.bak1`, ...). The output can also follow the input, as in `jess save screenshot Caius_Met.omwsave caius.jpg`. |
| `--no-backup` | Replace an existing file without backing it up. |

Writes the save's screenshot: the JPEG OpenMW stores, or, for a Morrowind save, a TGA made from its raw 128×128 pixels. Name the output to match.

```sh
jess save screenshot Caius_Met.omwsave --output caius.jpg
jess save screenshot Quicksave.ess                         # Quicksave.tga
```

```yaml
input: Caius_Met.omwsave
format: jpeg
bytes: 87035
written:
  path: caius.jpg
```

`format` is `jpeg` or `tga`. `written` has a `backup` too when the output replaced a file.

## dump

{{ api_signature(value="jess save dump [SELECTORS] [--list] [--raw] [--raw-output <PATH> [--header] [--no-backup]] <INPUT>") }}

Dumps the save's records readably, including decoded Lua data, or their byte layout with `--raw`. It takes the same options as [`plugin dump`](@/docs/cli/plugin.md#dump): the [selectors](@/docs/cli/plugin.md#selectors), `--list`, `--raw`, `--raw-output`, `--header`, and `--no-backup`.

```sh
jess save dump --list Caius_Met.omwsave
jess save dump --type PLAY Caius_Met.omwsave
jess save dump --raw --type GLOB Caius_Met.omwsave
```

```yaml
records:
- record: 335
  type: PLAY
  subrecords:
  - FRMR: 1 (generated)
  - NAME: player
  - DATA:
      position: [0.0, 0.0, 0.0]
      rotation: [0.0, 0.0, 0.0]
  - LUAS: 0 (scripts/omw/mwui/init.lua)
  - LUAD: {}
  - LUAS: 13 (scripts/omw/input/playercontrols.lua)
  - LUAD:
      sneaking: false
  # ...
```

Save records are shown subrecord by subrecord, with values decoded where jess knows them. `--list` gives `record`, `type`, and `id` for records that have one (OpenMW's typed ids spelled the way OpenMW spells them, like `#-5 4` for an exterior cell). Record types are four-letter tags as the save spells them: `PLAY`, `CSTA`, `GLOB`, `JOUR`, `NPC_`, and so on for OpenMW; `CREC`, `NPCC`, `QUES`, and friends for Morrowind.

`--raw` prints the byte layout as JSON unless `--format` says otherwise. `--raw-output <PATH>` writes the matching records to a new file instead, with `--header` adding a `TES3` header; a file already at that path is backed up first unless `--no-backup`.
