--!strict
--[[
Writes the canaries' synthetic plugins, so they run without any game data:

  jess run tools/bench/canaries/setup.luau <folder> <scale>

`lights-1.esp` to `lights-8.esp` each hold 1,000 × scale lights and as many misc items, with the
same ids in every plugin, like a load order overriding itself. `lands.esp` holds 32 × scale LAND
records with every part.
]]
local folder, scaleText = ...
local scale = tonumber(scaleText) or error 'usage: setup.luau <folder> <scale>'
local Plugins, Records, Lands = 8, 1000 * scale, 32 * scale

jess.fs.createDir(folder)

for pluginIndex = 1, Plugins do
  local plugin = jess.plugin.new()
  for index = 1, Records do
    plugin:insert {
      type = 'Light',
      id = `light_{index}`,
      name = `Light {index}`,
      mesh = 'l/light_com_candle_01.nif',
      data = {
        flags = if index % 2 == 0 then 'CAN_CARRY | FIRE' else 'DYNAMIC',
        radius = (index + pluginIndex) % 512,
        time = index,
        weight = 1,
        value = index % 100,
        color = { index % 256, 128, pluginIndex, 0 },
      },
    }

    plugin:insert { type = 'MiscItem', id = `misc_{index}`, name = `Misc {index}` }
  end

  plugin:save(jess.fs.join(folder, `lights-{pluginIndex}.esp`), { backup = false })
end

local lands = jess.plugin.new()
local vertices = 65 * 65
for index = 0, Lands - 1 do
  local land = lands:insert {
    type = 'Landscape',
    grid = { index % 64, index // 64 },
    landscapeFlags = 'USES_VERTEX_HEIGHTS_AND_NORMALS | USES_VERTEX_COLORS | USES_TEXTURES',
  }
  local heights, normals, colors, textures =
    buffer.create(vertices * 4),
    buffer.create(vertices * 3),
    buffer.create(vertices * 3),
    buffer.create(16 * 16 * 2)

  for vertex = 0, vertices - 1 do
    buffer.writei32(heights, vertex * 4, vertex % 65 + (vertex // 65) * 2)
    buffer.writei8(normals, vertex * 3 + 2, 127)
    buffer.writeu8(colors, vertex * 3, (vertex + index) % 256)
  end

  for texture = 0, 16 * 16 - 1 do
    buffer.writeu16(textures, texture * 2, texture % 4)
  end

  jess.land.write(land, {
    heightOffset = index,
    heights = heights,
    normals = normals,
    colors = colors,
    textures = textures,
  })
end

lands:save(jess.fs.join(folder, 'lands.esp'), { backup = false })
