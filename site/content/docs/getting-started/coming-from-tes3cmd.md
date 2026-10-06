---
title: 'Coming from tes3cmd'
description: 'Every tes3cmd command and what to run in jess instead, what behaves differently, what is gone on purpose, and how Luau replaces modify --run.'
weight: 60
extra:
  kind: guide
---

If your muscle memory says `tes3cmd clean --gmsts`, most of it carries over. The command names are grouped (`jess plugin clean`), the flags mostly kept their names, and a few old spellings are still accepted so your shell history keeps working.

`tes3cmd` solves plugin maintenance as a collection of native commands. jess keeps most of those commands and exposes the operations underneath them as a [Luau](@/docs/scripting/_index.md) API, so the things tes3cmd did with a Perl one-liner become a script you can read the next day.

## Command for command

Every command in tes3cmd 0.40, including the ones its `help` hides:

| tes3cmd | jess | Notes |
| --- | --- | --- |
| `active` | a script: [`openmw:content()`](@/docs/api/openmw.md) | Not a CLI command. A script loads `openmw.cfg` with `jess.openmw.load` and lists the load order, with the file each entry resolves to; `jess init`'s `main.luau` does exactly that. |
| `active --on X`, `--off X` | a script: `openmw:activate(X)`, `openmw:deactivate(X)`, then `openmw:save()` | `save` writes your user `openmw.cfg`, after backing it up. `activate` refuses names that aren't in your [data folders](@/docs/reference/glossary.md#data-folders) unless `{ force = true }`. |
| `clean` | [`jess plugin clean`](@/docs/cli/plugin.md#clean) | With no cleanups named, runs all of them, as tes3cmd does. Edits in place with a backup instead of writing `Clean_<name>`; `--output` writes elsewhere. |
| `clean --gmsts` | `jess plugin clean --evil-gmsts` | `--gmsts` still works. Same list of 72 GMSTs, matched only at the expansion's value. |
| `clean --dups` | `jess plugin clean --duplicate-records` | `--dups` still works. |
| `clean --junk-cells` | `jess plugin clean --junk-cells` | Stricter: jess removes a `CELL` only when it has nothing but `NAME`, `DATA`, and `NAM0` and no references. |
| `clean --cell-params` | `jess plugin clean --cell-params` | Different job: removes cell header subrecords repeated within the same `CELL`, not `AMBI`/`WHGT` copied from the master. |
| `clean --instances` | none | jess doesn't clean references duplicated from a master. |
| `common` | [`jess plugin common`](@/docs/cli/plugin.md#common) | |
| `delete` | [`jess plugin delete`](@/docs/cli/plugin.md#delete) | Same selectors. Needs at least one: a bare `delete` is an error, not "delete everything". Deleting a moved reference also removes its `MVRF` group. |
| `diff` | [`jess plugin diff`](@/docs/cli/plugin.md#diff) | One document with field-level changes instead of `plugin1-diff.txt` and `plugin2-diff.txt`. `--equal` and `--ignore-type` kept; the `--1-not-2`-style section filters are gone, since every section is a field you can filter with `jq`. |
| `dump` | [`jess plugin dump`](@/docs/cli/plugin.md#dump) | YAML records instead of text. `--list` kept. |
| `dump --binary --output X` | `jess plugin dump --raw-output X` | `--header` gives the extract its own `TES3` header, as tes3cmd's `--header` does. |
| `dump --format "%id% %name%"` | `jess plugin dump --format json … \| jq` | Pick fields out of the JSON instead of a format string. |
| `esm`, `esp` | [`jess plugin esm`, `jess plugin esp`](@/docs/cli/plugin.md#esm) | Output lands next to the input, as in tes3cmd. `--overwrite` kept. |
| `fixit` | none | See [below](#intentionally-absent). |
| `header` | [`jess plugin header`](@/docs/cli/plugin.md#header) | `--synchronize` (or `--sync`), `--update-masters`, `--update-record-count`, `--author`, `--description` with the same leading `+` to append. |
| `help <command>` | `jess help <group> <command>`, `jess man` | Also `--help` on anything. |
| `lint` | [`jess plugin lint`](@/docs/cli/plugin.md#lint) | Same check names. Findings have severities, and the command exits `1` on errors; see [Linting](@/docs/guides/linting.md). |
| `modify --replace`, `--replacefirst` | [`jess plugin modify`](@/docs/cli/plugin.md#modify) `--replace`, `--replace-first` | `--replacefirst` still works. Works on typed string fields. |
| `modify --run`, `--program-file` | `jess plugin modify --script fix.luau` | Luau instead of Perl. See [Replacing --run](#replacing-run-with-luau). |
| `multipatch` | [TES3Merge](@/docs/bundled-scripts/tes3merge.md) | Leveled lists, fog bug, summons, and cell names, as a [load-order tool](@/docs/guides/load-order-scripts.md) in the Dreamwind package. |
| `overdial` | [`jess plugin overdial`](@/docs/cli/plugin.md#overdial) | `--single` kept. |
| `recover` | none | See [below](#intentionally-absent). |
| `resetdates` | none | OpenMW's load order is `content=` order, not file dates. There's nothing to reset. |
| `run` | `jess run script.luau` | |
| `shell` | none | Write a script and `jess run` it. |
| `stats` | [`jess plugin stats`](@/docs/cli/plugin.md#stats) | |
| `testcodec` | `jess plugin test-codec` | Debug builds only (`cargo run -p jess-cli -- plugin test-codec`). Byte-for-byte, not "orthogonal". |
| `undelete` | none | Removed on purpose. See [below](#undelete-would-eat-your-cell). |

Flags tes3cmd shared across commands:

| tes3cmd | jess |
| --- | --- |
| `--active` | none on the command line: every command takes files by path. A script that loads `openmw.cfg` walks the active content itself. |
| `--backup-dir`, `--hide-backups` | Backups sit next to the file as `.bak`, `.bak1`, and so on, and are never overwritten. `--no-backup` skips them. |
| `--output` | `--output`, on every command that writes. It's a literal path. |
| `--ignore-plugin` | none; name the plugins you want. |
| `--debug`, `--verbose` | `-v`/`--verbose` where there's more to show (`plugin assets`, `nif check`). |
| none | `--dry-run` on every command that edits a file. |

## What's different everywhere

**It takes paths, and leaves load orders to scripts.** tes3cmd looks for `Morrowind.ini` and `Data Files` next to itself. jess's commands look for nothing: `jess plugin lint Morrowind.esm` reads `./Morrowind.esm`, and masters are found beside the plugin. Scripts read OpenMW's `openmw.cfg` chain the way OpenMW does, which means several [data folders](@/docs/reference/glossary.md#data-folders) and archives instead of one. [Paths, Configs, and Encodings](@/docs/getting-started/openmw-config.md) has the details; if you don't use OpenMW at all, read [Using jess without OpenMW](@/docs/getting-started/without-openmw.md).

**Output is a document, not a report.** tes3cmd prints prose for people. jess prints one YAML document (or JSON, or TOML) per command, the same shape every time, so the thing you read and the thing a script parses are one thing. See [Output Formats](@/docs/getting-started/output-formats.md).

**Edits happen in place, with a backup.** tes3cmd's `clean` writes `Clean_MyMod.esp` unless you say `--overwrite`. jess edits `MyMod.esp` after copying it to `MyMod.esp.bak`. If you want tes3cmd's behavior, pass `--output Clean_MyMod.esp`. Every edit takes `--dry-run`, which does all the work and writes nothing.

**Regexes are Rust regexes, not Perl.** `--id`, `--match`, and friends ignore case, as in tes3cmd, but there are no lookarounds or backreferences in the pattern. `--match` and `--no-match` run over the record's bytes; `--sub-match` runs over each subrecord as text (`mesh:m\foo.nif`). Morrowind-era plugins mostly need `^`, `$`, `.*`, and alternation, which work the same.

**Untouched bytes stay untouched.** tes3cmd rebuilds every record it writes. jess copies records it didn't change from the original file byte for byte, and rewrites only what an edit touched. A `header --sync` changes the header and nothing else.

## Replacing --run with Luau

tes3cmd's `modify --run` evaluates Perl against each matching record, with `$R` as the record:

```sh
tes3cmd modify --type cont --run '$R->set({f=>"count"}, -3)' ale_patch.esp
```

In jess, a modify script gets the whole plugin and picks its own records. The selector flags still work on the command line for `--replace`; with `--script`, the script does the selecting:

```luau
--!strict
-- restock.luau: make every container's items restock. A negative count restocks.
local plugin: Plugin = ...
for container in plugin:select { type = 'CONT' } do
  container:edit(function(value)
    if value.type ~= 'Container' then return end

    -- Each inventory entry is { count, id }.
    for _, item in value.inventory do
      local count = item[1] :: number
      item[1] = -math.abs(count)
    end
  end)
end
```

```sh
jess plugin modify --script restock.luau --dry-run ale_patch.esp
jess plugin modify --script restock.luau ale_patch.esp
```

The script works on decoded records instead of subrecord indices, and the plugin is written only if the script changed something. When a job outgrows one plugin (a patch built from the whole load order, a report across every mod), write a standalone script and `jess run` it. [Modify Scripts](@/docs/scripting/modify-scripts.md) and [Your First Script](@/docs/scripting/intro.md) cover both.

## Intentionally absent

### undelete would eat your cell

tes3cmd's `undelete` turns a deleted reference back on and marks it disabled with a `ZNAM` subrecord. Morrowind.exe reads that. OpenMW doesn't: when it meets the `ZNAM` it stops reading the cell's references, and every reference after that point in the cell is gone. On OpenMW, "undeleting" one reference could silently delete a hundred. jess used to have it; it was removed rather than left as a trap. [Format Notes](@/docs/contributing/format-notes.md#why-there-s-no-undelete) walks through the OpenMW code that does it.

### fixit, multipatch, recover

- **`multipatch`** is what [TES3Merge](@/docs/bundled-scripts/tes3merge.md) does, with the same patch names (`fogbug`, `summons`, `cellnames`) plus the rest of TES3Merge's merging. Run it as a [load-order script](@/docs/guides/load-order-scripts.md) with a bare `jess`.
- **`fixit`** was `clean` on every plugin, `header --synchronize`, `multipatch`, and `resetdates` in one go. Cleaning and syncing every plugin you have installed, unasked, is a decision; jess makes you make it. A load-order script that calls `plugin:clean{}` on the files you choose does the same job in about ten lines.
- **`recover`** salvaged readable records from a physically damaged file. jess refuses to parse a file it can't read and tells you where it stopped (`failed to parse 'x.esp': truncated record header at offset 0`). Guessing past corruption is how you get a plugin that loads and is wrong.
