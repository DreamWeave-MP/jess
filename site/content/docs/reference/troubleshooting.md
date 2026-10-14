---
title: 'Troubleshooting'
description: 'The errors jess prints, what they mean, and what to do about them: missing files, masters, encodings, lint exit codes, backups, scripts, and editor types.'
weight: 20
extra:
  kind: reference
---

Find the text jess printed, read what it means, do the thing. Errors go to stderr as lines starting `error:`, with `hint:` lines after some of them; the document on stdout is never mixed with them.

## Finding files

### doesn't exist; jess takes plugins by their path

```text
error: 'BT-v2.esp' doesn't exist; jess takes plugins by their path
```

There's no file at that path. Every command takes files by path, relative to where you ran jess or absolute; nothing is looked up in a load order. Give the path (`"C:/Games/Morrowind/Data Files/BT-v2.esp"`), or `cd` to the folder that has it. [Paths, Configs, and Encodings](@/docs/getting-started/openmw-config.md) has the rules.

### doesn't exist

```text
error: 'test/thing.nif' doesn't exist
```

A NIF input that isn't a file is taken as a [VFS](@/docs/reference/glossary.md#vfs) path, and looked for in the `--data` folders and archives; with no `--data`, there's nowhere to look. Name the archive or folder that has it (`--data "Data Files/Morrowind.bsa"`), and give the whole VFS path from the top: `meshes/test/thing.nif`, not `test/thing.nif`. A plugin's mesh field (`test\thing.nif`) is relative to `meshes/`, so copying it straight out of a dump gets you this error. Backslashes and case don't matter in VFS paths.

### unexpected argument '--openmw-cfg' found

```text
error: unexpected argument '--openmw-cfg' found
error: unrecognized subcommand 'openmw'
```

The `jess` CLI doesn't read OpenMW configs: no `--openmw-cfg`, no `--no-openmw-config`, no `jess openmw`, and the `JESS_OPENMW_CFG` and `JESS_NO_OPENMW_CONFIG` variables are ignored. Commands take files by path. Load-order work (listing, enabling, merging, checking a whole load order) happens in Luau scripts that load the config with [`jess.openmw.load(path)`](@/docs/api/openmw.md); [Merging and Fixing a Load Order](@/docs/guides/load-order-scripts.md) has the one most people want.

### failed to resolve TES3 master

```text
error: failed to resolve TES3 master 'Tribunal.esm'
```

`plugin header --update-masters` (or `--synchronize`) needs each master's file, to read its size, and one of them isn't found. Masters are tried as paths relative to where you ran jess, then in the plugin's own folder, ignoring case. Install the master, put the plugin beside it, or run jess from the folder that has it.

## Reading plugins

### failed to parse

```text
error: failed to parse './bad.esp': truncated record header at offset 0
```

The file isn't a readable TES3 file: truncated, damaged, or not a plugin at all (a renamed archive, a Skyrim plugin). The offset is where reading stopped. jess doesn't guess past it. If Morrowind.exe or OpenMW loads the file anyway, that's a jess bug worth reporting, with the file.

### Text looks garbled

Names like `Ñåðäöå` where you expected `Сердце` mean the plugin's text is in another code page. Morrowind's text isn't UTF-8: Western releases use Windows-1252, Russian uses Windows-1251, Polish and other Central European releases use Windows-1250. jess reads text as Windows-1252 unless told otherwise. A plugin in another code page needs telling:

```sh
jess plugin dump --encoding win1251 --type book Morrowind.esm
jess save --encoding win1251 info Quicksave.ess
```

Scripts read in the `encoding=` of the config a call resolves through, else Windows-1252, and take `encoding` in `jess.plugin.open`. Reading with the wrong encoding never damages a file: it still round-trips byte for byte, the text displays wrong.

### can't be written in Windows-1252, the plugin's encoding

```text
error: failed to modify plugin: STAT 'marker' NAME: 'м' (U+043C) can't be written in Windows-1252, the plugin's encoding; nothing was written
hint: if the plugin's text isn't Windows-1252, pass its code page with --encoding (win1250, win1251, or win1252); if it is, the replacement needs characters Windows-1252 has
```

A `plugin modify --replace` produced a character the plugin's encoding can't hold, here Cyrillic into a Windows-1252 plugin. The error names the record, the field, the character, and the code page jess used. Plugins don't record their encoding, so jess used the one you gave it, or your config's `encoding=`, or `win1252`. If the plugin is really Russian (`win1251`) or Central European (`win1250`), say so with `--encoding`. If it really is Windows-1252, the character has no place in it, and no flag will change that.

## Editing

### already exists; use --overwrite

```text
error: ./AllTypes.esm already exists; use --overwrite to replace it
```

`plugin esm` and `esp` won't replace an existing file unless you ask. `--overwrite` backs it up first.

### --output must differ from resolved input

```text
error: clean --output must differ from resolved input
```

`--output` is for writing somewhere else. To edit in place, leave it off; jess backs up the original.

### delete needs at least one selector

```text
error: delete needs at least one selector (--type, --id, --exact-id, --flag, --match, ...); without one it would delete every record
```

A bare `plugin delete` used to delete everything but the header. Now it refuses. Name what to delete.

### Backups are piling up

Every in-place edit copies the original to `<file>.bak` first, then `.bak1`, `.bak2`, and so on. jess never overwrites or deletes a backup: a second fix mustn't eat your only copy of the original. Cleaning them up is yours:

```sh
ls "Data Files"/*.bak*          # see them
rm "Data Files"/MyMod.esp.bak*  # when you're sure
```

To stop making them, pass `--no-backup` on commands you trust, or write to a new file with `--output`. `--dry-run` first is cheaper than either.

## Exit codes

### lint exits 1

```text
error: 1 error was found
error: 2 errors or warnings were found
```

`plugin lint` printed its full report, and then found at least one finding at or above `--fail-on`: `error` by default, which means a broken plugin (a missing master, expansion functions without the expansion). Read the `summary` and `findings`. If you're in a CI job and want warnings to fail too, `--fail-on warning`; if you want lint to report and never fail, `--fail-on never`.

Other checking commands exit `1` when their check fails too, after printing: `compare`, `validate`, `assets`, `nif check`, `save check`, `run`. [Output Formats](@/docs/getting-started/output-formats.md#exit-codes) has the table. Exit `2` is always a command line jess didn't understand.

## Scripts

### isn't a file

```text
error: 'nope.jsl' isn't a file
```

`jess run` takes the script's path, and there's no file there. It runs files only: bundled `@jess/...` names aren't scripts it runs.

### OpenMW config resolution is disabled

```text
error: OpenMW config resolution is disabled
```

A script called `jess.openmw.load()` with no path, and it has no default config to load: a `jess run` script starts without one. Pass the `openmw.cfg` (or its folder): `jess.openmw.load(config)`, with the path taken from the script's arguments.

### isn't in the load order, so its size is unknown

```text
error: master 'Morrowind.esm' isn't in the load order, so its size is unknown; give it as { name = "Morrowind.esm", size = <bytes> }
```

`setMasters` (or `jess.plugin.new`'s `masters`) was given a master by name, and it isn't in the config the call looked through, or beside the plugin's file. A `jess run` script has no default config, so pass the load order's: `plugin:setMasters(names, { openmw = openmw })` (or `{ config = path }`). Or give the size, as the message says.

### module not found

```text
error: module not found: scripts.coolmod.patch
```

`require` looked for a VFS module and found nothing. Dotted and bare names are VFS paths, and they resolve through the script's default config, which a `jess run` script doesn't have until it calls `jess.openmw.configPath(config)`. A module beside the script is `require('./patch')`; a package's export needs `--package`.

### unknown field

```text
error: unknown field 'data.weigth'
```

A script wrote a field the record type doesn't have, and nothing changed. jess refuses instead of skipping it, because a patch that silently didn't apply is worse than one that stopped. Check the spelling against [Record Types](@/docs/api/record-types.md), or let the [editor types](@/docs/getting-started/editor-setup.md) catch it before you run.

## Editors and types

### luau-lsp doesn't know `jess`

Everything `jess.` is underlined, or has type `any`. luau-lsp isn't loading `types/jess.d.jsl`:

- The folder isn't there: `jess init <folder>` writes it.
- The editor opened a parent folder. Settings files (`.vscode/`, `.zed/`, `.helix/`) and relative definition paths are read from the folder you opened. Open the script folder itself.
- The settings name the file with the wrong path, or the wrong key. [Editor Setup](@/docs/getting-started/editor-setup.md) has the exact settings per editor.

### `require('@jess/...')` is an unknown require

`require('@jess/log')` or `require('@dream/vfs')` is underlined as unknown, or its value is `any`: the `.jessrc` beside your scripts has no aliases for `jess` and `dream`, which point those names at `types/`. A folder set up before the types became a folder has an old `.jessrc`; add the aliases `jess init` prints, or rerun it with `--force`. `require 'scripts.myMod.util'` and `require 'vfstool'` are OpenMW's names, not Luau paths, and no `.jessrc` makes them resolve: [use `./` paths, or call them untyped](@/docs/getting-started/editor-setup.md#why-a-folder).

### Neovim doesn't start luau-lsp

`jess init --editor neovim` writes `.nvim.lua`, and Neovim ignores that file unless `vim.o.exrc = true` is in your `init.lua`. With it on, Neovim reads `.nvim.lua` only from the folder you start it in, and asks once whether to trust it. Start `nvim` inside the script folder, answer the prompt, and check `:checkhealth vim.lsp`. It also needs Neovim 0.11 or later and `luau-lsp` on your `PATH`.

### Types that can't be right

Errors on code that runs fine, especially around writing a whole table to a struct field (`light.data = {...}`) or around read-only arrays: luau-lsp is using the old type solver, which can't express those types. The settings `jess init` writes turn the new one on, so this means your editor is using settings from somewhere else. Turn it on there ([how, per editor](@/docs/getting-started/editor-setup.md#the-new-type-solver)).

### Types for functions that don't exist (or missing ones that do)

`types/` is from another version of jess. Rerun `jess init <folder>` after every upgrade; it refreshes `types/` and leaves your files alone.

## Windows

- **Paths.** Quote anything with spaces: `"C:/Games/Morrowind/Data Files/MyMod.esp"`. Forward slashes work everywhere and don't need escaping. VFS paths take either slash.
- **Quoting regexes.** In PowerShell, single quotes pass a pattern through untouched: `--sub-match '^mesh:'`. In `cmd.exe`, use double quotes, and double any `%`.
- **Non-ASCII output.** jess writes UTF-8. An old console code page shows accented names as junk even when the plugin is fine; `chcp 65001` in `cmd.exe`, or redirect to a file and open that. For scripts, use `--format json` and read it with something that knows UTF-8.
- **Redirecting in Windows PowerShell 5.1.** `>` writes UTF-16. Fine for reading, wrong for files other tools read (JSON for `jq`). Use PowerShell 7, or `| Out-File -Encoding utf8`.
- **Old terminals keep the old `Path`.** After adding jess to your `Path`, open a new terminal.
