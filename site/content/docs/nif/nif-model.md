---
title: 'The NIF Model'
description: 'Records, links, roots, and how Morrowind NIFs differ from later games''.'
weight: 10
extra:
  kind: guide
---

A NIF is a flat array of records that point at each other by index. The tree NifSkope draws for you isn't in the file. It's reconstructed by following those pointers from a handful of roots. jess models a Morrowind NIF exactly that way: a list of typed records plus a list of roots, and nothing else. Think in those terms and everything on the [editing](@/docs/nif/editing.md) and [optimization](@/docs/nif/optimization-cookbook.md) pages follows.

## What's in the file

A Morrowind NIF (version 4.0.0.2) has almost no framing:

| Part | Bytes |
| --- | --- |
| Header | The 40-byte line `NetImmerse File Format, Version 4.0.0.2\n`, then the version as a `u32` (`0x04000002`). |
| Record count | A `u32`. |
| Records | Each one is its type name as a length-prefixed string (`NiNode`, `NiTriShapeData`, ...), then that type's fields. |
| Roots | A `u32` count, then that many record indices. |

There's no record type table, no record sizes, and no string table. Those came later, presumably after someone hit the obvious problem: a reader knows how long a record is only by knowing its type, so one unknown type makes everything after it unreadable. That's why OpenMW refuses the whole file over a single record it doesn't know (`unknown_record_type`, see [Check Rules](@/docs/nif/checking-rules.md)). There's no skipping ahead when nothing says how far ahead is.

Every one of the 7,343 NIFs in `Morrowind.bsa` opens with `jess.nif.openBytes`, and a file jess saved saves back to identical bytes. Vanilla files themselves don't quite. Their 4-byte true/false fields ("has a texture", "has normals") often hold uninitialized memory in the upper bytes, like `00 e7 c9 00`, courtesy of the exporter. jess writes those back as a clean `1`. OpenMW reads any nonzero value as true (`NIFStream::read<bool>`), so nothing it loads changes, but a byte-level diff against a vanilla original will show those fields. That's the exporter's garbage leaving, not jess's damage arriving.

## Links

A field that refers to another record is a **link**: a 32-bit record index, with `-1` for "nothing". Links point in every direction. Parents point at children, shapes at their data, properties at textures, controllers at their targets.

jess shows links three ways, depending on where you look:

- **`jess nif dump` and `record:decode()`** give the plain index (`children: [5, 6]`), 0-based, the same number the file stores. A null link inside an array is `jess.null` in Luau (so the array keeps its positions) and `null` in YAML or JSON.
- **Live views** (`node.children[1]`, `shape.geometryData`) give a record handle you can keep following. A null link reads as `nil`.
- **Writes** take either one. You can assign a handle or an index.

Handles survive edits. Indices don't: inserting records adds new numbers, and saving renumbers every record in file order. If you're holding onto something across edits, hold the handle. An index kept across a save points at whatever landed there, which is a bug you find much later.

## Roots and reachability

The roots list is the only entry point. For a model, the first root is the scene root, almost always an `NiNode` named after the file. It can also be a bare `NiTriShape`: `meshes/o/contain_crate_01.nif` in the vanilla game is exactly that. For a `.kf` animation the root is an `NiSequenceStreamHelper`.

jess saves **only records reachable from the roots**. A record you stop linking to disappears on the next save, and so does anything only it linked to. That's usually what you want, and it's why [collapsing a node](@/docs/nif/optimization-cookbook.md#collapse-redundant-ninodes) is just "splice its children into its parent". It also means a file with unreachable junk shrinks the first time you save it, even if you changed nothing else. `jess nif check` reports that junk as `unreachable_records`, and `nif fix` drops it.

OpenMW's reader follows the same rule when it builds the scene: whatever the roots can't reach is parsed and then ignored.

## The record types

Every record type is a Rust struct that embeds its parent type. In Luau, a subtype's fields are a superset of its parent's. The branches you'll touch most:

```text
NiObject
├── NiObjectNET                name, extraData, controller
│   ├── NiAVObject             flags, translation, rotation, scale, properties, boundingVolume
│   │   ├── NiNode             children, effects
│   │   │   ├── RootCollisionNode, AvoidNode, NiBillboardNode, NiSwitchNode (NiLODNode, NiFltAnimationNode)
│   │   │   └── NiBSAnimationNode, NiBSParticleNode, NiSortAdjustNode, NiCollisionSwitch, ...
│   │   ├── NiGeometry         geometryData, skinInstance
│   │   │   ├── NiTriBasedGeom → NiTriShape, NiTriStrips
│   │   │   ├── NiParticles → NiAutoNormalParticles, NiRotatingParticles
│   │   │   └── NiLines
│   │   ├── NiDynamicEffect → NiLight (NiPointLight, NiSpotLight, ...), NiTextureEffect
│   │   └── NiCamera
│   ├── NiProperty             flags
│   │   └── NiTexturingProperty, NiMaterialProperty, NiAlphaProperty, NiZBufferProperty,
│   │       NiVertexColorProperty, NiSpecularProperty, NiWireframeProperty, NiStencilProperty, ...
│   ├── NiTexture → NiSourceTexture
│   └── NiSequenceStreamHelper
├── NiGeometryData             vertices, normals, bound, vertexColors, uvSets
│   ├── NiTriBasedGeomData → NiTriShapeData (triangles), NiTriStripsData (stripLengths, strips)
│   └── NiParticlesData, NiLinesData
├── NiExtraData                next
│   └── NiStringExtraData (value), NiTextKeyExtraData, NiVertWeightsExtraData, ...
├── NiTimeController           next, flags, target, ...
│   └── NiKeyframeController, NiVisController, NiUVController, NiFlipController, NiAlphaController, ...
└── NiSkinInstance, NiSkinData, NiPixelData, NiKeyframeData, NiUVData, NiMorphData, ...
```

`libs/nif/src/types.rs` lists all 112 of them. The generated `types/jess.d.luau` (`jess luau-types`) has a Luau type for each: the plain table (`NiTriShapeData`), the live view (`NiTriShapeDataView`), and the handle (`NiTriShapeDataRecord`).

## Scene graph vs data

The split that trips people up: **objects** and their **data** are separate records.

- `NiTriShape` is the object. It has a name, a transform, a property list, and a link to its geometry.
- `NiTriShapeData` is the geometry: vertices, normals, UV sets, vertex colors, triangles, and a bounding sphere.

Several shapes can share one data record, and several objects can share one property. It's a graph, not a tree, and jess preserves that sharing. The vanilla game uses it too. `random_objects.nif` in jess's test assets has 8 shapes over 7 data records.

Properties apply to the object that lists them and to everything under it. A child's property of the same type replaces its parent's. That's how OpenMW's `handleNode` walks them too.

Three more kinds of record hang off objects rather than sitting in the tree:

- **Controllers** animate things. An object's `controller` link starts a chain, and each controller's `next` continues it.
- **Extra data** carries strings and animation keys. `extraData` starts a chain, and each record's `next` continues it. A handful of strings on the root change how the engine treats the whole model (`MRK`, `NCC`, `RCN`). [Check Rules](@/docs/nif/checking-rules.md) covers them.
- **Effects** (`NiNode.effects`) are lights and texture effects. Only the ones in some node's effect list do anything.

Flags matter too. Bit `0x1` on an `NiAVObject` hides it and everything under it.

## Looking at a real file

`jess nif info` prints the scene graph and statistics, and `jess nif dump` prints every record. Here's the crate from [Building From Scratch](@/docs/nif/building-from-scratch.md):

```sh
jess nif dump crate.nif --list
```

```yaml
path: crate.nif
source: crate.nif
roots:
- 0
records:
- index: 0
  type: NiNode
  name: jess_crate
- index: 1
  type: NiTriShapeData
- index: 2
  type: NiSourceTexture
- index: 3
  type: NiTexturingProperty
- index: 4
  type: NiMaterialProperty
- index: 5
  type: NiTriShape
  name: Tri jess_crate
- index: 6
  type: RootCollisionNode
- index: 7
  type: NiTriShapeData
- index: 8
  type: NiTriShape
```

Drop `--list` (or add `--index 0`) to see the fields. The CLI prints them in snake_case (`extra_data`, `bounding_volume`) and Luau uses camelCase (`extraData`, `boundingVolume`). The values are the same.

## Morrowind vs everything else

OpenMW loads NIFs from every Bethesda game from Morrowind to Starfield, and so does jess's reader. `jess nif check`, `info`, and `dump` (and their Luau versions) read Oblivion, Fallout 3 and New Vegas, Skyrim, Skyrim SE, Fallout 4, and Fallout 76 files **the way OpenMW reads them**. That's a port of OpenMW's own `Nif::Reader` over the same record set, with the same refusals. The later formats add a header with a record type table, record sizes, a string table, and user and Bethesda versions. Their collision comes from `BSXFlags` and render geometry, not `RootCollisionNode`. `check` knows those rules.

**Editing is Morrowind-only.** `jess.nif.open`, `openBytes`, and `jess nif modify` refuse anything that isn't 4.0.0.2, with an error that names the game and version. The editable model is the typed record set above, and a round trip has to be byte-identical. jess holds Morrowind's format to that bar. Nothing later has been built to it yet, and a writer that's merely close would be worse than none. Point `open` at a Skyrim mesh and you get told, not a half-written file.

## Next

- [Editing NIFs](@/docs/nif/editing.md): handles, views, links, and saving.
- [Building From Scratch](@/docs/nif/building-from-scratch.md): a complete mesh from nothing.
- [Check Rules](@/docs/nif/checking-rules.md): what OpenMW does with every record you write.
