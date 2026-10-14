---
title: 'Using jess without OpenMW'
description: 'For Morrowind.exe and MGE XE setups: the CLI works on plain paths, and a two-line openmw.cfg gives scripts a load order.'
weight: 35
extra:
  kind: guide
---

jess doesn't need OpenMW installed, and it doesn't care which engine you play on. A plugin is a plugin. The command line never reads an `openmw.cfg` at all: every command takes files by path, so if you play with Morrowind.exe, with or without MGE XE and MWSE, the CLI is exactly the same tool for you as for anyone else. The parts that are about a whole load order live in Luau scripts, and those need about two lines of config.

## What works on plain paths

Every command:

- **Plugins:** `plugin dump`, `stats`, `diff`, `common`, `compare`, `overdial`, `validate`, `lint`, `delete`, `modify`, `clean`, `header`, `esm`, `esp`, and `assets`.
- **Saves:** `save info`, `save dump`, and `save screenshot` read Morrowind `.ess` saves (`save --game morrowind` if you want to insist). `save check` and `save strip` are OpenMW-only and say so.
- **NIFs:** `nif info`, `dump`, `check`, `fix`, and `modify` on files, folders, and archives. Add `--data "C:/Games/Morrowind/Data Files"` to `nif check` (or `plugin assets`) so textures and meshes resolve against your install, and another `--data` naming `Morrowind.bsa` to include what's in it.
- **Scripts:** `jess run script.jsl` with paths in the script.

```sh
jess plugin lint "C:/Games/Morrowind/Data Files/Suran_Underworld_2.5.esp"
jess plugin clean --dry-run "C:/Games/Morrowind/Data Files/Suran_Underworld_2.5.esp"
jess save info C:/Games/Morrowind/Saves/quiksave.ess
```

Some of them need the plugin's masters: lint's `missing_master` and `master_sync`, the identical-to-master half of `clean --duplicate-records`, and `header --update-masters`. A master's name is tried relative to where you ran jess, then in the plugin's own folder, case ignored. Plugins live in `Data Files` beside their masters, so `Morrowind.esm` turns up wherever you run from. A master that's in neither place is reported missing by lint, and stops `header --synchronize`:

```text
error: failed to resolve TES3 master 'Morrowind.esm'
```

That happens when the plugin isn't installed yet (it's sitting in a download folder) and the masters are. Install it, or run jess from `Data Files`.

## What needs a load order

Some things are about the whole setup, and there's no path to hand them. They're scripts, and a script gets its load order from an `openmw.cfg` you name:

| Needs a load order | Why |
| --- | --- |
| [TES3Merge](@/docs/bundled-scripts/tes3merge.md), [S3LightFixes](@/docs/bundled-scripts/s3lightfixes.md), and [Merged Lands](@/docs/bundled-scripts/mergedlands.md) | They read the load order and write a plugin into it. |
| Checking every asset or model the load order uses | "Everything in my game" is the load order. |
| Listing the content files, with where each is installed | That's what a config is. |

## A config that isn't OpenMW's

"Needs a load order" doesn't mean "needs OpenMW". It needs an `openmw.cfg` file, which is a text file. Make one that describes your Morrowind install:

```ini
# C:\Games\Morrowind\jess\openmw.cfg
data="C:/Games/Morrowind/Data Files"
fallback-archive=Morrowind.bsa
fallback-archive=Tribunal.bsa
fallback-archive=Bloodmoon.bsa
content=Morrowind.esm
content=Tribunal.esm
content=Bloodmoon.esm
content=Suran_Underworld_2.5.esp
```

and hand its path to the script. The example `jess init` writes lists each plugin's lights:

```sh
jess init my-scripts
jess run my-scripts/main.jsl C:/Games/Morrowind/jess/openmw.cfg
```

Nothing here touches `Morrowind.ini`, and Morrowind.exe never reads this file. A script that activates content (`openmw:activate`, a tool's `activate = true`) edits this file, not your INI.

### Import it from Morrowind.ini

Writing the `content=` lines by hand is tedious once you have more than a handful of plugins. The `Morrowind.ini` importer, the one OpenMW's installer runs, is [`@dreamwind/openmw-import-morrowind`](@/docs/scripting/packages.md#openmw-import-morrowind), part of the Dreamwind package in the jess repository (`jpk/dreamwind`). It turns `[Game Files]` and `[Archives]` into `content=` and `fallback-archive=` lines and finds `Data Files` next to the INI. Activate the package with `--package` and `require` it:

```luau
--!strict
-- import-ini.jsl: write an openmw.cfg for jess from a Morrowind.ini. Reads the INI, writes only the output.
local ini = require '@dreamwind/openmw-import-morrowind'
local iniPath: string, outputPath: string = ...
local result = ini.importPaths { ini = iniPath, gameFiles = true }

for _, warning in result.warnings do
  print(`warning: {warning.message}`)
end

ini.saveCfg(result.cfg, outputPath)
print(`wrote {outputPath}`)
```

```sh
# jess/ is a checkout of the jess repository.
mkdir jess-cfg
jess run --package jess/jpk/dreamwind import-ini.jsl "C:/Games/Morrowind/Morrowind.ini" jess-cfg/openmw.cfg
```

`gameFiles = true` imports your active plugins as `content=` lines, and `saveCfg` writes the file through a temporary file and a rename, so a failed write leaves nothing half-written. A `GameFile` entry the importer can't find stops the import of that entry and every one after it, with a warning (the package also logs it through `@jess/log`), so read what the script prints:

```text
warning: GameFile entry not found: Gone.esp. Later GameFile entries were not imported.
wrote jess-cfg/openmw.cfg
```

`[Archives]` entries (and `Morrowind.bsa`) have to exist too, or `importPaths` raises an error naming the one it couldn't find. Rerun the script after you change your load order.

## Load order: dates versus lines

Morrowind.exe loads masters first, then plugins, each group oldest file first. OpenMW, and so jess, loads `content=` lines top to bottom. The importer above writes them in Morrowind.exe's order: masters first, then by file date, which is what you get in game. Change a file's date (mlox and most Morrowind mod managers do exactly that) and the config is stale until you rerun the import. jess won't notice on its own, and it will happily tell you which plugin wins a conflict in the order it was given.
