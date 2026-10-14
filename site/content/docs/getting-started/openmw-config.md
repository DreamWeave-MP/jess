---
title: 'Paths, Configs, and Encodings'
description: 'How the CLI finds what you name (by path), where masters and textures come from, which code page text is read in, and how scripts load an openmw.cfg.'
weight: 30
extra:
  kind: guide
---

`jess plugin dump Morrowind.esm` dumps `./Morrowind.esm`. That's the whole rule: every file you hand a jess command is a path, relative to where you ran jess or absolute. The CLI doesn't read an `openmw.cfg`, doesn't search data folders, and doesn't guess.

```sh
jess plugin stats Morrowind.esm                                 # ./Morrowind.esm
jess plugin stats ~/Games/Morrowind/Data\ Files/Morrowind.esm   # that file
```

A name that isn't a file there is an error:

```text
error: 'Morrowind.esm' doesn't exist; jess takes plugins by their path
```

Load orders still matter, a lot. They're a script's business: a Luau script loads a config with [`jess.openmw.load`](@/docs/api/openmw.md), and from there names resolve the way OpenMW resolves them. [In scripts](#in-scripts) below.

## What else a command looks up

A few commands need files besides the ones you named. Each looks in places derived from your inputs, and in nothing you didn't give it.

- **Masters** (`plugin lint`'s master checks, `plugin clean --duplicate-records`, `plugin header --update-masters` and `--synchronize`): a master's name is tried as a path relative to where you ran jess, then in the plugin's own folder, case ignored. Plugins live beside their masters in `Data Files`, so this finds them wherever you run from. A master in neither place is reported missing by lint, and stops `header --synchronize`:

  ```text
  error: failed to resolve TES3 master 'Morrowind.esm'
  ```

  That happens when the plugin isn't installed yet (it's sitting in a download folder) and the masters are. Copy it in beside them, or run jess from the folder that has them.
- **Textures, meshes, and the rest** (`nif check`, `info`, `dump`, `modify`, `fix`, and `plugin assets`): resolved the way OpenMW resolves them, through the inputs' own data folders first, then each `--data` folder or archive, later ones winning. `--data "Data Files" --data "Data Files/Morrowind.bsa"` stands in for a vanilla install. A [VFS](@/docs/reference/glossary.md#vfs) path input like `meshes/f/furn_de_firepit_01.nif` is found in the `--data` folders and archives too:

  ```sh
  jess nif info --data ~/Games/Morrowind/Data\ Files/Morrowind.bsa meshes/f/furn_de_firepit_01.nif
  ```

- **Saves** are paths, and `save check` and `save strip` look nothing else up: see [Working with Saves](@/docs/guides/saves.md) for what that leaves the CLI, and the script that does the rest.

### Output paths are literal

`--output`, `--raw-output`, and every other place you name a file to *write* are plain filesystem paths, relative to where you ran jess. `jess plugin esm` and `plugin esp` without `--output` write the converted file *next to the input*; see [Converting ESP and ESM](@/docs/guides/converting-esp-esm.md).

## Encodings

Morrowind's text isn't UTF-8: English, French, German, and other Western releases use Windows-1252, Russian uses Windows-1251, and Polish and other Central European releases use Windows-1250. jess reads text as Windows-1252 unless told otherwise. To say otherwise, pass `--encoding`, on each plugin command that reads text, and on the `save` group (before or after the subcommand):

```sh
jess plugin dump --encoding win1251 --type book Morrowind.esm
jess save --encoding win1251 info Quicksave.ess
```

It affects everything that decodes or encodes text: typed dumps, `plugin diff` field changes, `plugin modify` replacements, lint checks that read script and dialogue text, and save info and dumps. It doesn't affect anything that treats the file as bytes: `plugin compare`, `plugin stats`, raw layouts, and the byte regexes of `--match`/`--no-match`. A plugin read with the wrong encoding still round-trips byte for byte; the text just displays wrong, and a replacement that introduces a character the encoding can't hold is an error rather than garbage.

Scripts are different: a call given a config reads text in that config's `encoding=`, the code page OpenMW reads your content in, unless the call names an `encoding`.

## In scripts

A script that wants a load order loads one:

```luau
--!strict
local config: string = ...
local openmw = jess.openmw.load(config)

for index, content in openmw:content() do
  print(index, content.name, content.path or 'not installed')
end
```

```sh
jess run list-content.jsl ~/.config/openmw/openmw.cfg
```

`jess.openmw.load(path)` takes an `openmw.cfg` or the folder holding it. A config that can't be read is an error about that config:

```text
error: failed to load OpenMW config: Unable to determine whether /nonexistent was a file or directory, refusing to read.
```

Under `jess run` a script starts with no default config: `jess.openmw.default()` is `nil`, `jess.openmw.load()` with no path is `error: OpenMW config resolution is disabled`, and a plugin name is a path, as on the command line. `jess.openmw.configPath(path)` sets a default, and from then on names resolve through it:

- **Plugins** (`.esp`, `.esm`, `.omwaddon`, ...) are looked up as loose files in the config's [`data=` folders](@/docs/reference/glossary.md#data-folders), case-insensitively, later folders winning, same as OpenMW. Archives are never searched for plugins: OpenMW doesn't load plugins from them either. A name that's nowhere is `error: 'Nope.esp' doesn't exist and isn't in the OpenMW data paths (1 data directories)`.
- **NIFs and other assets** resolve through the full VFS: loose files *and* archives.
- **Saves** are always paths.

A path that exists always wins over a lookup. Individual calls take `{ config = path }` or `{ openmw = context }` instead of the default.

`jess.openmw.configPath()` with no path makes OpenMW's own config the default, found the way OpenMW finds it, stopping at the first it finds:

1. `OPENMW_CONFIG`: the path of an `openmw.cfg`, or the folder holding one.
2. `OPENMW_CONFIG_DIR`: a list of folders (separated by `:`, or `;` on Windows); the first with an `openmw.cfg` in it wins.
3. OpenMW's root config: an `openmw.cfg` next to the `jess` binary (the "local" config), then the global one (`/etc/openmw` on Linux, or `OPENMW_GLOBAL_CONFIG_PATH`).
4. Your user config: `~/.config/openmw/openmw.cfg` on Linux, `Documents\My Games\OpenMW\openmw.cfg` on Windows, `~/Library/Preferences/openmw/openmw.cfg` on macOS.

Configs chain, exactly as in OpenMW: a root config's `config=` lines (usually `config="?userconfig?"`) pull in the next one, and later configs add to or override earlier ones. A loaded context has the result (`dataDirs`, `fallbackArchives`, `content()`) after the whole chain is read. On Windows and macOS there's no global config, so with no local one the user config is used directly. Flatpak OpenMW keeps its config somewhere else; jess notices when it runs inside a Flatpak, and `OPENMW_CONFIG_USING_FLATPAK=1` tells it to look there from outside one.

[OpenMW and the VFS](@/docs/scripting/openmw-and-vfs.md) has the whole story, and [Using jess without OpenMW](@/docs/getting-started/without-openmw.md) how to write a config for a Morrowind.exe install.
