---
title: 'Working with Saves'
description: 'Check a save against a load order, strip mods out of it, and read .ess saves.'
weight: 80
extra:
  kind: guide
---

Goal: know whether a save will load before you load it, and take a mod out of a playthrough without wrecking the playthrough.

Saves are always given by path. They're under OpenMW's user data folder: `~/.local/share/openmw/saves/<character>/` on Linux, `Documents\My Games\OpenMW\saves\<character>\` on Windows, `~/Library/Application Support/openmw/saves/<character>/` on macOS.

## What's in a save

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
# ...
dependencies:
- builtin.omwscripts
- Morrowind.esm
- Tribunal.esm
- Bloodmoon.esm
```

`dependencies` is the load order the save was made with. That list is what the rest of this page is about.

## Will it load?

```sh
jess save check Caius_Met.omwsave
```

From the command line, `check` looks at the file alone: a readable header, a format version OpenMW loads, saved cells laid out the way the engine reads them. A sound file gets `verdict: structure_valid` and `load_order_checked: false`. That catches damage, and nothing else, because the question you actually have is about your load order, and the CLI doesn't read one.

Comparing the save with your *current* load order takes a script that loads the config. [`jess.save.check`](@/docs/api/save.md#jess-save-check) does the work:

```luau
--!strict
-- check-save.jsl <save> <openmw.cfg>: does a save load with this load order?
local savePath: string, config: string = ...
local report = jess.save.check(savePath, { config = config })

print(`{report.verdict}, {#report.findings} findings`)

for _, content in report.missingContent do
  print(`missing {content.name}{if content.strippable then ', strippable' else ''}`)
end

for _, finding in report.findings do
  print(`{finding.severity} {finding.problem}: {finding.id}`)
end
```

```sh
jess run check-save.jsl Caius_Met.omwsave ~/.config/openmw/openmw.cfg
```

It looks up every record the save refers to. Say you've just turned off Tribunal and Bloodmoon. The whole report, as YAML (the script gets the same fields in camelCase):

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
# ...
dropped_on_load:
  cell_ref_base_object: 90
  global_script: 8
  global_variable: 70
  inventory_item_base_object: 170
  saved_cell_state: 6
  spell_state_spell: 16
warnings: []
```

Read it top down:

- **`verdict`**: the answer. From worst to best: `invalid_structure` (the file's damaged), `will_not_load` (OpenMW will refuse it: missing birthsign, race, or class), `broken_state` (loads, but something's wrong), `loads_with_warnings`, `loads_cleanly`. `structure_valid` means there was no load order to check against, only the file.
- **`missing_content`**: what the save needs that's gone. `strippable: true` means it can be stripped out properly (below): the file's still on disk, or the rest of the save's content is, so jess can tell what came from it. It's `false` only when several files are uninstalled; strip those together.
- **`findings`**: the records OpenMW *won't* quietly cope with. Here an NPC in a saved cell is following Bloodmoon's `BM_werewolf_skaal1A3`, who no longer exists; OpenMW keeps the package and the NPC stands around forever.
- **`dropped_on_load`**: what OpenMW *will* quietly drop when it loads the save: 170 inventory items and 90 placed objects from the missing plugins, gone. Not errors, just a preview of your losses.

`jess save check` exits `1` only for `invalid_structure` and `will_not_load`, so a launcher script can refuse to start the game on a save that's damaged.

## Removing a mod mid-playthrough

Turning a plugin off and loading your save *works*, in the sense that OpenMW starts. It then drops what it can, keeps what it can't, and you find out later: the door that goes nowhere, the follower who stopped following, the lock you can never open. Stripping the mod from the save first fixes or removes those, or tells you why it can't.

Stripping needs to know which of the save's records came from the mod, and that means reading the content files the save keeps. `jess save strip` on the command line has no config to find them through, so on a real save it stops at `target_ref_ids_with_incomplete_prior_provenance` and writes nothing. [`jess.save.strip`](@/docs/api/save.md#jess-save-strip) takes the config. Keep this script beside the check script:

```luau
--!strict
-- strip-save.jsl <save> <openmw.cfg> <output | --dry-run> <content file>...
local savePath: string, config: string, output: string = ...
local dependencies = { select(4, ...) }
local dryRun = output == '--dry-run'

local report = jess.save.strip(savePath, dependencies, {
  config = config,
  dryRun = dryRun,
  output = if dryRun then nil else output,
})

for _, step in report.steps do
  print(`{step.dependency}: {step.blockers.status}, dry run {step.dryRun.status}`)

  for _, concern in step.blockers.hard do
    print(`  {concern.kind} ({concern.count}): {concern.action}`)
  end
end

local written = report.written
if written then print(`wrote {written.path}`) end
```

The whole workflow:

**1. Keep your config pointing at the rest of your load order.** The mod itself can be installed or already gone:

- **Still installed** (active or not), strip reads it and knows exactly which records came from it.
- **Already uninstalled**, strip takes its records to be the ones the save refers to that nothing else defines, the same ids the check reports missing. Each step's `warnings` says so. That needs every other content file the save uses to be installed, or stripped in the same run; otherwise the step is blocked with `target_ref_ids_with_incomplete_prior_provenance`.

**2. Dry run.**

```sh
jess run strip-save.jsl Caius_Met.omwsave ~/.config/openmw/openmw.cfg --dry-run Bloodmoon.esm Tribunal.esm
```

```text
Bloodmoon.esm: noKnownBlockers, dry run verified
Tribunal.esm: noKnownBlockers, dry run verified
```

List as many content files as you're removing, in any order; jess strips dependents before their masters. `verified` means the stripped save was built in memory, re-read, and nothing pointing at the removed content is left. Each step carries a long plan (every edit with its reason); [StripReport](@/docs/api/save.md#stripreport) has the fields.

**3. If it's blocked, read why.**

```sh
jess run strip-save.jsl Caius_Met.omwsave ~/.config/openmw/openmw.cfg --dry-run Morrowind.esm
```

```text
Morrowind.esm: blocked, dry run blocked
  dependentsRequireRemovedMaster (2): removeTheDependentContentTooOrKeepThisDependency
  targetIntroducedRefIds (7): deleteOrRewriteSaveStateThatDependsOnTargetOnlyRecords
```

Each blocker comes with the action that would clear it. Here, Tribunal and Bloodmoon depend on Morrowind, and the save has state that only Morrowind defines (your race, class, birthsign, head, and hair among it). Some strips just can't be done: if your character's race comes from the mod, the save needs that mod.

**4. Strip it for real.** Write to a new file, so the original stays untouched:

```sh
jess run strip-save.jsl Caius_Met.omwsave ~/.config/openmw/openmw.cfg Caius_Met-vanilla.omwsave Bloodmoon.esm Tribunal.esm
```

```text
Bloodmoon.esm: noKnownBlockers, dry run verified
Tribunal.esm: noKnownBlockers, dry run verified
wrote Caius_Met-vanilla.omwsave
```

Every removal is applied and verified in memory before anything is written; a blocked strip writes nothing. Without `output`, `jess.save.strip` edits the save in place, after a backup (`Caius_Met.omwsave.bak`, or `.bak1` and on if that exists), which `written.backup` names. An output file that already exists is backed up the same way. Put the output in your saves folder under a new name to see it in the load menu.

**5. Deactivate the mod, and check.** Turn it off in the OpenMW launcher (or from a script: `openmw:deactivate('Bloodmoon.esm')`, then `openmw:save()`, which backs up `openmw.cfg` first), then:

```sh
jess run check-save.jsl Caius_Met-vanilla.omwsave ~/.config/openmw/openmw.cfg
```

```text
loadsCleanly, 0 findings
```

**6. Load it and play.** Uninstall the mod, if you haven't.

What strip changes, field by field, and why each fix is what it is: the repository's [`docs/save-strip.md`](https://github.com/DreamWeave-MP/jess/blob/main/docs/save-strip.md). It edits save format versions 37 to 40; the check's `strippable_format` tells you whether yours is one.

## The screenshot

```sh
jess save screenshot Caius_Met.omwsave --output caius.jpg
```

```yaml
input: Caius_Met.omwsave
format: jpeg
bytes: 87035
written:
  path: caius.jpg
```

A file already at the output path is backed up (`caius.jpg.bak`) before it's replaced.

## Everything else in the save

```sh
jess save dump --list Caius_Met.omwsave         # every record's number and type
jess save dump --type PLAY Caius_Met.omwsave    # the player
jess save dump --type GLOB Caius_Met.omwsave    # global variables
```

Save records are shown subrecord by subrecord, with OpenMW's Lua script data (`LUAD`) decoded into readable values:

```yaml
records:
- record: 335
  type: PLAY
  subrecords:
  - FRMR: 1 (generated)
  - NAME: player
  # ...
  - LUAS: 13 (scripts/omw/input/playercontrols.lua)
  - LUAD:
      sneaking: false
  - LUAS: 14 (scripts/omw/camera/camera.lua)
  - LUAD:
      version: 0
      distance: 30
```

The same [selectors](@/docs/cli/plugin.md#selectors) as `plugin dump` work, and `--raw` gives the byte layout.

## Morrowind saves

Morrowind.exe's `.ess` saves work with `info`, `dump`, and `screenshot`:

```sh
jess save info Quicksave.ess
jess save screenshot Quicksave.ess          # writes Quicksave.tga
jess save dump --type NPCC --list Quicksave.ess
```

```yaml
format: morrowind
description: test
player:
  name: testboi
  level: 1
  class_id: Acrobat
cell: Seyda Neen
# ...
dependencies:
- Morrowind.esm
- Bloodmoon.esm
- Tribunal.esm
```

The screenshot is a 128×128 TGA built from the save's raw pixels. `check` and `strip` are OpenMW-only and say so (`error: save check isn't supported for Morrowind saves`).

jess tells the two formats apart by their contents, not their extension. To insist, pass `--game openmw` or `--game morrowind` (before or after the subcommand); a save of the other kind is then an error. Saves are read as `win1252` unless you say otherwise; a Russian save takes `jess save --encoding win1251 ...`, a Central European one `win1250`.

## Related

- [`jess save`](@/docs/cli/save.md): every flag and field.
- [Saves in Luau](@/docs/scripting/saves.md): check, strip, and edit saves from a script.
