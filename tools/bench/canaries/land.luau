--!strict
--!native
-- `jess.land.read` and `write` around a walk over every vertex, Merged Lands' inner loop.
local folder = ...
local Rounds = 20
local plugin =
  jess.plugin.open(jess.fs.join(folder, 'lands.esp'), { types = { 'LAND' }, openmw = false })

local started, total = os.clock(), 0
for _ = 1, Rounds do
  for record in plugin:select { type = 'LAND' } do
    local terrain = jess.land.read(record)
    local heights, colors = terrain.heights :: buffer, terrain.colors :: buffer
    for vertex = 0, 65 * 65 - 1 do
      local height = buffer.readi32(heights, vertex * 4)
      total += math.abs(height) + buffer.readu8(colors, vertex * 3)
      buffer.writeu8(colors, vertex * 3 + 1, (height + vertex) % 256)
    end

    jess.land.write(record, { colors = colors })
  end
end

assert(total > 0)
print(`canary {os.clock() - started}`)
