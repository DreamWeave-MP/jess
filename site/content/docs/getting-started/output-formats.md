---
title: 'Output Formats'
description: 'YAML, JSON, or TOML: every command prints one structured document.'
weight: 40
extra:
  kind: guide
---

jess has no "pretty" text output. Every command prints exactly one document to stdout: YAML by default, JSON or TOML when you ask. What you read in the terminal is what a script parses, so there's nothing to scrape.

Two exceptions, both on purpose:

- **`jess run`** prints whatever the script prints, as plain text, and no document of its own. The script decides its output; `--format` doesn't touch it. A script that wants to be parsed should print JSON itself.
- **`jess completions` and `man`** (without `--output`) print the file they generate: shell script or roff. `jess luau-types` writes a folder and reports it like any other command.

A `plugin modify --script` or `nif modify --script` script can print too. Its lines go to stdout ahead of the report, so a program parsing that report wants a script that doesn't print, or prints to a file.

## Pick a format

`--format` is global, so it goes anywhere on the command line:

```sh
jess plugin header Tribunal.esm                  # YAML
jess plugin header Tribunal.esm --format json    # JSON
jess --format toml plugin header Tribunal.esm    # TOML
```

```yaml
input: Tribunal.esm
path: Tribunal.esm
author: Bethesda Softworks
description: "The main data file for Tribunal.\r\n(requires Morrowind.esm to run)"
record_count: 9999
masters_updated: []
written: null
```

```json
{"input":"Tribunal.esm","path":"Tribunal.esm","author":"Bethesda Softworks","description":"The main data file for Tribunal.\r\n(requires Morrowind.esm to run)","record_count":9999,"masters_updated":[],"written":null}
```

- **YAML** is the default, for people. Long lists (record dumps) stream as they're found, so the first records show up before a big plugin finishes.
- **JSON** is compact, one line, for programs. It streams too.
- **TOML** is there if you want it. TOML has no `null`, so fields that would be `null` are left out, and it has no top-level lists, so dumps are collected before anything prints.

`plugin dump --raw` and `save dump --raw` (the byte layout of records) default to JSON instead of YAML, since they're for programs. `--format yaml` still gets you YAML.

Floats print in the shortest spelling that reads back as the same value, in every format: `0.1`, not `0.10000000149011612` or a rounded `0.1`, and very small or very large ones in scientific notation (`3.140798e-39`), so a dump loses nothing and never prints a wall of zeros.

Field names are `snake_case` (`format_version`, `record_count`), and so are report codes (`loads_cleanly`, `missing_mesh`, `wont_load`). The Luau API reports the same things in camelCase.

Reports about one plugin or model start with `input`, what you typed, and `path`, the file jess actually read. The CLI takes files by path, so today the two are the same; scripts that read reports keep using `path`. Most save reports give `input` only.

Errors and warnings go to stderr, as plain lines starting `error:` or `warning:`, so they never end up in the document.

## Commands that write files

A command that writes a file says so in a `written` field:

```yaml
input: MyMod.esp
path: MyMod.esp
removed_records: 1
removed_subrecords: 0
record_count_updated: false
written:
  path: MyMod.esp
  backup: MyMod.esp.bak
```

`written: null` means nothing changed, so nothing was written. Asking to clean a clean plugin leaves its timestamp alone.

The rules every command that writes a file follows:

- **Without `--output`**, jess edits the file in place, after copying the original to a backup beside it. The first backup is `<file>.bak`; if that exists, `<file>.bak1`, then `.bak2`, and so on. An old backup is never overwritten, so a second fix doesn't eat your only copy of the original.
- **With `--output <path>`**, the input isn't touched. If a file is already at the output path, it's backed up the same way before it's replaced; otherwise there's nothing to back up, and `written` has no `backup` field.
- `--output` must differ from the input; jess refuses to write over the file it's reading.
- `--output` is a literal path, relative to where you ran jess.
- **`--no-backup`** writes without the backup, on every command that makes one.
- **`--dry-run`**, on every command that edits files (`plugin delete`, `modify`, `clean`, `header`, `esm`, `esp`, `nif fix`, `nif modify`, `save strip`), does everything but write: the report says what would change, has `dry_run: true`, and `written: null`.

Every command that writes a file reports it as `written: {path, backup}`: `plugin delete`, `modify`, `clean`, `header`, `esp`, `esm`, `dump --raw-output`, `save strip` and `save screenshot`, and `nif modify`. `nif fix` reports a `written` for each file it fixed, and `man --output` lists the pages it wrote under `written`.

## Piping into other tools

JSON into [jq](https://jqlang.org):

```sh
# names of every book in Morrowind.esm
jess plugin dump --type book --format json Morrowind.esm | jq -r '.records[].name'

# just the verdict
jess save check --format json Caius_Met.omwsave | jq -r .verdict
```

YAML into [yq](https://github.com/mikefarah/yq):

```sh
jess plugin stats Morrowind.esm | yq '.record_types.NPC_'
```

Piping into `head` is fine: jess stops quietly when the reader goes away.

## Exit codes

- `0`: the command ran, and found nothing it calls a failure.
- `1`: an error, or one of the failures below. The error line on stderr says which.
- `2`: jess didn't understand the command line (an unknown flag, a missing argument).

Commands that describe exit `0` whatever they find: a diff with differences, a save that loads with broken state. Their document says what they found. Commands that check exit `1` when the check fails, *after* printing their full document, so a script or CI job can stop on them:

| Command | Exits 1 when |
| --- | --- |
| `plugin compare` | the files differ |
| `plugin validate` | the plugin breaks the profile |
| `plugin lint` | a finding is at least as severe as `--fail-on` (default: any `error`) |
| `plugin assets` | a referenced file is missing, a plugin can't be read, or a model won't load |
| `nif check` | any file won't load |
| `nif fix` | a file that needed fixes couldn't be written |
| `save check` | the save can't load (`invalid_structure` or `will_not_load`) |
| `save strip` | the strip is blocked (except with `--dry-run`) |
| `run` | a script raises an error |
| `plugin test-codec` | a file doesn't round-trip |

Lint findings have severities (`error`, `warning`, `info`); only errors fail by default, and `--fail-on warning|any|never` moves the line. Choosing the checks decides what's looked for: `jess plugin lint MyMod.esp` runs the recommended ones, `--evil-gmsts --junk-cells` only those.
