--!strict
--!native
-- Native vectors packed into and out of a buffer: vector arithmetic plus buffer crossings without
-- attaching the workload to any particular game-format representation.
local function run(scale: number): number
  local count = math.max(1, math.floor(2000000 * scale))
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
  return os.clock() - started
end

return table.freeze {
  name = 'vector',
  path = 'tools/bench/canaries/vector.luau',
  run = run,
}
