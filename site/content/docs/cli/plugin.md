---
title: 'jess plugin'
description: 'assets, compare, dump, delete, modify, stats, header, lint, clean, common, diff, overdial, validate, esp, esm.'
weight: 10
extra:
  kind: reference
---

{{ api_signature(value="jess plugin <COMMAND>") }}

Inspect, check, and edit `.esp`, `.esm`, and `.omwaddon` files.

`jess plugin --help` lists the commands under Inspect, Check, and Edit, as the table below does.

**Text encoding.** The commands that read or write text, `dump`, `delete`, `modify`, `header`, `lint`, `clean`, `common`, `diff`, `overdial`, `validate`, and `assets`, take `--encoding <ENCODING>`: the code page the plugin's text is in, `win1252` (English and Western European releases), `win1251` (Russian), or `win1250` (Central European). It goes after the command: `jess plugin dump --encoding win1251 X.esp`. Without it, text is read as `win1252` ([Encodings](@/docs/getting-started/openmw-config.md#encodings)).

Plus the [global options](@/docs/cli/global-options.md).

**Plugin inputs** are paths, relative to where you ran jess or absolute. A name that isn't a file there is an error: `error: 'MyMod.esp' doesn't exist; jess takes plugins by their path`. See [Paths, Configs, and Encodings](@/docs/getting-started/openmw-config.md).

**Editing commands** (`delete`, `modify`, `clean`, `header`, `esm`, `esp`) write in place after a backup (`.bak`, `.bak1`, ...), or to `--output`, and print `written: {path, backup}`, or `written: null` when nothing changed. `--output` must differ from the input. Each takes `--dry-run`, which reports what would change with `dry_run: true` and writes nothing, and `--no-backup`, which writes without the backup. Every command that writes a file (these, `dump --raw-output`, `esm`, `esp`) backs up a file it replaces the same way, `--output` targets included, and reports it in `written`.

Untouched records are always written back byte for byte. An edit changes only what it says it changes.

| Group | Command | Does |
| --- | --- | --- |
| Inspect | [`dump`](#dump) | Print records, or their byte layout; extract them to a new file. |
| | [`stats`](#stats) | Count records and subrecords. |
| | [`header`](#header) | Show or update the `TES3` header. |
| | [`diff`](#diff) | What changed between two plugins, by record. |
| | [`common`](#common) | Records two plugins share. |
| | [`compare`](#compare) | Compare two files byte for byte. |
| | [`overdial`](#overdial) | Dialogue topics that swallow other topics. |
| Check | [`lint`](#lint) | Content problems: dirt, likely mistakes, missing dependencies. |
| | [`validate`](#validate) | The file's format, against what Morrowind.exe or OpenMW loads. |
| | [`assets`](#assets) | Every file plugins refer to. |
| Edit | [`clean`](#clean) | Remove safe dirt. |
| | [`delete`](#delete) | Delete selected records, subrecords, or references. |
| | [`modify`](#modify) | Regex-replace in string fields, or run a Luau script on the plugin. |
| | [`esm`](#esm) | Convert a plugin to a master. |
| | [`esp`](#esp) | Convert a master to a plugin. |
| Development | [`test-codec`](#test-codec) | Check files round-trip byte for byte (debug builds only). |

**Which check?** `plugin lint` asks whether a plugin's content is sensible, `plugin validate` whether its format loads in Morrowind.exe or OpenMW, [`nif check`](@/docs/cli/nif.md#check) whether models load, and [`save check`](@/docs/cli/save.md#check) whether a save is sound.

## Selectors

`dump`, `delete`, and `modify` (and [`save dump`](@/docs/cli/save.md#dump)) pick records with the same flags. With none, every record matches.

| Option | Matches |
| --- | --- |
| `-t`, `--type <TYPE>` | Records of this type: a four-letter tag (`book`, `NPC_`, `cell`, `npc` for `NPC_`) or the type's name as typed dumps print it (`Light`, `MiscItem`, `gamesetting`), either in any case. Repeatable; any may match. |
| `--flag <FLAG>` | Records with this header flag: `deleted`, `persistent`, `ignored`, `blocked`, or a number (`0x400`). Repeatable; any may match. |
| `--id <REGEX>` | Records whose ID matches this case-insensitive regex. Repeatable; all must match. |
| `--exact-id <ID>` | Records whose ID is exactly this, ignoring case. Repeatable; any may match. |
| `--match <REGEX>` | Records whose raw bytes match this case-insensitive regex. Repeatable; all must match. |
| `--no-match <REGEX>` | Skip records whose raw bytes match. Repeatable; any skips. |
| `--sub-match <REGEX>` | Subrecords whose rendered text matches. |
| `--sub-no-match <REGEX>` | Skip subrecords whose rendered text matches. |
| `--instance-match <REGEX>` | `CELL` references whose rendered text matches. Only `CELL` records are picked. |
| `--instance-no-match <REGEX>` | Skip `CELL` references whose rendered text matches. Only `CELL` records are picked. |
| `--interior` | Only interior `CELL`s. |
| `--exterior` | Only exterior `CELL`s. |

All the options given must hold at once.

**Rendered text.** Subrecord and instance filters match text, not bytes. A string field renders as `name: value` using the field's typed name, the one typed dumps use (`id: bt_balmora_bench`, `mesh: bt\balmora_bench.nif`, `name: Grand Soul Gem`); anything else as `TAG: payload`, the payload read as text. A `CELL` reference renders as all its subrecords together. So `--sub-match '^mesh:'` means "the model path", and `--instance-match fargoth` means "any reference mentioning Fargoth".

**What gets picked.** With `--sub-match`/`--sub-no-match`, `dump` shows whole records that have a matching subrecord, while `delete` deletes and `modify` edits only the matching subrecords. With instance filters, only `CELL`s are picked, for their matching references, and `dump` shows only those; other record types never match, with or without `--type`. `dump --raw-output` writes records whole, so a sub-filtered record is copied with all its subrecords, but an instance-filtered cell keeps only its matching references.

Record numbers (`record` in dumps and `--list`, `index` in `--raw` layouts) are 0-based positions in the file, header included, and match the Luau API's `jess.meta(record).index`.

## assets

{{ api_signature(value="jess plugin assets [--show <LEVEL>] [-v] [--no-meshes] [--data <PATH>]... <PLUGIN>...") }}

| Argument / option | Default | Does |
| --- | --- | --- |
| `<PLUGIN>...` | | Plugins to check, by path. At least one. |
| `--show <LEVEL>` | `collision` | The least severe findings to include: `wont-load`, `broken`, `missing-asset`, `collision`, `performance`, `info`; or `files` for just the list of files the plugins load. |
| `-v`, `--verbose` | off | Everything: all findings, every model, every reference, every file used. |
| `--no-meshes` | off | Don't run the NIF check on the models. |
| `--data <PATH>` | | A folder or archive to look files up in. Repeatable; later ones win, and the plugins' own folders win over all of them. |

Finds every file the plugins' records name (models and their distant `_dist` versions, animated models' `x` variants, icons, magic effect icons and their `b_` big icons, textures, sounds, voice lines, book images, OpenMW Lua scripts, and what scripts `Say`, `StreamMusic`, and `PlayBink`), looks each up the way OpenMW does, and runs every model found through the [NIF check](@/docs/cli/nif.md#check). Named plugins' own data folders are searched first, as if installed last.

```sh
jess plugin assets BalmoraTweaks.esp
```

```yaml
summary:
  plugins:
  - BalmoraTweaks.esp
  unreadable_plugins: 0
  references: 6
  files: 5
  missing: 1
  meshes_checked: 2
  meshes_wont_load: 0
  by_code:
    missing_mesh: 2
  meshes_by_code: {}
findings:
- severity: missing_asset
  code: missing_mesh
  message: model "bt\balmora_bench.nif" not found at meshes/bt/balmora_bench.nif
  plugin: BalmoraTweaks.esp
  record_type: STAT
  id: bt_balmora_bench
  path: bt\balmora_bench.nif
```

| Key | Present | Holds |
| --- | --- | --- |
| `summary` | always | Counts: references, distinct `files`, `missing`, models checked and won't-load, findings by code. |
| `findings` | always | Problems with references: `missing_mesh`, `missing_texture`, `missing_icon`, `missing_sound`, `missing_voice`, `missing_book_art`, `missing_script`, `unreadable_plugin`, and so on. |
| `meshes` | when any are shown | NIF check results for models with findings at `--show` or worse (every model with `-v`). |
| `references` | `-v` | Every reference: record, `kind`, `path` as written, `resolved` VFS path, `source`. |
| `used_files` | `-v`, `--show files` | Every file the plugins load: `file` (the loose file or its archive), VFS `path`, `in_archive`. |

`--show files` prints only `used_files`, and names each missing file on stderr.

**Exit code:** `1` when a referenced file is missing, a plugin can't be read, or a model won't load, after printing the report (`error: 1 referenced file is missing`). The guide: [Checking Assets](@/docs/guides/asset-checking.md).

## compare

{{ api_signature(value="jess plugin compare <LEFT> <RIGHT>") }}

Alias: `comp`. Compares two files byte for byte (any files; they aren't parsed).

```sh
jess plugin compare BalmoraTweaks.esp BalmoraTweaks.esm
```

```yaml
left: BalmoraTweaks.esp
right: BalmoraTweaks.esm
identical: false
left_bytes: 940
right_bytes: 940
first_difference: 28
```

`first_difference` is the first differing byte offset (the shorter file's length when one is a prefix of the other), or `null`. **Exit code:** `1` when the files differ (`error: files differ`).

## dump

{{ api_signature(value="jess plugin dump [SELECTORS] [--list] [--raw] [--raw-output <PATH> [--header] [--no-backup]] <INPUT>") }}

| Option | Does |
| --- | --- |
| [selectors](#selectors) | Which records. |
| `--list` | Only each record's `record` number, `type`, and `id`. With `--raw`, the layout without subrecords. |
| `--raw` | The byte layout (tags, sizes, byte ranges, decoded string fields) instead of typed records. JSON unless `--format` says otherwise. |
| `--raw-output <PATH>` | Write the matching records' bytes to a file at `PATH` instead of printing them. Sub-filtered records are copied whole; instance-filtered cells keep only their matching references. A file already there is backed up first. |
| `--header` | With `--raw-output`, start the file with the input's `TES3` header, record count updated. |
| `--no-backup` | With `--raw-output`, replace a file already there without backing it up. |

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

Typed records use the type's full name (`MiscItem`, `Book`, `Cell`) and its fields; `record` is the index. `--list` uses the four-letter tag:

```yaml
records:
- record: 3002
  type: MISC
  id: Misc_SoulGem_Grand
```

`--raw`:

```json
{"records":[{"index":3002,"type":"MISC","id":"Misc_SoulGem_Grand","flags":0,"size":134,"range":{"start":1057313,"end":1057463},"subrecords":[{"index":0,"type":"NAME","size":19,"range":{"start":1057329,"end":1057356},"field":"id","value":"Misc_SoulGem_Grand"},{"index":1,"type":"MODL","size":25,"range":{"start":1057356,"end":1057389},"field":"mesh","value":"m\\Misc_soulgem_grand.NIF"}, ...]}]}
```

`--raw-output`:

```sh
jess plugin dump --type book --id '^bk_BriefHistoryEmpire[1-4]$' --raw-output histories.esp --header Morrowind.esm
```

```yaml
input: Morrowind.esm
path: /home/you/Games/Morrowind/Data Files/Morrowind.esm
bytes: 22276
written:
  path: histories.esp
```

The header `--header` copies is the input's, file type, author, and masters included: extracting from `Morrowind.esm` gives a file that calls itself a master with no masters.

The guide: [Dumping and Inspecting](@/docs/guides/dumping-and-inspecting.md).

## delete

{{ api_signature(value="jess plugin delete [SELECTORS] [--output <OUTPUT>] [--dry-run] [--no-backup] <INPUT>") }}

| Option | Does |
| --- | --- |
| [selectors](#selectors) | What to delete: whole records; or, with subrecord filters, matching subrecords; or, with instance filters, matching `CELL` references. At least one is required. |
| `--output <OUTPUT>` | Write here instead of editing in place. |
| `--dry-run` | Report what would be deleted; write nothing. |
| `--no-backup` | Don't back up the file written over. |

Deleting records keeps the header's record count right (`record_count_updated`); to fix a count that was already wrong, use [`header --update-record-count`](#header).

```sh
jess plugin delete --type gmst --exact-id sMaxSale BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
deleted: 1
unit: records
record_count_updated: true
written:
  path: BalmoraTweaks.esp
  backup: BalmoraTweaks.esp.bak
```

`unit` is `records`, or `items` when subrecord or instance filters made it count subrecords and references.

With no selectors, `delete` refuses to run instead of deleting every record (`error: delete needs at least one selector ...`). `jess plugin dump --list` with the same selectors shows what a delete will take.

## modify

{{ api_signature(value="jess plugin modify [SELECTORS] (--replace </PATTERN/REPLACEMENT/> | --replace-first </PATTERN/REPLACEMENT/> | --script <SCRIPT>) [--output <OUTPUT>] [--dry-run] [--no-backup] <INPUT>") }}

| Option | Does |
| --- | --- |
| [selectors](#selectors) | Which records, and with `--sub-match`, which string fields. Instance filters aren't supported. |
| `--replace </PAT/REP/>` | Replace every match of the regex `PAT` with `REP` in each selected string field. |
| `--replace-first </PAT/REP/>` | Replace only the first match in each selected string field. tes3cmd's `--replacefirst` works too. |
| `--script <SCRIPT>` | Run a Luau script on the plugin instead, which can change anything. Can't be combined with `--replace`/`--replace-first`. |
| `--output <OUTPUT>` | Write here instead of editing in place. |
| `--dry-run` | Report what would change (or run the script); write nothing. |
| `--no-backup` | Don't back up the file written over. |

One of `--replace`, `--replace-first`, or `--script` is required.

Replacements work on typed string fields (IDs, names, model and icon paths, script names, text), decoded with `--encoding`. The pattern is a case-insensitive regex, and `REP` can use its captures as `$1` or `${name}`. The first character of the expression is the delimiter, so `|a/b|c/d|` works when the pattern has slashes; there's no escaping a delimiter inside it. Without `--sub-match`, every string field of every selected record is fair game, IDs included, so narrow it:

```sh
# prefix every static's ID, but not its mesh path
jess plugin modify --type stat --sub-match '^id:' --replace '/^/PC_/' MyMod.esp

# point the bench at a new mesh
jess plugin modify -t stat --sub-match '^mesh:' --replace '/balmora_bench/balmora_bench_02/' BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
fields: 2
records: 2
written:
  path: BalmoraTweaks.esp
  backup: BalmoraTweaks.esp.bak
```

With `--script`, the script gets `(plugin, path)` as `...`, a plugin handle and its path, and the plugin is written if the script changed it:

```yaml
input: MyMod.esp
path: MyMod.esp
script: fix.jsl
changed: true
written: { path: MyMod.esp, backup: MyMod.esp.bak }
```

See [Modify Scripts](@/docs/scripting/modify-scripts.md).

## stats

{{ api_signature(value="jess plugin stats <INPUT>...") }}

Record and subrecord counts, summed over every input (at least one).

```sh
jess plugin stats Morrowind.esm
```

```yaml
files: 1
records: 48296
record_types:
  ACTI: 697
  # ...
subrecord_types:
  ACTI.FNAM: 697
  # ...
list_subrecords:
  ALCH.ENAM: 22
  # ...
```

`subrecord_types` counts subrecords by `RECORD.SUBRECORD`; `list_subrecords` counts the records in which that subrecord repeats.

## header

{{ api_signature(value="jess plugin header [--update-record-count] [--update-masters] [--synchronize] [--author <AUTHOR>] [--description <DESCRIPTION>] [--output <OUTPUT>] [--dry-run] [--no-backup] <INPUT>") }}

| Option | Does |
| --- | --- |
| `--update-record-count` | Set the header's record count to the records that follow it. |
| `--update-masters` | Set each master's recorded size to its file's actual size. Masters are found as paths, then beside the plugin. |
| `--synchronize` | Both of the above: tes3cmd's name for it (`--sync` works too). |
| `--author <AUTHOR>` | Set the author. A leading `+` appends instead. |
| `--description <DESCRIPTION>` | Set the description. A leading `+` appends; `\n` and `\r` are expanded. |
| `--output <OUTPUT>` | Write here instead of editing in place. Needs one of the flags above. |
| `--dry-run` | Show the header as it would be; write nothing. |
| `--no-backup` | Don't back up the file written over. |

With no flags, shows the header; with any, updates it and shows the result.

The author, description, and master names are in the `--encoding` code page, the way OpenMW reads them, so `Русификатор.esm` in a `win1251` plugin is found on disk under that name. An OpenMW save is the exception: OpenMW writes its header in UTF-8, so jess reads it that way whatever `--encoding` says. Text the code page can't hold is an error that names the character; masters you didn't change keep their bytes.

```sh
jess plugin header --synchronize BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
author: Ashur
description: Makes Balmora a bit nicer.
record_count: 6
masters_updated:
- name: Morrowind.esm
  old_size: 79837000
  new_size: 79837552
  path: /home/you/Games/Morrowind/Data Files/Morrowind.esm
written:
  path: BalmoraTweaks.esp
  backup: BalmoraTweaks.esp.bak
```

A master that can't be found is an error (`failed to resolve TES3 master 'Morrowind.esm'`). The guide: [Masters and Headers](@/docs/guides/masters-and-headers.md).

## lint

{{ api_signature(value="jess plugin lint [--all | --recommended] [CHECKS]... [--fail-on <LEVEL>] <INPUT>") }}

Runs checks and lists what they find. With no checks named, runs the recommended ones. Named checks and presets add up: `--recommended --overrides` runs the recommended checks and `overrides`.

| Option | Recommended | Severity | Reports |
| --- | --- | --- | --- |
| `--all` | | | Every check below. |
| `--recommended` | | | The checks marked ✓. |
| `--duplicate-records` | | warning | Two records with the same type and ID. |
| `--evil-gmsts` | | warning | Evil GMSTs: the 11 Tribunal and 61 Bloodmoon GMSTs the Construction Set leaks into plugins, still at the expansion's value (tes3cmd's list; IDs match ignoring case). |
| `--overrides` | | info | Records that override a master's record. Needs the masters. |
| `--missing-author` | | info | An empty author. |
| `--missing-description` | | info | An empty description. |
| `--missing-version` | ✓ | info | A description with no line starting `Version`. |
| `--missing-master` | | error | Masters that can't be found. |
| `--master-sync` | | warning | Master sizes that don't match the files. |
| `--cell-params` | | warning | Repeated `CELL` header subrecords before the references. |
| `--junk-cells` | ✓ | warning | `CELL`s with nothing but a name and data: no references, no changes. |
| `--record-count` | | warning | A header record count that doesn't match. |
| `--fogbug` | ✓ | warning | Interior cells with fog density 0. |
| `--fogsync` | ✓ | warning | Cells whose `DATA` and `AMBI` fog densities disagree. |
| `--autocalc-spells` | | info | Spells marked autocalc. |
| `--bloodmoon-dependency` | ✓ | error | Bloodmoon functions in scripts without `Bloodmoon.esm` as a master. |
| `--cell-00` | | warning | Exterior cells at grid 0,0 (often an accidental copy). |
| `--deprecated-lists` | ✓ | warning | Scripts that `AddItem`/`RemoveItem` on leveled lists. |
| `--getsoundplaying` | | info | Scripts using `GetSoundPlaying`. |
| `--menumode` | | info | Scripts that never mention `MenuMode`. |
| `--scripted-doors` | | info | Doors with scripts. |
| `--dialogue-teleports` | ✓ | warning | Dialogue results that `Position` or `PositionCell` the player. |
| `--duplicate-info` | | warning | Duplicate `INFO` IDs. |
| `--modified-info` | | warning | `INFO`s that change a master's `INFO`. Needs the masters. |
| `--modified-info-id` | ✓ | warning | The same dialogue text under different `INFO` IDs. |
| `--expansion-dependency` | ✓ | error | Expansion functions without either expansion as a master. |
| `--no-bloodmoon-functions` | | info | `Bloodmoon.esm` as a master with no Bloodmoon functions used. |
| `--no-tribunal-functions` | | info | `Tribunal.esm` as a master with no Tribunal functions used. |
| `--fail-on <LEVEL>` | | | The least severe finding that makes lint exit `1`: `error` (the default), `warning`, `any`, or `never`. |

(`--modified-info_id` is accepted as an alias for `--modified-info-id`, and tes3cmd's `--gmsts` for `--evil-gmsts`.)

```sh
jess plugin lint BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
checks:
- junk_cells
- missing_version
- fogbug
- fogsync
- bloodmoon_dependency
- deprecated_lists
- dialogue_teleports
- expansion_dependency
- modified_info_id
summary:
  errors: 0
  warnings: 1
  info: 1
findings:
- check: missing_version
  severity: info
  message: TES3 description has no Version line
- check: junk_cells
  severity: warning
  message: CELL Balmora, Caius Cosades' House at record 6
```

`checks` lists what ran, so an empty `findings` means something. Every finding has its check's **severity**: `error` breaks the plugin for players (a missing master, expansion functions without the expansion as a master), `warning` is dirt or a likely mistake worth fixing, and `info` is worth a look but often on purpose. `summary` counts them.

**Exit code:** `1` when a finding is at least as severe as `--fail-on`, after printing the report (`error: 1 error was found`), like `validate` and `assets`; otherwise `0`. By default only errors fail, so a missing version line doesn't break a build; `--fail-on warning` also fails on dirt, `--fail-on any` on every finding, and `--fail-on never` never fails. What each check is for: [Linting Plugins](@/docs/guides/linting.md).

## clean

{{ api_signature(value="jess plugin clean [--evil-gmsts] [--duplicate-records] [--cell-params] [--junk-cells] [--output <OUTPUT>] [--dry-run] [--no-backup] <INPUT>") }}

| Option | Removes |
| --- | --- |
| `--evil-gmsts` | Evil GMSTs (the ones `lint --evil-gmsts` reports). tes3cmd's `--gmsts` works too. |
| `--duplicate-records` | Later copies of a record with the same type and ID, and records identical, byte for byte, to their master's copy. Masters are found as paths, then beside the plugin. tes3cmd's `--dups` works too. |
| `--cell-params` | Repeated `CELL` header subrecords before the references. |
| `--junk-cells` | `CELL`s with nothing but a name and data. |
| `--output <OUTPUT>` | Write here instead of editing in place. |
| `--dry-run` | Report what would be removed; write nothing. |
| `--no-backup` | Don't back up the file written over. |

With no cleanups named, all four run, like tes3cmd's `clean`: each is safe on its own. Naming any runs only those. `cleanups` in the report lists what ran. Removing records keeps the header's record count right (`record_count_updated`).

```sh
jess plugin clean BalmoraTweaks.esp
jess plugin clean --evil-gmsts BalmoraTweaks.esp          # only the GMSTs
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
cleanups:
- duplicate_records
- evil_gmsts
- cell_params
- junk_cells
removed_records: 4
removed_subrecords: 0
record_count_updated: true
written:
  path: BalmoraTweaks.esp
  backup: BalmoraTweaks.esp.bak
```

The guide: [Cleaning Plugins](@/docs/guides/cleaning-plugins.md).

## common

{{ api_signature(value="jess plugin common <LEFT> <RIGHT>") }}

Records (by type and ID) that both plugins have.

```sh
jess plugin common BalmoraTweaks.esp Morrowind.esm
```

```yaml
left: BalmoraTweaks.esp
right: /home/you/Games/Morrowind/Data Files/Morrowind.esm
records:
- type: CELL
  id: Balmora, Caius Cosades' House
- type: MISC
  id: Misc_SoulGem_Common
```

## diff

{{ api_signature(value="jess plugin diff [--equal] [--ignore-type <TYPE>]... <LEFT> <RIGHT>") }}

| Option | Does |
| --- | --- |
| `--equal` | Also list records both plugins have with identical bytes. |
| `--ignore-type <TYPE>` | Leave out this record type. Repeatable. |

Records by type and ID: only in the left, only in the right, and changed.

```sh
jess plugin diff BalmoraTweaks.esp BalmoraTweaks-v2.esp
```

```yaml
left_only: []
right_only: []
changed:
- type: STAT
  id: bt_balmora_bench
  raw_changed: true
  fields:
  - field: mesh[1]
    left: bt\balmora_bench.nif
    right: bt\balmora_bench_02.nif
- type: MISC
  id: Misc_SoulGem_Common
  raw_changed: true
  fields: []
```

`fields` lists the string fields that changed (name, then subrecord position). A change to numbers or flags shows up as `raw_changed: true` with no `fields`; dump both records to see it. With `--equal`, an `equal` list comes before `changed`. The guide: [Comparing Plugins](@/docs/guides/comparing-plugins.md).

## overdial

{{ api_signature(value="jess plugin overdial [--single] <INPUT>...") }}

| Option | Does |
| --- | --- |
| `--single` | Only check the first plugin's topics: report those that contain a topic from a later plugin. Later plugins' topics aren't compared with each other, and a later topic containing one of the first plugin's isn't reported. |

Finds dialogue topics that contain another plugin's topic (ignoring case), comparing each plugin with every later one. Needs at least two inputs.

```sh
jess plugin overdial Morrowind.esm Tribunal.esm
```

```yaml
overlaps:
- earlier_plugin: /home/you/Games/Morrowind/Data Files/Morrowind.esm
  later_plugin: /home/you/Games/Morrowind/Data Files/Tribunal.esm
  outer_topic: B8_Failed_Nerevarine
  inner_topic: Nerevarine
# ...
```

## test-codec

{{ api_signature(value="jess plugin test-codec [--continue] [--ignore-cruft] [-x <TYPE>]... [INPUT]...") }}

Debug builds only (`cargo run -p jess-cli -- plugin test-codec ...`); release binaries don't have it. Parses each file and saves it back, checking the bytes come out identical.

| Option | Does |
| --- | --- |
| `--continue` | Keep checking later files after one fails. |
| `--ignore-cruft` | Ignore bytes before or after the parseable plugin. |
| `-x`, `--exclude-type <TYPE>` | Leave this record type out of the check. Repeatable. |

```yaml
results:
- input: Morrowind.esm
  path: /home/you/Games/Morrowind/Data Files/Morrowind.esm
  ok: true
  error: null
```

**Exit code:** `1` when any file fails.

## validate

{{ api_signature(value="jess plugin validate [--profile <PROFILE>] <INPUT>") }}

| Option | Values | Default |
| --- | --- | --- |
| `--profile <PROFILE>` | `raw`, `tes3`, `omwaddon`, `omwsave` | `omwaddon` |

Checks the plugin against a format profile, and lists what the profile checked as `checks`, so a `valid: true` says what it vouches for.

| Profile | `checks` | Reports |
| --- | --- | --- |
| every profile | `structure` | What every engine and tool relies on: a readable `TES3` header as the first record and nowhere else, `MAST`/`DATA` master entries in pairs, and record and subrecord tags that are text. Parsing already refuses records and subrecords that don't fit the file. |
| `raw` | `structure` | Only that. |
| `tes3` | + `morrowind_format` | Every record type and subrecord Morrowind.exe and the Construction Set don't know, such as OpenMW-only records and extensions. For plugins that should work in both engines. |
| `omwaddon` | + `openmw_record_types`, `dialogue_order`, `record_contents` | What OpenMW's content loader refuses or skips: record types it doesn't load (it refuses the whole file), `INFO`s with no `DIAL` before them (skipped), and records it can't read the way its loaders read them. Records flagged ignored are skipped, as OpenMW skips them. |
| `omwsave` | `structure`, `save_metadata`, `save_format`, `cell_state` | An OpenMW save's own structure, as `save check` sees it without a load order: a readable `SAVE` header, a format version OpenMW loads, and saved cells laid out the way the engine reads them. |

```sh
jess plugin validate MyMod.esp
jess plugin validate --profile tes3 MyMod.omwaddon
```

A diagnostic gives the byte `offset` and what's wrong: `the first record is STAT, not the TES3 header`, `SELG isn't a record type OpenMW loads; it refuses the file ("Unknown record")`, `INFO has no DIAL before it; OpenMW skips it`, `<TAG> is an OpenMW-only record type`, `<TAG>.<SUB> is an OpenMW extension`, and so on. An OpenMW save is TES3 underneath, which makes it a good demonstration of the `tes3` profile:

```yaml
input: Caius_Met.omwsave
checks:
- structure
- morrowind_format
valid: false
diagnostics:
- offset: 16
  message: TES3.FORM is an OpenMW extension
- offset: 212
  message: SAVE is not a Morrowind record type
# ...
```

**Exit code:** `1` when `valid` is `false` (`error: profile validation failed`).

## esp

{{ api_signature(value="jess plugin esp [--overwrite] [--output <OUTPUT>] [--dry-run] [--no-backup] <INPUT>") }}

Converts a master to a plugin: sets the header's file type to plugin and writes the result. The input must be named `.esm`. Without `--output` the result goes beside the input, with an `.esp` extension, as tes3cmd does.

| Option | Does |
| --- | --- |
| `--output <OUTPUT>` | Where to write. Must differ from the input. |
| `--overwrite` | Replace the output if it exists, after backing it up (`.bak`, `.bak1`, ...). Without it, an existing output is an error. |
| `--dry-run` | Check the conversion and report where it would go; write nothing. |
| `--no-backup` | With `--overwrite`, replace the output without backing it up. |

```yaml
input: BalmoraTweaks.esm
path: BalmoraTweaks.esm
file_type: plugin
output: BalmoraTweaks.esp
written:
  path: BalmoraTweaks.esp
```

## esm

{{ api_signature(value="jess plugin esm [--overwrite] [--output <OUTPUT>] [--dry-run] [--no-backup] <INPUT>") }}

The reverse of [`esp`](#esp): the input must be named `.esp`, the file type becomes master, and the default output has an `.esm` extension.

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
file_type: master
output: BalmoraTweaks.esm
written:
  path: BalmoraTweaks.esm
```

The guide: [Converting ESP and ESM](@/docs/guides/converting-esp-esm.md).

## tes3cmd parity

jess covers most of what people used [tes3cmd](https://github.com/john-moonsugar/tes3cmd) for.

| tes3cmd | jess |
| --- | --- |
| `active` | Not in the CLI. A Luau script edits OpenMW's content list (not `Morrowind.ini`) through [`jess.openmw.load`](@/docs/api/openmw.md): `content`, `activate`, `deactivate`, `save`. |
| `clean` | `plugin clean`; master-aware cleaning is partial |
| `common` | `plugin common` |
| `delete` | `plugin delete` |
| `diff` | `plugin diff` |
| `dump` | `plugin dump` |
| `esm`, `esp` | `plugin esm`, `plugin esp` |
| `header` | `plugin header` |
| `lint` | `plugin lint`; master-aware checks are partial |
| `modify` | `plugin modify --replace`/`--replace-first` (`--replacefirst` works too); `--script` (Luau) replaces `--run` and `--program-file` (Perl) |
| `overdial` | `plugin overdial` |
| `stats` | `plugin stats` |
| `testcodec` | `plugin test-codec`, debug builds only |
| `undelete` | Not ported, on purpose: it marks references disabled with `ZNAM`, which OpenMW doesn't read. OpenMW stops loading the cell there and loses every reference after it. |
| `fixit`, `multipatch`, `recover` | Not planned for now. [TES3Merge](@/docs/bundled-scripts/tes3merge.md) covers much of what `multipatch` did. |

jess-only: `plugin assets`, `compare`, and `validate`; the `save` and `nif` groups; `run`, `init`, `luau-types`, `completions`, and `man`.
