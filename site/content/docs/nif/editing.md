---
title: 'Editing NIFs'
description: 'jess.nif.open, openBytes, and new: record handles and live views over a mesh.'
weight: 20
extra:
  kind: guide
---

`jess.nif` gives you a Morrowind NIF as a set of record handles you can read, change, add to, and cut from, then save back. What you didn't touch saves as it was. This page walks through all of it on real files. It assumes you've read [The NIF Model](@/docs/nif/nif-model.md). For the exact signatures, see the [`jess.nif` reference](@/docs/api/nif.md).

Every complete script here runs with `jess run script.jsl` and type-checks in `--!strict` mode against the types `jess luau-types` writes.

## Opening a NIF

There are three ways in:

```luau
local fromFile = jess.nif.open 'Data Files/meshes/f/furn_com_bed_01.nif'
local fromVfs = jess.nif.open 'meshes/f/furn_com_bed_01.nif' -- whichever file your load order uses
local fromBytes = jess.nif.openBytes(assert(jess.fs.read 'rock.nif'), 'rock.nif')
local empty = jess.nif.new()
```

- **`jess.nif.open(input, options?)`** takes a file path, or a VFS path it finds the way OpenMW would. `data` adds folders or archives to look in, and `openmw = false` skips your `openmw.cfg`. Put both together and you can open a NIF straight out of a BSA, no config needed: `jess.nif.open("meshes/f/furn_com_bed_01.nif", { openmw = false, data = "Morrowind.bsa" })`.
- **`jess.nif.openBytes(bytes, path?)`** takes a buffer or string you already have, from `jess.fs.read`, an archive, or `openmw:readFile`. The `path` only names it in error messages.
- **`jess.nif.new()`** is empty. See [Building From Scratch](@/docs/nif/building-from-scratch.md).

Anything that isn't a Morrowind (4.0.0.2) NIF is an error that names the game and version. To read other games' NIFs, use `jess.nif.dump` and `jess.nif.info` instead.

{% usage_note(title="Where save() goes") %}
`nif.path` is where `nif:save()` writes when you don't give it a path. For a file, that's the file. For a VFS path that resolved to a loose file, it's **that mod's file**. `nif:save()` would overwrite another mod's mesh (with a backup, but still). A NIF read out of an archive, or opened with `openBytes`, has no path, and `save()` without one is an error. When you edit through the VFS, always pass an output path.
{% end %}

## Walking the records

```luau
--!strict
local archive: string = ... -- path to Morrowind.bsa
local nif = jess.nif.open('meshes/f/furn_com_bed_01.nif', { openmw = false, data = archive })
print(nif.count, nif.path)

local root = nif:roots()[1] :: NiNodeRecord
print(root, root.flags, #root.children)
for position, child in root.children do
  print(position, child)
end

for _, record in nif:find { type = 'NiTriShape', name = '^tri furn' } do
  local shape = record :: NiTriShapeRecord
  local data = shape.geometryData :: NiTriShapeDataRecord
  print(shape.name, #data.vertices, #data.triangles, data.bound.radius)
end
```

```text
34	nil
NiNode 0 "Furn_Com_Bed_01"	10	8
1	nil
2	NiTriShape 1 "Tri Furn_Com_Bed_01 0"
3	NiTriShape 6 "Tri Furn_Com_Bed_01 1"
...
8	RootCollisionNode 31
Tri Furn_Com_Bed_01 0	192	102	148.4251251220703
Tri Furn_Com_Bed_01 1	96	48	137.1683807373047
...
```

Look at the output. The vanilla bed's first child link is empty, so the view gives `nil` for position 1 and keeps going. Real files are full of empty links, so loops over links skip nils.

The ways to get at records:

| | |
| --- | --- |
| `nif:roots()` | The root records, in order. |
| `nif:records()` | An iterator over every record, by index, reachable or not. |
| `nif:record(index)` | One record by the index links hold, or `nil`. |
| `nif:find({ type = ..., name = ... })` | Records of any of the listed types (exact type names, not subtypes) whose name matches a case-insensitive regex. |
| `nif.count` | How many records there are, reachable or not. |

## Handles and views

`tostring` on a handle (`NifRecord`) gives `NiNode 0 "Furn_Com_Bed_01"`. Handles compare equal when they're the same record. A handle stays valid while other records are inserted or deleted.

Every name on a handle is a field of its record or one of its methods (`decode`, `set`, `attach`, ...), and no NIF type has a field named like a method. The handle's own metadata, its index, type name, and NIF, is [jess.meta](@/docs/api/jess.md#jess-meta)'s: `jess.meta(shape).typeName` is `"NiTriShape"`, and `shape.name` is the shape's name.

The fields read **live** through the handle. `shape.name` and `shape.geometryData` read the record as it is right now, not as it was when you got the handle. In `--!strict` code, cast the handle to its type (`record :: NiTriShapeRecord`) and the analyzer knows every field. A field the record's type doesn't have reads `nil`, which is useful: `(record :: NiAVObjectRecord).properties` is `nil` on anything that isn't an `NiAVObject`, so one test replaces a list of types. Writing a field the type doesn't have is an error (`unknown field`), and changes nothing.

What a field gives you:

- **Scalars and strings** read as values and write straight into the record: `shape.name = "Tri Crate"`.
- **Links** read as handles (`nil` when empty) and take a handle or an index: `root.extraData = marker`.
- **Lists** (`children`, `properties`, `vertices`, `triangles`, `strips`) are live `List`s. You can index them, use `#` on them, iterate them with `for i, v in list`, change them with `list:push(v)`, `list:insert(i, v)`, `list:remove(i)` (not `table.insert`), and copy them out with `list:toTable()`.
- **Structs** with their own view type (`bound`, `boundingVolume`) are views too.
- **Everything else** (a vertex `{ x, y, z }`, a UV pair, `textureMaps`, `uvSets`, `source`) is a **copy**. Changing the copy changes nothing. Assign it back.

That last point is the one that bites. The assignment below succeeds, and nothing happens:

```luau
data.vertices[1][3] = -40 -- does nothing: vertices[1] is a copy
data.vertices[1] = { -32, -32, -40 } -- writes the vertex

local maps = texturing.textureMaps -- a copy
maps.base.clampMode = 'ClampSClampT'
texturing.textureMaps = maps -- written
```

## decode, toTable, set, update, edit

For work on a whole record at once:

| | |
| --- | --- |
| `record:decode()` | The whole record as a table (`NifRecordValue`), with camelCase fields and links as indices. It's the shape `jess nif dump` prints. |
| `record:toTable()` | The same table. On a handle cast to its type (`record :: NiTriShapeRecord`) it's typed as that record's table (`NiTriShape`); on a plain `NifRecord` it's a `NifRecordValue`, like `decode()`'s, to narrow by `type`. |
| `record:set(value)` | Replaces the record. Fields you leave out take the type's defaults. If `value.type` names a different type, **the record changes type** and keeps its handle and every link to it. The cookbook's [strips-to-lists recipe](@/docs/nif/optimization-cookbook.md#triangle-strips-to-lists-and-back) depends on that. |
| `record:update(fields)` | Merges fields over the record. |
| `record:edit(fn)` | Decodes, calls `fn(value)`, and writes back what it returns, or the value itself if it changed it in place. |

```luau
--!strict
local nif = jess.nif.open('crate.nif', { openmw = false })
local root = nif:roots()[1] :: NiNodeRecord
local shape = nif:find({ type = 'NiTriShape', name = '^Tri jess_crate$' })[1] :: NiTriShapeRecord

-- Assign a field, or merge several.
shape.name = 'Tri jess_crate_big'
shape:update { scale = 2, translation = { 0, 0, 64 } }

-- Lists are live: replace an element and it's written in place. The element itself (a
-- vertex, a UV) is a copy, so replace it whole rather than one coordinate.
local data = shape.geometryData :: NiTriShapeDataRecord
data.vertices[1] = { -32, -32, -40 }
nif:updateBounds() -- field writes don't touch the bound; update/edit/set do

-- Or take a copy, change it, and write it back in one go.
data:edit(function(value)
  if value.type == 'NiTriShapeData' then
    for _, uv in value.uvSets[1] do
      uv[1] *= 2 -- tile the texture twice across
    end
  end
  return value
end)

-- Typed copies: toTable() on a cast handle is the record's own table type.
local copy = shape:toTable()
print(copy.name, copy.scale, copy.geometryData, #copy.properties)

-- Texture maps are a plain table, not a view: change it, then assign it back.
local texturing = shape.properties[1] :: NiTexturingPropertyRecord
local maps = texturing.textureMaps
if maps.base then maps.base.clampMode = 'ClampSClampT' end
texturing.textureMaps = maps

-- New records, attached with handles.
local light = nif:insert { type = 'NiNode', name = 'AttachLight', translation = { 0, 0, 40 } }
root:attach(light)
local marker = nif:insert { type = 'NiStringExtraData', value = 'MRK' }
root.extraData = marker

-- Take a record out: detach it first, then delete it.
root:detach(light)
light:delete()
nif:save 'crate_edited.nif'
```

```text
Tri jess_crate_big	2	1	2
```

## Derived data

A few values in geometry follow from the rest: a data record's bounding sphere (`bound`), its normals, and a strip record's triangle count. `insert`, `set`, `update`, and `edit` recompute them when the vertices or triangles change and the same change didn't set them itself:

- **Bounds** are recomputed whenever the vertices change.
- **Normals** are recomputed for new geometry, and for existing geometry that already had normals. They're area-weighted smooth normals. Geometry that had none stays without.
- **`numTriangles`** on `NiTriStripsData` follows `stripLengths`.

**Writes through a view skip all of this.** `data.vertices[1] = ...` changes the vertex and nothing else. Call `nif:updateBounds()` afterwards (it recomputes every geometry's bound and returns how many), or make the change with `update`.

There's no call to recompute the normals of geometry whose vertices didn't change. The cookbook has [a Luau version](@/docs/nif/optimization-cookbook.md#recompute-normals-and-bounds) for that.

## Links, inserting, and deleting

- **`nif:insert(fields)`** adds a record. `type` is required, and the other fields default to the type's defaults. It returns a handle. Link fields take handles or indices: `nif:insert({ type = "NiNode", children = { shape, 7 } })`.
- **`node:attach(child)`** pushes onto `children`. **`node:detach(child)`** takes it out and says whether it was there. Only nodes have children.
- **`nif:setRoots({ ... })`** replaces the roots.
- **`record:delete()`** removes the record. Links to it become empty, and it's dropped from the roots.

{% usage_note(title="Detach before you delete") %}
A deleted record's slot is reused by the next `insert`. Until you save, a live view of a link that pointed at the deleted record can show the **new** record in that slot, while `decode()` still shows the old index. The saved file gets it right (an empty link), but a script that reads links in between sees a record that was never linked there. Take the record out of every list and link field that holds it (`node:detach(child)`, `owner.extraData = nil`), then delete it. Or skip `delete` entirely: whatever nothing links to isn't saved.
{% end %}

Record indices change when you insert (new records continue the numbering) and when you save (records are numbered in file order, unreachable ones dropped). Hold handles, not numbers.

## Textures

```luau
for _, texture in nif:textures() do
  print(jess.meta(texture.record).index, texture.path) -- every external NiSourceTexture path, as spelled
end
local changed = nif:setTexture('Tx_wood_cherry.tga', 'tx_wood_cherry.dds')
```

`setTexture` matches regardless of case and slash direction, repoints every `NiSourceTexture` spelled that way, and returns how many it changed. An embedded texture is `source = { internal = <NiPixelData index> }` and isn't listed.

## Checking, fixing, saving

| | |
| --- | --- |
| `nif:check(options?)` | `jess.nif.checkBytes` on the NIF as it is now: findings, stats, and textures, resolved through the load order (or the config `options` pick). |
| `nif:fix(only?)` | Applies `jess nif fix`'s repairs (`rcn`, `ncc`, `markers`, `skinWeights`, `unreachable`) and returns what it did. |
| `nif:dump(options?)` | `jess.nif.dump` of the NIF in memory. |
| `nif:bytes()` | The file it would save, as a buffer. |
| `nif:save(path?, { backup = false }?)` | Writes the file and returns `{ path, backup }`. An existing file is backed up first unless `backup = false`. Missing folders on the way are created. |

## Original bits, NaN, and nulls

Preservation is the default. What you don't change stays as it was:

- **A record whose values an edit leaves as they were keeps its exact bytes**, even when you `set(record:decode())` it. A NIF whose every record went through Luau tables saves exactly as it did before.
- **Numbers keep their bits** across the Rust/Luau boundary, `-0` included. A record that really changes writes any NaN it holds (saved particle state has them) as a plain NaN, since Luau can't carry the exact NaN payload.
- **Arrays keep their positions.** An empty link inside a list decodes as `jess.null`, not `nil`, because `nil` would shift everything after it. Write `jess.null` to make an empty slot.

## One file from the command line

`jess nif modify --script` runs a script on one NIF. The script gets `(nif, path)` as `...`. jess writes the file if the script changed it (with a backup, or to `--output`), and leaves it alone otherwise:

```luau
--!strict
local nif: Nif, path: string = ...
local changed = nif:setTexture('tx_wood_brown_box_01.dds', 'tx_jess_crate.dds')
print(path, changed)
```

```sh
jess nif modify --script retexture.jsl meshes/jess/crate.nif --output "Data Files/meshes/jess/crate.nif"
```

A NIF inside an archive needs `--output`. For hundreds of files, don't loop `nif modify` in a shell. Use one `jess run` script over the VFS or an archive, as in the [cookbook](@/docs/nif/optimization-cookbook.md#the-batch-harness). [Modify Scripts](@/docs/scripting/modify-scripts.md) covers `modify` for plugins and NIFs together.
