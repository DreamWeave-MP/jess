--!strict
-- A textured surface lit by one directional light: normals through a matrix parameter,
-- normalize, dot, max, swizzles, clamp, and a vector built back from parts.
local shader = require '@jess/shader'

return shader.program {
  parameters = {
    normalMatrix = shader.mat3,
    lightDirection = shader.vec3,
    ambient = shader.float,
  },

  textures = {
    diffuse = shader.texture2d,
  },

  vertex = function(input)
    return {
      position = input.projection * input.view * input.model * shader.vec4(input.position, 1.0),
      normal = input.normalMatrix * input.normal,
      uv = input.uv,
    }
  end,

  fragment = function(input)
    local diffuse = shader.sample(input.diffuse, input.uv)
    local facing = shader.dot(shader.normalize(input.normal), input.lightDirection)
    local light = shader.max(facing, 0.0) + input.ambient
    local lit = shader.clamp(diffuse.xyz * light, 0.0, 1.0)

    return shader.vec4(lit, diffuse.w)
  end,
}
