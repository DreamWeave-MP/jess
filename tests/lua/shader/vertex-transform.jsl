--!strict
-- The handoff's success test: the model, view, and projection semantics, a vertex color passed
-- to the fragment stage, and a tint.
local shader = require '@jess/shader'

return shader.program {
  parameters = {
    tint = shader.vec4,
  },

  vertex = function(input)
    return {
      position = input.projection * input.view * input.model * shader.vec4(input.position, 1.0),
      color = input.color,
    }
  end,

  fragment = function(input) return input.color * input.tint end,
}
