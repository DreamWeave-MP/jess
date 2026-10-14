---
title: 'Optimization Cookbook'
description: 'Recipes for batch work over the VFS and archives: strip, retexture, fix normals and bounds, strips, and more.'
weight: 40
extra:
  kind: guide
---

These are recipes for doing things to thousands of meshes at once. Each one has a goal, a function you drop into one batch script, and what to check afterwards. Every number on this page comes from running these exact scripts over the 7,343 NIFs in vanilla `Morrowind.bsa`. Every output was re-checked against its original: no new findings, the same triangles, shapes, and collision, and a byte-identical re-save.

## Know what OpenMW already does

Before you spend an evening optimizing meshes: OpenMW already optimizes them, every time it loads one. `SceneManager` in `components/resource/scenemanager.cpp` runs its scene optimizer on every loaded model with **`FLATTEN_STATIC_TRANSFORMS`**, **`REMOVE_REDUNDANT_NODES`**, **`MERGE_GEOMETRY`**, and **`SHARE_DUPLICATE_STATE`**. It bakes static transforms into vertices, drops empty grouping nodes, merges shapes that share state, and shares identical materials and textures. It skips files whose name starts with `x`, `base_anim`, or `skin`, and anything with `vfx_pattern` in its path. It never touches nodes with reserved names (`Bip01`, `AttachLight`, `Weapon Bone`, `Bounding Box`, and so on, with or without a `Tri ` prefix). The `OPENMW_OPTIMIZE` environment variable turns individual passes off.

So collapsing nodes and deduplicating properties in the file mostly won't change your frame rate in OpenMW. The engine already did both, at load, without asking. What offline work buys you:

- **Smaller files and fewer records** to read and build. That matters on load, for distant statics, and for tools.
- **The meshes OpenMW won't optimize:** animated `x` models and skeletons.
- **Things the optimizer can't do:** correct texture paths, sane normals and bounds, cheap collision, and anything `jess nif check` calls `wontLoad` or `broken`.
- **The original engine and other tools**, which get none of the above.

Measure. Don't assume. Every recipe here reports what it changed.

## Ground rules

- **Never write over the originals.** Write to a new data folder (an override) and put it after the mod in `openmw.cfg`. If a recipe goes wrong, delete the folder.
- **Refuse to write a file that got worse.** The harness below runs `jess.nif.checkBytes` on the original and the result and skips anything that went `wontLoad`.
- **Verify afterwards** with `jess nif check <override>` and the [verify script](#verify-the-output).

## The batch harness

One script, three sources: a loose data folder, a BSA, or an `openmw.cfg` (every mesh that load order actually uses, loose files and archives alike, winners only). It writes changed files under the output folder at their VFS paths, so the output *is* a data folder:

```luau
--!strict
-- jess run --package jpk/dreamwind optimize.jsl <source> <output folder> [path prefix]
-- <source> is a data folder, a .bsa, or an openmw.cfg for the whole load order.
local source: string, output: string, prefix: string? = ...
assert(source and output, 'usage: optimize.jsl <source> <output folder> [path prefix]')

-- Calls `visit(path, bytes)` for every NIF under `prefix` (a lowercase VFS path like "meshes/f/").
local function eachNif(visit: (path: string, bytes: buffer | string) -> ())
  local wanted = (prefix or 'meshes/'):lower()
  local function matches(path: string): boolean
    local normal = path:lower():gsub('\\', '/')
    return normal:sub(1, #wanted) == wanted and normal:match '%.nif$' ~= nil
  end

  if source:lower():match '%.cfg$' then
    local openmw = jess.openmw.load(source)
    for _, entry in openmw:vfs():pathsWith 'meshes' do
      if matches(entry.key) then visit(entry.key, entry.file:readAll()) end
    end
  elseif source:lower():match '%.bsa$' then
    local archive = require('@dreamwind/archive').openPath(source)
    for _, entry in archive:entries() do
      if entry.path and matches(entry.path) then
        visit((entry.path:lower():gsub('\\', '/')), archive:readFileRequired(entry.path))
      end
    end
  else
    local function walk(folder: string, relative: string)
      for _, name in jess.fs.list(folder) do
        local full, path = jess.fs.join(folder, name), relative .. name
        if jess.fs.isDir(full) then
          walk(full, path .. '/')
        elseif matches(path) then
          visit(path:lower(), assert(jess.fs.read(full)))
        end
      end
    end
    walk(source, '')
  end
end

local function optimize(nif: Nif, path: string): number
  return 0 -- the recipes go here; return how many changes they made
end

local totals =
  { files = 0, written = 0, bytesBefore = 0, bytesAfter = 0, recordsBefore = 0, recordsAfter = 0 }
eachNif(function(path: string, bytes: buffer | string)
  -- `openBytes` throws on a file that isn't a Morrowind NIF, or can't be read.
  local ok, nif = pcall(jess.nif.openBytes, bytes, path)
  if not ok then
    print('skipped', path, nif)
    return
  end

  totals.files += 1
  local before = nif.count
  if optimize(nif, path) == 0 then return end
  local out = nif:bytes()
  -- Never write a file the check says got worse.
  local was = jess.nif.checkBytes(bytes, path, { openmw = false })
  local now = jess.nif.checkBytes(out, path, { openmw = false })
  if now.worst == 'wontLoad' and was.worst ~= 'wontLoad' then
    print('refused', path)
    return
  end
  local target = jess.fs.join(output, path)
  jess.fs.createDir(assert(jess.fs.parent(target)))
  jess.fs.write(target, out)
  totals.written += 1
  totals.bytesBefore += if type(bytes) == 'string' then #bytes else buffer.len(bytes)
  totals.bytesAfter += buffer.len(out)
  totals.recordsBefore += before
  totals.recordsAfter += jess.nif.openBytes(out).count
end)
print(
  string.format(
    '%d NIFs, %d written: %d -> %d bytes, %d -> %d records',
    totals.files,
    totals.written,
    totals.bytesBefore,
    totals.bytesAfter,
    totals.recordsBefore,
    totals.recordsAfter
  )
)
```

```sh
jess run --package jpk/dreamwind optimize.jsl ~/.config/openmw/openmw.cfg "Data Files Optimized"   # the whole load order
jess run --package jpk/dreamwind optimize.jsl "Mods/Foo/Data Files" "Mods/Foo Optimized"           # one mod
jess run --package jpk/dreamwind optimize.jsl "Data Files/Morrowind.bsa" out meshes/f/
```

The details that matter:

- `vfs:pathsWith("meshes")` takes a directory, with or without a trailing slash, and it's recursive.
- Archive entries come back with backslashes (`meshes\f\...`). The harness lowercases paths and uses forward slashes throughout, which is what OpenMW does too.
- `jess.nif.openBytes` refuses non-Morrowind NIFs, so a load order with Skyrim-format meshes in it gets `skipped` lines, not errors.
- Only changed files are written. Run it twice and the second run writes nothing, because every recipe here is idempotent.

Each recipe below is a local function to paste above `optimize`. Then call it from `optimize` and add its count to the result.

## Strip exporter junk from extra data

**Goal:** drop the `NiStringExtraData` that exporters leave behind and nothing reads. That's `sgoKeep` (a Scene Graph Optimizer hint), MAX user-property dumps like `KeyPerFrame = `, `Clamp = `, `zMode10`, `billboard = `, and `NiOptimizeStripify =`, plus `NiVertWeightsExtraData`.

OpenMW reads exactly these strings: `MRK`, `RCN`, `BONE`, anything starting with `NC` (any case), and anything starting with `omw:data` (`handleNode` in `nifloader.cpp`, `handleRoot` in `bulletnifloader.cpp`). `NiTextKeyExtraData` holds animation keys and stays.

```luau
-- Strings OpenMW reads from NiStringExtraData: MRK, RCN, BONE, NC* (any case), omw:data...
local function openmwReads(value: string): boolean
  return value == 'MRK'
    or value == 'RCN'
    or value == 'BONE'
    or value:upper():sub(1, 2) == 'NC'
    or value:sub(1, 8) == 'omw:data'
end

local function isJunk(extra: NifRecord): boolean
  local typeName = jess.meta(extra).typeName
  if typeName == 'NiVertWeightsExtraData' then return true end

  if typeName == 'NiStringExtraData' then
    return not openmwReads((extra :: NiStringExtraDataRecord).value)
  end
  return false
end

local function stripExtraData(nif: Nif): number
  local removed = 0
  for record in nif:records() do
    local owner = record :: NiObjectNETRecord
    -- Walk the chain, relinking past junk. Records without the field read nil.
    local previous: NiExtraDataRecord? = nil
    local current = owner.extraData
    while current do
      local extra = current :: NiExtraDataRecord
      local following = extra.next
      if isJunk(extra) then
        if previous then
          previous.next = following
        else
          owner.extraData = following
        end
        removed += 1
      else
        previous = extra
      end
      current = following
    end
  end
  return removed
end
```

Extra data is a linked list (`extraData` on the owner, then `next` on each entry), so removing one means pointing its predecessor past it. The unlinked record is then unreachable and isn't saved.

**Vanilla:** 2,199 junk records across the BSA, 1,818 of them `sgoKeep`.

**Check afterwards:** the collision source and marker handling in `jess nif info` shouldn't change, since `NC*`, `MRK`, and `RCN` are kept.

## Deduplicate properties and textures

**Goal:** one `NiMaterialProperty` instead of six identical ones, and one `NiSourceTexture` per texture file. Exporters write a separate material per shape (`Material #1`, `Material #2`, ...) even when they're identical.

```luau
-- A record's value as a string, for spotting duplicates. `name` is left out: OpenMW never looks
-- properties or textures up by name.
local function fingerprint(value: unknown): string
  if type(value) ~= 'table' then return tostring(value) end
  local parts: { string } = {}
  for key, item in value :: { [unknown]: unknown } do
    if key ~= 'name' then table.insert(parts, tostring(key) .. '=' .. fingerprint(item)) end
  end
  table.sort(parts)
  return '{' .. table.concat(parts, ',') .. '}'
end

-- The first record with each fingerprint, by index of every duplicate.
local function duplicates(nif: Nif, recordType: string): { [number]: NifRecord }
  local first: { [string]: NifRecord } = {}
  local replace: { [number]: NifRecord } = {}
  for _, record in nif:find { type = recordType } do
    local net = record :: NiObjectNETRecord
    -- Animated or tagged records aren't interchangeable.
    if net.controller or net.extraData then continue end

    local key = fingerprint(record:decode())
    local keep = first[key]
    if keep then
      replace[jess.meta(record).index] = keep
    else
      first[key] = record
    end
  end
  return replace
end

local MapSlots = { 'base', 'dark', 'detail', 'gloss', 'glow', 'bump' }

local function dedupe(nif: Nif): number
  local merged = 0
  -- Texture sources first, so texturing properties that differ only by them compare equal.
  local sources = duplicates(nif, 'NiSourceTexture')
  for _, record in nif:find { type = 'NiTexturingProperty' } do
    local texturing = record :: NiTexturingPropertyRecord
    local maps = texturing.textureMaps
    local changed = false
    local function repoint(map: NifMap?)
      local keep = map and map.texture and sources[map.texture]
      if map and keep then
        map.texture = jess.meta(keep).index
        changed = true
      end
    end

    for _, slot in MapSlots do
      repoint((maps :: { [string]: NifMap? })[slot])
    end

    for _, decal in maps.decals or {} do
      repoint(decal)
    end

    if changed then
      texturing.textureMaps = maps -- a copy: write it back
    end
  end

  for _ in sources do
    merged += 1
  end

  local kinds = {
    'NiTexturingProperty',
    'NiMaterialProperty',
    'NiAlphaProperty',
    'NiZBufferProperty',
    'NiVertexColorProperty',
  }

  for _, kind in kinds do
    local replace = duplicates(nif, kind)
    for record in nif:records() do
      local properties = (record :: NiAVObjectRecord).properties
      if properties then
        for position = 1, #properties do
          local property = properties[position]
          local keep = property and replace[jess.meta(property).index]
          if keep then properties[position] = keep end
        end
      end
    end

    for _ in replace do
      merged += 1
    end
  end
  return merged
end
```

It works because links are just pointers: repoint every property list at the survivor, and the duplicates stop being reachable and aren't saved. Records with a controller or extra data are left alone, since an `NiAlphaController` on one material shouldn't start animating another shape. Texture sources other records point at (`NiTextureEffect`, `NiFlipController`) keep their own copy, because those links aren't repointed.

**Vanilla:** 18,398 duplicate records merged. `furn_com_bed_01.nif` goes from 6 materials to 1, and from 34 records to 29.

## Collapse redundant NiNodes

**Goal:** remove grouping nodes that do nothing, by splicing their children into their parent. A node qualifies when it is:

- exactly an `NiNode` (not a subtype), and not a root
- at an identity transform (within 1e-5, like the check's own test)
- not hidden, with no properties, effects, controller, extra data, or bounding volume
- not named with one of OpenMW's reserved names

Animated and skinned models are skipped outright. Their node names are what animations and skins bind to.

```luau
-- Name prefixes OpenMW's own optimizer never merges away (scenemanager.cpp), plus "Tri " forms.
local Reserved = {
  'Head',
  'Neck',
  'Chest',
  'Groin',
  'Right Hand',
  'Left Hand',
  'Right Wrist',
  'Left Wrist',
  'Shield Bone',
  'Right Forearm',
  'Left Forearm',
  'Right Upper Arm',
  'Left Upper Arm',
  'Right Foot',
  'Left Foot',
  'Right Ankle',
  'Left Ankle',
  'Right Knee',
  'Left Knee',
  'Right Upper Leg',
  'Left Upper Leg',
  'Right Clavicle',
  'Left Clavicle',
  'Weapon Bone',
  'Tail',
  'Bip01',
  'Root Bone',
  'BoneOffset',
  'AttachLight',
  'Arrow',
  'Camera',
  'Collision',
  'Right_Wrist',
  'Left_Wrist',
  'Shield_Bone',
  'Right_Forearm',
  'Left_Forearm',
  'Right_Upper_Arm',
  'Left_Clavicle',
  'Weapon_Bone',
  'Root_Bone',
  'Bounding Box',
}

local function reserved(name: string): boolean
  local lower = name:lower()
  for _, prefix in Reserved do
    local reservedPrefix = prefix:lower()
    if
      lower:sub(1, #reservedPrefix) == reservedPrefix
      or lower:sub(1, #reservedPrefix + 4) == 'tri ' .. reservedPrefix
    then
      return true
    end
  end
  return false
end

local function near(value: number, expected: number): boolean
  return math.abs(value - expected) <= 1e-5
end

local Identity = { 1, 0, 0, 0, 1, 0, 0, 0, 1 }

-- A plain NiNode that does nothing but group its children.
local function redundant(node: NiNodeRecord): boolean
  if
    jess.meta(node).typeName ~= 'NiNode'
    or reserved(node.name)
    or bit32.band(node.flags, 1) ~= 0
  then
    return false
  end

  if
    node.controller
    or node.extraData
    or node.boundingVolume
    or #node.properties > 0
    or #node.effects > 0
  then
    return false
  end

  if not near(node.scale, 1) then return false end

  for axis = 1, 3 do
    if not near(node.translation[axis], 0) then return false end
  end

  for index = 1, 9 do
    if not near(node.rotation[index], Identity[index]) then return false end
  end
  return true
end

-- Animated and skinned models name their nodes for a reason; leave them alone.
local function animated(nif: Nif, path: string): boolean
  local file = (path:match '[^/\\]+$' or path):lower()
  if file:sub(1, 1) == 'x' then return true end
  return #nif:find { type = { 'NiSkinInstance', 'NiKeyframeController', 'NiTextKeyExtraData' } } > 0
end

local function collapseNodes(nif: Nif, path: string): number
  if animated(nif, path) then return 0 end
  local collapsed = 0
  local roots: { [number]: boolean } = {}
  for _, root in nif:roots() do
    roots[jess.meta(root).index] = true
  end

  for _, record in nif:find { type = { 'NiNode', 'RootCollisionNode' } } do
    local parent = record :: NiNodeRecord
    local spliced: { number } = {}
    local changed = false
    for _, index in parent.children:toTable() do
      local node = if type(index) == 'number' then nif:record(index) :: NiNodeRecord? else nil
      if node and not roots[jess.meta(node).index] and redundant(node) then
        for _, grandchild in node.children:toTable() do
          if type(grandchild) == 'number' then table.insert(spliced, grandchild) end
        end
        collapsed += 1
        changed = true
      elseif type(index) == 'number' then
        table.insert(spliced, index)
      end
    end

    if changed then parent:update { children = spliced } end
  end
  return collapsed
end
```

Children keep their order, which matters: OpenMW looks for the `RootCollisionNode` among the root's children in reverse. Empty child links are dropped along the way. Nested redundant nodes need more than one pass, so call it until it returns 0:

```luau
repeat
  local collapsed = collapseNodes(nif, path)
  changes += collapsed
until collapsed == 0
```

**Vanilla:** 384 nodes collapsed. That's modest, because Bethesda's exporter didn't leave many. Mod meshes that went through three tools and an export script from 2004 usually have more.

## Retarget textures

**Goal:** make every texture path in the file name the file OpenMW actually loads. Morrowind's meshes say `Tx_wood_cherry.tga`. OpenMW's `correctTexturePath` quietly loads `textures/tx_wood_cherry.dds` instead, and so does every modern texture pack. Writing that path into the file makes the mesh say what it means, for every other tool and the original engine. It also strips exporter junk like `C:\...\Textures\` prefixes.

```luau
-- Retarget textures to the file OpenMW actually loads.
-- `exists` answers whether a VFS path (lowercase, forward slashes) exists.
local function retargetTextures(nif: Nif, exists: (path: string) -> boolean): number
  local changed = 0
  local done: { [string]: boolean } = {}
  for _, texture in nif:textures() do
    local original = texture.path
    if done[original] then continue end

    done[original] = true
    -- What correctTexturePath does: forward slashes, lowercase, under textures/.
    local path = original:lower():gsub('\\', '/')
    path = path:match '^.-textures/(.*)$' or path:gsub('^/+', '')
    local dds = path:gsub('%.%w+$', '') .. '.dds'
    if exists('textures/' .. dds) and dds ~= original:lower() then
      changed += nif:setTexture(original, (dds:gsub('/', '\\')))
    end
  end
  return changed
end
```

Only paths whose `.dds` actually exists are touched. Pointing a mesh at a DDS that isn't there would turn a working fallback into `missing_texture`. Build `exists` from what you're optimizing against:

```luau
-- Over the load order:
local vfs = jess.openmw.load(config):vfs()
local function exists(path: string): boolean return vfs:contains(path) end

-- Over one archive:
local files: { [string]: boolean } = {}
for _, entry in require('@dreamwind/archive').openPath('Morrowind.bsa'):entries() do
  if entry.path then files[(entry.path:lower():gsub('\\', '/'))] = true end
end

local function existsInArchive(path: string): boolean return files[path] == true end
```

**Vanilla, against the BSA's own textures:** 18,583 texture records in 6,046 meshes retargeted.

To swap one texture everywhere (a texture replacer that renames files, say), use `nif:setTexture(from, to)` directly. It ignores case and slash direction.

## Recompute normals and bounds

**Bounds:** `nif:updateBounds()` recomputes every geometry record's bounding sphere from its vertices. You only need it after writing vertices through a live view (`data.vertices[i] = ...`), since `update`, `set`, and `edit` already recompute it. It returns how many it updated.

**Normals:** jess computes normals for new geometry, and again when the vertices or triangles change. There's no call to recompute them for geometry that didn't change, so here's the same computation in Luau, with one addition. `weld` makes vertices at the same position share a normal, so UV seams on a smooth surface stop showing up as hard edges:

```luau
-- Area-weighted smooth normals, like the ones jess computes for new geometry. With `weld`,
-- vertices at the same position share a normal, so UV seams don't show as hard edges.
local function recomputeNormals(data: NiTriShapeDataRecord, weld: boolean)
  local vertices = data.vertices:toTable()
  local keys: { string } = {}
  for index, vertex in vertices do
    keys[index] = if weld
      then string.format('%.4f %.4f %.4f', vertex[1], vertex[2], vertex[3])
      else tostring(index)
  end
  local byKey: { [string]: { number } } = {}
  for _, triangle in data.triangles:toTable() do
    local first, second, third =
      vertices[triangle[1] + 1], vertices[triangle[2] + 1], vertices[triangle[3] + 1]
    local ux, uy, uz = second[1] - first[1], second[2] - first[2], second[3] - first[3]
    local vx, vy, vz = third[1] - first[1], third[2] - first[2], third[3] - first[3]
    -- The cross product's length is twice the triangle's area, which does the weighting.
    local nx, ny, nz = uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx
    for _, corner in triangle do
      local key = keys[corner + 1]
      local sum = byKey[key] or { 0, 0, 0 }
      sum[1] += nx
      sum[2] += ny
      sum[3] += nz
      byKey[key] = sum
    end
  end
  local normals: { { number } } = {}
  for index in vertices do
    local sum = byKey[keys[index]] or { 0, 0, 0 }
    local length = math.sqrt(sum[1] ^ 2 + sum[2] ^ 2 + sum[3] ^ 2)
    normals[index] = if length > 0
      then { sum[1] / length, sum[2] / length, sum[3] / length }
      else { 0, 0, 0 }
  end
  data:update { normals = normals }
end
```

Don't run it over everything. Hard edges are usually authored: a crate's corners have split vertices so they stay sharp, and welding rounds them off. Use it on meshes you know have broken normals, on shapes named for it (`nif:find({ type = "NiTriShape", name = "rock" })`), or after you've moved vertices around yourself. Skinned shapes are fine: normals are in the shape's own space.

## Triangle strips to lists and back

**Goal:** turn `NiTriStrips` into `NiTriShape`. OpenMW draws each strip as its own `DrawElements` primitive set (`handleNiGeometryData`), so a mesh stripified into hundreds of short strips costs hundreds of primitive sets. A triangle list is one. The GPUs strips were built for are long gone.

`record:set` with a different `type` changes a record's type in place, keeping its handle and every link to it. So the conversion rewrites the two records and touches nothing else:

```luau
-- NiTriStrips to NiTriShape, keeping the records (and every link to them) in place.
local function stripsToLists(nif: Nif): number
  local converted = 0
  for _, record in nif:find { type = 'NiTriStripsData' } do
    local strips = record :: NiTriStripsDataRecord
    local points = strips.strips:toTable()
    local triangles: { { number } } = {}
    local start = 1
    for _, length in strips.stripLengths:toTable() do
      for offset = 0, length - 3 do
        local first, second, third =
          points[start + offset], points[start + offset + 1], points[start + offset + 2]
        if first ~= second and second ~= third and first ~= third then
          -- Every other triangle in a strip is wound the other way.
          table.insert(
            triangles,
            if offset % 2 == 0 then { first, second, third } else { first, third, second }
          )
        end
      end
      start += length
    end
    local value = strips:toTable()
    record:set {
      type = 'NiTriShapeData',
      vertices = value.vertices,
      normals = value.normals,
      bound = value.bound,
      vertexColors = value.vertexColors,
      uvSets = value.uvSets,
      triangles = triangles,
    }
    converted += 1
  end

  for _, record in nif:find { type = 'NiTriStrips' } do
    -- Same fields, other type: NiTriShape and NiTriStrips add nothing to NiTriBasedGeom.
    record:update { type = 'NiTriShape' }
  end
  return converted
end
```

The triangles unroll in the same order and with the same winding as jess's (and OpenMW's) own reading of the strips, dropping degenerate ones. jess sees the same triangles as before and **keeps the authored normals and bound**. It only recomputes them when the geometry really changes.

Vanilla Morrowind has no strips at all, so this is for mod meshes.

**Going the other way**, for the original engine or a tool that wants strips, needs a stripifier. This greedy one grows a strip from each unused triangle, in each of its three rotations, while an unused neighbor shares the strip's last edge with the winding the strip needs, and keeps the longest:

```luau
local function stripify(triangles: { { number } }): { { number } }
  local byEdge: { [string]: { number } } = {}
  for index, triangle in triangles do
    for corner = 1, 3 do
      local key = triangle[corner] .. ':' .. triangle[corner % 3 + 1]
      byEdge[key] = byEdge[key] or {}
      table.insert(byEdge[key], index)
    end
  end
  local used: { [number]: boolean } = {}
  local function grow(first: number, second: number, third: number): ({ number }, { number })
    local strip, taken = { first, second, third }, {}
    local claimed: { [number]: boolean } = {}
    while true do
      local length = #strip
      local previous, last = strip[length - 1], strip[length]
      -- The next triangle holds edge last->previous after an odd-length strip, previous->last after
      -- an even one.
      local key = if length % 2 == 1 then last .. ':' .. previous else previous .. ':' .. last
      local found: number? = nil
      for _, candidate in byEdge[key] or {} do
        if not used[candidate] and not claimed[candidate] then
          found = candidate
          break
        end
      end

      if not found then return strip, taken end
      claimed[found] = true
      table.insert(taken, found)
      for _, corner in triangles[found] do
        if corner ~= previous and corner ~= last then table.insert(strip, corner) end
      end
    end
  end
  local strips: { { number } } = {}
  for index, triangle in triangles do
    if used[index] then continue end

    used[index] = true
    local best, bestTaken = grow(triangle[1], triangle[2], triangle[3])
    for _, rotation in
      { { triangle[2], triangle[3], triangle[1] }, { triangle[3], triangle[1], triangle[2] } }
    do
      local strip, taken = grow(rotation[1], rotation[2], rotation[3])
      if #strip > #best then
        best, bestTaken = strip, taken
      end
    end

    for _, taken in bestTaken do
      used[taken] = true
    end
    table.insert(strips, best)
  end
  return strips
end

local function listsToStrips(nif: Nif): number
  local converted = 0
  for _, record in nif:find { type = 'NiTriShapeData' } do
    local data = record :: NiTriShapeDataRecord
    local lengths: { number }, points: { number } = {}, {}
    for _, strip in stripify(data.triangles:toTable()) do
      table.insert(lengths, #strip)
      for _, point in strip do
        table.insert(points, point)
      end
    end
    local value = data:toTable()
    record:set {
      type = 'NiTriStripsData',
      vertices = value.vertices,
      normals = value.normals,
      bound = value.bound,
      vertexColors = value.vertexColors,
      uvSets = value.uvSets,
      stripLengths = lengths,
      strips = points,
    }
    converted += 1
  end

  for _, record in nif:find { type = 'NiTriShape' } do
    record:update { type = 'NiTriStrips' }
  end
  return converted
end
```

On jess's `random_objects.nif` test mesh, lists to strips turns 3,246 triangles into 1,778 strips. That's 1,778 primitive sets where there was one, which is the whole case against strips. The file shrinks from 232,548 to 230,219 bytes. Going back with `stripsToLists` restores the same 4,214 drawn triangles and the exact original size.

## Cheap collision for boxy statics

**Goal:** stop OpenMW building collision from every rendered triangle of a prop that's basically a box. With no `RootCollisionNode`, the render geometry *is* the collision mesh (`handleRoot` in `bulletnifloader.cpp`). `jess nif check` calls that `render_geometry_collision` past 5,000 triangles, but a 500-triangle barrel is still 500 triangles of physics for a cylinder you bump into.

This adds a `RootCollisionNode` with a 12-triangle box around everything the model draws:

```luau
type Point = { number }

-- `point` moved from `object`'s space into its parent's: scaled, rotated, then translated.
local function toParent(object: NiAVObjectRecord, point: Point): Point
  local rotation, scale, translation = object.rotation, object.scale, object.translation
  local out: Point = {}
  for row = 0, 2 do
    out[row + 1] = translation[row + 1]
      + scale
        * (rotation[row * 3 + 1] * point[1] + rotation[row * 3 + 2] * point[2] + rotation[row * 3 + 3] * point[3])
  end
  return out
end

-- The box around every shape under `root`, in the root's space.
local function bounds(root: NiNodeRecord): (Point, Point)
  local low: Point = { math.huge, math.huge, math.huge }
  local high: Point = { -math.huge, -math.huge, -math.huge }
  local function visit(record: NifRecord, toRoot: (Point) -> Point)
    local object = record :: NiAVObjectRecord
    local place = function(point: Point): Point return toRoot(toParent(object, point)) end
    local shape = record :: NiTriShapeRecord
    if shape.geometryData then
      for _, vertex in (shape.geometryData :: NiGeometryDataRecord).vertices:toTable() do
        local point = place(vertex)
        for axis = 1, 3 do
          low[axis] = math.min(low[axis], point[axis])
          high[axis] = math.max(high[axis], point[axis])
        end
      end
    end
    local node = record :: NiNodeRecord
    if node.children then
      for _, child in node.children do
        if child then visit(child, place) end
      end
    end
  end

  -- OpenMW ignores the root's own transform, so start below it.
  for _, child in root.children do
    if child then visit(child, function(point: Point): Point return point end) end
  end
  return low, high
end

-- Replaces render-geometry collision with a box: 12 triangles instead of every rendered one.
local function boxCollision(nif: Nif): boolean
  local root = nif:roots()[1] :: NiNodeRecord
  if #nif:find { type = 'RootCollisionNode' } > 0 then return false end
  local low, high = bounds(root)
  local corners: { Point } = {}
  for corner = 0, 7 do
    table.insert(corners, {
      if bit32.band(corner, 1) == 0 then low[1] else high[1],
      if bit32.band(corner, 2) == 0 then low[2] else high[2],
      if bit32.band(corner, 4) == 0 then low[3] else high[3],
    })
  end

  local box = nif:shape {
    name = 'Collision Box',
    vertices = corners,
    -- Two outward-facing triangles per side.
    triangles = {
      { 0, 2, 1 },
      { 1, 2, 3 },
      { 4, 5, 6 },
      { 5, 7, 6 }, -- bottom, top
      { 0, 1, 4 },
      { 1, 5, 4 },
      { 2, 6, 3 },
      { 3, 6, 7 }, -- front, back
      { 0, 4, 2 },
      { 2, 4, 6 },
      { 1, 3, 5 },
      { 3, 7, 5 }, -- left, right
    },
  }
  root:attach(nif:insert { type = 'RootCollisionNode', children = { box } })
  return true
end
```

Transforms compose as OpenMW's `NiTransform::toMatrix` does: scale, then the row-major rotation, then translation, from each shape up to the root.

**Vanilla `meshes/m/beerbarrel.nif`:** `jess nif info` goes from `collision.source: render_geometry` with 511 collision triangles to `root_collision_node` with 12. The rendered triangles are unchanged and `nif check` finds nothing.

**Don't run this blindly.** A box is wrong for anything you walk into, onto, or through: buildings, bridges, caves, archways, stairs. A doorway with box collision is a wall. Pick the files by name or by list. It also doesn't matter for creatures and NPCs, which OpenMW collides as a box anyway.

## Apply `nif fix`

Everything `jess nif fix` repairs is available on a handle:

```luau
local function fix(nif: Nif): number
  return #nif:fix() -- or nif:fix({ "rcn", "ncc", "markers", "skinWeights", "unreachable" })
end
```

Each fix keeps what the model looks like in Morrowind and makes OpenMW agree: `RCN` string data for a nested `RootCollisionNode`, `ncc` uppercased to `NCC`, `MRK` for visible `Tri EditorMarker` shapes, skin weights for vertices that don't exist dropped (that one's `wontLoad` otherwise), and unreachable records dropped. [Check Rules](@/docs/nif/checking-rules.md) lists which finding each fix answers.

For a fix-only pass you don't need the harness at all. `jess nif fix <folder> --output <override>` (or `jess.nif.fix(folder, { output = override })`) does it in one line, and `--dry-run` shows what it would do first.

## Putting it together

Paste the recipes you want above `optimize` and call them from it:

```luau
local function optimize(nif: Nif, path: string): number
  local changes = stripExtraData(nif) + dedupe(nif)
  repeat
    local collapsed = collapseNodes(nif, path)
    changes += collapsed
  until collapsed == 0
  changes += stripsToLists(nif)
  return changes + fix(nif)
end
```

Run over `meshes/f/` in vanilla `Morrowind.bsa`:

```sh
jess run --package jpk/dreamwind optimize.jsl "Data Files/Morrowind.bsa" out meshes/f/
```

```text
1096 NIFs, 425 written: 11909051 -> 11715736 bytes, 17356 -> 14097 records
```

Over the whole BSA, the same `optimize` changes 2,146 of the 7,343 files. Across the archive that's 222,935 records down to 201,964 (-9.4%), and 297,493,465 bytes to 295,782,017 (-0.6%). The bytes barely move because geometry is most of every file, and none of this touches geometry. Records are what the reader allocates and walks, and that's where the savings are. Shape and triangle counts are identical before and after, as they should be.

## Verify the output

Two checks. First, the override folder on its own, with textures resolved the way OpenMW will:

```sh
jess nif check out --data "Data Files/Morrowind.bsa" -v
```

Second, each output against its original: no finding code the original didn't have, the same drawn and collision triangles, and a byte-identical re-save:

```luau
--!strict
-- jess run --package jpk/dreamwind verify.jsl <archive with the originals> <output folder>
local archivePath: string, outDir: string = ...
local archive = require('@dreamwind/archive').openPath(archivePath)
local checked, bad = 0, 0

local function counts(file: NifFile): { [string]: number }
  local found: { [string]: number } = {}
  for _, finding in file.findings do
    found[finding.code] = (found[finding.code] or 0) + 1
  end
  return found
end

local function compare(path: string, original: string, optimized: string)
  local was = jess.nif.checkBytes(original, path, { openmw = false })
  local now = jess.nif.checkBytes(optimized, path, { openmw = false })
  local problems: { string } = {}
  local before, after = counts(was), counts(now)
  for code, count in after do
    if count > (before[code] or 0) then table.insert(problems, 'new ' .. code) end
  end
  local wasStats, nowStats = was.stats :: NifStats, now.stats :: NifStats
  if
    wasStats.triangles ~= nowStats.triangles
    or wasStats.shapes ~= nowStats.shapes
    or wasStats.collisionTriangles ~= nowStats.collisionTriangles
  then
    table.insert(problems, 'geometry changed')
  end

  if buffer.tostring(jess.nif.openBytes(optimized):bytes()) ~= optimized then
    table.insert(problems, 'doesn\'t round-trip')
  end
  checked += 1
  if #problems > 0 then
    bad += 1
    print(path, table.concat(problems, ', '))
  end
end

local function walk(folder: string, relative: string)
  for _, name in jess.fs.list(folder) do
    local full = jess.fs.join(folder, name)
    if jess.fs.isDir(full) then
      walk(full, relative .. name .. '/')
    else
      local path = relative .. name
      compare(path, archive:readFileRequired(path), assert(jess.fs.read(full)))
    end
  end
end
walk(outDir, '')
print(checked, 'checked', bad, 'with problems')
```

```text
425 checked	0 with problems
```

When the originals come from the load order instead, read them with `jess.openmw.load(config):readFile(path)` (a buffer, so `buffer.tostring` it). The [box collision](#cheap-collision-for-boxy-statics) recipe changes collision triangles on purpose, so leave that comparison out when you use it.
