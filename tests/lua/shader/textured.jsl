--!strict
-- A texture sampled with the sampler declared alongside it, at interpolated UVs.
local shader = require '@jess/shader'

return shader.program {
  textures = {
    diffuse = shader.texture2d,
  },

  vertex = function(input)
    return {
      position = input.projection * input.view * input.model * shader.vec4(input.position, 1.0),
      uv = input.uv,
    }
  end,

  fragment = function(input) return shader.sample(input.diffuse, input.uv) end,
}
