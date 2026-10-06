---
title: 'jess.nif, Nif, and NifRecord'
description: 'Check, info, dump, fix, and edit NIFs; every Nif and NifRecord member.'
weight: 60
extra:
  kind: api
---

{{ api_signature(value="jess.nif: { check, checkBytes, info, infoBytes, sceneBytes, dump, dumpBytes, fix, open, openBytes, new }") }}

`jess.nif` has two halves. `check`, `info`, `dump`, and `fix` are the `jess nif` commands as functions: they read NIFs and KFs from every game OpenMW supports and return reports. `open`, `openBytes`, and `new` give you a `Nif` handle to edit a Morrowind NIF record by record.

The split is deliberate. Reading follows OpenMW's loaders, so jess reads what OpenMW reads. Other games' records are read only as far as OpenMW reads them, and writing that back would drop the rest, so jess only writes Morrowind NIFs (4.0.0.2), whose records it models completely.

{% usage_note(title="Inputs and the load order") %}
Functions that take `inputs` accept a NIF or KF file, a data folder, or a BSA/BA2 archive, or a list of them. Paths that aren't files are looked up as VFS paths (`meshes/x/rock.nif`) in your load order. Textures resolve through the inputs first and then the load order, the way OpenMW finds them. A config that can't be loaded is an error; without inputs (the whole load order) a missing config is one too. `openmw = false` works without a config.
{% end %}

## Functions

### jess.nif.check

{{ api_signature(value="jess.nif.check(inputs: (string | { string })?, options: (ConfigOptions & ShowOptions & { data: (string | { string })? })?) -> NifCheck") }}

Checks NIFs the way OpenMW loads them, like `jess nif check`: what won't load, what draws or collides wrong, missing textures, and performance smells.

- **Parameters:** `inputs`, files, folders, or archives; `nil` checks every NIF in the load order. `options.data`, folders or archives to look textures up in without checking them. `options.show` and `options.verbose`, see [ShowOptions](#showoptions). `options.openmw` and `options.config`, see [ConfigOptions](#configoptions).
- **Returns:** a [NifCheck](#nifcheck).
- **Errors:** `checking the load order needs an OpenMW config` (no inputs, no config), `unknown severity '<name>': ...`, or a config that can't be loaded.

```luau
--!strict
local report = jess.nif.check('rock.nif', { openmw = false })
print(report.summary.files, report.summary.wontLoad)
for _, file in report.files do
  for _, finding in file.findings do
    print(file.path, finding.severity, finding.code, finding.message)
  end
end
```

Related: [NIF Checking Rules](@/docs/nif/checking-rules.md), [Checking and fixing NIFs](@/docs/guides/nif-checking-and-fixing.md).

### jess.nif.checkBytes

{{ api_signature(value="jess.nif.checkBytes(bytes: buffer | string, path: string?, options: ConfigOptions?) -> NifFile") }}

Checks one NIF held in memory, with its textures resolved through the load order.

- **Parameters:** `bytes`, the file. `path`, the VFS path to report it under (default `""`). `options`, see [ConfigOptions](#configoptions).
- **Returns:** a [NifFile](#niffile).

```luau
--!strict
local bytes = jess.fs.read 'rock.nif'
if bytes then
  local file = jess.nif.checkBytes(bytes, 'meshes/x/rock.nif', { openmw = false })
  print(file.worst, file.stats and file.stats.triangles)
end
```

### jess.nif.info

{{ api_signature(value="jess.nif.info(inputs: string | { string }, options: (ConfigOptions & { data: (string | { string })?, tree: boolean? })?) -> { NifInfo }") }}

Describes NIFs, like `jess nif info`: header, record types, scene graph, collision setup, statistics, and textures with where OpenMW finds them.

- **Parameters:** `inputs`, at least one file, folder, or archive. `options.tree = false` leaves out the scene graph, like `--no-tree`. `options.data`, extra folders or archives to look textures up in.
- **Returns:** one [NifInfo](#nifinfo) per file.
- **Errors:** `jess.nif.info needs at least one file, folder, or archive`.

```luau
--!strict
for _, info in jess.nif.info('rock.nif', { openmw = false }) do
  print(info.version, info.game, info.recordCount, info.collision and info.collision.source)
  for _, root in info.tree do
    print(root.type, root.name, root.children and #root.children)
  end
end
```

### jess.nif.infoBytes

{{ api_signature(value="jess.nif.infoBytes(bytes: buffer | string, path: string?, options: ConfigOptions?) -> NifInfo") }}

Describes one NIF held in memory; `path` names it in the result.

```luau
--!strict
local bytes = jess.fs.read 'rock.nif'
if bytes then
  print(jess.nif.infoBytes(bytes, 'meshes/x/rock.nif', { openmw = false }).recordCount)
end
```

### jess.nif.sceneBytes

{{ api_signature(value="jess.nif.sceneBytes(bytes: buffer | string) -> { [string]: any }") }}

What a renderer draws from a Morrowind NIF, as jess-nif builds it: nodes with local and world transforms, drawables, geometries, materials with the state OpenMW gives them, textures, skins and their bones, the packed geometry buffers, and the issues it doesn't translate. Records are their indices, transforms are three matrix columns and a translation, and `text` is the scene as a readable dump.

```luau
--!strict
local bytes = jess.fs.read 'rock.nif'
if bytes then
  print(jess.nif.sceneBytes(bytes).text)
end
```

### jess.nif.dump

{{ api_signature(value="jess.nif.dump(input: string, options: (ConfigOptions & NifDumpOptions & { data: (string | { string })? })?) -> NifDump") }}

Returns every record of one NIF or KF with its fields, like `jess nif dump`.

- **Parameters:** `input`, one file, or a VFS path in the load order (`options.data` adds folders or archives to find it in). `options.type`, `index`, `name`, and `list` narrow the output; see [NifDumpOptions](#nifdumpoptions).
- **Returns:** a [NifDump](#nifdump). Links are 0-based record indices, and `type` names each record's type. Morrowind records have jess's editable fields (the same shape as [NifRecord:decode](#nifrecord-decode)); other games' records have the fields OpenMW reads.
- **Errors:** `jess.nif.dump takes one NIF or KF file` (a folder or archive), or a file that can't be parsed.

```luau
--!strict
local dump = jess.nif.dump('rock.nif', { openmw = false, type = 'NiTriShape', list = true })
print(dump.path, #dump.records)
for _, record in dump.records do
  print(record.index, record.type)
end
```

`records` holds every NIF record type, so narrow a record by `type` before reading its own fields (`if record.type == "NiTriShape" then`).

### jess.nif.dumpBytes

{{ api_signature(value="jess.nif.dumpBytes(bytes: buffer | string, path: string?, options: NifDumpOptions?) -> NifDump") }}

`dump` for a NIF held in memory; `path` names it in errors and the result, whose `source` is `nil`. With `reader = 'gamebryo'` every version, Morrowind's included, is read by the reader OpenMW uses rather than the editable model, so its records have the same fields whatever game the file is from.

```luau
--!strict
local bytes = jess.fs.read 'rock.nif'
if bytes then
  local dump = jess.nif.dumpBytes(bytes, 'rock.nif', { index = { 0, 1 } })
  print(#dump.records, dump.records[2].type) -- 2  NiTriShape
end
```

### jess.nif.fix

{{ api_signature(value="jess.nif.fix(inputs: (string | { string })?, options: (ConfigOptions & { data: (string | { string })?, only: { string }?, dryRun: boolean?, output: string? })?) -> NifFixReport") }}

Repairs the problems `check` finds that have one clear fix, like `jess nif fix`. It edits Morrowind NIFs only.

- **Parameters:**
  - `inputs`: files, folders, or archives; `nil` fixes the whole load order, which needs `output` or `dryRun`.
  - `options.only`: the fixes to apply, from `rcn`, `ncc`, `markers`, `skinWeights`, and `unreachable` (default: all). [NIF Checking Rules](@/docs/nif/checking-rules.md) describes each.
  - `options.dryRun`: report what would change without writing.
  - `options.output`: a data folder to write fixed files into, at their VFS paths. A file already there is backed up before it's replaced. Without `output`, each file is fixed in place with a `.bak` backup.
  - `options.data`: folders or archives to look textures up in without fixing them.
- **Returns:** `{ summary, files }`. `summary` has `files`, `fixed`, `written`, `skipped`, and `byFix` (counts by fix). Each file has `path`, `source`, `fixes` (`{ fix, message, record? }`), `written` (`{ path, backup? }`, when written), and `skipped` (why not, when it wasn't).
- **Errors:** `fixing the whole load order in place would edit other mods' files; pass output or dryRun`, `unknown fix '<name>'`.

```luau
--!strict
local report = jess.nif.fix('ncc.nif', { openmw = false, dryRun = true })
for _, file in report.files do
  for _, fix in file.fixes do
    print(file.path, fix.fix, fix.message)
  end
end
```

### jess.nif.open

{{ api_signature(value="jess.nif.open(input: string, options: (ConfigOptions & { data: (string | { string })? })?) -> Nif") }}

Opens a Morrowind NIF to edit: a file, or a VFS path in the load order.

- **Returns:** a [Nif](#nif) whose `path` is the file it came from.
- **Errors:** `jess.nif.open takes one NIF or KF file`, `<path> is a <game> <version> NIF; jess only edits Morrowind (4.0.0.2) NIFs so far`, or a parse error.

```luau
--!strict
local nif = jess.nif.open('rock.nif', { openmw = false })
print(nif.path, nif.count, #nif:roots())
```

Related: [Editing NIFs](@/docs/nif/editing.md).

### jess.nif.openBytes

{{ api_signature(value="jess.nif.openBytes(bytes: buffer | string, path: string?) -> Nif") }}

Opens a Morrowind NIF held in memory; `path` names it in errors. The handle has no `path`, so save it with one.

```luau
--!strict
local bytes = jess.fs.read 'rock.nif'
if bytes then
  local nif = jess.nif.openBytes(bytes, 'rock.nif')
  print(nif.path, nif.count) -- nil  20
end
```

### jess.nif.new

{{ api_signature(value="jess.nif.new() -> Nif") }}

Starts an empty Morrowind NIF: insert records, then `setRoots`.

```luau
--!strict
local nif = jess.nif.new()
local root = nif:insert { type = 'NiNode', name = 'Root' }
nif:setRoots { root }
print(nif.count, buffer.len(nif:bytes()))
```

Related: [Building from Scratch](@/docs/nif/building-from-scratch.md).

## Option tables

### ConfigOptions

{{ api_signature(value="type ConfigOptions = { openmw: (boolean | OpenMwContext)?, config: string? }") }}

Which OpenMW config a call looks files up in. Also taken by `nif:check`, `jess.plugin.assets`, `jess.compare.bytes`, `jess.save.check`, and `jess.save.strip`.

| Key | Effect |
| --- | --- |
| `openmw` | `false` goes without a config; an [OpenMwContext](@/docs/api/openmw.md#openmwcontext) uses that config. |
| `config` | An `openmw.cfg` path, or its folder, to use instead of the default config. It can't be combined with `openmw = false` or a context. |

### ShowOptions

{{ api_signature(value="type ShowOptions = { show: Severity?, verbose: boolean? }") }}

How much of a report to keep, like `--show` and `--verbose`. Without either, you get the whole report.

- `show`: a severity. Findings less severe are left out, and so are files (or models) left without any.
- `verbose = true`: keep every file and model, and every finding.

Severities, most severe first: `wontLoad`, `broken`, `missingAsset`, `collision`, `performance`, `info`.

```luau
--!strict
local report = jess.nif.check({ 'rock.nif', 'ncc.nif' }, { openmw = false, show = 'collision' })
print(#report.files) -- 1: only ncc.nif has a finding that severe
```

### NifDumpOptions

{{ api_signature(value="type NifDumpOptions = { type: (string | { string })?, index: (number | { number })?, name: string?, list: boolean? }") }}

| Key | Keeps |
| --- | --- |
| `type` | Records of these types (`"NiTriShape"`). |
| `index` | Records with these 0-based indices. |
| `name` | Records whose name matches this case-insensitive regex. |
| `list` | Just each record's `index`, `type`, and `name` (when `true`). |

## Reports

### NifCheck

{{ api_signature(value="type NifCheck = { summary: { texturesChecked, loadOrderChecked, files, deeplyChecked, wontLoad, byWorstSeverity, byCode }, files: { NifFile } }") }}

What `check` returns, the camelCase form of `jess nif check --format json`. `summary.texturesChecked` says whether textures were looked up at all, and `summary.loadOrderChecked` whether the load order was searched (with `openmw = false`, textures only resolve against the inputs' data folders and `data`). `summary.byWorstSeverity` counts files by their worst finding, and `summary.byCode` counts findings by code.

### NifFile

{{ api_signature(value="type NifFile = { path, source, version?, game?, deeplyChecked, worst?, findings: { NifFinding }, textures, stats: NifStats? }") }}

One checked file.

| Field | Meaning |
| --- | --- |
| `path` | The VFS path, such as `meshes/f/furn_de_bed_01.nif`. |
| `source` | The data folder, archive, or file it came from. |
| `version`, `game` | From the header, such as `4.0.0.2` and `Morrowind`. |
| `deeplyChecked` | Whether the records were checked, not just the header. |
| `worst` | The most severe finding's severity, or `nil` with none. |
| `findings` | `NifFinding`s: `severity`, `code`, `message`, and `record` (the index it's about). |
| `textures` | `{ path, resolved?, source? }`: textures as the NIF names them, the VFS path OpenMW loads (`nil` when missing), and where that file is. |
| `stats` | `NifStats`: `records`, `reachableRecords`, `nodes`, `shapes`, `vertices`, `triangles`, `collisionTriangles`, `controllers`, `embeddedTextures`, `embeddedTextureBytes`, `skinned`. |

### NifInfo

{{ api_signature(value="type NifInfo = { path, source, version?, game?, userVersion?, bethVersion?, author?, recordCount?, recordTypes, error?, collision?, stats?, textures, tree: { NifTreeNode } }") }}

One described file, the camelCase form of `jess nif info --format json`.

- `recordTypes` counts records by type. `error` says why the records couldn't be read, when they couldn't.
- `collision` is `{ source, node?, hidesMarkers, bsxFlags? }`, where `source` is `rootCollisionNode`, `cameraOnly`, `renderGeometry`, or `none`.
- `tree` is the scene graph from each root. A `NifTreeNode` has `index`, `type`, `name`, and, when not empty, `hidden`, `controllers` (`{ index, type, active }`), `properties` and `effects` (`NifRecordRef`s: `{ index, type }`), `extraData` (`{ index, type, value? }`), `collision` (a `NifRecordRef`), `shape` (`{ data?, vertices, triangles, skin? }`), and `children`. A record reached a second time, through a shared or looping link, is listed without its `children`, and so is one 256 levels down.

### NifDump

{{ api_signature(value="type NifDump = { path: string, source: string?, roots: { number? }, records: { ... } }") }}

What `dump` returns: the file's `path`, the folder or archive it came from (`nil` for `dumpBytes` and `nif:dump`), the root record indices, and the records, each with its `index`.

## Nif

A handle to a Morrowind NIF being edited. Its fields are read-only properties. Two handles are `==` when they're the same NIF, so `jess.meta(record).nif == nif` tells whether a record belongs to `nif`. `#nif` counts its records, like `nif.count`, and `for record in nif do` iterates them in index order, like [nif:records()](#nif-records).

Only records reachable from the roots are saved: a NIF has no way to mark a record as garbage other than nothing linking to it, so jess drops what nothing links to. Saving numbers records in file order, so indices of records you inserted can change after `save`. Handles don't; keep those.

### nif.path

{{ api_signature(value="nif.path: string?") }}

Where the NIF saves by default: the file it was opened from, or where it was last saved. `nil` for `new` and `openBytes` until the first `save(path)`.

```luau
--!strict
print(jess.nif.open('rock.nif', { openmw = false }).path, jess.nif.new().path) -- rock.nif  nil
```

### nif.count

{{ api_signature(value="nif.count: number") }}

How many records the NIF holds, reachable or not.

```luau
--!strict
print(jess.nif.open('rock.nif', { openmw = false }).count) -- 20
```

### nif:records

{{ api_signature(value="nif:records() -> () -> NifRecord?") }}

Returns an iterator over every record handle, in index order.

```luau
--!strict
for record in jess.nif.open('rock.nif', { openmw = false }):records() do
  local meta = jess.meta(record)
  -- Records without a name (NiTriShapeData) read it as nil.
  print(meta.index, meta.typeName, (record :: NiObjectNETRecord).name)
end
```

### nif:record

{{ api_signature(value="nif:record(index: number) -> NifRecord?") }}

Returns the record at an index, or `nil`. An index that isn't a whole number from 0 is an error, as for [plugin:record](@/docs/api/plugin.md#plugin-record).

```luau
--!strict
local nif = jess.nif.open('rock.nif', { openmw = false })
local root = nif:record(0)
print(root and jess.meta(root).typeName, nif:record(999)) -- NiNode  nil
```

### nif:find

{{ api_signature(value="nif:find(spec: { type: (string | { string })?, name: string? }?) -> { NifRecord }") }}

Returns the records of any of `type` (ignoring case) whose name matches `name`, a case-insensitive regex. With no spec, every record.

```luau
--!strict
local nif = jess.nif.open('rock.nif', { openmw = false })
for _, shape in nif:find { type = 'NiTriShape', name = '^cone' } do
  print(jess.meta(shape).index, (shape :: NiTriShapeRecord).name)
end
```

### nif:roots

{{ api_signature(value="nif:roots() -> { NifRecord }") }}

Returns the root records, skipping any that were deleted.

```luau
--!strict
local root = jess.nif.open('rock.nif', { openmw = false }):roots()[1] :: NiNodeRecord
print(jess.meta(root).typeName, root.name)
```

### nif:setRoots

{{ api_signature(value="nif:setRoots(roots: { NifRecord | number }) -> ()") }}

Replaces the root list with record handles or indices.

- **Errors:** `there's no record <index>`, `this record has been deleted` (a deleted record's handle), or `that record belongs to a different NIF`.

```luau
--!strict
local nif = jess.nif.new()
nif:setRoots { nif:insert { type = 'NiNode', name = 'Root' } }
print(#nif:roots())
```

### nif:insert

{{ api_signature(value="nif:insert(value: NifRecordFields | NifRecordValue) -> NifRecord") }}

Adds a record built from a table, filling the fields you leave out with the type's defaults.

- **Parameters:** `value`, a table with `type` (a NIF record type such as `"NiNode"`) and any fields. Link fields take record handles or indices. New geometry gets its bound and normals computed.
- **Returns:** a handle that stays valid while other records come and go.
- **Errors:** `a record table needs a type, such as "NiNode"`, `unknown NIF record type '<type>'`, or a field of the wrong shape.

```luau
--!strict
local nif = jess.nif.new()
local texture = nif:insert { type = 'NiSourceTexture', source = { external = 'tx_jess_crate.dds' } }
local texturing =
  nif:insert { type = 'NiTexturingProperty', textureMaps = { base = { texture = texture } } }
local root = nif:insert { type = 'NiNode', name = 'Root', properties = { texturing } }
nif:setRoots { root }
```

### nif:shape

{{ api_signature(value="nif:shape(spec: { name?, vertices, triangles, normals?, vertexColors?, uvs?, uvSets?, texture?, material?, properties?, parent?, translation?, rotation?, scale? }) -> NifRecord") }}

Adds a triangle shape and its data in one call, with a texture and a material when asked for, attached to a parent.

- **Parameters:**
  - `vertices` (`{ {x, y, z} }`) and `triangles` (`{ {a, b, c} }`, 0-based vertex indices): required.
  - `normals`, `vertexColors`: per vertex.
  - `uvs` is one set of texture coordinates; `uvSets` is several. Pass one or the other.
  - `texture`: a texture path; adds an `NiSourceTexture` and an `NiTexturingProperty` with it as the base map.
  - `material`: `NiMaterialProperty` fields; adds that property.
  - `properties`: more property records or indices to attach.
  - `parent`: a node to add the shape to as a child.
  - `name`, `translation`, `rotation` (a 3×3 matrix as 9 numbers), `scale`: the shape's own.
- **Returns:** the `NiTriShape`.
- **Errors:** `unknown Nif:shape option '<key>': expected material, name, ...` for unknown keys, `pass uvs or uvSets, not both`, a `texture` that isn't a string, a `material` that isn't a table, or `only nodes have children` for a parent that isn't a node.

```luau
--!strict
local nif = jess.nif.new()
local root = nif:insert { type = 'NiNode', name = 'Root' }
nif:setRoots { root }
nif:shape {
  name = 'Crate',
  vertices = { { -64, -64, 0 }, { 64, -64, 0 }, { 64, 64, 0 }, { -64, 64, 0 } },
  triangles = { { 0, 1, 2 }, { 0, 2, 3 } },
  uvs = { { 0, 1 }, { 1, 1 }, { 1, 0 }, { 0, 0 } },
  texture = 'tx_jess_crate.dds',
  material = { alpha = 0.5 },
  parent = root,
}
nif:save 'crate.nif'
```

### nif:updateBounds

{{ api_signature(value="nif:updateBounds() -> number") }}

Recomputes the bound of every geometry data record, returning how many.

Edits that change vertices already do this for their record; call it after changing geometry some other way.

```luau
--!strict
print(jess.nif.open('rock.nif', { openmw = false }):updateBounds()) -- 7
```

### nif:textures

{{ api_signature(value="nif:textures() -> { { record: NifRecord, path: string } }") }}

Lists every external texture path, as the file spells it, with its `NiSourceTexture` record.

```luau
--!strict
local nif = jess.nif.open('crate.nif', { openmw = false })
for _, texture in nif:textures() do
  print(jess.meta(texture.record).index, texture.path)
end
```

### nif:setTexture

{{ api_signature(value="nif:setTexture(from: string, to: string) -> number") }}

Points every texture spelled `from` (ignoring case and slash direction) at `to`, returning how many changed.

```luau
--!strict
local nif = jess.nif.open('crate.nif', { openmw = false })
print(nif:setTexture('TX_JESS_CRATE.DDS', 'tx_crate_new.dds')) -- 1
```

### nif:bytes

{{ api_signature(value="nif:bytes() -> buffer") }}

Returns the NIF as it would be saved.

- **Errors:** a record that can't be written, such as geometry with more vertices than its 16-bit count holds. A link to a deleted record isn't one: it saves as an empty link.

```luau
--!strict
print(buffer.len(jess.nif.open('rock.nif', { openmw = false }):bytes()))
```

### nif:save

{{ api_signature(value="nif:save(path: string?, options: { backup: boolean? }?) -> { path: string, backup: string? }") }}

Writes the NIF to `path` or its own path, keeping a `.bak` of any file it overwrites unless `backup = false`.

- **Returns:** the path written and the backup's path. The NIF's `path` becomes the written one.
- **Errors:** `this NIF has no path; pass one to save`, or a write failure.

```luau
--!strict
local nif = jess.nif.open('rock.nif', { openmw = false })
local written = nif:save('rock-copy.nif', { backup = false })
print(written.path, nif.path) -- rock-copy.nif  rock-copy.nif
```

### nif:check

{{ api_signature(value="nif:check(options: ConfigOptions?) -> NifFile") }}

Checks the NIF as it is now, like [jess.nif.checkBytes](#jess-nif-checkbytes) on `nif:bytes()`: textures resolve through the load order, or the config [ConfigOptions](#configoptions) pick.

```luau
--!strict
local nif = jess.nif.open('rock.nif', { openmw = false })
print(nif:check().worst)
```

### nif:fix

{{ api_signature(value="nif:fix(only: { string }?) -> { { fix: string, message: string, record: number? } }") }}

Applies the fixes `jess nif fix` would, all of them or those in `only`, returning what changed.

```luau
--!strict
local nif = jess.nif.open('ncc.nif', { openmw = false })
for _, fixed in nif:fix { 'ncc' } do
  print(fixed.fix, fixed.message, fixed.record)
end
```

### nif:dump

{{ api_signature(value="nif:dump(options: NifDumpOptions?) -> NifDump") }}

Dumps the NIF as it is now, like [jess.nif.dump](#jess-nif-dump).

```luau
--!strict
local dump = jess.nif.open('rock.nif', { openmw = false }):dump { type = 'NiNode' }
print(#dump.records, dump.roots[1])
```

## NifRecord

A handle to one record in a [Nif](#nif). It stays valid while other records are inserted or deleted. Once its record is deleted, every access raises `this record has been deleted`.

Every name on it is its record's field or one of the methods below, like a plugin record's; its index, type, and NIF are [jess.meta](#nifrecord-metadata)'s. The fields are live views: `node.children[1].name` reads through a link, since links read as record handles (`nil` when empty). Cast a handle to its type to reach them in typed code: `local node = record :: NiNodeRecord`. Writing a link field takes a handle or an index. Views and lists work as they do for [plugin records](@/docs/api/record.md#views-and-lists).

`tostring(record)` gives `NiTriShape 1 "Cone"`, and `==` compares handles to the same record of the same NIF.

```luau
--!strict
local nif = jess.nif.open('rock.nif', { openmw = false })
local root = nif:roots()[1] :: NiNodeRecord
root.name = 'Rock'
for index, child in root.children do
  if child then print(index, jess.meta(child).typeName, (child :: NiAVObjectRecord).name) end
end
```

### NifRecord metadata

{{ api_signature(value="jess.meta(record: NifRecord) -> NifRecordMeta") }}

[jess.meta](@/docs/api/jess.md#jess-meta) returns what the handle knows about its record besides the record's fields, read when asked:

| Property | Type | What it is |
| --- | --- | --- |
| `index` | `number` | The record's index: what links to it hold. Records inserted later continue the numbering, or take a deleted record's index; saving renumbers in file order. |
| `typeName` | `string` | The record's type, such as `"NiNode"`, as `decode()` spells `type`. |
| `nif` | `Nif` | The NIF the record belongs to. |

A record's name isn't here, because it's data: `node.name`, for the types that have one (`nil` for those that don't).

```luau
--!strict
local shape = jess.nif.open('rock.nif', { openmw = false }):find({ type = 'NiTriShape' })[1] :: NiTriShapeRecord
local meta = jess.meta(shape)
print(meta.index, meta.typeName, shape.name, meta.nif.count) -- 1  NiTriShape  Cone  20
```

### nifRecord:decode

{{ api_signature(value="nifRecord:decode() -> NifRecordValue") }}

Returns a copy of the record as a table: camelCase fields, links as record indices, empty links as [jess.null](@/docs/api/jess.md#jess-null), and `type` naming the record type.

```luau
--!strict
local root = jess.nif.open('rock.nif', { openmw = false }):roots()[1]
local value = root:decode()
if value.type == 'NiNode' then print(value.name, #value.children) end
```

### nifRecord:toTable

{{ api_signature(value="nifRecord:toTable() -> NifRecordValue (NiNode for a NiNodeRecord; each type returns its own table type)") }}

Copies the record into plain tables: the same table `decode()` gives, `type` included. On a plain `NifRecord` it returns a `NifRecordValue`, to narrow by `type`; on a handle cast to its record type, that type's table.

```luau
--!strict
local root = jess.nif.open('rock.nif', { openmw = false }):roots()[1] :: NiNodeRecord
local copy = root:toTable()
print(copy.name, #copy.children)
```

### nifRecord:set

{{ api_signature(value="nifRecord:set(value: NifRecordFields | NifRecordValue) -> ()") }}

Replaces the record with a table. Fields left out take their defaults, and the type stays unless the table names another.

Geometry whose vertices or triangles change gets its bound and normals recomputed, and strips their triangle count, unless the same value sets them. A value that leaves the record as it was keeps its exact bytes, NaNs included.

```luau
--!strict
local nif = jess.nif.new()
local node = nif:insert { type = 'NiNode', name = 'Old' } :: NiNodeRecord
node:set { name = 'New', scale = 2 }
print(node.name) -- New
```

### nifRecord:update

{{ api_signature(value="nifRecord:update(fields: NifRecordFields) -> ()") }}

Merges some fields over the record's current value. A `type` among them changes the record's type, keeping the fields the two types share: `strips:update({ type = "NiTriShape" })` turns an `NiTriStrips` into an `NiTriShape`, handle and links included.

- **Errors:** `unknown field '<name>'` for a name the record type doesn't have, or a field value of the wrong shape.

```luau
--!strict
local shape = jess.nif.open('rock.nif', { openmw = false }):find({ type = 'NiTriShape' })[1] :: NiTriShapeRecord
shape:update { name = 'Rock01' }
print(shape.name)
```

### nifRecord:edit

{{ api_signature(value="nifRecord:edit(edit: (value: NifRecordValue) -> ...NifRecordValue) -> ()") }}

Decodes the record, calls `edit` with the table (it may change it or return a new one), and writes it back.

```luau
--!strict
local root = jess.nif.open('rock.nif', { openmw = false }):roots()[1]
root:edit(function(value)
  if value.type == 'NiNode' then value.scale = 2 end
end)
```

### nifRecord:delete

{{ api_signature(value="nifRecord:delete() -> ()") }}

Removes the record, and from the roots if it's one. Links to it read as `nil` through views, copy into tables (`decode`, `toTable`, `dump`) as `jess.null`, and save as empty links, even once a record inserted later takes its index.

```luau
--!strict
local nif = jess.nif.open('rock.nif', { openmw = false })
local shape = nif:find({ type = 'NiTriShape' })[1] :: NiTriShapeRecord
shape:delete()
-- A deleted record's handle throws on any use.
print(pcall(function() return shape.name end)) -- false  this record has been deleted
```

### nifRecord:attach

{{ api_signature(value="nifRecord:attach(child: NifRecord | number) -> ()") }}

Appends a child to a node's `children`.

- **Errors:** `only nodes have children`, `there's no record <index>`, `this record has been deleted` (a deleted record's handle), or `that record belongs to a different NIF`.

```luau
--!strict
local nif = jess.nif.new()
local root = nif:insert { type = 'NiNode', name = 'Root' }
root:attach(nif:insert { type = 'NiNode', name = 'Child' })
print(#(root :: NiNodeRecord).children) -- 1
```

### nifRecord:detach

{{ api_signature(value="nifRecord:detach(child: NifRecord | number) -> boolean") }}

Removes a child from a node's `children`, returning whether it was there. The child record stays in the NIF.

```luau
--!strict
local nif = jess.nif.new()
local root = nif:insert { type = 'NiNode', name = 'Root' }
local child = nif:insert { type = 'NiNode', name = 'Child' }
root:attach(child)
print(root:detach(child), root:detach(child)) -- true  false
```
