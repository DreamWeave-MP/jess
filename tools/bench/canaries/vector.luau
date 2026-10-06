--!strict
--!native
-- Native vectors packed into and out of a buffer: the normals math of a terrain merge.
local _, scaleText = ...
local count = 2000000 * (tonumber(scaleText) or 1)
local normals = buffer.create(65 * 65 * 3)

local started, total = os.clock(), vector.zero
for index = 0, count - 1 do
  local vertex = index % 4225
  local step = vector.create(1, 0, (index % 13) - 6)
  local across = vector.create(0, 1, (index % 7) - 3)
  local normal = vector.normalize(vector.cross(step, across)) * 127
  buffer.writei8(normals, vertex * 3, math.floor(normal.x))
  buffer.writei8(normals, vertex * 3 + 1, math.floor(normal.y))
  buffer.writei8(normals, vertex * 3 + 2, math.floor(normal.z))
  total += vector.create(
    buffer.readi8(normals, vertex * 3),
    buffer.readi8(normals, vertex * 3 + 1),
    buffer.readi8(normals, vertex * 3 + 2)
  )
end

assert(vector.magnitude(total) > 0)
print(`canary {os.clock() - started}`)
