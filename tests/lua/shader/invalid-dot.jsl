--!strict
-- A dot product of a Vec3f and a Vec4f: it fails at line 15, where it's written.
local shader = require '@jess/shader'

return shader.program {
  parameters = {
    tint = shader.vec4,
  },

  vertex = function(input)
    return { position = shader.vec4(input.position, 1.0), normal = input.normal }
  end,

  fragment = function(input)
    local facing = shader.dot(shader.normalize(input.normal), input.tint)

    return shader.vec4(facing)
  end,
}
