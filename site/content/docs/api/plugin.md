---
title: 'jess.plugin and Plugin'
description: 'Open, parse, create, and save plugins; every Plugin method.'
weight: 20
extra:
  kind: api
---

{{ api_signature(value="jess.plugin: { open, openAll, parse, new, assets, convert, checkRoundTrip }") }}

`jess.plugin` opens `.esp`, `.esm`, and `.omwaddon` files as `Plugin` handles. A handle holds the plugin's raw records and edits them in place. Records you don't touch keep their exact bytes when you save, and a plugin you don't touch at all saves byte for byte. Typed views are convenient; the original bytes are authoritative.

{% usage_note(title="Names resolve through OpenMW") %}
A path that exists is used as it is. Any other name is looked up in the data folders of your OpenMW config, the way `jess plugin` commands do, so `jess.plugin.open("Morrowind.esm")` finds the file OpenMW would load. Pass `openmw = false` to use plain paths only, or an [OpenMwContext](@/docs/api/openmw.md) to resolve through another config.
{% end %}

## Functions

### jess.plugin.open

{{ api_signature(value="jess.plugin.open(path: string, options: OpenOptions?) -> Plugin") }}

Reads and parses a plugin file.

- **Parameters:** `path`, a file path or a content name. `options`, an [OpenOptions](#openoptions) table.
- **Returns:** a [Plugin](#plugin) whose `path` is the resolved file.
- **Errors:** `'<path>' doesn't exist`, or `... isn't in the OpenMW data paths` when the name can't be resolved. `failed to parse '<path>': ...` for a damaged file. An invalid option value.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
print(plugin.path, #plugin, plugin.header.author)
```

Related: [jess.plugin.openAll](#jess-plugin-openall), [jess.plugin.parse](#jess-plugin-parse).

### jess.plugin.openAll

{{ api_signature(value="jess.plugin.openAll(paths: { string }, options: OpenOptions?) -> ({ Plugin }, { [string]: string })") }}

Opens many plugins in parallel, returning the ones that opened and why the others didn't.

- **Parameters:** `paths`, a list of paths or names. `options`, an [OpenOptions](#openoptions) table applied to every plugin. With `decode = true`, the records are decoded in parallel too.
- **Returns:** two values. The first is the plugins that opened, in the order given. The second maps each path that failed, as you spelled it, to its error message.
- **Errors:** only for invalid options. A plugin that can't be found or parsed goes into the failures table.

```luau
--!strict
local plugins, failed = jess.plugin.openAll(
  { 'MyMod.esp', 'Missing.esp' },
  { openmw = false, types = { 'LIGH' } }
)
print(#plugins) -- 1
for path, problem in failed do
  print(path, problem) -- Missing.esp  'Missing.esp' doesn't exist
end
```

### jess.plugin.parse

{{ api_signature(value="jess.plugin.parse(bytes: buffer | string, options: { encoding: Encoding? }?) -> Plugin") }}

Parses a plugin from bytes in memory.

- **Parameters:** `bytes`, the whole file. `options.encoding`, the text encoding (by default the config's `encoding=`, else `win1252`, as for [OpenOptions](#openoptions)).
- **Returns:** a [Plugin](#plugin) with no `path`. Save it with `plugin:save(path)`.
- **Errors:** `failed to parse plugin: ...`.

```luau
--!strict
local original = jess.plugin.open 'MyMod.esp'
local copy = jess.plugin.parse(original:bytes())
print(copy.path, #copy == #original) -- nil  true
```

### jess.plugin.new

{{ api_signature(value="jess.plugin.new(fields: { fileType: (\"plugin\" | \"master\")?, author: string?, description: string?, masters: { string | Master }?, encoding: Encoding? }?) -> Plugin") }}

Starts an empty plugin: just a `TES3` header.

- **Parameters:** `fileType`, `"plugin"` (the default) or `"master"`. `author` and `description`, the header text. `masters`, a list of names or `{ name, size }` tables. A master without a size gets it from the file the load order resolves it to. `encoding`, how the plugin's text is written: `"win1250"`, `"win1251"`, or `"win1252"`; by default the config's `encoding=`, else `"win1252"`.
- **Returns:** a [Plugin](#plugin) with no `path` and one record, the header.
- **Errors:** a `fileType` other than `"plugin"` or `"master"`, a master that's neither a string nor a table, or `master '<name>' isn't in the load order, so its size is unknown` for a master without a size that the load order doesn't have. Give such a master's size yourself: `{ name = "Other.esp", size = 0 }`.

```luau
--!strict
local plugin = jess.plugin.new {
  author = 'Me',
  description = 'Rocks',
  masters = { 'Morrowind.esm', { name = 'Tribunal.esm', size = 4565686 } },
}
plugin:insert { type = 'Static', id = 'my_rock', mesh = 'rock.nif' }
print(plugin:save('Rocks.esp').path)
```

Related: [Creating Plugins](@/docs/scripting/creating-plugins.md).

### jess.plugin.assets

{{ api_signature(value="jess.plugin.assets(plugins: (Plugin | string | { Plugin } | { string } | { Plugin | string })?, options: (ConfigOptions & ShowOptions & { encoding: Encoding?, meshes: boolean?, references: boolean?, releaseMemory: boolean?, data: (string | { string })? })?) -> PluginAssets") }}

Checks every file the plugins refer to, the way `jess plugin assets` does.

It covers models, icons, textures, sounds, voice lines, book images, Lua scripts, and the voice, music, and video files scripts play. Each is looked up the way OpenMW looks it up, in the plugin's own folder and then the load order. Each model, with its distant versions, also goes through the [NIF check](@/docs/api/nif.md#jess-nif-check).

- **Parameters:**
  - `plugins`: one plugin handle, path, or name, or a list of them. A handle's unsaved edits are included. With `nil`, the winning records of every active plugin in the load order are checked.
  - `options.meshes`: `false` skips the NIF check of models.
  - `options.references`: `false` leaves `references` and `usedFiles` empty. A whole load order has hundreds of thousands of references, and a script that only wants the findings shouldn't pay to turn each one into a table.
  - `options.releaseMemory`: `true` hands the memory the check freed back to the system when it's done. A whole load order's check reads hundreds of megabytes of plugins and models, and on Linux the allocator keeps that memory for the process otherwise; a script with more heavy work after the check, like the [doctor](@/docs/bundled-scripts/doctor.md) opening every plugin, peaks lower with it. Elsewhere it does nothing.
  - `options.data`: a folder or archive, or a list of them, to look files up in on top of the load order.
  - `options.show` and `options.verbose`: trim findings and models by severity, like `--show` and `--verbose` (see [ShowOptions](@/docs/api/nif.md#showoptions)).
  - `options.openmw`, `options.config`: which config to use (see [ConfigOptions](@/docs/api/nif.md#configoptions)). `options.encoding`: the plugins' text encoding, by default that config's `encoding=`, else `win1252`.
- **Returns:** a `PluginAssets` report:
  - `summary`: `plugins`, `unreadablePlugins`, `references`, `files`, `missing`, `meshesChecked`, `meshesWontLoad`, `byCode`, and `meshesByCode`.
  - `findings`: problems, most severe first. Each has `severity`, `code`, `message`, `plugin`, `recordType`, `id`, and `path`.
  - `references`: every reference. Each has `plugin`, `recordType`, `id`, `kind`, `path`, `resolved` (the VFS path OpenMW loads, `nil` when missing), and `source`.
  - `meshes`: a [NifFile](@/docs/api/nif.md#niffile) per model, plus `usedBy`, a list of `"TYPE id"` strings.
  - `usedFiles`: every file loaded, with `file` (absolute path of the loose file or archive), `path`, and `inArchive`.
- **Errors:** `checking the load order needs an OpenMW config` when `plugins` is `nil` and there's no config. A name that can't be resolved. An unknown `show` severity.

```luau
--!strict
local report = jess.plugin.assets('MyMod.esp', { meshes = false })
print(report.summary.missing)
for _, finding in report.findings do
  print(finding.code, finding.recordType, finding.id, finding.path)
end
```

Related: [Asset Checking](@/docs/guides/asset-checking.md), [jess plugin assets](@/docs/cli/plugin.md).

### jess.plugin.convert

{{ api_signature(value="jess.plugin.convert(path: string, fileType: \"master\" | \"plugin\", options: (ConfigOptions & { output: string?, overwrite: boolean? })?) -> { input: string, fileType: \"master\" | \"plugin\", written: { path: string, backup: string? } }") }}

Converts a `.esp` to a master, or a `.esm` to a plugin, the way `jess plugin esm` and `jess plugin esp` do, through the same code (`jess_plugin::files::convert_file_type`). Only the header's file type changes; every record after it is copied as it was.

- **Parameters:** `path` is a path, or a name looked up through the config like [jess.plugin.open](#jess-plugin-open). `fileType` is what to write, `"master"` or `"plugin"`. `options.output` is where to write; without it, the result goes next to the input with the new extension. `options.overwrite` replaces a file already there, after backing it up.
- **Returns:** the resolved `input`, the `fileType` written, and `written`, `{ path, backup }`, as every command that writes a file reports it.
- **Errors:** a `fileType` other than `"master"` or `"plugin"`, `input must be a .esp file` (or `.esm`), an output that's the input, `<output> already exists; pass overwrite = true to replace it`, or a file that can't be read or written. The input is never overwritten: converting a plugin to itself is refused, not attempted.

```luau
--!strict
local converted = jess.plugin.convert('BalmoraTweaks.esp', 'master')
print(converted.written.path)
```

Related: [Converting ESP and ESM](@/docs/guides/converting-esp-esm.md).

### jess.plugin.checkRoundTrip

{{ api_signature(value="jess.plugin.checkRoundTrip(plugin: string | buffer, options: (ConfigOptions & { excludeTypes: { string }?, ignoreCruft: boolean? })?) -> { input: string?, ok: boolean, error: string? }") }}

Checks that jess reads a plugin and writes it back byte for byte, like `jess plugin test-codec` (which only exists in debug builds of the CLI; this function is always there).

Byte-identical round trips are the promise everything else in jess rests on. If a plugin fails this, jess would change it on save without being asked to, and that's a jess bug worth reporting with the file attached.

- **Parameters:** `plugin` is a path or name (looked up like [jess.plugin.open](#jess-plugin-open)), or the plugin's bytes as a buffer. `options.excludeTypes` leaves record types out of the comparison, as tags or type names. `options.ignoreCruft` skips anything before the first `TES3`.
- **Returns:** `ok`, the `error` when it isn't (`failed to parse plugin: ...` or `codec changed bytes`), and the resolved `input` path (`nil` for bytes).
- **Errors:** a `plugin` that's neither a string nor a buffer, a name that can't be resolved, an unknown record type in `excludeTypes`. A plugin that fails the round trip isn't an error; it's `ok = false`.

```luau
--!strict
local result = jess.plugin.checkRoundTrip 'MyMod.esp'
print(result.ok, result.error) -- true  nil

local plugin = jess.plugin.open 'MyMod.esp'
print(jess.plugin.checkRoundTrip(plugin:bytes(), { excludeTypes = { 'CELL' } }).ok) -- true
```

## OpenOptions

{{ api_signature(value="type OpenOptions = { openmw: (boolean | OpenMwContext)?, config: string?, encoding: Encoding?, types: { string }?, decode: boolean?, cell: { header: boolean?, references: boolean? }? }") }}

Options for [jess.plugin.open](#jess-plugin-open) and [jess.plugin.openAll](#jess-plugin-openall).

| Key | Default | Effect |
| --- | --- | --- |
| `openmw` | `true` | `false` resolves names as plain paths only. An [OpenMwContext](@/docs/api/openmw.md#openmwcontext) resolves through that config. |
| `config` | the default config | An `openmw.cfg` path, or its folder, to resolve names through. It can't be combined with `openmw = false` or a context. |
| `encoding` | the config's `encoding=`, else `"win1252"` | Text encoding: `"win1250"`, `"win1251"`, or `"win1252"`, in any case. Left out, it's the `encoding=` of the config names resolve through (the default one, `config`, or an `openmw` context), the code page OpenMW reads content in; `"win1252"` when that config doesn't set one, when there's no config, or with `openmw = false`. A default config that's there but doesn't load is an error: `...; pass encoding to say which code page the text is in`. |
| `types` | every type | Keeps only these record types, as four-letter tags (`"LIGH"`, `"cell"`; `"npc"` means `NPC_`) or type names (`"Light"`), in any case. The header always stays. |
| `decode` | `false` | Decodes the kept records up front (in parallel with `openAll`), so the first field access doesn't pay for it. |
| `cell` | both `true` | `references = false` reads only each `CELL`'s header (`name`, `data`, `region`, `mapColor`, `waterHeight`, `atmosphereData`). `header = false` reads only its references. |

{% usage_note(title="Partial plugins can't be saved") %}
A plugin opened with `types` or with either `cell` part set to `false` raises an error from `save`, since saving would drop the records and parts it left out. Reading or writing a `CELL` part it left out raises an error too, so a missing part never looks empty. `clone` and `extract` keep a partial plugin partial.
{% end %}

```luau
--!strict
local plugin = jess.plugin.open('MyMod.esp', { types = { 'CELL' }, cell = { references = false } })
for record in plugin:records() do
  if jess.meta(record).tag ~= 'CELL' then continue end
  local cell = record :: CellRecord
  print(cell.name, cell.waterHeight)
end
```

## Plugin

A plugin handle. It's userdata: its fields are read-only properties, and its methods use `:`.

### plugin.path

{{ api_signature(value="plugin.path: string?") }}

The file the plugin was opened from or last saved to, or `nil` for a plugin from `parse`, `new`, or `extract`.

`save()` with no argument writes here, and `save(path)` changes it.

```luau
--!strict
print(jess.plugin.open('MyMod.esp').path, jess.plugin.new().path) -- MyMod.esp  nil
```

### plugin.encoding

{{ api_signature(value="plugin.encoding: Encoding") }}

The plugin's text encoding: `"win1250"`, `"win1251"`, or `"win1252"`.

```luau
--!strict
print(jess.plugin.open('MyMod.esp', { encoding = 'win1251' }).encoding) -- win1251
```

### plugin.header

{{ api_signature(value="plugin.header: { version: number, fileType: string, author: string, description: string, recordCount: number }") }}

A copy of the `TES3` header's fields (type `PluginHeader`).

`fileType` is `"plugin"` or `"master"`. `recordCount` is the number stored in the header. It follows inserts and deletes on its own; see [updateRecordCount](#plugin-updaterecordcount). Each read makes a fresh table; change the header with [setHeader](#plugin-setheader).

```luau
--!strict
local header = jess.plugin.open('MyMod.esp').header
print(header.fileType, header.recordCount, header.author)
```

### plugin.masters

{{ api_signature(value="plugin.masters: { Master }") }}

A copy of the master list: `{ name: string, size: number }` entries, in header order.

Change it with [setMasters](#plugin-setmasters) or [syncMasters](#plugin-syncmasters).

```luau
--!strict
for _, master in jess.plugin.open('MyMod.esp').masters do
  print(master.name, master.size)
end
```

### Length, iteration, and tostring

{{ api_signature(value="#plugin -> number; for record in plugin do ... end; tostring(plugin); plugin == other") }}

`#plugin` counts records, header included. `for record in plugin do` yields every [record handle](@/docs/api/record.md) in file order, the same as [plugin:records()](#plugin-records). `tostring` gives `Plugin(<path>, <n> records)`. Two handles are `==` when they're the same plugin, such as `jess.meta(record).plugin` and the plugin you opened; a [clone](#plugin-clone) is a different plugin.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
print(#plugin, tostring(plugin))
for record in plugin do
  local meta = jess.meta(record)
  print(meta.index, meta.tag, meta.id)
end
```

### plugin:records

{{ api_signature(value="plugin:records() -> () -> Record?") }}

Returns an iterator over every record handle, header first.

The iterator walks a snapshot of the records taken when you call `records()`. Records you delete during the loop are skipped, and records you insert aren't visited.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
for record in plugin:records() do
  if jess.meta(record).tag == 'LIGH' then print((record :: LightRecord).id) end
end
```

### plugin:record

{{ api_signature(value="plugin:record(index: number) -> Record?") }}

Returns the record handle at a 0-based record number, or `nil` past the end.

Record 0 is the `TES3` header. The numbers match `jess.meta(record).index` and `jess plugin dump`.

- **Errors:** `expected a record index (a whole number from 0), not <value>` for a negative number, a fraction, or anything else that isn't a number.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local header = plugin:record(0)
print(header and jess.meta(header).tag, plugin:record(#plugin)) -- TES3  nil
```

### plugin:find

{{ api_signature(value="plugin:find(recordType: string, id: string) -> Record?") }}

Returns the first record of a type with an id, ignoring case, or `nil`.

- **Parameters:** `recordType`, a four-letter tag (`"LIGH"`, `"npc"` for `NPC_`) or a type name (`"Light"`), in any case. `id`, the record id.
- **Errors:** an unknown record type.
- **Behavior:** lookups use an index built once per plugin change, so repeated finds don't rescan the plugin. Finding a few hundred ids in a loop that doesn't edit the plugin costs one pass, not a few hundred.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local light = plugin:find('LIGH', 'TEST_LIGHT') :: LightRecord?
print(light and light.id) -- test_light
```

### plugin:select

{{ api_signature(value="plugin:select(spec: SelectorSpec | Selector | nil) -> Selection") }}

Returns a live [Selection](@/docs/api/selection.md) of the records a spec picks, or of every record with `nil`.

- **Errors:** an unknown spec key, an invalid type, flag, or regex.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
print(plugin:select({ type = 'LIGH' }):count())
```

Related: [Selection and Selectors](@/docs/api/selection.md).

### plugin:layout

{{ api_signature(value="plugin:layout(spec: SelectorSpec | Selector | nil, options: { subrecords: boolean?, fields: boolean? }?) -> { RecordLayout }") }}

Returns the byte layout of the records a spec picks: what `jess plugin dump --raw` prints.

- **Parameters:** `spec`, as for [select](#plugin-select). `options.subrecords` (default `true`) includes each record's subrecords. `options.fields` (default `true`) names each subrecord's field and shows its value.
- **Returns:** a list of `RecordLayout`: `index`, `type`, `id`, `flags`, `size`, `range` (`start` and `end` byte offsets), and `subrecords`. Each `SubrecordLayout` has `index`, `type`, `size`, `range`, and, with `fields`, `field` and `value`.

```luau
--!strict
local layout = jess.plugin.open('MyMod.esp'):layout { type = 'LIGH' }
local first = layout[1]
print(first.id, first.size, first.range.start, first.range['end'])
for _, subrecord in first.subrecords or {} do
  print(subrecord.type, subrecord.field, subrecord.value)
end
```

### plugin:validate

{{ api_signature(value="plugin:validate(profile: (\"raw\" | \"tes3\" | \"omwaddon\" | \"omwsave\")?) -> PluginValidation") }}

Checks the plugin against a compatibility profile, like `jess plugin validate`, with the same checks and report.

- **Parameters:** `profile` (type `Profile`), default `"omwaddon"`. Every profile checks the file's structure. `"raw"` checks the TES3 framing permissively, `"tes3"` also against the vanilla Morrowind format, `"omwaddon"` also OpenMW's content-file record types and dialogue order, and that every record reads the way OpenMW's loaders read it, and `"omwsave"` against OpenMW's save format.
- **Returns:** a `PluginValidation`: `input` (the plugin's path, `nil` for one in memory), `checks` (the checks that ran, in camelCase: `structure`, `morrowindFormat`, `openmwRecordTypes`, `dialogueOrder`, `recordContents`, `saveMetadata`, `saveFormat`, `cellState`), `valid`, and `diagnostics`, one per problem with the byte `offset` it's at.
- **Errors:** an unknown profile.

```luau
--!strict
local validation = jess.plugin.open('MyMod.esp'):validate 'tes3'
print(validation.valid, table.concat(validation.checks, ', '))
for _, problem in validation.diagnostics do
  print(problem.offset, problem.message)
end
```

### plugin:stats

{{ api_signature(value="plugin:stats() -> { records: number, recordTypes: { [string]: number }, subrecordTypes: { [string]: number }, listSubrecords: { [string]: number } }") }}

Counts records and subrecords, like `jess plugin stats`.

`recordTypes` is keyed by tag (`"LIGH"`). `subrecordTypes` and `listSubrecords` are keyed by record and subrecord tag (`"CELL.NAME"`).

```luau
--!strict
local stats = jess.plugin.open('MyMod.esp'):stats()
print(stats.records, stats.recordTypes.LIGH, stats.subrecordTypes['LIGH.NAME'])
```

### plugin:lint

{{ api_signature(value="plugin:lint(options: { preset: (\"all\" | \"recommended\")?, checks: { string }?, openmw: (boolean | OpenMwContext)?, config: string? }?) -> { LintFinding }") }}

Runs lint checks, like `jess plugin lint`, and returns what they found. It reads the plugin and changes nothing.

- **Parameters:**
  - `preset`: `"recommended"` or `"all"`.
  - `checks`: check names, in camelCase or any spelling the CLI takes (`evilGmsts`, `evil_gmsts`, `evil-gmsts`). A preset and checks together run both.
  - With neither, the recommended preset runs, as `jess plugin lint` does with no checks named.
  - `openmw`, `config`: how masters resolve for the checks that read them. `openmw = false` uses plain paths. Either way, a plugin read from a file also finds masters in its own folder.
- **Returns:** a list of `LintFinding`s, `{ check, severity, message }`: `check` in camelCase, `severity` `"error"`, `"warning"`, or `"info"`. An empty list means nothing was found.
- **Errors:** an unknown check or preset, a `checks` that isn't a list.

A finding's severity comes from its check:

| Severity | Means | Checks |
| --- | --- | --- |
| `error` | The plugin is broken for players. | `missingMaster`, `bloodmoonDependency`, `expansionDependency` |
| `warning` | Dirt, or a likely mistake. | `duplicateRecords`, `evilGmsts`, `cellParams`, `junkCells`, `recordCount`, `masterSync`, `fogbug`, `fogsync`, `cell00`, `deprecatedLists`, `dialogueTeleports`, `duplicateInfo`, `modifiedInfo`, `modifiedInfoId` |
| `info` | Worth a look, often done on purpose. | `missingAuthor`, `missingDescription`, `missingVersion`, `overrides`, `autocalcSpells`, `getsoundplaying`, `menumode`, `noBloodmoonFunctions`, `noTribunalFunctions`, `scriptedDoors` |

The recommended preset is `junkCells`, `missingVersion`, `fogbug`, `fogsync`, `bloodmoonDependency`, `deprecatedLists`, `dialogueTeleports`, `expansionDependency`, and `modifiedInfoId`. [Linting](@/docs/guides/linting.md) explains each check.

`jess plugin lint --fail-on` has no Luau twin, because a script can decide for itself what's fatal:

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local counts: { [string]: number } = { error = 0, warning = 0, info = 0 }
for _, finding in plugin:lint { preset = 'all' } do
  counts[finding.severity] += 1
  if finding.severity ~= 'info' then print(finding.severity, finding.check, finding.message) end
end
print(`{counts.error} errors, {counts.warning} warnings, {counts.info} notes`)
```

### plugin:clean

{{ api_signature(value="plugin:clean(options: { duplicateRecords: boolean?, evilGmsts: boolean?, junkCells: boolean?, cellParams: boolean?, openmw: (boolean | OpenMwContext)?, config: string? }?) -> { removedRecords: number, removedSubrecords: number }") }}

Removes safe dirt from the plugin in memory, like `jess plugin clean`. Nothing is written until you `save`.

Cleaning is normalization, and normalization is a decision: records nobody asked about stay as they were, and a plugin with nothing to clean keeps its bytes.

- **Parameters:** the cleaners set to `true` run, and only those; with none set to `true`, every cleaner runs except the ones set to `false`, like `jess plugin clean` with no cleanup flags:
  - `duplicateRecords`: records duplicated within the plugin, or identical to a master's copy.
  - `evilGmsts`: the known dirty `GMST` records.
  - `junkCells`: conservatively empty `CELL` records.
  - `cellParams`: repeated `CELL` header subrecords.

  `openmw` is how masters resolve for the comparisons against them; `false` uses plain paths.
- **Returns:** how many records and subrecords were removed.
- **Errors:** a master that can't be found or read.
- **Behavior:** record handles follow their records past the removed ones, and the header's record count follows the removals.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local report = plugin:clean { junkCells = false, cellParams = false }
print(report.removedRecords, report.removedSubrecords)
```

Related: [Cleaning Plugins](@/docs/guides/cleaning-plugins.md).

### plugin:bytes

{{ api_signature(value="plugin:bytes() -> buffer") }}

Returns the whole plugin as it would be saved, pending edits included.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
print(buffer.len(plugin:bytes()))
```

### plugin:clone

{{ api_signature(value="plugin:clone() -> Plugin") }}

Returns an independent copy of the plugin, edits included. The copy has no `path`: save it with one, so it can't overwrite the original by accident.

Record handles into the original don't reach the copy. A partial plugin's copy is partial too.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
local copy = plugin:clone()
copy:select({ type = 'GMST' }):delete()
print(#plugin > #copy, copy.path) -- true  nil
copy:save 'MyMod-no-gmsts.esp'
```

### plugin:save

{{ api_signature(value="plugin:save(path: string?, options: { backup: boolean? }?) -> { path: string, backup: string? }") }}

Writes the plugin to `path`, or to its own path.

- **Parameters:** `path`, where to write. `options.backup` (default `true`) keeps a `.bak` copy of any file it overwrites.
- **Returns:** the path written and the backup's path (`nil` when no file was overwritten or `backup = false`). The plugin's `path` becomes the written one.
- **Errors:** `this plugin has no path; pass one to save`. Saving a partial plugin. A pending field edit that can't be encoded (the error names the record). A write failure.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
plugin:setHeader { author = 'Me' }
local written = plugin:save()
print(written.path, written.backup) -- MyMod.esp  MyMod.esp.bak
```

### plugin:insert

{{ api_signature(value="plugin:insert(value: RecordFields | RecordValue, at: number?) -> Record") }}

Adds a record built from a table, filling the fields you leave out with the type's defaults.

- **Parameters:** `value`, a table with `type`, a type name (`"Static"`) or tag (`"STAT"`), and any fields; a whole record from `decode()` or `toTable()` works as it is. `at`, the 0-based record number the new record gets, from 1 (right after the header) to `#plugin` (the end, the default).
- **Returns:** the new record's handle.
- **Errors:** a missing or unknown `type`, `unknown field '<name>'` for a name the type doesn't have, a value of the wrong shape for a field, or `at` out of range.
- **Behavior:** the header's record count follows the insert.

```luau
--!strict
local plugin = jess.plugin.new()
local rock = plugin:insert { type = 'Static', id = 'my_rock', mesh = 'rock.nif' }
local lamp = plugin:insert({ type = 'LIGH', id = 'my_lamp' }, 1)
print(jess.meta(rock).index, jess.meta(lamp).index) -- 2  1
```

Related: [Record](@/docs/api/record.md), [Record and NIF Types](@/docs/api/record-types.md).

### plugin:setHeader

{{ api_signature(value="plugin:setHeader(fields: { author: string?, description: string?, fileType: (\"master\" | \"plugin\")? }) -> boolean") }}

Changes header fields, returning whether anything changed.

- **Errors:** an unknown `fileType`.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
print(plugin:setHeader { author = 'Me', fileType = 'master' }, plugin.header.fileType) -- true  master
```

Related: [Converting ESP and ESM](@/docs/guides/converting-esp-esm.md).

### plugin:updateRecordCount

{{ api_signature(value="plugin:updateRecordCount() -> boolean") }}

Sets the header's record count to the number of records after it, returning whether it changed.

Inserting and deleting records (`insert`, `delete`, `clean`, a selection's `delete`) set the count for you, the next time anything reads the plugin: its `header`, `bytes`, or `save`. A plugin you never add or remove records from keeps the count it was opened with, byte for byte, even a wrong one. Call this to correct such a count.

```luau
--!strict
local plugin = jess.plugin.new()
plugin:insert { type = 'Static', id = 'my_rock' }
print(plugin.header.recordCount, plugin:updateRecordCount()) -- 1  false
```

### plugin:setMasters

{{ api_signature(value="plugin:setMasters(masters: { string | Master }, options?) -> boolean") }}

Replaces the master list, returning whether it changed.

A master without a size gets its file's size, found the way [syncMasters](#plugin-syncmasters) finds it: through `options.openmw` or `options.config` when given, else the script's default config, then in the folder the plugin was read from. `{ name, size }` tables are used as given.

- **Errors:** `master '<name>' isn't in the load order, so its size is unknown`, for a master without a size that can't be found that way. Give its size yourself (`{ name = "Other.esp", size = 0 }`), or pass the load order's config.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
print(plugin:setMasters { 'Morrowind.esm', { name = 'Tribunal.esm', size = 4565686 } })
```

Related: [Masters and Headers](@/docs/guides/masters-and-headers.md).

### plugin:syncMasters

{{ api_signature(value="plugin:syncMasters(options: { openmw: (boolean | OpenMwContext)?, config: string? }?) -> { { name: string, oldSize: number, newSize: number, path: string } }") }}

Updates each master's recorded size to the size of the file it resolves to: through the config (or as plain paths with `openmw = false`), then in the folder the plugin was read from.

The Construction Set writes each master's file size into the header, and Morrowind.exe warns that the master has changed when it no longer matches. OpenMW ignores the size entirely. Sync after a master changes size if the plugin still has to load in the original engine.

- **Returns:** one entry per master whose size changed, with the file it was measured from.
- **Errors:** `failed to resolve TES3 master '<name>'` when a master can't be found.

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
for _, change in plugin:syncMasters() do
  print(change.name, change.oldSize, '->', change.newSize, change.path)
end
plugin:save()
```
