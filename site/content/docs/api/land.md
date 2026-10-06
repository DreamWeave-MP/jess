---
title: 'jess.land'
description: 'Read and write LAND terrain as buffers, without base64 in the way.'
weight: 35
extra:
  kind: api
---

{{ api_signature(value="jess.land: { read, write }") }}

A LAND record is five fixed-size arrays wearing a record costume: 65×65 heights, normals, and vertex colors, 16×16 texture indices, and a 9×9 map preview. A record view hands each one out as base64 text, because that's how blobs survive a trip through a table. That's fine for one field of one record. For a load order's terrain it's thousands of encodes and decodes of data nobody wanted as text.

`jess.land` skips the text. `read` copies each subrecord out of the record's bytes into a `buffer` of its own, summing VHGT's deltas into heights on the way, and `write` takes the same shapes back.

## LandTerrain

{{ api_signature(value="type LandTerrain = { grid: { number }?, flags: number?, heightOffset: number?, heights: buffer?, normals: buffer?, colors: buffer?, textures: buffer?, worldMap: buffer? }") }}

Every buffer is little-endian and row-major, starting at the cell's south-west corner. A part the record doesn't have is `nil`.

| Field | Subrecord | Contents |
| --- | --- | --- |
| `grid` | INTV | The cell's `{ x, y }`. |
| `flags` | DATA | Which parts the record uses, as a number: 1 heights and normals, 2 colors, 4 textures. |
| `heightOffset` | VHGT | The height the deltas start from. |
| `heights` | VHGT | 65×65 `i32`s: VHGT's deltas summed down the first column and along each row. Vertex `i` stands at `(heightOffset + heights[i]) * 8` game units. |
| `normals` | VNML | 65×65 `i8` x, y, and z. |
| `colors` | VCLR | 65×65 `u8` red, green, and blue. |
| `textures` | VTEX | 16×16 `u16`s in the order the file keeps them. 0 is no texture; anything else is one more than an LTEX index in the same plugin. |
| `worldMap` | WNAM | 9×9 `i8`s, the world map's heights. |

`LandParts` is the same table with `false` allowed for each buffer, for building one yourself. [write](#jess-land-write) takes a `LandInput`: `LandParts` with read-only fields, so both a `LandParts` you built and the `LandTerrain` that `read` returned fit without a cast.

VHGT stores each vertex as a signed byte of difference from its neighbor, so a height is only meaningful once everything before it has been added up. `heights` is that sum, done in Rust, so a script can read any vertex with `buffer.readi32(heights, index * 4)` instead of walking the row to it.

## jess.land.read

{{ api_signature(value="jess.land.read(record: RecordHandle, parts: { LandPart }?) -> LandTerrain") }}

A LAND record's terrain: always its `grid` and `flags`, and the `parts` asked for (`'heights'`, `'normals'`, `'colors'`, `'textures'`, `'worldMap'`), every one by default. A subrecord the record gives twice counts as its last, as decoding does.

- **Errors:** the record isn't a LAND, a part name is unknown, or a subrecord isn't the size LAND needs (`LAND VHGT is 12 bytes, not 4232`).

```luau
--!strict
local plugin = jess.plugin.open('Morrowind.esm', { types = { 'LAND' } })
for record in plugin:select { type = 'LAND' } do
  local terrain = jess.land.read(record, { 'heights' })
  local heights = terrain.heights
  if not heights then continue end

  local offset = terrain.heightOffset :: number
  local grid = terrain.grid :: { number }
  local center = (offset + buffer.readi32(heights, (32 * 65 + 32) * 4)) * 8
  print(grid[1], grid[2], center)
end
```

## jess.land.write

{{ api_signature(value="jess.land.write(record: RecordHandle, terrain: LandInput) -> ()") }}

Writes the terrain's parts into a LAND record. Parts left out stay as they are; `heights` without `heightOffset` keeps the record's offset. Which subrecords the record ends up with follows its `flags`, as it does for any typed LAND: set the flag, or the part isn't written.

The one exception is a part given as `false`, which leaves its subrecord out even though the flags would write it. Real plugins do this: a LAND with heights and no WNAM is common enough that a tool rebuilding one has to be able to say so. A later typed write to the record puts the subrecord back, because the typed LAND has no way to know it was ever missing.

- **Errors:** the record isn't a LAND; a part isn't a buffer of its exact size; or two neighboring `heights` differ by more than a signed byte holds (`LAND heights step too far at vertex (1, 0)`), since VHGT has no way to store that. Clamp before writing if clamping is what you mean.

```luau
--!strict
local plugin = jess.plugin.new()
local land = plugin:insert {
  type = 'Landscape',
  grid = { 0, 0 },
  landscapeFlags = 'USES_VERTEX_HEIGHTS_AND_NORMALS',
}

local heights, normals = buffer.create(65 * 65 * 4), buffer.create(65 * 65 * 3)
for index = 0, 65 * 65 - 1 do
  buffer.writei32(heights, index * 4, index % 65)
  buffer.writei8(normals, index * 3 + 2, 127)
end

jess.land.write(land, { heightOffset = 0, heights = heights, normals = normals })
```
