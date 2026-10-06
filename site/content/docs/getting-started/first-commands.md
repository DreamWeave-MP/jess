---
title: 'First Commands'
description: 'Start with a bare jess, set up a script folder, then read your load order, a plugin, a save, and a mesh through jess''s eyes.'
weight: 20
extra:
  kind: guide
---

A handful of commands to run against your own setup. Only `jess init` writes anything, and only into a new folder of your choosing; the rest read. By the end you'll know how jess sees your game.

The output below comes from a small load order (the three official masters and *Siege at Firemoth*), trimmed where it runs long. Yours will be longer and name your own folders.

## Run jess

```sh
jess
```

With no command, jess prints its help: every command, and where to start:

```text
Tools for Bethesda game data files

Usage: jess [OPTIONS] [COMMAND]

Commands:
  plugin       Inspect, check, and modify plugin files
  save         Inspect, check, and strip save files
  nif          Check, describe, dump, fix, and script NIF models and animations
  run          Run a Luau script with the jess API
  # ...

Start here:
  jess init my-scripts        Set up a folder for Luau scripts, with types for your editor
  jess plugin lint MyMod.esp  Check a plugin for problems
  jess run script.luau        Run a Luau script with the jess API

Docs: https://DreamWeave-MP.github.io/jess
```

Every command takes files by path. jess doesn't go looking for your `openmw.cfg`; a script that wants your load order is handed it. [Paths, Configs, and Encodings](@/docs/getting-started/openmw-config.md) has the rules.

## Set up a script folder

```sh
jess init my-scripts
```

```yaml
dir: my-scripts
editor: vscode
types:
  folder: my-scripts/types
  definitions: my-scripts/types/jess.d.luau
  files: 33
  removed: []
  aliases:
    dream: types/dream
    jess: types/jess
written:
- path: my-scripts/.luaurc
- path: my-scripts/main.luau
- path: my-scripts/.vscode/settings.json
skipped: []
next_steps:
- 'run the example over a load order: jess run my-scripts/main.luau path/to/openmw.cfg'
- 'or on one plugin: jess run my-scripts/main.luau MyMod.esp'
# ...
```

A folder with the Luau types for this jess, a strict `.luaurc`, editor settings (`--editor neovim`, `zed`, `helix`, or `none` for others), and a `main.luau` that counts the lights in every plugin a load order loads. Hand it your `openmw.cfg`, or the folder holding it:

```sh
jess run my-scripts/main.luau ~/.config/openmw/openmw.cfg
```

```text
1. Morrowind.esm: 574 lights
2. Tribunal.esm: 44 lights
3. Bloodmoon.esm: 74 lights
4. Siege at Firemoth.esp: 0 lights
692 lights in the load order
```

That's every `content=` line in the config (your [content files](@/docs/reference/glossary.md#content-files)), in load order, each looked up in your [`data=` folders](@/docs/reference/glossary.md#data-folders) the way OpenMW looks it up. One that isn't installed anywhere prints `not installed`, which means OpenMW won't find it either. Read `main.luau`: it's short, and [OpenMW and the VFS](@/docs/scripting/openmw-and-vfs.md) has the rest of what a loaded config gives a script.

Script output is whatever the script prints, as plain text. If you never plan to script, skip this; nothing else on this page needs it. [Editor Setup](@/docs/getting-started/editor-setup.md) covers the editor half.

## What's inside a plugin

Run these from your `Data Files` folder, or spell out the path:

```sh
cd ~/Games/Morrowind/Data\ Files
jess plugin stats Morrowind.esm
```

```yaml
files: 1
records: 48296
record_types:
  ACTI: 697
  ALCH: 258
  BOOK: 574
  CELL: 2538
  DIAL: 2358
  INFO: 23693
  NPC_: 2675
  # ...
subrecord_types:
  ACTI.FNAM: 697
  ACTI.MODL: 697
  # ...
```

`Morrowind.esm` there is a path, relative to where you are; from anywhere else it's `error: 'Morrowind.esm' doesn't exist; jess takes plugins by their path`. Nearly half of Morrowind is dialogue (`INFO`), in case you wondered where 79 MB went.

## Read some records

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
```

`--type` takes the four-letter tag the counts above list, or the type's name as a full dump prints it (`--type Book`), in any case. `--list` shows which records match without their contents. Drop it, and narrow to one record, to read the whole thing:

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

[Dumping and Inspecting](@/docs/guides/dumping-and-inspecting.md) covers every way to pick records.

## Look at a save

Saves are read by path, like everything else:

```sh
jess save info ~/.local/share/openmw/saves/Ashur/Caius_Met.omwsave
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

Your saves live under OpenMW's user data folder: `~/.local/share/openmw/saves` on Linux, `Documents\My Games\OpenMW\saves` on Windows, `~/Library/Application Support/openmw/saves` on macOS. Morrowind's own `.ess` saves work here too. See [Working with Saves](@/docs/guides/saves.md) for what else you can do with them.

## Look at a mesh

```sh
jess nif info --no-tree --data Morrowind.bsa meshes/f/furn_de_firepit_01.nif
```

```yaml
files:
- path: meshes/f/furn_de_firepit_01.nif
  source: Morrowind.bsa
  version: 4.0.0.2
  game: Morrowind
  # ...
  record_count: 14
  record_types:
    NiNode: 1
    NiTriShape: 3
    RootCollisionNode: 1
    # ...
  error: null
  collision:
    source: root_collision_node
    node: 11
    hides_markers: false
  stats:
    shapes: 2
    vertices: 389
    triangles: 577
    collision_triangles: 16
    # ...
  textures:
  - path: Tx_stone_darkgrey_01.tga
    resolved: textures/tx_stone_darkgrey_01.dds
    source: Morrowind.bsa
  # ...
```

That NIF isn't a loose file: it's inside `Morrowind.bsa`, and `--data` told jess to look there. It found it the way OpenMW would, by its [VFS](@/docs/reference/glossary.md#vfs) path. Same for its textures, including the `.tga` → `.dds` swap OpenMW makes. Without `--no-tree` you also get the scene graph.

## Getting help

- `jess help` or `jess --help` lists the command groups.
- `jess plugin --help` lists a group's commands; `jess plugin dump --help` a command's flags.
- `jess help plugin dump` is the same thing, spelled differently.
- The [CLI Reference](@/docs/cli/_index.md) has all of it, with output shapes and exit codes.

Every command prints YAML by default. Add `--format json` when a program is reading it; see [Output Formats](@/docs/getting-started/output-formats.md).

## Where next

- Got a job to do? The [Guides](@/docs/guides/_index.md) start from the problem: [merging your load order](@/docs/guides/load-order-scripts.md), [cleaning a plugin](@/docs/guides/cleaning-plugins.md), [stripping a mod out of a save](@/docs/guides/saves.md), [finding broken meshes](@/docs/guides/nif-checking-and-fixing.md).
- Used tes3cmd? [Coming from tes3cmd](@/docs/getting-started/coming-from-tes3cmd.md) maps every command.
- Want to automate it? [Your First Script](@/docs/scripting/intro.md), with [Editor Setup](@/docs/getting-started/editor-setup.md) for the types.
- Got an error? [Troubleshooting](@/docs/reference/troubleshooting.md) is keyed by what jess printed.
