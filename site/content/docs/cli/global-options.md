---
title: 'Global Options'
description: 'Options every command takes (--format, --help, --version); the save group''s --encoding and --game; and plugin commands'' --encoding.'
weight: 5
extra:
  kind: reference
---

{{ api_signature(value="jess [--format <FORMAT>] [COMMAND]") }}

With no command, `jess` prints its help: the commands, and a few to start with. It doesn't read an `openmw.cfg`, and no command does: every file a command works on is named by its path. A script that wants a load order loads one itself, with [`jess.openmw.load`](@/docs/api/openmw.md).

These work on every command and can go anywhere on the command line: `jess --format json plugin stats MyMod.esp` and `jess plugin stats MyMod.esp --format json` are the same.

| Option | Value | Default | Does |
| --- | --- | --- | --- |
| `--format <FORMAT>` | `yaml`, `json`, `toml` | `yaml` (`json` for `dump --raw`) | The output document's format. JSON is compact, one line. |
| `-h`, `--help` | | | Help for the command it follows. |
| `-V`, `--version` | | | Print `jess <version>` (subcommands print their own name, like `jess-plugin 0.1.0`). |

What the formats look like: [Output Formats](@/docs/getting-started/output-formats.md). How inputs are found: [Paths, Masters, and Encodings](@/docs/getting-started/openmw-config.md).

## Group options

These belong to the `save` group, and go anywhere after `save`, before or after the subcommand: `jess save --encoding win1251 info X.ess` and `jess save info --encoding win1251 X.ess` are the same.

| Option | On | Values | Default | Does |
| --- | --- | --- | --- | --- |
| `--encoding <ENCODING>` | `jess save` | `win1250`, `win1251`, `win1252` | `win1252` | The code page for strings in save info and dumps. |
| `--game <GAME>` | `jess save` | `auto`, `openmw`, `morrowind` | `auto` | Which game wrote the save. `auto` detects it; the others insist, and fail on the other kind. `check` and `strip` take OpenMW saves only. |

The plugin commands that read or write text take `--encoding` (`win1250`, `win1251`, or `win1252`) after the command: `dump`, `delete`, `modify`, `header`, `lint`, `clean`, `common`, `diff`, `overdial`, `validate`, and `assets`. Commands that only see bytes (`stats`, `compare`, `esm`, `esp`) don't have it.

Without `--encoding`, both read text as `win1252`. See [Encodings](@/docs/getting-started/openmw-config.md#encodings).

## Writing files

Every command that edits a file follows the same rules; see [Output Formats](@/docs/getting-started/output-formats.md#commands-that-write-files) for the full story.

- Without `--output`, the input is edited in place after a backup is copied beside it: `<file>.bak`, then `.bak1`, `.bak2`, never overwriting an older one.
- With `--output <path>`, the input is untouched. A file already at the output path is backed up the same way before it's replaced. The path is literal, relative to the working directory, and must differ from the input.
- The report says what was written as `written: {path, backup}`; nothing changed means nothing written, and `written: null`.
- `--no-backup` skips the backup, and `--dry-run` writes nothing and reports what would change, with `dry_run: true`.

## Errors and exit codes

Errors print to stderr as `error: <message>`; warnings as `warning: <message>`. Exit code `0` is success, `1` an error or a failure the command reports on purpose (see [exit codes](@/docs/getting-started/output-formats.md#exit-codes)), `2` a command line jess couldn't parse.
