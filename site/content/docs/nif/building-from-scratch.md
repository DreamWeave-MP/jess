---
title: 'Building From Scratch'
description: 'Make a mesh from nothing with jess.nif.new and the geometry helpers.'
weight: 30
extra:
  kind: guide
---

You don't need a 3D package to make a Morrowind mesh. It helps for anything shaped like a person, but a crate is 24 vertices and some arithmetic. `jess.nif.new()` gives you an empty NIF, `nif:shape` builds a textured triangle shape in one call, and jess fills in the bounds and normals. This page builds a crate that loads in OpenMW with its own collision box, then a sign built record by record so you can see every part.

## A crate, start to finish

```luau
--!strict
-- jess run crate.jsl <output.nif>
local output: string = ...

type Point = { number }

local Size = 32 -- half the crate's width, in game units (a crate 64 units wide)

-- Each face gets its own four corners, so it gets its own UVs and a flat normal.
local Faces: { { normal: Point, across: Point, up: Point } } = {
  { normal = { 0, 0, 1 }, across = { 1, 0, 0 }, up = { 0, 1, 0 } }, -- top
  { normal = { 0, 0, -1 }, across = { -1, 0, 0 }, up = { 0, 1, 0 } }, -- bottom
  { normal = { 1, 0, 0 }, across = { 0, 1, 0 }, up = { 0, 0, 1 } }, -- east
  { normal = { -1, 0, 0 }, across = { 0, -1, 0 }, up = { 0, 0, 1 } }, -- west
  { normal = { 0, 1, 0 }, across = { -1, 0, 0 }, up = { 0, 0, 1 } }, -- north
  { normal = { 0, -1, 0 }, across = { 1, 0, 0 }, up = { 0, 0, 1 } }, -- south
}

local vertices: { Point }, uvs: { Point }, triangles: { Point } = {}, {}, {}
for _, face in Faces do
  local first = #vertices -- vertex indices are 0-based
  for _, corner in { { -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, 1 } } do
    local point: Point = {}
    for axis = 1, 3 do
      point[axis] = Size
        * (face.normal[axis] + corner[1] * face.across[axis] + corner[2] * face.up[axis])
    end
    table.insert(vertices, point)
    -- Texture space runs down from the top left.
    table.insert(uvs, { (corner[1] + 1) / 2, (1 - corner[2]) / 2 })
  end
  -- Counter-clockwise seen from outside, so the faces point out.
  table.insert(triangles, { first, first + 1, first + 2 })
  table.insert(triangles, { first, first + 2, first + 3 })
end

local nif = jess.nif.new()
local root = nif:insert { type = 'NiNode', name = 'jess_crate' }
nif:setRoots { root }

-- The visible crate: a shape, its data, a texture, and a material, in one call.
nif:shape {
  name = 'Tri jess_crate',
  vertices = vertices,
  triangles = triangles,
  uvs = uvs,
  texture = 'tx_wood_brown_box_01.dds',
  -- A material's fields default to zero: black, and fully transparent. Set them.
  material = {
    ambientColor = { 1, 1, 1 },
    diffuseColor = { 1, 1, 1 },
    specularColor = { 0, 0, 0 },
    emissiveColor = { 0, 0, 0 },
    shine = 10,
    alpha = 1,
  },
  parent = root,
}

-- Collision: a RootCollisionNode under the root replaces the render geometry. A bare box with
-- 8 corners is all it needs; the collision node is never drawn.
local corners: { Point } = {}
for corner = 0, 7 do
  table.insert(corners, {
    if bit32.band(corner, 1) == 0 then -Size else Size,
    if bit32.band(corner, 2) == 0 then -Size else Size,
    if bit32.band(corner, 4) == 0 then -Size else Size,
  })
end
local collision = nif:insert { type = 'RootCollisionNode' }
nif:shape {
  vertices = corners,
  triangles = {
    { 0, 2, 1 },
    { 1, 2, 3 },
    { 4, 5, 6 },
    { 5, 7, 6 },
    { 0, 1, 4 },
    { 1, 5, 4 },
    { 2, 6, 3 },
    { 3, 6, 7 },
    { 0, 4, 2 },
    { 2, 4, 6 },
    { 1, 3, 5 },
    { 3, 7, 5 },
  },
  parent = collision,
}
root:attach(collision)

local check = nif:check()
for _, finding in check.findings do
  print(finding.severity, finding.code, finding.message)
end
local stats = check.stats :: NifStats
print(
  string.format(
    '%d records, %d shapes, %d triangles, %d collision triangles',
    stats.records,
    stats.shapes,
    stats.triangles,
    stats.collisionTriangles
  )
)
nif:save(output)
```

```sh
jess run crate.jsl "Data Files/meshes/jess/crate.nif"
```

```text
9 records, 1 shapes, 12 triangles, 12 collision triangles
```

No findings. `nif:check()` looks the texture up through the script's default config when it has one; a `jess run` script has none until it calls `jess.openmw.configPath`, so here it has nowhere to look and skips textures. Check the saved file the way OpenMW will load it, with the textures looked up in its data folder and in `Morrowind.bsa`:

```sh
jess nif check --data "Data Files/Morrowind.bsa" "Data Files/meshes/jess/crate.nif" -v
```

```yaml
# trimmed to the interesting parts
files:
- path: meshes/jess/crate.nif
  worst: null
  findings: []
  textures:
  - path: tx_wood_brown_box_01.dds
    resolved: textures/tx_wood_brown_box_01.dds
  stats:
    records: 9
    nodes: 2
    shapes: 1
    vertices: 24
    triangles: 12
    collision_triangles: 12
```

`jess nif info` confirms `collision.source: root_collision_node`: OpenMW builds collision from the 12-triangle box and never draws it. Point a Static at `jess\crate.nif` and it's a crate.

## What `nif:shape` does

`nif:shape(spec)` inserts an `NiTriShape` and its `NiTriShapeData`, and optionally a texture and a material, then returns the shape:

| Field | |
| --- | --- |
| `vertices`, `triangles` | Required. Triangles index the vertices from 0. |
| `normals` | Optional. Computed (area-weighted, smooth) when left out. |
| `uvs` or `uvSets` | One UV set, or a list of them. Not both. |
| `vertexColors` | RGBA, one per vertex. |
| `texture` | A path. Adds an `NiSourceTexture` and an `NiTexturingProperty` with it as the base map. |
| `material` | `NiMaterialProperty` fields. Adds one. |
| `properties` | Records or indices of properties you made yourself. They come before the texture and material. |
| `parent` | A node to attach the shape to. |
| `name`, `translation`, `rotation` (9 numbers, row-major), `scale` | The shape's own fields. |

The bound is computed from the vertices. An unknown field is an error: `textrue = "..."` fails instead of giving you an untextured crate and no idea why.

## Record by record

`nif:shape` is a shortcut, and everything it does is `nif:insert`. Here's a sign with a glow map, vertex colors, and alpha blending, built without it:

```luau
--!strict
-- jess run sign.jsl <output.nif>
local output: string = ...
local nif = jess.nif.new()

local function texture(path: string): NifRecord
  return nif:insert { type = 'NiSourceTexture', source = { external = path } }
end

-- Named slots: base, dark, detail, gloss, glow, bump, decals (a list).
local texturing = nif:insert {
  type = 'NiTexturingProperty',
  textureMaps = {
    base = { texture = texture 'tx_sign_jess.dds' },
    glow = { texture = texture 'tx_sign_jess_glow.dds' },
  },
}
-- Blend on (bit 0), source SRC_ALPHA (6 << 1), destination ONE_MINUS_SRC_ALPHA (7 << 5).
local alpha = nif:insert { type = 'NiAlphaProperty', flags = 0x00ED }
local material = nif:insert {
  type = 'NiMaterialProperty',
  ambientColor = { 1, 1, 1 },
  diffuseColor = { 1, 1, 1 },
  alpha = 1,
}

-- Vertex colors light the sign as ambient and diffuse color.
local vertexColor = nif:insert {
  type = 'NiVertexColorProperty',
  sourceVertexMode = 'AmbientDiffuse',
  lightingMode = 'EmissiveAmbientDiffuse',
}

local data = nif:insert {
  type = 'NiTriShapeData',
  vertices = { { -32, 0, 0 }, { 32, 0, 0 }, { 32, 0, 32 }, { -32, 0, 32 } },
  vertexColors = { { 1, 1, 1, 1 }, { 1, 1, 1, 1 }, { 1, 0.8, 0.6, 1 }, { 1, 0.8, 0.6, 1 } },
  uvSets = { { { 0, 1 }, { 1, 1 }, { 1, 0 }, { 0, 0 } } },
  -- Wound to face -Y; the other side is culled.
  triangles = { { 0, 1, 2 }, { 0, 2, 3 } },
}

local sign = nif:insert {
  type = 'NiTriShape',
  name = 'Tri Sign',
  geometryData = data,
  properties = { texturing, alpha, material, vertexColor },
}
local root = nif:insert { type = 'NiNode', name = 'jess_sign', children = { sign } }
nif:setRoots { root }

local value = (data :: NiTriShapeDataRecord):toTable()
print(#value.normals, value.normals[1][2], value.bound.radius)
print(nif:check().worst)
nif:save(output)
```

```text
4	-1	35.7770881652832
nil
```

The normals came out facing -Y and the bound was filled in. The things to know when building by hand:

- **Texture maps are named slots:** `base`, `dark`, `detail`, `gloss`, `glow`, `bump`, and `decals` (a list). Each map's `texture` is a link to an `NiSourceTexture`. A map also takes `clampMode` (`WrapSWrapT` by default), `filterMode` (`Trilerp`), and `textureIndex`, the UV set it uses. OpenMW falls back to set 0 when that set doesn't exist (`uv_set_out_of_range`).
- **A texture source is `{ external = path }` or `{ internal = index }`.** `internal` links an `NiPixelData`, a texture stored inside the file. Don't do this. `jess nif check` flags embedded textures as a performance smell, since they can't be shared, compressed, or replaced, and nobody downstream will thank you.
- **Texture paths** are relative to `textures\` the way the game spells them. OpenMW tries `.dds` first whatever extension you write, so `tx_foo.dds` and `tx_foo.tga` both load `textures/tx_foo.dds` when it exists.
- **`uvSets`** is a list of sets, each one coordinate pair per vertex. `vertexColors` are RGBA.
- **Defaults are zero.** Every field you leave out takes the type's default, and for `NiMaterialProperty` that's black at alpha 0, and nothing warns you. Always set a material's colors and alpha. For nodes and shapes the defaults are what you want: identity transform, scale 1, flags 0.
- **Winding decides the front.** A triangle's front is the side its corners run counter-clockwise from. Backs are culled unless an `NiStencilProperty` says otherwise.
- **Only what the roots reach is saved.** An inserted record nobody links to just isn't in the file.

## Triangle strips

`NiTriStripsData` stores strips instead of a triangle list: `stripLengths` and every strip's indices one after another in `strips`. `numTriangles` is filled in for you:

```luau
local vertices: { { number } } = {}
for step = 0, 4 do
  table.insert(vertices, { step * 10, 0, 0 })
  table.insert(vertices, { step * 10, 10, 0 })
end

local data = nif:insert {
  type = 'NiTriStripsData',
  vertices = vertices,
  stripLengths = { 10 },
  strips = { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9 },
}
nif:insert { type = 'NiTriStrips', name = 'Strip', geometryData = data }
-- numTriangles is 8, and normals and the bound are computed as for a triangle list
```

OpenMW draws each strip as a separate primitive set, so for new meshes a triangle list (`nif:shape`) is the better choice. See [the cookbook](@/docs/nif/optimization-cookbook.md#triangle-strips-to-lists-and-back).

## Collision and markers

How OpenMW collides with a model (every rule is on the [Check Rules](@/docs/nif/checking-rules.md) page):

- **No `RootCollisionNode`:** everything that draws collides. That's fine for small props and wasteful for big ones.
- **A `RootCollisionNode` child of the root:** only its geometry collides, and it's never drawn. Put it directly under the root, or add `RCN` string data to the root if it's deeper.
- **An empty `RootCollisionNode`**, or **`NCC` string data** on the root: only the camera collides, with the render geometry. `NCO` (or any other string starting with `NC` without an uppercase third `C`) turns collision off.
- **Actors** (NPCs and creatures) collide as a box, from a node named `Bounding Box` with a box bounding volume if there is one.

String data goes on the root's extra data chain:

```luau
local noCollision = nif:insert { type = 'NiStringExtraData', value = 'NCO' }
root.extraData = noCollision
```

Shapes named `Tri EditorMarker...` are editor-only only if the root also has `MRK` string data. Otherwise they draw in game (`visible_editor_marker`, which `nif:fix({ "markers" })` fixes by adding it).

## Check before you ship

`nif:check()` runs every rule on the NIF in memory, and it's cheap enough to run after every build. It looks textures up in the load order only (`openmw = false` skips even that). To resolve them against a folder or archive too, save the file into a data folder and run `jess nif check` on it with `--data`, or `jess.nif.check` on the folder. A file with no findings at `wontLoad` or `broken` loads and draws the way you built it.
