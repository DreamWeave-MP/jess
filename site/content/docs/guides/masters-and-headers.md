---
title: 'Masters and Headers'
description: 'Fix master sizes, record counts, authors, and descriptions.'
weight: 40
extra:
  kind: guide
---

Goal: get a plugin's `TES3` header ([`HEDR`](@/docs/reference/glossary.md#hedr), plus its master list) right: who made it, what it is, which masters it needs, and the bookkeeping numbers Morrowind.exe checks.

## Read the header

```sh
jess plugin header BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
author: Ashur
description: Makes Balmora a bit nicer.
record_count: 6
masters_updated: []
written: null
```

With no flags, `header` just shows it. For the masters and their recorded sizes, dump the header record:

```sh
jess plugin dump --type tes3 BalmoraTweaks.esp
```

```yaml
records:
- record: 0
  type: Header
  version: 1.3
  file_type: Esp
  author: Ashur
  description: Makes Balmora a bit nicer.
  num_objects: 6
  masters:
  - name: Morrowind.esm
    size: 79837000
```

## Sync master sizes and the record count

Every master entry records the master's file size at the time the plugin was saved. Update Morrowind.esm (a patch, a cleaned copy, a GOTY vs. retail install) and the size no longer matches. **Morrowind.exe** notices and warns that the master has changed; **OpenMW** ignores the size entirely. If anyone might play your plugin in Morrowind.exe, keep them in sync:

```sh
jess plugin lint --master-sync --record-count BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
checks:
- record_count
- master_sync
summary:
  errors: 0
  warnings: 1
  info: 0
findings:
- check: master_sync
  severity: warning
  message: Morrowind.esm DATA has 79837000, actual is 79837552 (/home/you/Games/Morrowind/Data Files/Morrowind.esm)
```

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

`--synchronize` (`--sync` for short) is `--update-masters` plus `--update-record-count`; either works alone. Masters are found as paths first (relative to where you ran jess), then in the plugin's own folder, case ignored, so the sizes come from the masters *you* have. A master jess can't find is an error, and nothing is written:

```text
error: failed to resolve TES3 master 'Morrowind.esm'
```

That means the master is neither in the current folder nor beside the plugin.

The record count is the number of records after the header. OpenMW doesn't need it either, but it's cheap to keep right: `plugin delete` and `plugin clean` set it whenever they remove records, and `header --update-record-count` fixes a count that was already wrong. Add `--dry-run` to see the header as it would be without writing anything.

## Author and description

```sh
jess plugin header --author "Ashur" --description "Makes Balmora a bit nicer." BalmoraTweaks.esp
```

A leading `+` appends instead of replacing, and `\n` becomes a newline, which is how you add a version line without retyping the description:

```sh
jess plugin header --author '+ & friends' --description '+\nVersion 1.1' BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
author: Ashur & friends
description: |-
  Makes Balmora a bit nicer.
  Version 1.1
record_count: 6
masters_updated: []
written:
  path: BalmoraTweaks.esp
  backup: BalmoraTweaks.esp.bak1
```

Use single quotes so your shell leaves the `\n` alone. The header has room for 32 bytes of author and 256 of description; anything longer is an error (`header string is too long for 32-byte field`), and nothing is written.

All the flags combine in one command, and `--output` writes the result elsewhere (it needs at least one flag to have something to write).

## Rename or remove masters

The command line doesn't edit the master list itself. That's a job for a short Luau script:

```luau
-- masters.jsl <plugin>
local pluginPath: string = ...
local plugin = jess.plugin.open(pluginPath)
plugin:setMasters { 'Morrowind.esm', 'Tribunal.esm' }
plugin:save()
```

```sh
jess run masters.jsl "Data Files/BalmoraTweaks.esp"
```

Master sizes come from the files beside the plugin, the way the command line finds them. For masters elsewhere in a load order, pass its config: `plugin:setMasters(names, { openmw = jess.openmw.load(config) })`.

`setHeader { author, description, fileType }`, `setMasters { ... }`, `syncMasters()`, and `updateRecordCount()` are the Luau versions of everything on this page; see the [plugin API](@/docs/api/plugin.md).

Removing a master a plugin actually uses leaves its references pointing at nothing. Know what you're taking out before you take it out: `jess plugin lint --overrides` lists what the plugin changes in its masters.

## Related

- [`plugin header`](@/docs/cli/plugin.md#header): every flag.
- [Converting ESP and ESM](@/docs/guides/converting-esp-esm.md): the header's file type.
